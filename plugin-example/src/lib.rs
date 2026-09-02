//! A dummy plugin that exists only to exercise the ABI boundary; not
//! included in release zips (see docs/architecture/plugins.md). Returns a single
//! fixed `OpenUri` result, or a `CopyToClipboard` result, depending on the
//! query (path conversion isn't part of this check — `RAction::Launch`'s
//! `PathBuf` conversion is verified separately on the `issen` side, in
//! `PluginProvider`). A query containing `panic` panics on purpose, to
//! verify that the `catch_unwind` boundary around
//! `RSearchProvider::search` (in this file) keeps that panic from
//! propagating into the host (see the tests in `src/search/plugin.rs`).

use abi_stable::{
    export_root_module,
    prefix_type::PrefixTypeTrait,
    sabi_trait::prelude::TD_Opaque,
    std_types::{RStr, RString, RVec},
};
use issen_plugin_api::{
    BoxedRGuiToolProvider, BoxedRSearchProvider, PluginModule, PluginModuleRef, RAction,
    RGuiToolMode, RGuiToolProvider, RGuiToolProvider_TO, RSearchProvider, RSearchProvider_TO,
    RSearchResult,
};

struct ExampleProvider;

impl RSearchProvider for ExampleProvider {
    fn search(&self, query: RStr<'_>) -> RVec<RSearchResult> {
        // A panic inside a plugin crosses the FFI boundary and takes the
        // host down with it, so it must always be caught here (see the
        // issen-plugin-api docs).
        let query = query.to_string();
        std::panic::catch_unwind(|| search_impl(&query)).unwrap_or_default()
    }
}

fn search_impl(query: &str) -> RVec<RSearchResult> {
    let query_lower = query.to_lowercase();
    if query_lower.contains("panic") {
        panic!("issen-plugin-example: deliberate panic, for exercising the catch_unwind boundary");
    }
    if query_lower.contains("clipboard") {
        return RVec::from(vec![RSearchResult {
            title: RString::from("42"),
            subtitle: RString::from("issen-plugin-example (copy to clipboard)"),
            action: RAction::CopyToClipboard(RString::from("42")),
            score: 100,
        }]);
    }
    if !query.is_empty() && !"example plugin".contains(&query_lower) {
        return RVec::new();
    }
    RVec::from(vec![RSearchResult {
        title: RString::from("Example Plugin Result"),
        subtitle: RString::from("issen-plugin-example"),
        action: RAction::OpenUri(RString::from("https://example.com/issen-plugin-example")),
        score: 100,
    }])
}

extern "C" fn new_provider() -> BoxedRSearchProvider {
    RSearchProvider_TO::from_value(ExampleProvider, TD_Opaque)
}

/// A dummy for exercising the `RGuiToolMode::Standalone` side of the ABI
/// boundary; doesn't actually open a window (see
/// `prototypes/gui-plugin/thread-eframe` / `raw-window` for a real
/// implementation).
struct StandaloneExampleTool;

impl RGuiToolProvider for StandaloneExampleTool {
    fn name(&self) -> RString {
        RString::from("Example Standalone Tool")
    }
    fn mode(&self) -> RGuiToolMode {
        RGuiToolMode::Standalone
    }
    fn launch_standalone(&self) {}
    fn create_embedded(&self, _parent_hwnd: usize) -> usize {
        0
    }
}

/// A dummy for exercising the `RGuiToolMode::Embedded` side of the ABI
/// boundary; doesn't actually create a child HWND (see
/// `prototypes/gui-plugin/setparent` for a real implementation).
struct EmbeddedExampleTool;

impl RGuiToolProvider for EmbeddedExampleTool {
    fn name(&self) -> RString {
        RString::from("Example Embedded Tool")
    }
    fn mode(&self) -> RGuiToolMode {
        RGuiToolMode::Embedded
    }
    fn launch_standalone(&self) {}
    fn create_embedded(&self, _parent_hwnd: usize) -> usize {
        0
    }
}

extern "C" fn gui_tools() -> RVec<BoxedRGuiToolProvider> {
    RVec::from(vec![
        RGuiToolProvider_TO::from_value(StandaloneExampleTool, TD_Opaque),
        RGuiToolProvider_TO::from_value(EmbeddedExampleTool, TD_Opaque),
    ])
}

#[export_root_module]
pub fn get_root_module() -> PluginModuleRef {
    PluginModule {
        new_provider,
        gui_tools,
    }
    .leak_into_prefix()
}
