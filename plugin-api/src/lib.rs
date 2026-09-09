//! ABI boundary for Issen search-provider plugins (see
//! docs/architecture/plugins.md).
//!
//! Both the host (`issen` itself) and plugin DLLs depend on this crate.
//! Types crossing this boundary are limited to plain data (strings,
//! numbers, enums) — never `egui` types or host-internal state (`Ui`/
//! `Context`, etc.). Separately compiled Rust compilation units have no
//! guarantee that `repr(Rust)` type layouts match, so sharing a GUI
//! framework's runtime state across this boundary would be undefined
//! behavior.
//!
//! This crate's own `Cargo.toml` `version` doubles as the plugin API
//! version, checked for compatibility at load time via
//! `package_version_strings!()` (compared internally by
//! `abi_stable::library::LibHeader::init_root_module`).

// `#[sabi_trait]`-generated `impl`s trip the `non_local_definitions` lint
// (a known false positive with abi_stable 0.11.3 on recent rustc — the impl
// lives inside a macro-generated anonymous const, so `#[allow]` on the
// trait definition itself can't suppress it; it has to be crate-wide).
#![allow(non_local_definitions)]

use abi_stable::{
    declare_root_module_statics,
    library::RootModule,
    package_version_strings, sabi_trait,
    sabi_types::VersionStrings,
    std_types::{RBox, RStr, RString, RVec},
    StableAbi,
};

/// The plugin-facing counterpart to `issen`'s own `search::Action`.
/// `PathBuf` isn't `StableAbi`, so paths are represented as strings and
/// converted with `PathBuf::from` on the host side.
///
/// `CopyToClipboard` exists because plugins like a calculator, snippet
/// manager, clipboard history, or translator need to return computed or
/// converted text rather than launch something — the other three variants
/// can't express that (`Launch` would try to run the text as an
/// executable, and `OpenUri` means something different, a shell URI
/// launch). The host only copies to the clipboard; auto-sending the text
/// into whatever window is focused (e.g. via `SendInput`) is out of scope,
/// matching how the built-in tools (color picker etc.) also stop at
/// copying.
///
/// The displayed `title` and the copied text can differ (e.g. title
/// "2 + 2 = 4", copied text just "4"). Rather than adding a separate
/// "clipboard text" field to `RSearchResult`, the `RAction` payload itself
/// carries the text to copy — same expressiveness, no extra field, and a
/// plugin can set it independently of the title.
///
/// Adding an enum variant bumps the plugin API version
/// (`plugin-api/Cargo.toml`'s `version`). `abi_stable` verifies type
/// layout at load time, so a DLL built against a mismatched layout fails
/// to load cleanly (`load_one` turns it into a `Result::Err`) rather than
/// crashing the host — an old, unrebuilt plugin DLL is safe to leave in
/// place. No third-party plugins exist yet (none are distributed), so this
/// non-crashing behavior is really insurance for when that changes; treat
/// a variant addition like this as a breaking version bump regardless.
#[repr(C)]
#[derive(StableAbi, Clone, Debug)]
pub enum RAction {
    Launch { path: RString, args: RString },
    LaunchUwp { aumid: RString },
    OpenUri(RString),
    CopyToClipboard(RString),
}

/// The plugin-facing counterpart to `issen`'s own `search::SearchResult`.
///
/// `score` isn't trusted as-is on the host side — it gets clamped (by
/// `PluginProvider`) so a third-party plugin can't return something like
/// `i32::MAX` and permanently squat on the top result.
#[repr(C)]
#[derive(StableAbi, Clone, Debug)]
pub struct RSearchResult {
    pub title: RString,
    pub subtitle: RString,
    pub action: RAction,
    pub score: i32,
}

/// The search interface a plugin implements.
///
/// `search` is deliberately a synchronous call: the host calls every
/// provider's `search` on every keystroke (`src/app.rs`), so a slow plugin
/// directly costs UI responsiveness. Making this async/cancellable later
/// would need a breaking ABI change, so the contract is spelled out up
/// front instead: returning quickly is the plugin author's responsibility.
/// Search isn't farmed out to a separate process either, since IPC
/// round-trips would add latency on every keystroke.
///
/// A panic inside a plugin crosses the FFI boundary and takes the host
/// down with it, so implementations are responsible for catching their
/// own panics (e.g. `std::panic::catch_unwind`) — see `plugin-example`.
#[sabi_trait]
pub trait RSearchProvider {
    fn search(&self, query: RStr<'_>) -> RVec<RSearchResult>;
}

pub type BoxedRSearchProvider = RSearchProvider_TO<'static, RBox<()>>;

/// How a GUI tool plugin launches.
///
/// A plugin declares which mode it runs in, and the host only calls the
/// method matching that declaration (`launch_standalone` /
/// `create_embedded`). Calling the other one is a contract violation — the
/// plugin side is free to behave in an undefined way if that happens.
///
/// `prototypes/gui-plugin/` compares three approaches (a plugin's own
/// eframe thread, `SetParent` embedding, and an independent raw-Win32
/// top-level window). Embedding (`Embedded`) turned out to inherit the
/// host's undecorated window's move/resize limitations, so it wasn't used
/// for the built-in color picker / unit converter — but third-party
/// plugins may still want to integrate into an existing host screen, so
/// both modes stay available (see `prototypes/gui-plugin/README.md`).
#[repr(u8)]
#[derive(StableAbi, Clone, Copy, Debug, PartialEq, Eq)]
pub enum RGuiToolMode {
    /// The plugin owns its own top-level window with its own message loop
    /// (see `prototypes/gui-plugin/thread-eframe` / `raw-window` for
    /// reference implementations).
    Standalone,
    /// The plugin provides a child window meant to be embedded into the
    /// host's window via `SetParent` etc. (see
    /// `prototypes/gui-plugin/setparent` for a reference implementation).
    Embedded,
}

/// The GUI tool interface a plugin implements.
///
/// HWNDs cross this boundary as plain `usize` values. That looks like it
/// breaks the "data types only" FFI principle above, but a HWND is an
/// OS-level stable integer, not Rust GUI-runtime internal state like
/// `egui::Context` — this is the only way to hand off a native window
/// across the Rust compilation-unit boundary without relying on shared
/// `repr(Rust)` layout.
///
/// A HWND whose window class was registered by the plugin dangles the
/// moment the DLL unloads, since `lpfnWndProc` points into code that just
/// vanished with it. Safe today because the host (`PluginProvider`,
/// `src/search/plugin.rs`) never unloads a plugin DLL before process exit
/// — but this is exactly the constraint that will need solving first if
/// dynamic plugin unloading is ever added.
#[sabi_trait]
pub trait RGuiToolProvider {
    /// The tool's display name.
    fn name(&self) -> RString;
    /// Declares the launch mode; the host dispatches based on this value.
    fn mode(&self) -> RGuiToolMode;
    /// Only called for plugins where `mode()` is `Standalone`. The plugin
    /// opens its own independent top-level window with its own message
    /// loop (an async, fire-and-forget call).
    fn launch_standalone(&self);
    /// Only called for plugins where `mode()` is `Embedded`. Passes the
    /// host's HWND (as `usize`) and receives back the child HWND (as
    /// `usize`) created for embedding; `0` means failure.
    fn create_embedded(&self, parent_hwnd: usize) -> usize;
}

pub type BoxedRGuiToolProvider = RGuiToolProvider_TO<'static, RBox<()>>;

/// The plugin DLL's export surface.
///
/// This is a `Prefix` type so a future "plugin settings screen" field
/// (e.g. `settings_schema`) can be *appended* later without breaking
/// existing plugins — matching the same pattern as the
/// `AliasEntry`/`WindowsShortcutEntry` settings-editing UI: the plugin
/// would return structured field data, and the actual GUI drawing code
/// stays on the host side (`egui::Context`), never crossing the ABI
/// boundary.
///
/// `gui_tools`, unlike `new_provider`, is `#[sabi(missing_field(option))]`
/// because most plugins are expected to have search results but no GUI
/// tools — making it required would force every plugin author to
/// implement a stub that returns an empty vec. A plugin DLL built before
/// this field existed just gets `PluginModuleRef::gui_tools()` returning
/// `None` at load time, which the host treats as zero GUI tools (see
/// `src/search/plugin.rs::PluginProvider::load_one`).
#[repr(C)]
#[derive(StableAbi)]
#[sabi(kind(Prefix(prefix_ref = PluginModuleRef)))]
#[sabi(missing_field(panic))]
pub struct PluginModule {
    pub new_provider: extern "C" fn() -> BoxedRSearchProvider,
    #[sabi(last_prefix_field)]
    #[sabi(missing_field(option))]
    pub gui_tools: extern "C" fn() -> RVec<BoxedRGuiToolProvider>,
}

impl RootModule for PluginModuleRef {
    declare_root_module_statics! {PluginModuleRef}
    const BASE_NAME: &'static str = "issen_plugin";
    const NAME: &'static str = "issen_plugin";
    const VERSION_STRINGS: VersionStrings = package_version_strings!();
}
