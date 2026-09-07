//! GPUI移行後のメインウィンドウ(検索ボックス+結果ドロップダウン)。
//!
//! egui/eframe版からの移行に伴う設計変更点(詳細はdocs/architecture配下の
//! GPUI移行計画とdocs/architecture/window-lifecycle.mdを参照):
//!   - GPUI本体にTextEdit相当の完成品ウィジェットが無いため、`IssenApp`
//!     自身が`EntityInputHandler`を実装して検索ボックスを自前実装している
//!     (`examples/gpui_spike_input.rs`のTextInputを土台にした移植 — IME
//!     対応の下線付きmarked_range描画を含む)。
//!   - `window.request_animation_frame()`による毎フレームポーリングは
//!     使わず、ホットキー/トレイイベントは`futures::channel::mpsc`と
//!     `App::spawn`によるイベント駆動の待受けにしている
//!     (`hotkey.rs`/`tray.rs`のAPIもこの前提でegui非依存に書き換え済み)。
//!   - about/settings/toolsウィンドウ(`about_window.rs`/`settings_window.rs`/
//!     `tools/mod.rs`)を移植済み。設定・toolsのテキスト入力欄は
//!     `text_input.rs`の汎用`TextInput`エンティティを再利用している。
//!   - 右クリックコンテキストメニュー(GPUI本体に`context_menu`相当の完成品
//!     ウィジェットが無いため、`anchored()`+`deferred()`による自作オーバーレイ)
//!     とクエリ履歴パネルも移植済み。起動時の合成フリッカー対策(layered
//!     prime)とshow/hideのフェードインアニメーションは、Phase 0スパイクで
//!     GPUI自体にはちらつきが再発しないことを実機確認済みのため保留
//!     (再発が実際に観測された場合のみ追加する)。

use std::ops::Range;
use std::time::{Duration, Instant};

use futures::StreamExt;
use gpui::{
    actions, anchored, deferred, div, fill, hsla, point, prelude::*, px, size, white, AnyElement,
    App, AppContext, Bounds, ClipboardItem, Context, CursorStyle, ElementId, ElementInputHandler,
    Entity, EntityInputHandler, FocusHandle, Focusable, GlobalElementId, KeyBinding, LayoutId,
    MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, PaintQuad, Pixels, Point,
    ShapedLine, SharedString, Style, TextRun, UTF16Selection, UnderlineStyle, Window,
    WindowBackgroundAppearance, WindowBounds, WindowControlArea, WindowHandle, WindowKind,
    WindowOptions,
};
use gpui_platform::application;
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use unicode_segmentation::UnicodeSegmentation;
use windows::Win32::Foundation::HWND;
use windows::Win32::UI::WindowsAndMessaging::{
    GetWindowLongPtrW, SetWindowLongPtrW, SetWindowPos, GWL_EXSTYLE, SWP_NOACTIVATE, SWP_NOSIZE,
    SWP_NOZORDER, WS_EX_TOOLWINDOW,
};

use crate::about_window::{self, AboutWindow};
use crate::config::{self, Config};
use crate::hotkey::HotkeyListener;
use crate::i18n::{self, Lang, Strings};
use crate::search::alias::AliasProvider;
use crate::search::app_index::{AppIndexProvider, IndexScan, ScanConfig};
use crate::search::everything::EverythingProvider;
use crate::search::plugin::PluginProvider;
use crate::search::windows_settings::WindowsSettingsProvider;
use crate::search::{Action, SearchProvider, SearchResult};
use crate::tools::{self, ToolKind, ToolsWindow};
use crate::tray::{self, TrayAction, TrayHandle};
use crate::ui_chrome;

/// Main window size (zero-results height). See `docs/architecture/window-lifecycle.md`.
pub const MAIN_WINDOW_SIZE: (f32, f32) = (640.0, 60.0);
/// Fixed off-screen point the window is moved to when logically "hidden" —
/// same rationale as the egui/eframe version (see `docs/architecture/window-lifecycle.md`).
pub const OFFSCREEN_POSITION: (f32, f32) = (-8000.0, -8000.0);
pub(crate) const MAX_VISIBLE_ROWS: usize = 8;
const RESULT_RETENTION_CAP: usize = 50;
const RESULT_ROW_HEIGHT: f32 = 40.0;
const CONTENT_PADDING: f32 = 16.0;
const PERIODIC_RESCAN_INTERVAL: Duration = Duration::from_secs(30 * 60);
/// Extra window height added only while the right-click context menu is
/// open. The main window is a single undecorated OS window, and the menu
/// (a `deferred`/`anchored` overlay) can't physically render outside that
/// window's own pixel bounds. With an empty query (window height =
/// `MAIN_WINDOW_SIZE.1` only), right-clicking the input box would otherwise
/// open a menu (up to 5 items + a separator) taller than the available 60px
/// and get visibly cut off. Ported from the egui/eframe version's
/// `CONTEXT_MENU_HEADROOM` (`ctx.any_popup_open()`-gated); here it's gated
/// on `self.context_menu.is_some()` instead.
const CONTEXT_MENU_HEADROOM: f32 = 220.0;

actions!(
    issen,
    [
        MoveUp,
        MoveDown,
        Confirm,
        RunAsAdminAction,
        OpenLocationAction,
        EscapeAction,
        AltNum1,
        AltNum2,
        AltNum3,
        AltNum4,
        AltNum5,
        AltNum6,
        AltNum7,
        AltNum8,
        AltNum9,
        Backspace,
        Delete,
        Left,
        Right,
        SelectLeft,
        SelectRight,
        SelectAll,
        Home,
        End,
        Paste,
        Cut,
        Copy,
    ]
);

const KEY_CONTEXT: &str = "issen-search";

enum ResultActionKind {
    Default,
    RunAsAdmin,
    OpenLocation,
}

/// What the open right-click context menu is for — the search box's own
/// background (Settings/Reindex/Quit) or a specific result row (index into
/// `IssenApp::results` at the time it was opened).
#[derive(Clone, Copy)]
enum ContextMenuTarget {
    Main,
    Row(usize),
}

#[derive(Clone, Copy)]
struct OpenContextMenu {
    target: ContextMenuTarget,
    /// Window-space position the menu is anchored to (from the opening
    /// `MouseDownEvent::position`).
    position: Point<Pixels>,
}

pub struct IssenApp {
    pub(crate) config: Config,
    lang: Lang,
    pub(crate) strings: &'static Strings,

    // Search box text-input state (see `EntityInputHandler` impl below).
    focus_handle: FocusHandle,
    query: SharedString,
    selected_range: Range<usize>,
    selection_reversed: bool,
    marked_range: Option<Range<usize>>,
    last_layout: Option<ShapedLine>,
    last_bounds: Option<Bounds<Pixels>>,
    is_selecting: bool,

    results: Vec<SearchResult>,
    selected: usize,
    /// The window's current content height, guarding `Window::resize` so it's
    /// only called when the value actually changes (same pattern as the
    /// egui/eframe version's `set_window_height`).
    content_height: f32,

    visible: bool,
    /// Set `false` on every `show()`, `true` the first time this window is
    /// actually observed OS-active afterward. Guards the focus-loss auto-hide
    /// (`new`'s `observe_window_activation`) against a real race: `show()`
    /// requests OS activation asynchronously (`Window::activate_window`,
    /// dispatched via an executor task), so if Windows denies the foreground
    /// steal, a deactivation could arrive while `visible` is already `true`
    /// but this window was never actually the active one — without this
    /// guard, that would immediately hide the window it just showed.
    seen_active_since_show: bool,
    /// Set for the duration of a synchronous call into `settings_window::
    /// open`/`tools::open`/`about_window::open`. Those functions' `Option<
    /// WindowHandle<_>>` output field is only assigned *after* `cx.
    /// open_window` returns, but per gotcha #3 in the migration notes,
    /// `cx.open_window` runs the new window's first render (and can trigger
    /// this window's deactivation) synchronously, inside that same call —
    /// during which `has_open_secondary_window`'s entity-based check would
    /// still see `None` and hide this window out from under the one that's
    /// opening. This flag covers exactly that window.
    opening_secondary_window: bool,

    /// The query-history list (🕘 icon next to the input box). While open,
    /// it's drawn in place of the normal result list, and keyboard shortcuts
    /// that act on `results` (up/down, Enter, Alt+digit) are suspended —
    /// `results` stays stale from the last real search while this is open.
    history_panel_open: bool,
    /// The open right-click context menu, if any (search box background or
    /// a result row). GPUI core has no `context_menu` widget, so this is a
    /// custom `anchored()`/`deferred()` overlay drawn in `render()`.
    context_menu: Option<OpenContextMenu>,

    // Kept alive so its listener thread (and the global hotkey registration
    // it holds) isn't torn down. `update_hotkey` is called live from the
    // settings window's hotkey field (`settings_window.rs`).
    pub(crate) hotkey: HotkeyListener,
    pub(crate) tray: TrayHandle,
    about_window: Option<WindowHandle<AboutWindow>>,
    settings_window: Option<WindowHandle<crate::settings_window::SettingsWindow>>,
    tools_window: Option<WindowHandle<ToolsWindow>>,
    app_index: AppIndexProvider,
    history: crate::history::History,
    plugins: PluginProvider,
    pub(crate) scanning: bool,
    pub(crate) last_scan_finished: Option<Instant>,
    pub(crate) last_scan_count: usize,
    next_periodic_scan: Instant,

    main_hwnd: HWND,
}

impl IssenApp {
    fn new(
        window: &mut Window,
        cx: &mut Context<Self>,
        config: Config,
        hotkey: HotkeyListener,
        tray: TrayHandle,
    ) -> Self {
        let main_hwnd = window_hwnd(window).expect("failed to obtain main window HWND");
        set_tool_window_style(main_hwnd);
        // `WindowOptions.window_bounds`'s origin/size aren't reliably honored
        // when the window is created off-screen at a large negative origin
        // (observed on real hardware: the window came up full-monitor-width
        // and on-screen instead of at `OFFSCREEN_POSITION`/`MAIN_WINDOW_SIZE`).
        // Force both explicitly via the same raw Win32 calls `hide` uses,
        // rather than trusting `WindowOptions` for the off-screen case.
        window.resize(size(px(MAIN_WINDOW_SIZE.0), px(MAIN_WINDOW_SIZE.1)));
        move_window(
            main_hwnd,
            OFFSCREEN_POSITION.0 as i32,
            OFFSCREEN_POSITION.1 as i32,
        );

        let lang = i18n::resolve(config.language);
        let strings = Strings::for_lang(lang);

        let focus_handle = cx.focus_handle();
        focus_handle.focus(window, cx);

        let mut app = Self {
            lang,
            strings,
            focus_handle,
            query: SharedString::default(),
            selected_range: 0..0,
            selection_reversed: false,
            marked_range: None,
            last_layout: None,
            last_bounds: None,
            is_selecting: false,
            results: Vec::new(),
            selected: 0,
            content_height: MAIN_WINDOW_SIZE.1,
            visible: false,
            seen_active_since_show: false,
            opening_secondary_window: false,
            history_panel_open: false,
            context_menu: None,
            hotkey,
            tray,
            about_window: None,
            settings_window: None,
            tools_window: None,
            app_index: AppIndexProvider::empty(),
            history: crate::history::History::load_or_default(config::APP_NAME),
            plugins: PluginProvider::load_from_app_data(),
            scanning: false,
            last_scan_finished: None,
            last_scan_count: 0,
            next_periodic_scan: Instant::now() + PERIODIC_RESCAN_INTERVAL,
            main_hwnd,
            config,
        };
        app.start_scan(cx);

        // Matches the egui/eframe version's `WindowFocused(false)` handling:
        // losing OS focus hides the window, unless a secondary window (about/
        // settings/tools) is why focus moved — those are opened *over* the
        // main window and shouldn't cause it to vanish out from under them.
        // `cx.observe_window_activation` fires on both activate and
        // deactivate, so the `is_window_active()` check picks out the
        // deactivate case; `.detach()` keeps the subscription alive for the
        // window's lifetime (see `settings_window.rs` for the same pattern).
        cx.observe_window_activation(window, |this, window, cx| {
            if window.is_window_active() {
                this.seen_active_since_show = true;
                return;
            }
            if this.seen_active_since_show && !this.has_open_secondary_window(cx) {
                this.hide(window, cx);
            }
        })
        .detach();

        app
    }

    /// Whether the about/settings/tools window is currently open (or in the
    /// middle of opening — see `opening_secondary_window`'s doc comment).
    /// Those `Option<WindowHandle<_>>` fields are never cleared back to
    /// `None` when the user closes the window via its own close button (only
    /// when this app itself replaces/reopens it), so `.is_some()` alone would
    /// still see a stale, already-closed handle as "open" — `entity(cx)`
    /// fails once the underlying OS window is actually gone.
    fn has_open_secondary_window(&self, cx: &Context<Self>) -> bool {
        fn is_open<V: Render>(handle: &Option<WindowHandle<V>>, cx: &Context<IssenApp>) -> bool {
            handle.as_ref().is_some_and(|h| h.entity(cx).is_ok())
        }
        self.opening_secondary_window
            || is_open(&self.about_window, cx)
            || is_open(&self.settings_window, cx)
            || is_open(&self.tools_window, cx)
    }

    fn run_search(&mut self) {
        let aliases = AliasProvider::new(&self.config.aliases);
        let windows_settings =
            WindowsSettingsProvider::with_custom(self.lang, &self.config.custom_windows_shortcuts);

        let mut results = Vec::new();
        results.extend(aliases.search(&self.query));
        results.extend(windows_settings.search(&self.query));
        results.extend(self.app_index.search(&self.query));
        if self.config.everything_enabled {
            results.extend(EverythingProvider.search(&self.query));
        }
        results.extend(self.plugins.search(&self.query));
        for r in &mut results {
            if let Some(boost) = self
                .history
                .boost_for(&crate::search::target_key(&r.action))
            {
                r.score += boost;
            }
        }
        results.sort_by_key(|r| std::cmp::Reverse(r.score));
        results.truncate(RESULT_RETENTION_CAP);

        self.results = results;
        self.selected = 0;
    }

    /// Entry point for both the initial startup scan and a manual reindex
    /// (including from the settings window — a separate GPUI `Window`, so
    /// this deliberately doesn't take `&mut Window` itself; see below).
    /// No-op if a scan is already running. Unlike the egui/eframe version's
    /// `poll_scan` (called every frame from `logic()`), completion is
    /// awaited by a `cx.spawn` task that blocks on the scan's channel from a
    /// background executor thread and wakes this view once — no polling.
    pub(crate) fn start_scan(&mut self, cx: &mut Context<Self>) {
        if self.scanning {
            return;
        }
        let Some(scan) = IndexScan::spawn(ScanConfig::from_config(&self.config)) else {
            return;
        };
        self.scanning = true;
        self.tray.set_scanning(self.strings, true);

        // `Context::spawn` (unlike `App::spawn`) hands the async closure a
        // `WeakEntity<Self>`, used to get back into this view once the scan
        // finishes — no `&mut Window` needed for that (`on_scan_finished`
        // doesn't touch the window), which is what lets a *different*
        // window's event handler (settings' Rescan button) call this.
        cx.spawn(async move |this, cx| {
            let receiver = scan.into_receiver();
            let provider = cx
                .background_executor()
                .spawn(async move { receiver.recv().ok() })
                .await;
            if let Some(provider) = provider {
                let _ = this.update(cx, |view, cx| {
                    view.on_scan_finished(provider, cx);
                });
            }
        })
        .detach();
    }

    fn on_scan_finished(&mut self, provider: AppIndexProvider, cx: &mut Context<Self>) {
        self.last_scan_count = provider.len();
        self.last_scan_finished = Some(Instant::now());
        self.app_index = provider;
        self.scanning = false;
        self.tray.set_scanning(self.strings, false);
        self.next_periodic_scan = Instant::now() + PERIODIC_RESCAN_INTERVAL;
        if !self.query.is_empty() {
            self.run_search();
        }
        cx.notify();
    }

    /// Rebuilds runtime state that depends on `config.language` (display
    /// strings, tray icon) when it changes. Called directly from the
    /// settings window's language row on click — unlike the egui/eframe
    /// version's `apply_language` (called unconditionally every frame),
    /// there's no per-frame polling here, so this only needs to run once
    /// per actual change, at the point the change happens.
    pub(crate) fn apply_language(&mut self, cx: &mut Context<Self>) {
        let lang = i18n::resolve(self.config.language);
        if lang == self.lang {
            cx.notify();
            return;
        }
        self.lang = lang;
        self.strings = Strings::for_lang(lang);
        // Keep the existing tray icon if rebuilding fails (its labels stay
        // in the old language, but that's better than crashing the app).
        if let Some(tray) = TrayHandle::new(self.strings) {
            self.tray = tray;
            if self.scanning {
                self.tray.set_scanning(self.strings, true);
            }
        } else {
            eprintln!("issen: failed to rebuild tray icon after language change; keeping old one");
        }
        cx.notify();
    }

    fn run_result_action(
        &mut self,
        kind: ResultActionKind,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(result) = self.results.get(self.selected) else {
            return;
        };

        match kind {
            ResultActionKind::Default => {
                let history_key = (!matches!(result.action, Action::CopyToClipboard(_)))
                    .then(|| crate::search::target_key(&result.action));
                let ok = match &result.action {
                    Action::Launch { path, args } => {
                        crate::launch::open_with_args(&path.display().to_string(), args)
                    }
                    Action::OpenUri(uri) => crate::launch::open(uri),
                    Action::LaunchUwp { aumid } => crate::search::uwp::launch(aumid),
                    Action::CopyToClipboard(text) => crate::launch::copy_to_clipboard(text),
                };
                if ok {
                    let mut history_dirty = false;
                    if let Some(key) = history_key {
                        self.history.record_use(&key);
                        history_dirty = true;
                    }
                    if !self.query.trim().is_empty() {
                        self.history.record_query(&self.query);
                        history_dirty = true;
                    }
                    if history_dirty {
                        if let Err(err) = self.history.save(config::APP_NAME) {
                            eprintln!("issen: failed to save history.toml: {err}");
                        }
                    }
                    self.hide(window, cx);
                }
            }
            ResultActionKind::RunAsAdmin => {
                if let Action::Launch { path, args } = &result.action {
                    if crate::launch::open_elevated(&path.display().to_string(), args) {
                        self.hide(window, cx);
                    }
                }
            }
            ResultActionKind::OpenLocation => {
                if let Action::Launch { path, .. } = &result.action {
                    crate::launch::open_containing_folder(path);
                    self.hide(window, cx);
                }
            }
        }
    }

    /// Moves the window to its real on-screen position and focuses it. See
    /// `hide`'s doc comment for why the window is moved rather than
    /// shown/hidden at the OS level.
    fn show(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.visible {
            return;
        }
        self.visible = true;
        self.seen_active_since_show = false;
        let target = self.resolve_show_target();
        move_window(self.main_hwnd, target.0 as i32, target.1 as i32);
        // `window.focus` only moves GPUI's own internal notion of which view
        // has keyboard focus — it doesn't ask Windows for OS-level input
        // focus. Without `activate_window()` (raw `SetForegroundWindow`/
        // `SetActiveWindow`/`SetFocus`, see `gpui_windows`'s `activate`),
        // keystrokes typed right after a hotkey press went nowhere until the
        // user clicked the window once. Matches the egui/eframe version's
        // `ViewportCommand::Focus` on the same call site.
        window.activate_window();
        window.focus(&self.focus_handle, cx);
        cx.notify();
    }

    /// **Never uses OS-level Show/Hide.** Same rationale as the egui/eframe
    /// version (`docs/architecture/window-lifecycle.md`): the window stays
    /// OS-visible at all times, and "hidden" is represented purely by
    /// parking it at `OFFSCREEN_POSITION` via a raw `SetWindowPos` call.
    fn hide(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.visible {
            return;
        }
        self.visible = false;
        move_window(
            self.main_hwnd,
            OFFSCREEN_POSITION.0 as i32,
            OFFSCREEN_POSITION.1 as i32,
        );
        self.query = SharedString::default();
        self.selected_range = 0..0;
        self.marked_range = None;
        self.results.clear();
        self.selected = 0;
        self.history_panel_open = false;
        self.context_menu = None;
        self.sync_window_height(window);
        cx.notify();
    }

    fn resolve_show_target(&self) -> (f32, f32) {
        match self.config.display_target {
            config::DisplayTarget::Cursor => {
                crate::display::position_on_cursor_monitor(MAIN_WINDOW_SIZE)
            }
            config::DisplayTarget::Primary => {
                crate::display::position_on_primary_monitor(MAIN_WINDOW_SIZE)
            }
            config::DisplayTarget::FocusedWindow => {
                crate::display::position_on_foreground_window_monitor(MAIN_WINDOW_SIZE)
            }
        }
        .or_else(|| crate::display::position_on_primary_monitor(MAIN_WINDOW_SIZE))
        .unwrap_or((100.0, 100.0))
    }

    /// Syncs the window's actual OS height to the current result count (or,
    /// while the history panel is open, the query-history row count — even
    /// with zero history entries, one row ("no history yet") is still
    /// drawn, hence `max(1)`). Adds `CONTEXT_MENU_HEADROOM` while the
    /// right-click context menu is open, so it doesn't get cut off. Safe to
    /// call every frame (`Window::resize` is only invoked when the value
    /// changes).
    fn sync_window_height(&mut self, window: &mut Window) {
        let rows = if self.history_panel_open {
            self.history
                .queries
                .len()
                .max(1)
                .min(self.visible_rows_cap())
        } else {
            self.results.len().min(self.visible_rows_cap())
        } as f32;
        let mut height = MAIN_WINDOW_SIZE.1 + rows * RESULT_ROW_HEIGHT;
        if self.context_menu.is_some() {
            height += CONTEXT_MENU_HEADROOM;
        }
        if (self.content_height - height).abs() < f32::EPSILON {
            return;
        }
        self.content_height = height;
        window.resize(size(px(MAIN_WINDOW_SIZE.0), px(height)));
    }

    fn visible_rows_cap(&self) -> usize {
        (self.config.max_results as usize).min(MAX_VISIBLE_ROWS)
    }

    /// One of the search box's toolbar icon buttons (color picker, unit
    /// converter — see `crate::tools`).
    fn toolbar_icon_button(
        key: &'static str,
        glyph: &'static str,
        on_click: impl Fn(&MouseDownEvent, &mut Window, &mut App) + 'static,
    ) -> impl IntoElement {
        div()
            .id(key)
            .flex_none()
            .flex()
            .items_center()
            .justify_center()
            .size(px(28.))
            .rounded(px(6.))
            .cursor_pointer()
            .text_size(px(14.))
            .hover(|d| d.bg(hsla(0., 0., 1., 0.08)))
            // See the text field's own `.occlude()` comment in `render` —
            // without this, a click here would also count as landing on the
            // search box's background drag region behind it.
            .occlude()
            .on_mouse_down(MouseButton::Left, on_click)
            .child(glyph)
    }

    fn handle_tray_action(
        &mut self,
        action: TrayAction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match action {
            TrayAction::Open => self.show(window, cx),
            TrayAction::Settings => {
                // Matches the egui/eframe version: the tray's Settings
                // action also brings the main search window up (it's the
                // only route to Settings that can fire while the main
                // window is still hidden).
                self.show(window, cx);
                self.open_settings_window(cx);
            }
            TrayAction::Reindex => self.start_scan(cx),
            TrayAction::About => {
                // Matches the egui/eframe version: the tray's About action also
                // brings the main search window up (it's the only route to
                // About that can fire while the main window is still hidden).
                self.show(window, cx);
                self.opening_secondary_window = true;
                about_window::open(&mut self.about_window, self.strings, self.config.theme, cx);
                self.opening_secondary_window = false;
            }
        }
    }

    /// Toggles a toolbar tool open/closed: a repeat click on the button for
    /// the tool that's currently showing closes the window (matches the
    /// egui/eframe version, where both tools shared one viewport); clicking
    /// while the *other* tool is open switches its content in place instead
    /// of opening a second window.
    fn toggle_tool(&mut self, kind: ToolKind, cx: &mut Context<Self>) {
        if let Some(handle) = &self.tools_window {
            let current_kind = handle.update(cx, |view, _, _| view.kind()).ok();
            if current_kind == Some(kind) {
                let _ = handle.update(cx, |_, window, _| window.remove_window());
                self.tools_window = None;
                return;
            }
        }
        self.opening_secondary_window = true;
        tools::open(
            &mut self.tools_window,
            kind,
            self.strings,
            self.config.theme,
            self.config.accent_color,
            cx,
        );
        self.opening_secondary_window = false;
    }

    /// Opens the settings window (or brings an already-open one to front),
    /// building the `AppSnapshot` it needs from directly-accessible fields.
    /// Shared by the tray's Settings action, the main context menu's
    /// Settings item, and `register_selected_as_alias`.
    fn open_settings_window(&mut self, cx: &mut Context<Self>) {
        let weak = cx.weak_entity();
        let snapshot = crate::settings_window::AppSnapshot {
            config: self.config.clone(),
            strings: self.strings,
            scanning: self.scanning,
            last_scan_finished: self.last_scan_finished,
            last_scan_count: self.last_scan_count,
        };
        self.opening_secondary_window = true;
        crate::settings_window::open(&mut self.settings_window, weak, snapshot, cx);
        self.opening_secondary_window = false;
    }

    /// "Register as alias" from a result row's context menu: opens the
    /// settings window and prefills its "add alias" fields with the
    /// selected result's title/target. Matches the egui/eframe version's
    /// `SettingsWindow::prefill_alias`.
    fn register_selected_as_alias(&mut self, cx: &mut Context<Self>) {
        let Some(result) = self.results.get(self.selected) else {
            return;
        };
        let title = result.title.clone();
        let target = crate::search::target_key(&result.action);
        self.open_settings_window(cx);
        if let Some(handle) = &self.settings_window {
            let _ = handle.update(cx, |view, window, cx| {
                view.prefill_alias(title, target, window, cx);
            });
        }
    }

    /// "Pin"/"Unpin" from a result row's context menu. Both rebuild via
    /// `run_search` rather than patching the score in place, so the ranking
    /// reflects the change immediately — matches the egui/eframe version's
    /// `RowMenuAction::Pin`/`Unpin`.
    fn pin_selected(&mut self, cx: &mut Context<Self>) {
        if let Some(result) = self.results.get(self.selected) {
            let key = crate::search::target_key(&result.action);
            self.history.pin(&key);
            if let Err(err) = self.history.save(config::APP_NAME) {
                eprintln!("issen: failed to save history.toml: {err}");
            }
        }
        self.run_search();
        cx.notify();
    }

    fn unpin_selected(&mut self, cx: &mut Context<Self>) {
        if let Some(result) = self.results.get(self.selected) {
            let key = crate::search::target_key(&result.action);
            self.history.remove(&key);
            if let Err(err) = self.history.save(config::APP_NAME) {
                eprintln!("issen: failed to save history.toml: {err}");
            }
        }
        self.run_search();
        cx.notify();
    }

    /// Toggles the query-history panel (🕘 icon).
    fn toggle_history_panel(&mut self, cx: &mut Context<Self>) {
        self.history_panel_open = !self.history_panel_open;
        cx.notify();
    }

    /// Re-runs a past query picked from the history panel. Doesn't execute
    /// anything directly — it's a "re-search," not a "re-run" (matches the
    /// egui/eframe version's `show_history_panel`).
    fn use_history_query(&mut self, index: usize, cx: &mut Context<Self>) {
        let Some(query) = self.history.queries.get(index).cloned() else {
            return;
        };
        self.query = query.into();
        let len = self.query.len();
        self.selected_range = len..len;
        self.marked_range = None;
        self.history_panel_open = false;
        self.run_search();
        cx.notify();
    }

    // --- Result-list keyboard actions ---
    //
    // While the query-history panel is open, `self.results` is stale (from
    // the last real search), so every one of these guards on
    // `history_panel_open` and no-ops instead of acting on it — matches the
    // egui/eframe version's single early-return in `logic()`.

    fn move_up(&mut self, _: &MoveUp, _: &mut Window, cx: &mut Context<Self>) {
        if self.history_panel_open {
            return;
        }
        self.selected = self.selected.saturating_sub(1);
        cx.notify();
    }

    fn move_down(&mut self, _: &MoveDown, _: &mut Window, cx: &mut Context<Self>) {
        if self.history_panel_open {
            return;
        }
        if !self.results.is_empty() {
            self.selected = (self.selected + 1).min(self.results.len() - 1);
        }
        cx.notify();
    }

    fn confirm(&mut self, _: &Confirm, window: &mut Window, cx: &mut Context<Self>) {
        if self.history_panel_open {
            return;
        }
        self.run_result_action(ResultActionKind::Default, window, cx);
    }

    fn run_as_admin_action(
        &mut self,
        _: &RunAsAdminAction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.history_panel_open {
            return;
        }
        self.run_result_action(ResultActionKind::RunAsAdmin, window, cx);
    }

    fn open_location_action(
        &mut self,
        _: &OpenLocationAction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.history_panel_open {
            return;
        }
        self.run_result_action(ResultActionKind::OpenLocation, window, cx);
    }

    /// Closes an open context menu first, if any (GPUI has no built-in
    /// popup that would otherwise swallow this itself); only hides the
    /// whole window on a second Escape.
    fn escape_action(&mut self, _: &EscapeAction, window: &mut Window, cx: &mut Context<Self>) {
        if self.context_menu.take().is_some() {
            cx.notify();
            return;
        }
        self.hide(window, cx);
    }

    fn alt_digit(&mut self, idx: usize, window: &mut Window, cx: &mut Context<Self>) {
        if self.history_panel_open {
            return;
        }
        let visible_rows = self.results.len().min(self.visible_rows_cap());
        if idx < visible_rows {
            self.selected = idx;
            self.run_result_action(ResultActionKind::Default, window, cx);
        }
    }

    fn alt_num_1(&mut self, _: &AltNum1, w: &mut Window, cx: &mut Context<Self>) {
        self.alt_digit(0, w, cx);
    }
    fn alt_num_2(&mut self, _: &AltNum2, w: &mut Window, cx: &mut Context<Self>) {
        self.alt_digit(1, w, cx);
    }
    fn alt_num_3(&mut self, _: &AltNum3, w: &mut Window, cx: &mut Context<Self>) {
        self.alt_digit(2, w, cx);
    }
    fn alt_num_4(&mut self, _: &AltNum4, w: &mut Window, cx: &mut Context<Self>) {
        self.alt_digit(3, w, cx);
    }
    fn alt_num_5(&mut self, _: &AltNum5, w: &mut Window, cx: &mut Context<Self>) {
        self.alt_digit(4, w, cx);
    }
    fn alt_num_6(&mut self, _: &AltNum6, w: &mut Window, cx: &mut Context<Self>) {
        self.alt_digit(5, w, cx);
    }
    fn alt_num_7(&mut self, _: &AltNum7, w: &mut Window, cx: &mut Context<Self>) {
        self.alt_digit(6, w, cx);
    }
    fn alt_num_8(&mut self, _: &AltNum8, w: &mut Window, cx: &mut Context<Self>) {
        self.alt_digit(7, w, cx);
    }
    fn alt_num_9(&mut self, _: &AltNum9, w: &mut Window, cx: &mut Context<Self>) {
        self.alt_digit(8, w, cx);
    }

    // --- Text-input editing (ported from examples/gpui_spike_input.rs) ---

    fn text_left(&mut self, _: &Left, _: &mut Window, cx: &mut Context<Self>) {
        if self.selected_range.is_empty() {
            self.move_to(self.previous_boundary(self.cursor_offset()), cx);
        } else {
            self.move_to(self.selected_range.start, cx)
        }
    }

    fn text_right(&mut self, _: &Right, _: &mut Window, cx: &mut Context<Self>) {
        if self.selected_range.is_empty() {
            self.move_to(self.next_boundary(self.selected_range.end), cx);
        } else {
            self.move_to(self.selected_range.end, cx)
        }
    }

    fn select_left(&mut self, _: &SelectLeft, _: &mut Window, cx: &mut Context<Self>) {
        self.select_to(self.previous_boundary(self.cursor_offset()), cx);
    }

    fn select_right(&mut self, _: &SelectRight, _: &mut Window, cx: &mut Context<Self>) {
        self.select_to(self.next_boundary(self.cursor_offset()), cx);
    }

    fn select_all(&mut self, _: &SelectAll, _: &mut Window, cx: &mut Context<Self>) {
        self.move_to(0, cx);
        self.select_to(self.query.len(), cx)
    }

    fn home(&mut self, _: &Home, _: &mut Window, cx: &mut Context<Self>) {
        self.move_to(0, cx);
    }

    fn end(&mut self, _: &End, _: &mut Window, cx: &mut Context<Self>) {
        self.move_to(self.query.len(), cx);
    }

    fn text_backspace(&mut self, _: &Backspace, window: &mut Window, cx: &mut Context<Self>) {
        if self.selected_range.is_empty() {
            let prev = self.previous_boundary(self.cursor_offset());
            if self.cursor_offset() == prev {
                window.play_system_bell();
                return;
            }
            self.select_to(prev, cx)
        }
        self.replace_text_in_range(None, "", window, cx);
    }

    fn text_delete(&mut self, _: &Delete, window: &mut Window, cx: &mut Context<Self>) {
        if self.selected_range.is_empty() {
            let next = self.next_boundary(self.cursor_offset());
            if self.cursor_offset() == next {
                window.play_system_bell();
                return;
            }
            self.select_to(next, cx)
        }
        self.replace_text_in_range(None, "", window, cx);
    }

    fn on_mouse_down(
        &mut self,
        event: &MouseDownEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.is_selecting = true;
        if event.modifiers.shift {
            self.select_to(self.index_for_mouse_position(event.position), cx);
        } else {
            self.move_to(self.index_for_mouse_position(event.position), cx)
        }
    }

    fn on_mouse_up(&mut self, _: &MouseUpEvent, _window: &mut Window, _: &mut Context<Self>) {
        self.is_selecting = false;
    }

    fn on_mouse_move(&mut self, event: &MouseMoveEvent, _: &mut Window, cx: &mut Context<Self>) {
        if self.is_selecting {
            self.select_to(self.index_for_mouse_position(event.position), cx);
        }
    }

    fn paste(&mut self, _: &Paste, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) {
            self.replace_text_in_range(None, &text.replace('\n', " "), window, cx);
        }
    }

    /// Ctrl+C: copies the selected text if any, otherwise falls back to
    /// copying the selected result's path (same dual meaning as the
    /// egui/eframe version's `ResultActionKind::CopyPath`).
    fn copy(&mut self, _: &Copy, _: &mut Window, cx: &mut Context<Self>) {
        if !self.selected_range.is_empty() {
            cx.write_to_clipboard(ClipboardItem::new_string(
                self.query[self.selected_range.clone()].to_string(),
            ));
            return;
        }
        if let Some(SearchResult {
            action: Action::Launch { path, .. },
            ..
        }) = self.results.get(self.selected)
        {
            crate::launch::copy_to_clipboard(&path.display().to_string());
        }
    }

    fn cut(&mut self, _: &Cut, window: &mut Window, cx: &mut Context<Self>) {
        if !self.selected_range.is_empty() {
            cx.write_to_clipboard(ClipboardItem::new_string(
                self.query[self.selected_range.clone()].to_string(),
            ));
            self.replace_text_in_range(None, "", window, cx);
        }
    }

    fn move_to(&mut self, offset: usize, cx: &mut Context<Self>) {
        self.selected_range = offset..offset;
        cx.notify()
    }

    fn cursor_offset(&self) -> usize {
        if self.selection_reversed {
            self.selected_range.start
        } else {
            self.selected_range.end
        }
    }

    fn index_for_mouse_position(&self, position: Point<Pixels>) -> usize {
        if self.query.is_empty() {
            return 0;
        }
        let (Some(bounds), Some(line)) = (self.last_bounds.as_ref(), self.last_layout.as_ref())
        else {
            return 0;
        };
        if position.y < bounds.top() {
            return 0;
        }
        if position.y > bounds.bottom() {
            return self.query.len();
        }
        line.closest_index_for_x(position.x - bounds.left())
    }

    fn select_to(&mut self, offset: usize, cx: &mut Context<Self>) {
        if self.selection_reversed {
            self.selected_range.start = offset
        } else {
            self.selected_range.end = offset
        };
        if self.selected_range.end < self.selected_range.start {
            self.selection_reversed = !self.selection_reversed;
            self.selected_range = self.selected_range.end..self.selected_range.start;
        }
        cx.notify()
    }

    fn offset_from_utf16(&self, offset: usize) -> usize {
        let mut utf8_offset = 0;
        let mut utf16_count = 0;
        for ch in self.query.chars() {
            if utf16_count >= offset {
                break;
            }
            utf16_count += ch.len_utf16();
            utf8_offset += ch.len_utf8();
        }
        utf8_offset
    }

    fn offset_to_utf16(&self, offset: usize) -> usize {
        let mut utf16_offset = 0;
        let mut utf8_count = 0;
        for ch in self.query.chars() {
            if utf8_count >= offset {
                break;
            }
            utf8_count += ch.len_utf8();
            utf16_offset += ch.len_utf16();
        }
        utf16_offset
    }

    fn range_to_utf16(&self, range: &Range<usize>) -> Range<usize> {
        self.offset_to_utf16(range.start)..self.offset_to_utf16(range.end)
    }

    fn range_from_utf16(&self, range_utf16: &Range<usize>) -> Range<usize> {
        self.offset_from_utf16(range_utf16.start)..self.offset_from_utf16(range_utf16.end)
    }

    fn previous_boundary(&self, offset: usize) -> usize {
        self.query
            .grapheme_indices(true)
            .rev()
            .find_map(|(idx, _)| (idx < offset).then_some(idx))
            .unwrap_or(0)
    }

    fn next_boundary(&self, offset: usize) -> usize {
        self.query
            .grapheme_indices(true)
            .find_map(|(idx, _)| (idx > offset).then_some(idx))
            .unwrap_or(self.query.len())
    }

    // --- Right-click context menu ---
    //
    // GPUI core has no `context_menu`-style widget, unlike egui — this is a
    // custom overlay built from `anchored()` (keeps it inside the window
    // bounds) wrapped in `deferred()` (paints after every sibling, i.e. on
    // top). See `render_context_menu` for how it's actually drawn.

    fn open_main_context_menu(
        &mut self,
        event: &MouseDownEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.context_menu = Some(OpenContextMenu {
            target: ContextMenuTarget::Main,
            position: event.position,
        });
        // A right-click landing on the search box's drag region (see
        // `render`'s `search_box`) reaches this handler via Windows'
        // `WM_NCRBUTTONDOWN` path (`window_control_area`'s hit-test), not
        // the normal client-area `WM_RBUTTONDOWN`. Without stopping
        // propagation here, that NC message is left unhandled and falls
        // through to `DefWindowProc`, which pops the native OS system menu
        // on top of this custom one.
        cx.stop_propagation();
        cx.notify();
    }

    /// One row of a context menu (label + click handler). Visually distinct
    /// from `toolbar_icon_button` (full-width row vs. a square icon) but the
    /// same hover/cursor treatment.
    fn context_menu_item(
        key: &'static str,
        label: &'static str,
        on_click: impl Fn(&MouseDownEvent, &mut Window, &mut App) + 'static,
    ) -> impl IntoElement {
        div()
            .id(key)
            .h(px(32.))
            .px(px(12.))
            .flex()
            .items_center()
            .text_size(px(13.))
            .text_color(white())
            .cursor_pointer()
            .hover(|d| d.bg(hsla(0., 0., 1., 0.08)))
            .on_mouse_down(MouseButton::Left, on_click)
            .child(label)
    }

    fn context_menu_separator() -> impl IntoElement {
        div()
            .h(px(1.))
            .mx(px(6.))
            .my(px(4.))
            .bg(hsla(0., 0., 1., 0.12))
    }

    /// Builds the open context menu's overlay element, if any. Reads
    /// `self.context_menu` (target + position) fresh each render, so a
    /// `Row(i)` menu whose row disappeared (results changed while it was
    /// open) is closed rather than shown against a stale index.
    fn render_context_menu(&mut self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let menu = self.context_menu?;
        let items: Vec<AnyElement> = match menu.target {
            ContextMenuTarget::Main => vec![
                Self::context_menu_item(
                    "ctx-settings",
                    self.strings.tray_settings,
                    cx.listener(|this, _, _, cx| {
                        this.context_menu = None;
                        this.open_settings_window(cx);
                        cx.notify();
                    }),
                )
                .into_any_element(),
                Self::context_menu_item(
                    "ctx-reindex",
                    self.strings.tray_reindex,
                    cx.listener(|this, _, _, cx| {
                        this.context_menu = None;
                        this.start_scan(cx);
                        cx.notify();
                    }),
                )
                .into_any_element(),
                Self::context_menu_separator().into_any_element(),
                Self::context_menu_item("ctx-quit", self.strings.tray_quit, |_, _, _| {
                    // See `tray.rs`'s `ensure_event_forwarding` doc comment
                    // for why Quit exits directly rather than going through
                    // any window-close path.
                    std::process::exit(0);
                })
                .into_any_element(),
            ],
            ContextMenuTarget::Row(i) => {
                let Some(result) = self.results.get(i) else {
                    self.context_menu = None;
                    return None;
                };
                let is_file_action = matches!(result.action, Action::Launch { .. });
                let is_pinned = result.score >= crate::history::HISTORY_SCORE_BOOST;

                let mut items = vec![Self::context_menu_item(
                    "ctx-run",
                    self.strings.action_run,
                    cx.listener(move |this, _, window, cx| {
                        this.context_menu = None;
                        this.selected = i;
                        this.run_result_action(ResultActionKind::Default, window, cx);
                        cx.notify();
                    }),
                )
                .into_any_element()];
                if is_file_action {
                    items.push(
                        Self::context_menu_item(
                            "ctx-run-as-admin",
                            self.strings.action_run_as_admin,
                            cx.listener(move |this, _, window, cx| {
                                this.context_menu = None;
                                this.selected = i;
                                this.run_result_action(ResultActionKind::RunAsAdmin, window, cx);
                                cx.notify();
                            }),
                        )
                        .into_any_element(),
                    );
                    items.push(
                        Self::context_menu_item(
                            "ctx-open-location",
                            self.strings.action_open_location,
                            cx.listener(move |this, _, window, cx| {
                                this.context_menu = None;
                                this.selected = i;
                                this.run_result_action(ResultActionKind::OpenLocation, window, cx);
                                cx.notify();
                            }),
                        )
                        .into_any_element(),
                    );
                    items.push(
                        Self::context_menu_item(
                            "ctx-copy-path",
                            self.strings.action_copy_path,
                            cx.listener(move |this, _, _, cx| {
                                this.context_menu = None;
                                if let Some(SearchResult {
                                    action: Action::Launch { path, .. },
                                    ..
                                }) = this.results.get(i)
                                {
                                    crate::launch::copy_to_clipboard(&path.display().to_string());
                                }
                                cx.notify();
                            }),
                        )
                        .into_any_element(),
                    );
                }
                items.push(
                    Self::context_menu_item(
                        "ctx-register-alias",
                        self.strings.action_register_alias,
                        cx.listener(move |this, _, _, cx| {
                            this.context_menu = None;
                            this.selected = i;
                            this.register_selected_as_alias(cx);
                            cx.notify();
                        }),
                    )
                    .into_any_element(),
                );
                if is_pinned {
                    items.push(
                        Self::context_menu_item(
                            "ctx-unpin",
                            self.strings.action_unpin,
                            cx.listener(move |this, _, _, cx| {
                                this.context_menu = None;
                                this.selected = i;
                                this.unpin_selected(cx);
                            }),
                        )
                        .into_any_element(),
                    );
                } else {
                    items.push(
                        Self::context_menu_item(
                            "ctx-pin",
                            self.strings.action_pin,
                            cx.listener(move |this, _, _, cx| {
                                this.context_menu = None;
                                this.selected = i;
                                this.pin_selected(cx);
                            }),
                        )
                        .into_any_element(),
                    );
                }
                items
            }
        };

        Some(
            deferred(
                anchored().position(menu.position).snap_to_window().child(
                    div()
                        .id("context-menu")
                        .occlude()
                        .flex()
                        .flex_col()
                        .py(px(4.))
                        .min_w(px(190.))
                        .rounded(px(8.))
                        .bg(hsla(220. / 360., 0.12, 0.13, 0.98))
                        .border_1()
                        .border_color(hsla(0., 0., 1., 0.14))
                        .shadow_lg()
                        // Capture-phase + `stop_propagation()` so an
                        // outside click both closes the menu and doesn't
                        // also fall through to whatever's underneath it
                        // (e.g. launching a result row the click landed
                        // on) — see `dispatch_mouse_event`'s capture/
                        // bubble split in gpui's `window.rs`.
                        .on_mouse_down_out(cx.listener(|this, _, _, cx| {
                            this.context_menu = None;
                            cx.stop_propagation();
                            cx.notify();
                        }))
                        .children(items),
                ),
            )
            .with_priority(1)
            .into_any_element(),
        )
    }
}

impl EntityInputHandler for IssenApp {
    fn text_for_range(
        &mut self,
        range_utf16: Range<usize>,
        actual_range: &mut Option<Range<usize>>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<String> {
        let range = self.range_from_utf16(&range_utf16);
        actual_range.replace(self.range_to_utf16(&range));
        Some(self.query[range].to_string())
    }

    fn selected_text_range(
        &mut self,
        _ignore_disabled_input: bool,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        Some(UTF16Selection {
            range: self.range_to_utf16(&self.selected_range),
            reversed: self.selection_reversed,
        })
    }

    fn marked_text_range(
        &self,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Range<usize>> {
        self.marked_range
            .as_ref()
            .map(|range| self.range_to_utf16(range))
    }

    fn unmark_text(&mut self, _window: &mut Window, _cx: &mut Context<Self>) {
        self.marked_range = None;
    }

    fn replace_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        new_text: &str,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let range = range_utf16
            .as_ref()
            .map(|range_utf16| self.range_from_utf16(range_utf16))
            .or(self.marked_range.clone())
            .unwrap_or(self.selected_range.clone());

        self.query =
            (self.query[0..range.start].to_owned() + new_text + &self.query[range.end..]).into();
        self.selected_range = range.start + new_text.len()..range.start + new_text.len();
        self.marked_range.take();
        // Switch back to the normal search results, not the history panel,
        // as soon as the query is edited (showing both at once would be
        // confusing) — matches the egui/eframe version's `response.changed()`
        // handling.
        self.history_panel_open = false;
        // Every plain (non-IME) keystroke and every IME composition's final
        // commit go through this method (see `replace_and_mark_text_in_range`
        // for the in-progress-composition case), so this is the single place
        // that needs to re-run search after a query edit.
        self.run_search();
        cx.notify();
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        new_text: &str,
        new_selected_range_utf16: Option<Range<usize>>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let range = range_utf16
            .as_ref()
            .map(|range_utf16| self.range_from_utf16(range_utf16))
            .or(self.marked_range.clone())
            .unwrap_or(self.selected_range.clone());

        self.query =
            (self.query[0..range.start].to_owned() + new_text + &self.query[range.end..]).into();
        if !new_text.is_empty() {
            self.marked_range = Some(range.start..range.start + new_text.len());
        } else {
            self.marked_range = None;
        }
        self.selected_range = new_selected_range_utf16
            .as_ref()
            .map(|range_utf16| self.range_from_utf16(range_utf16))
            .map(|new_range| new_range.start + range.start..new_range.end + range.end)
            .unwrap_or_else(|| range.start + new_text.len()..range.start + new_text.len());

        self.history_panel_open = false;
        // Re-searching while composition is still in progress (not yet
        // committed) gives more responsive incremental results — the same
        // reasoning as searching on every keystroke of plain input.
        self.run_search();
        cx.notify();
    }

    fn bounds_for_range(
        &mut self,
        range_utf16: Range<usize>,
        bounds: Bounds<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        let last_layout = self.last_layout.as_ref()?;
        let range = self.range_from_utf16(&range_utf16);
        Some(Bounds::from_corners(
            point(
                bounds.left() + last_layout.x_for_index(range.start),
                bounds.top(),
            ),
            point(
                bounds.left() + last_layout.x_for_index(range.end),
                bounds.bottom(),
            ),
        ))
    }

    fn character_index_for_point(
        &mut self,
        point: Point<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<usize> {
        let line_point = self.last_bounds?.localize(&point)?;
        let last_layout = self.last_layout.as_ref()?;
        let utf8_index = last_layout.index_for_x(point.x - line_point.x)?;
        Some(self.offset_to_utf16(utf8_index))
    }
}

impl Focusable for IssenApp {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

/// Renders the search box's text (or placeholder) with IME marked-range
/// underlining, plus the cursor/selection quads. Ported from
/// `examples/gpui_spike_input.rs`'s `TextElement`.
struct TextElement {
    input: Entity<IssenApp>,
}

struct PrepaintState {
    line: Option<ShapedLine>,
    cursor: Option<PaintQuad>,
    selection: Option<PaintQuad>,
}

impl IntoElement for TextElement {
    type Element = Self;
    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for TextElement {
    type RequestLayoutState = ();
    type PrepaintState = PrepaintState;

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&gpui::InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        let mut style = Style::default();
        style.size.width = gpui::relative(1.).into();
        style.size.height = window.line_height().into();
        (window.request_layout(style, [], cx), ())
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&gpui::InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        let input = self.input.read(cx);
        let content = input.query.clone();
        let selected_range = input.selected_range.clone();
        let cursor = input.cursor_offset();
        let style = window.text_style();

        let (display_text, text_color) = if content.is_empty() {
            (
                SharedString::from(input.strings.search_hint),
                hsla(0., 0., 1., 0.4),
            )
        } else {
            (content, style.color)
        };

        let run = TextRun {
            len: display_text.len(),
            font: style.font(),
            color: text_color,
            background_color: None,
            underline: None,
            strikethrough: None,
        };
        let runs = if let Some(marked_range) = input.marked_range.as_ref() {
            vec![
                TextRun {
                    len: marked_range.start,
                    ..run.clone()
                },
                TextRun {
                    len: marked_range.end - marked_range.start,
                    underline: Some(UnderlineStyle {
                        color: Some(run.color),
                        thickness: px(1.0),
                        wavy: false,
                    }),
                    ..run.clone()
                },
                TextRun {
                    len: display_text.len() - marked_range.end,
                    ..run
                },
            ]
            .into_iter()
            .filter(|run| run.len > 0)
            .collect()
        } else {
            vec![run]
        };

        let font_size = style.font_size.to_pixels(window.rem_size());
        let line = window
            .text_system()
            .shape_line(display_text, font_size, &runs, None);

        let cursor_pos = line.x_for_index(cursor);
        let (selection, cursor) = if selected_range.is_empty() {
            (
                None,
                Some(fill(
                    Bounds::new(
                        point(bounds.left() + cursor_pos, bounds.top()),
                        size(px(2.), bounds.bottom() - bounds.top()),
                    ),
                    ui_chrome::accent_color(self.input.read(cx).config.accent_color),
                )),
            )
        } else {
            (
                Some(fill(
                    Bounds::from_corners(
                        point(
                            bounds.left() + line.x_for_index(selected_range.start),
                            bounds.top(),
                        ),
                        point(
                            bounds.left() + line.x_for_index(selected_range.end),
                            bounds.bottom(),
                        ),
                    ),
                    hsla(0.6, 0.9, 0.6, 0.25),
                )),
                None,
            )
        };
        PrepaintState {
            line: Some(line),
            cursor,
            selection,
        }
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&gpui::InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        let focus_handle = self.input.read(cx).focus_handle.clone();
        window.handle_input(
            &focus_handle,
            ElementInputHandler::new(bounds, self.input.clone()),
            cx,
        );
        if let Some(selection) = prepaint.selection.take() {
            window.paint_quad(selection)
        }
        let line = prepaint.line.take().unwrap();
        line.paint(
            bounds.origin,
            window.line_height(),
            gpui::TextAlign::Left,
            None,
            window,
            cx,
        )
        .unwrap();

        if focus_handle.is_focused(window) {
            if let Some(cursor) = prepaint.cursor.take() {
                window.paint_quad(cursor);
            }
        }

        self.input.update(cx, |input, _cx| {
            input.last_layout = Some(line);
            input.last_bounds = Some(bounds);
        });
    }
}

impl Render for IssenApp {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.sync_window_height(window);

        let accent = ui_chrome::accent_color(self.config.accent_color);
        let glass_bg = hsla(220. / 360., 0.12, 0.09, 0.88);
        let visible_rows_cap = self.visible_rows_cap();

        let search_box = div()
            .flex_none()
            .w_full()
            .h(px(MAIN_WINDOW_SIZE.1))
            // Right-clicking anywhere in the input row (background, text
            // field, or toolbar buttons — a button's own `Left`-only
            // listener doesn't intercept `Right`) opens the Settings/
            // Reindex/Quit menu, matching the egui/eframe version's shared
            // `show_main_context_menu` (attached to both its drag
            // background and the `TextEdit` response).
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(Self::open_main_context_menu),
            )
            .child(
                // With `with_decorations(false)` there's no OS title bar to
                // drag, so the input row doubles as one — an
                // absolutely-positioned, full-row background marked as a
                // `WindowControlArea::Drag` hit-test region. A sibling of the
                // padded content row below (not its parent, and not padded
                // itself), so `inset_0` spans the row's true full width
                // rather than just its content box; painted first so the
                // accent bar/text field/toolbar buttons (painted after, thus
                // on top) still claim their own clicks — this only ends up
                // "hit" in the gaps between them. Matches the egui/eframe
                // version's `drag_rect`/`drag-bg`, sized to just this row so
                // it can't compete with clicking a result row below.
                div()
                    .absolute()
                    .inset_0()
                    .window_control_area(WindowControlArea::Drag),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .size_full()
                    .px(px(CONTENT_PADDING))
                    .gap_2()
                    .child(div().w(px(3.)).h(px(28.)).rounded(px(1.5)).bg(accent))
                    .child(
                        div()
                            .flex_1()
                            // GPUI's hit-test walks every overlapping hitbox
                            // front-to-back and only stops at one marked
                            // `BlockMouse` (`Window::hit_test`) — without
                            // `.occlude()` here, a click anywhere on this
                            // div would *also* still count as landing on the
                            // background drag-bg behind it (same rect,
                            // unconditionally marked `WindowControlArea::
                            // Drag`), so typed-text drag-to-select would
                            // double as a window-drag on every click.
                            // `.occlude()` stops the hit-test from reaching
                            // that far, leaving only this div's own
                            // (conditional) drag registration in play.
                            .occlude()
                            .cursor(CursorStyle::IBeam)
                            .text_color(white())
                            .text_size(px(20.))
                            .line_height(px(28.))
                            // `TextEdit`-equivalent drag normally means
                            // left-drag selects text, so the window can't be
                            // dragged from on top of it. When the query is
                            // empty there's no selectable text, so this div
                            // doubles as a drag region too (matches the
                            // egui/eframe version's `response.
                            // drag_started_by` repurposing) — while typed
                            // text exists, only the background above still
                            // drags.
                            .when(self.query.is_empty(), |d| {
                                d.window_control_area(WindowControlArea::Drag)
                            })
                            // Mouse handlers for text selection are scoped to
                            // just this div (not the top-level container
                            // below) so clicking a result row doesn't also
                            // register as a text-selection mouse-down on the
                            // search box.
                            .on_mouse_down(MouseButton::Left, cx.listener(Self::on_mouse_down))
                            .on_mouse_up(MouseButton::Left, cx.listener(Self::on_mouse_up))
                            .on_mouse_up_out(MouseButton::Left, cx.listener(Self::on_mouse_up))
                            .on_mouse_move(cx.listener(Self::on_mouse_move))
                            .child(TextElement { input: cx.entity() }),
                    )
                    .child(Self::toolbar_icon_button(
                        "toolbar-color-picker",
                        "\u{1F3A8}",
                        cx.listener(|this, _, _, cx| this.toggle_tool(ToolKind::ColorPicker, cx)),
                    ))
                    .child(Self::toolbar_icon_button(
                        "toolbar-unit-converter",
                        "\u{1F4D0}",
                        cx.listener(|this, _, _, cx| this.toggle_tool(ToolKind::UnitConverter, cx)),
                    ))
                    .child(Self::toolbar_icon_button(
                        "toolbar-history",
                        "\u{1F558}",
                        cx.listener(|this, _, _, cx| this.toggle_history_panel(cx)),
                    )),
            );

        let rows: Vec<AnyElement> = if self.history_panel_open {
            if self.history.queries.is_empty() {
                vec![div()
                    .flex_none()
                    .flex()
                    .items_center()
                    .h(px(RESULT_ROW_HEIGHT))
                    .px(px(CONTENT_PADDING + 10.0))
                    .text_color(hsla(0., 0., 1., 0.5))
                    .child(self.strings.history_empty)
                    .into_any_element()]
            } else {
                self.history
                    .queries
                    .iter()
                    .enumerate()
                    // `sync_window_height` sizes the window for at most
                    // `visible_rows_cap` history rows — without this cap,
                    // GPUI's default `flex-shrink: 1` on `flex_col` children
                    // compresses every row to fit whatever's actually
                    // rendered, making row height (and thus the whole list)
                    // visibly wobble as the history count crosses that cap.
                    .take(visible_rows_cap)
                    .map(|(i, query)| {
                        div()
                            .id(("history-row", i))
                            .flex_none()
                            .flex()
                            .items_center()
                            .h(px(RESULT_ROW_HEIGHT))
                            .px(px(CONTENT_PADDING + 10.0))
                            .cursor_pointer()
                            .hover(|d| d.bg(hsla(0., 0., 1., 0.09)))
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(move |this, _, _, cx| this.use_history_query(i, cx)),
                            )
                            .text_color(white())
                            .child(query.clone())
                            .into_any_element()
                    })
                    .collect()
            }
        } else {
            self.results
                .iter()
                .enumerate()
                // See the history-row `.take` above — `sync_window_height`
                // sizes the window for at most `visible_rows_cap` rows, so
                // rendering more than that would let `flex_col`'s default
                // shrink behavior compress every row to fit, bobbing the
                // whole list (and the search box above it, per `flex_none`
                // there) up and down on every keystroke as the result count
                // changes.
                .take(visible_rows_cap)
                .map(|(i, result)| {
                    let is_selected = i == self.selected;
                    let is_pinned = result.score >= crate::history::HISTORY_SCORE_BOOST;
                    let hint = if i == 0 {
                        "\u{23ce}".to_string()
                    } else if i < visible_rows_cap {
                        format!("Alt+{}", i + 1)
                    } else {
                        String::new()
                    };
                    div()
                        .id(("result-row", i))
                        .flex_none()
                        .flex()
                        .items_center()
                        .h(px(RESULT_ROW_HEIGHT))
                        .px(px(CONTENT_PADDING + 10.0))
                        .gap_2()
                        .when(is_selected, |d| d.bg(hsla(0., 0., 1., 0.09)))
                        .cursor_pointer()
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(move |this, _, window, cx| {
                                this.selected = i;
                                this.run_result_action(ResultActionKind::Default, window, cx);
                            }),
                        )
                        .on_mouse_down(
                            MouseButton::Right,
                            cx.listener(move |this, event: &MouseDownEvent, _, cx| {
                                this.selected = i;
                                this.context_menu = Some(OpenContextMenu {
                                    target: ContextMenuTarget::Row(i),
                                    position: event.position,
                                });
                                cx.notify();
                            }),
                        )
                        .child(if is_pinned {
                            div()
                                .text_size(px(11.))
                                .text_color(accent)
                                .child("\u{1F4CC}")
                        } else {
                            div()
                        })
                        .child(
                            div()
                                .flex_1()
                                .overflow_hidden()
                                .flex()
                                .items_center()
                                .gap_2()
                                .child(
                                    div()
                                        .text_color(if is_selected { accent } else { white() })
                                        .child(result.title.clone()),
                                )
                                .child(
                                    div()
                                        .text_color(hsla(0., 0., 1., 0.55))
                                        .text_size(px(12.))
                                        .child(result.subtitle.clone()),
                                ),
                        )
                        .child(
                            div()
                                .text_size(px(10.))
                                .text_color(hsla(0., 0., 1., 0.5))
                                .child(hint),
                        )
                        .into_any_element()
                })
                .collect()
        };

        let context_menu_overlay = self.render_context_menu(cx);

        div()
            .key_context(KEY_CONTEXT)
            .track_focus(&self.focus_handle)
            .on_action(cx.listener(Self::move_up))
            .on_action(cx.listener(Self::move_down))
            .on_action(cx.listener(Self::confirm))
            .on_action(cx.listener(Self::run_as_admin_action))
            .on_action(cx.listener(Self::open_location_action))
            .on_action(cx.listener(Self::escape_action))
            .on_action(cx.listener(Self::alt_num_1))
            .on_action(cx.listener(Self::alt_num_2))
            .on_action(cx.listener(Self::alt_num_3))
            .on_action(cx.listener(Self::alt_num_4))
            .on_action(cx.listener(Self::alt_num_5))
            .on_action(cx.listener(Self::alt_num_6))
            .on_action(cx.listener(Self::alt_num_7))
            .on_action(cx.listener(Self::alt_num_8))
            .on_action(cx.listener(Self::alt_num_9))
            .on_action(cx.listener(Self::text_backspace))
            .on_action(cx.listener(Self::text_delete))
            .on_action(cx.listener(Self::text_left))
            .on_action(cx.listener(Self::text_right))
            .on_action(cx.listener(Self::select_left))
            .on_action(cx.listener(Self::select_right))
            .on_action(cx.listener(Self::select_all))
            .on_action(cx.listener(Self::home))
            .on_action(cx.listener(Self::end))
            .on_action(cx.listener(Self::paste))
            .on_action(cx.listener(Self::cut))
            .on_action(cx.listener(Self::copy))
            .size_full()
            .flex()
            .flex_col()
            .bg(glass_bg)
            .rounded(px(0.))
            .border_1()
            .border_color(hsla(0., 0., 1., 0.14))
            .child(search_box)
            .children(rows)
            .children(context_menu_overlay)
    }
}

// --- Win32 helpers ---

/// `Window` has an inherent `window_handle()` method (returns GPUI's own
/// `AnyWindowHandle`, not an HWND) with the same name as the
/// `HasWindowHandle` trait method, so the trait method must be called
/// fully-qualified to get the actual HWND.
fn window_hwnd(window: &Window) -> Option<HWND> {
    let handle = HasWindowHandle::window_handle(window).ok()?;
    match handle.as_raw() {
        RawWindowHandle::Win32(handle) => Some(HWND(handle.hwnd.get() as *mut core::ffi::c_void)),
        _ => None,
    }
}

/// Excludes the window from Alt+Tab and the taskbar (needed since the
/// window is permanently OS-visible — see `hide`'s doc comment).
fn set_tool_window_style(hwnd: HWND) {
    unsafe {
        let current = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
        let _ = SetWindowLongPtrW(hwnd, GWL_EXSTYLE, current | (WS_EX_TOOLWINDOW.0 as isize));
    }
}

fn move_window(hwnd: HWND, x: i32, y: i32) {
    unsafe {
        let _ = SetWindowPos(
            hwnd,
            None,
            x,
            y,
            0,
            0,
            SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE,
        );
    }
}

/// App entry point, called from `main.rs`.
pub fn run(config: Config) {
    let strings = Strings::for_lang(i18n::resolve(config.language));
    let tray = TrayHandle::new(strings).expect("failed to create tray icon");
    let (hotkey, hotkey_rx) = HotkeyListener::spawn(config.hotkey.clone());
    let menu_rx = tray::take_menu_event_receiver();
    let icon_rx = tray::take_tray_icon_event_receiver();

    application().run(move |cx: &mut App| {
        cx.bind_keys([
            KeyBinding::new("up", MoveUp, Some(KEY_CONTEXT)),
            KeyBinding::new("down", MoveDown, Some(KEY_CONTEXT)),
            KeyBinding::new("enter", Confirm, Some(KEY_CONTEXT)),
            KeyBinding::new("ctrl-shift-enter", RunAsAdminAction, Some(KEY_CONTEXT)),
            KeyBinding::new("ctrl-shift-e", OpenLocationAction, Some(KEY_CONTEXT)),
            KeyBinding::new("escape", EscapeAction, Some(KEY_CONTEXT)),
            KeyBinding::new("alt-1", AltNum1, Some(KEY_CONTEXT)),
            KeyBinding::new("alt-2", AltNum2, Some(KEY_CONTEXT)),
            KeyBinding::new("alt-3", AltNum3, Some(KEY_CONTEXT)),
            KeyBinding::new("alt-4", AltNum4, Some(KEY_CONTEXT)),
            KeyBinding::new("alt-5", AltNum5, Some(KEY_CONTEXT)),
            KeyBinding::new("alt-6", AltNum6, Some(KEY_CONTEXT)),
            KeyBinding::new("alt-7", AltNum7, Some(KEY_CONTEXT)),
            KeyBinding::new("alt-8", AltNum8, Some(KEY_CONTEXT)),
            KeyBinding::new("alt-9", AltNum9, Some(KEY_CONTEXT)),
            KeyBinding::new("backspace", Backspace, Some(KEY_CONTEXT)),
            KeyBinding::new("delete", Delete, Some(KEY_CONTEXT)),
            KeyBinding::new("left", Left, Some(KEY_CONTEXT)),
            KeyBinding::new("right", Right, Some(KEY_CONTEXT)),
            KeyBinding::new("shift-left", SelectLeft, Some(KEY_CONTEXT)),
            KeyBinding::new("shift-right", SelectRight, Some(KEY_CONTEXT)),
            KeyBinding::new("ctrl-a", SelectAll, Some(KEY_CONTEXT)),
            KeyBinding::new("home", Home, Some(KEY_CONTEXT)),
            KeyBinding::new("end", End, Some(KEY_CONTEXT)),
            KeyBinding::new("ctrl-v", Paste, Some(KEY_CONTEXT)),
            KeyBinding::new("ctrl-x", Cut, Some(KEY_CONTEXT)),
            KeyBinding::new("ctrl-c", Copy, Some(KEY_CONTEXT)),
        ]);
        cx.bind_keys(crate::text_input::key_bindings());

        let bounds = Bounds {
            origin: gpui::point(px(OFFSCREEN_POSITION.0), px(OFFSCREEN_POSITION.1)),
            size: size(px(MAIN_WINDOW_SIZE.0), px(MAIN_WINDOW_SIZE.1)),
        };

        let window_handle = cx
            .open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(bounds)),
                    titlebar: None,
                    focus: true,
                    show: true,
                    kind: WindowKind::PopUp,
                    // `true` so the search box's own drag region
                    // (`WindowControlArea::Drag`, see `render`'s
                    // `search_box`) can actually move the window — on
                    // Windows, `is_movable: false` disables the OS-level
                    // `HTCAPTION` hit-test regardless of what the app marks
                    // as a drag area (`gpui_windows`'s `handle_hit_test_msg`).
                    is_movable: true,
                    window_background: WindowBackgroundAppearance::Transparent,
                    ..Default::default()
                },
                {
                    let config = config.clone();
                    move |window, cx| cx.new(|cx| IssenApp::new(window, cx, config, hotkey, tray))
                },
            )
            .expect("failed to open main window");

        // Event-driven hotkey/tray wake-up: each of these tasks blocks on its
        // channel (no `request_animation_frame` polling — see the module doc
        // comment) and, once woken, updates the window and lets GPUI's own
        // render scheduling take it from there.
        {
            let mut hotkey_rx = hotkey_rx;
            cx.spawn(async move |cx| {
                while hotkey_rx.next().await.is_some() {
                    let _ = window_handle.update(cx, |view, window, cx| view.show(window, cx));
                }
            })
            .detach();
        }
        if let Some(mut menu_rx) = menu_rx {
            cx.spawn(async move |cx| {
                while let Some(event) = menu_rx.next().await {
                    let _ = window_handle.update(cx, |view, window, cx| {
                        if let Some(action) = view.tray.match_menu_action(&event) {
                            view.handle_tray_action(action, window, cx);
                        }
                    });
                }
            })
            .detach();
        }
        if let Some(mut icon_rx) = icon_rx {
            cx.spawn(async move |cx| {
                while let Some(event) = icon_rx.next().await {
                    if let Some(action) = TrayHandle::match_tray_icon_action(&event) {
                        let _ = window_handle.update(cx, |view, window, cx| {
                            view.handle_tray_action(action, window, cx)
                        });
                    }
                }
            })
            .detach();
        }

        // Shows the main window at startup via the real `show()` path (OS
        // activation, `visible = true`, the focus-loss observer's guard
        // reset — everything a real hotkey press does), for verifying
        // show()-dependent behavior (focus-on-show, focus-loss auto-hide,
        // drag) without needing to synthesize the actual global hotkey.
        if std::env::var_os("ISSEN_DEBUG_SHOW").is_some() {
            let _ = window_handle.update(cx, |view, window, cx| view.show(window, cx));
        }

        // Opens the about window at startup, via the same tray-action path a
        // real click takes, for manual visual verification without needing
        // tray-icon UI automation.
        if std::env::var_os("ISSEN_DEBUG_OPEN_ABOUT").is_some() {
            let _ = window_handle.update(cx, |view, window, cx| {
                view.handle_tray_action(TrayAction::About, window, cx)
            });
        }
        if std::env::var_os("ISSEN_DEBUG_OPEN_SETTINGS").is_some() {
            let _ = window_handle.update(cx, |view, window, cx| {
                view.handle_tray_action(TrayAction::Settings, window, cx)
            });
        }
        // Tools has no tray menu entry (only reachable via the search box's
        // toolbar buttons in normal use), so this is the only way to open
        // it for verification without driving mouse input.
        let debug_tool = match std::env::var("ISSEN_DEBUG_OPEN_TOOL").as_deref() {
            Ok("color-picker") => Some(ToolKind::ColorPicker),
            Ok("unit-converter") => Some(ToolKind::UnitConverter),
            _ => None,
        };
        if let Some(kind) = debug_tool {
            let _ = window_handle.update(cx, |view, window, cx| {
                view.show(window, cx);
                view.toggle_tool(kind, cx);
            });
        }

        cx.activate(true);
    });
}
