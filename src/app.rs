use std::time::{Duration, Instant};

use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use windows::Win32::Foundation::{COLORREF, HWND, RECT};
use windows::Win32::UI::WindowsAndMessaging::{
    GetWindowLongPtrW, GetWindowRect, SetLayeredWindowAttributes, SetWindowLongPtrW, GWL_EXSTYLE,
    LWA_ALPHA, WS_EX_LAYERED, WS_EX_TOOLWINDOW,
};

use crate::config::{self, Config, Theme};
use crate::hotkey::HotkeyListener;
use crate::i18n::{self, Lang, Strings};
use crate::search::alias::AliasProvider;
use crate::search::app_index::{AppIndexProvider, IndexScan, ScanConfig};
use crate::search::everything::EverythingProvider;
use crate::search::plugin::PluginProvider;
use crate::search::windows_settings::WindowsSettingsProvider;
use crate::search::{Action, SearchProvider, SearchResult};
use crate::settings_window::SettingsWindow;
use crate::tray::{TrayAction, TrayHandle};

/// Interval between periodic automatic rescans (see docs/architecture/search.md).
const PERIODIC_RESCAN_INTERVAL: Duration = Duration::from_secs(30 * 60);

/// Main window size. Must match `main.rs`'s `ViewportBuilder::with_inner_size`
/// (used when computing where to center the window on the cursor's display when shown).
/// This is also the window's height with zero search results (the window grows
/// downward from this height as results appear).
pub const MAIN_WINDOW_SIZE: (f32, f32) = (640.0, 60.0);

/// The coordinate the main window is actually moved to when logically "hidden".
/// A fixed point outside every monitor's bounds (`-32000` is avoided because Windows
/// treats it as a sentinel value for a minimized window's position). `main.rs` also
/// uses this as the window's initial position at startup (see
/// docs/architecture/window-lifecycle.md, and `set_visible`'s doc comment).
pub const OFFSCREEN_POSITION: (f32, f32) = (-8000.0, -8000.0);

/// Startup layered-window priming (see docs/architecture/window-lifecycle.md).
/// This is how long to wait after launch before invisibly warping the
/// window to its real display position and back, consuming the "first time this
/// window is actually composited onto a monitor after process start" moment before
/// the real (user-triggered) reveal, so the DWM white placeholder never appears
/// during a real reveal.
const LAYERED_PRIME_ARM_DELAY_MS: u64 = 800;
/// How many frames to stay invisible at the real position, giving DWM enough time to
/// actually finish compositing.
const LAYERED_PRIME_HOLD_FRAMES: u8 = 5;
/// Upper bound, in frames, on how long to wait for `GetWindowRect` to confirm the
/// move back off-screen (a safety valve). This should normally resolve in 1-2
/// frames; the cap exists so the window can't get stuck invisible indefinitely if
/// confirmation is never observed for some reason.
const LAYERED_PRIME_OFFSCREEN_TIMEOUT_FRAMES: u8 = 30;

/// Progress state for startup layered-window priming.
///
/// The core lesson baked into this state machine: when restoring opacity after the
/// off-screen move, the code must explicitly confirm via `GetWindowRect` that the
/// queued `ViewportCommand::OuterPosition` move has actually landed *before*
/// restoring opacity, rather than assuming an order between it and the immediate
/// Win32 opacity call. An earlier version sent "restore opacity" and "move
/// off-screen" (queued, applied later by egui/winit) without that confirmation step,
/// and hit a rare race where opacity was restored just before the position change
/// had actually taken effect — reproducing, on real hardware, the exact flicker this
/// priming step exists to prevent. Two operations that travel through different
/// paths with different timing (an immediate Win32 call vs. a queued
/// `ViewportCommand`) need an explicit confirmation between them, not an assumed
/// order. (Purely off-screen approaches — toggling `ViewportCommand::Transparent`,
/// re-invoking `DwmEnableBlurBehindWindow`, or a full-size off-screen-only prime —
/// were also tried and had no effect on the real on-screen flicker, which is why
/// priming has to actually move the window on-screen rather than manipulate it
/// while off-screen.)
enum LayeredPrimeState {
    /// Waiting since startup (value is the deadline).
    Waiting(Instant),
    /// Just made invisible (alpha 0); next step is to move to the real position.
    MoveOnScreen,
    /// Counting down the frames spent invisible at the real position.
    Hold(u8),
    /// Just issued the move off-screen; next step is to confirm arrival via `GetWindowRect`.
    MoveOffscreen,
    /// Polling `GetWindowRect` for arrival off-screen (value is remaining timeout frames).
    WaitOffscreen(u8),
    /// Finished (no further action). Also reached if a real `set_visible(true)` cuts
    /// priming short partway through.
    Done,
}

/// How long `ui()` spends fade-in/scale animating, purely on the rendering side,
/// based on elapsed time since `set_visible(true)`. Implemented via
/// `egui::Context::set_transform_layer`, a pure post-transform on the render layer —
/// it never touches the window's actual size/position (`OuterPosition`/`InnerSize`)
/// or any OS-level Show/Hide transition, so it can't reintroduce the white-flash
/// issue those transitions caused (see "Resident process & display model").
const SHOW_ANIM_DURATION: Duration = Duration::from_millis(140);
/// The scale the animation starts at, growing to 1.0.
const SHOW_ANIM_SCALE_FROM: f32 = 0.92;

/// Height of one dropdown-style search-result row. Used both when dynamically
/// resizing the window (`sync_window_height`) and when drawing result rows (`ui()`),
/// so the number of rows actually drawn always matches the window height sent to the OS.
const RESULT_ROW_HEIGHT: f32 = 40.0;

/// Cap on how many result rows are shown/sized for at once without scrolling, so the
/// window never grows off-screen. `config.max_results` (the settings window's
/// "maximum results shown" field) can't be set above this (see the `DragValue` range
/// in `settings_window.rs`). Candidates beyond this cap are still kept, up to
/// `RESULT_RETENTION_CAP`, and reachable by scrolling the `egui::ScrollArea`.
pub(crate) const MAX_VISIBLE_ROWS: usize = 8;

/// Cap on how many search results are retained for scrolling. Independent of
/// `config.max_results` (the visible-without-scrolling count, itself capped at
/// `MAX_VISIBLE_ROWS`) — this bounds how many fuzzy-matched candidates are kept at
/// all. Left unbounded, sorting and drawing cost would grow without limit on every
/// keystroke on a large index, so it's cut off at a fixed value.
const RESULT_RETENTION_CAP: usize = 50;

/// Extra window height added only while the right-click context menu
/// (`egui::Response::context_menu`) is open. The main window is a single
/// undecorated OS window (`with_decorations(false)`), and the menu (an
/// `Order::Foreground` layer) can't physically render outside that window's own
/// pixel bounds. With an empty query (window height = `MAIN_WINDOW_SIZE.1` only),
/// right-clicking the input box would otherwise open a menu (up to 5 items + a
/// separator) taller than the available 60px and get visibly cut off.
/// `ctx.any_popup_open()` detects the open menu and this headroom is added only
/// while it's open.
const CONTEXT_MENU_HEADROOM: f32 = 220.0;

/// Highlight for the selected result row. Kept translucent so it blends with the
/// `ui_chrome::glass_panel` glass background underneath (an opaque color would break
/// the glass look).
const RESULT_SELECTED_BG: egui::Color32 =
    egui::Color32::from_rgba_unmultiplied_const(255, 255, 255, 24);
/// Inner padding from the main window's left edge, used to fit
/// `ui_chrome::glass_panel`'s rounded corner and the input box's accent bar.
const CONTENT_PADDING: f32 = 16.0;
/// Width reserved out of the title/subtitle display area for the `Alt+N` hint chip
/// drawn on the right side of each result row. Sized a bit wider than the longest
/// chip (`Alt+9`).
const HINT_CHIP_RESERVED_WIDTH: f32 = 60.0;

enum ResultActionKind {
    Default,
    RunAsAdmin,
    OpenLocation,
    CopyPath,
}

/// An action triggered by clicking a result row or its right-click menu. Deferred
/// rather than run inside `ui()`'s results `for` loop — see `pending_row_action`'s
/// doc comment for why.
enum RowMenuAction {
    Run(ResultActionKind),
    RegisterAlias,
    Pin,
    Unpin,
}

pub struct IssenApp {
    config: Config,
    lang: Lang,
    strings: &'static Strings,
    query: String,
    results: Vec<SearchResult>,
    selected: usize,
    /// Changed every time `run_search` runs. Used as the `egui::ScrollArea` ID salt
    /// for the results area, so scroll position resets to the top on every new query
    /// (relying on egui's standard "a changed ID means a fresh scroll state" behavior).
    search_generation: u64,
    /// Set only right after a key action (arrow keys, Alt+digit) changes `selected`.
    /// While set, `ui()` calls `Response::scroll_to_me` on the selected row to pull a
    /// selection that scrolled out of view back into range. Not set for mouse-click
    /// selection (already visible, since it had to be clicked).
    pending_scroll_to_selected: bool,
    /// Open/closed state of the query-history panel (opened via the history icon
    /// next to the input box). While open, `show_history_panel` draws in place of
    /// the normal search results. Closes automatically when the query is edited
    /// (`response.changed()`), the icon is clicked again, or the window is hidden.
    history_panel_open: bool,
    /// The app's *intended* visibility — whether the window is logically at its real
    /// on-screen position or parked at `OFFSCREEN_POSITION`. Since the window is
    /// always OS-visible (`with_visible(true)`), this does not correspond to
    /// `IsWindowVisible` (see `set_visible`'s doc comment).
    visible: bool,
    /// The instant `set_visible(true)` was last called. Until `SHOW_ANIM_DURATION`
    /// has elapsed, `ui()` uses this to drive the fade-in/scale animation (see
    /// `SHOW_ANIM_DURATION`'s doc comment).
    show_anim_start: Option<Instant>,
    /// The last `config.hotkey` value actually applied to `HotkeyListener`. Guards
    /// `update_hotkey` so it's only called when the value changes (same pattern as
    /// `apply_language`).
    last_hotkey_spec: String,
    /// The last `config.font_scale` value applied to `egui::Style`. Guards
    /// `ctx.style_mut` calls (which trigger font-atlas relayout) so they only happen
    /// when the value actually changes.
    applied_font_scale: f32,
    /// The last `config.ui_font` value applied via `crate::fonts::apply_fonts`.
    /// Guarded the same way as `applied_font_scale`, since it also rebuilds the font atlas.
    content_height: f32,
    /// The window height last sent to (or that should be sent to) the OS. Guards
    /// `ViewportCommand::InnerSize` so it's only sent when the value changes (see
    /// `sync_window_height`).
    applied_ui_font: crate::config::UiFont,
    hotkey: HotkeyListener,
    tray: TrayHandle,
    app_index: AppIndexProvider,
    /// Persisted state for the "usage-based pinning" feature that boosts previously
    /// chosen results back to the top (`src/history.rs`). Saved to `history.toml`,
    /// separate from `config.toml` (see `History`'s doc comment).
    history: crate::history::History,
    /// Loaded once at startup from `%APPDATA%\Issen\plugins` (DLL loading has real
    /// I/O cost, so unlike `app_index`'s background rescans, this doesn't happen on
    /// every keystroke). Manual/periodic rescans (`start_scan`) only cover the app
    /// index — plugins aren't reloaded (hot-reload is out of scope for now; see
    /// docs/architecture/plugins.md).
    plugins: PluginProvider,
    pending_scan: Option<IndexScan>,
    next_periodic_scan: Instant,
    last_scan_finished: Option<Instant>,
    last_scan_count: usize,
    settings: SettingsWindow,
    about_open: bool,
    tools: crate::tools::ToolsState,
    /// When the tray's "Settings"/"About" is clicked while the main window is
    /// hidden, actually opening it (`settings.open()`/`about_open = true`) is
    /// deferred by one frame — see `handle_tray_action`'s doc comment for why.
    pending_open: Option<PendingOpen>,
    /// Test hook: while `ISSEN_DEBUG_AUTO_CYCLE_MS` is set, repeatedly toggles
    /// `set_visible` at a fixed interval (show -> hide -> show -> ...) without using
    /// any real keyboard/hotkey automation. Exists so an external screen-capture
    /// harness can mechanically trigger "bugs that only reproduce right after
    /// showing" many times over. `(toggle interval, next toggle time)`. Has no
    /// effect on normal operation while `None`. Not a permanent feature.
    debug_auto_cycle: Option<(Duration, Instant)>,
    /// Test hook: while `ISSEN_DEBUG_VISIBLE_DELAY_MS` is set, a background thread
    /// sleeps for the given number of milliseconds after startup, then sends one
    /// notification and calls `ctx.request_repaint()` — mimicking the same "an
    /// external thread wakes a sleeping event loop" path `HotkeyListener` uses on a
    /// real hotkey press. `logic()` only calls `set_visible(true)` when it actually
    /// receives this notification via `try_recv`; the rest of the time it doesn't
    /// call `request_repaint_after` at all, so the loop genuinely sleeps (a timed
    /// poll entirely inside `logic()` would keep waking the loop while waiting,
    /// which wouldn't reproduce the real hotkey path of "an external thread wakes a
    /// sleeping loop"). Used to verify whether the first-ever reveal still flickers
    /// even after a long delay post-launch. Resets to `None` once fired, after which
    /// it has no further effect. Not a permanent feature.
    debug_delayed_show: Option<std::sync::mpsc::Receiver<()>>,
    /// The main window's HWND. Already obtained in `new()`, but kept as a field
    /// because startup layered-window priming (`tick_layered_prime`) needs to call
    /// raw Win32 APIs every frame from `logic()`.
    main_hwnd: HWND,
    /// Progress state for startup layered-window priming (see `LayeredPrimeState`).
    layered_prime: LayeredPrimeState,
}

#[derive(Clone, Copy)]
enum PendingOpen {
    Settings,
    About,
}

impl IssenApp {
    pub fn new(cc: &eframe::CreationContext<'_>, config: Config) -> Self {
        let main_hwnd = window_hwnd(cc).expect("failed to obtain main window HWND");
        // Exclude from Alt+Tab (needed once the window became permanently OS-visible;
        // see `with_taskbar(false)`'s doc comment in `main.rs`). `with_taskbar(false)`
        // only removes the taskbar entry, not the Alt+Tab entry, so the extended
        // window style is set directly here.
        set_tool_window_style(main_hwnd);

        // eframe's bundled fonts have no CJK glyphs, so Windows-bundled fonts are
        // added as a fallback to render Japanese app names in search results and
        // Japanese UI strings.
        crate::fonts::apply_fonts(&cc.egui_ctx, config.ui_font);
        apply_theme(&cc.egui_ctx, config.theme);

        let lang = i18n::resolve(config.language);
        let strings = Strings::for_lang(lang);

        let hotkey = HotkeyListener::spawn(cc.egui_ctx.clone(), config.hotkey.clone());
        let last_hotkey_spec = config.hotkey.clone();
        let tray = TrayHandle::new(&cc.egui_ctx, strings).expect("failed to create tray icon");
        let pending_scan = IndexScan::spawn(ScanConfig::from_config(&config));
        if pending_scan.is_some() {
            tray.set_scanning(strings, true);
        }

        // Hidden windows and secondary viewports are hard to verify via normal
        // screenshot-based testing, so debug environment variables control startup
        // state for that purpose (unset by default = normal behavior).
        let mut settings = SettingsWindow::new(&config);
        let mut about_open = false;
        if std::env::var_os("ISSEN_DEBUG_OPEN_SETTINGS").is_some() {
            settings.open();
        }
        if std::env::var_os("ISSEN_DEBUG_OPEN_ABOUT").is_some() {
            about_open = true;
        }
        let mut tools = crate::tools::ToolsState::default();
        match std::env::var("ISSEN_DEBUG_OPEN_TOOL").as_deref() {
            Ok("color-picker") => tools.toggle(crate::tools::ToolKind::ColorPicker),
            Ok("unit-converter") => tools.toggle(crate::tools::ToolKind::UnitConverter),
            _ => {}
        }

        let mut app = Self {
            strings,
            lang,
            query: String::new(),
            results: Vec::new(),
            selected: 0,
            search_generation: 0,
            pending_scroll_to_selected: false,
            history_panel_open: false,
            // Initial visibility from the `ISSEN_DEBUG_VISIBLE` debug variable is
            // applied by starting `false` here and calling `set_visible(true)` after
            // construction (below), so it correctly passes through `set_visible`'s
            // "only send a command when the value changes" guard.
            visible: false,
            show_anim_start: None,
            content_height: MAIN_WINDOW_SIZE.1,
            last_hotkey_spec,
            // `NAN` never compares `< EPSILON` against any value, so it acts as a
            // sentinel that guarantees `apply_font_scale`'s diff guard always passes
            // through on the very first call after startup. (If `config.toml` has a
            // saved `font_scale` other than `1.0`, initializing this field to
            // `config.font_scale` directly would make the startup guard see "no
            // change" and skip applying the scale, leaving the UI at the default size.)
            applied_font_scale: f32::NAN,
            // Unlike `applied_font_scale`'s NAN sentinel, this starts as
            // `config.ui_font` itself, since the `apply_fonts` call at the top of
            // `new()` has already applied it — the initial value can correctly
            // reflect "already applied" here.
            applied_ui_font: config.ui_font,
            hotkey,
            tray,
            app_index: AppIndexProvider::empty(),
            history: crate::history::History::load_or_default(config::APP_NAME),
            plugins: PluginProvider::load_from_app_data(),
            pending_scan,
            next_periodic_scan: Instant::now() + PERIODIC_RESCAN_INTERVAL,
            last_scan_finished: None,
            last_scan_count: 0,
            settings,
            about_open,
            tools,
            pending_open: None,
            config,
            debug_auto_cycle: std::env::var("ISSEN_DEBUG_AUTO_CYCLE_MS")
                .ok()
                .and_then(|s| s.parse::<u64>().ok())
                .map(|ms| {
                    let interval = Duration::from_millis(ms);
                    (interval, Instant::now() + interval)
                }),
            debug_delayed_show: std::env::var("ISSEN_DEBUG_VISIBLE_DELAY_MS")
                .ok()
                .and_then(|s| s.parse::<u64>().ok())
                .map(|ms| {
                    let (tx, rx) = std::sync::mpsc::channel();
                    let ctx = cc.egui_ctx.clone();
                    std::thread::spawn(move || {
                        std::thread::sleep(Duration::from_millis(ms));
                        let _ = tx.send(());
                        ctx.request_repaint();
                    });
                    rx
                }),
            main_hwnd,
            // While `ISSEN_DEBUG_NO_LAYERED_PRIME` (test-only) is set, priming itself
            // is skipped entirely — used to record a control group when comparing
            // against other rejected mitigations.
            layered_prime: if std::env::var_os("ISSEN_DEBUG_NO_LAYERED_PRIME").is_some() {
                LayeredPrimeState::Done
            } else {
                LayeredPrimeState::Waiting(
                    Instant::now() + Duration::from_millis(LAYERED_PRIME_ARM_DELAY_MS),
                )
            },
        };
        if let Ok(q) = std::env::var("ISSEN_DEBUG_QUERY") {
            app.query = q;
            app.run_search();
        }
        if std::env::var_os("ISSEN_DEBUG_VISIBLE").is_some() {
            app.set_visible(&cc.egui_ctx, true);
        }
        app
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
        // Usage-based pinning: just adds a large boost (`HISTORY_SCORE_BOOST`) to the
        // cross-provider fuzzy score, so a candidate the query no longer matches
        // (and so isn't in `results` at all) is never force-surfaced.
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
        // Bump the scroll area's ID on every query change so a new query always
        // starts scrolled to the top rather than inheriting the previous manual
        // scroll position (see `ScrollArea::id_salt` in `ui()`).
        self.search_generation = self.search_generation.wrapping_add(1);
    }

    /// Entry point for both manual and periodic rescans. No-op if a scan is already
    /// running (prevents overlapping scans).
    fn start_scan(&mut self) {
        if self.pending_scan.is_some() {
            return;
        }
        self.pending_scan = IndexScan::spawn(ScanConfig::from_config(&self.config));
        if self.pending_scan.is_some() {
            self.tray.set_scanning(self.strings, true);
        }
    }

    fn poll_scan(&mut self) {
        let Some(scan) = &self.pending_scan else {
            return;
        };
        if let Some(provider) = scan.try_recv() {
            self.last_scan_count = provider.len();
            self.last_scan_finished = Some(Instant::now());
            self.app_index = provider;
            self.pending_scan = None;
            self.tray.set_scanning(self.strings, false);
            self.next_periodic_scan = Instant::now() + PERIODIC_RESCAN_INTERVAL;
            if !self.query.is_empty() {
                self.run_search();
            }
        }
    }

    fn run_result_action(&mut self, ctx: &egui::Context, kind: ResultActionKind) {
        let Some(result) = self.results.get(self.selected) else {
            return;
        };

        match kind {
            ResultActionKind::Default => {
                // Clipboard-copy results (calculator, snippets, and other plugin
                // results) are entirely query-dependent, one-off values, so they're
                // excluded from pinning history.
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
                    // Query-string history (the list reachable via the history icon
                    // next to the input box, for re-running a past search). Must be
                    // recorded here, before `set_visible` clears `self.query`.
                    if !self.query.trim().is_empty() {
                        self.history.record_query(&self.query);
                        history_dirty = true;
                    }
                    if history_dirty {
                        if let Err(err) = self.history.save(config::APP_NAME) {
                            eprintln!("issen: failed to save history.toml: {err}");
                        }
                    }
                    self.set_visible(ctx, false);
                }
            }
            ResultActionKind::RunAsAdmin => {
                if let Action::Launch { path, args } = &result.action {
                    if crate::launch::open_elevated(&path.display().to_string(), args) {
                        self.set_visible(ctx, false);
                    }
                }
            }
            ResultActionKind::OpenLocation => {
                if let Action::Launch { path, .. } = &result.action {
                    crate::launch::open_containing_folder(path);
                    self.set_visible(ctx, false);
                }
            }
            ResultActionKind::CopyPath => {
                if let Action::Launch { path, .. } = &result.action {
                    crate::launch::copy_to_clipboard(&path.display().to_string());
                }
            }
        }
    }

    /// The single entry point for toggling window visibility. Sends its OS command
    /// immediately when called, so it takes effect reliably regardless of whether
    /// `logic()` or `ui()` (including a mouse click) triggered it. (An earlier
    /// version compared the value at the start and end of a frame and sent a command
    /// only if it differed — a change made in `ui()` or late in `logic()` could miss
    /// that comparison window, leaving the window visible when it should have hidden,
    /// most noticeably when a launch succeeded without the window ever losing focus.)
    ///
    /// **Never uses OS-level Show/Hide** (see `with_visible(true)`'s doc comment in
    /// `main.rs`). The main window stays OS-visible at all times; visibility is
    /// represented by moving it, via `OuterPosition`, between its real on-screen
    /// position and `OFFSCREEN_POSITION` (outside every monitor's bounds).
    /// `ViewportCommand::Visible` is never sent.
    fn set_visible(&mut self, ctx: &egui::Context, visible: bool) {
        if self.visible == visible {
            return;
        }
        self.visible = visible;
        if visible {
            // If startup layered-window priming (see `LayeredPrimeState`) is
            // mid-flight (window currently invisible), a real show request takes
            // priority and restores opacity immediately. It's harmless if priming's
            // remaining steps (e.g. a queued position move) get overwritten after this.
            self.finish_layered_prime_immediately();
            // Start of `ui()`'s fade-in/scale animation (`SHOW_ANIM_DURATION`). While
            // `ISSEN_DEBUG_NO_SHOW_ANIM` is set, the animation itself doesn't start
            // (`ui()` always takes the same branch as the non-animated case) — used
            // to isolate whether a visual glitch comes from the animation code
            // (`set_transform_layer`/`set_opacity`) itself or from the
            // off-screen/on-screen position-swap trick. Test-only, not a permanent feature.
            if std::env::var_os("ISSEN_DEBUG_NO_SHOW_ANIM").is_none() {
                self.show_anim_start = Some(Instant::now());
            }
            // In case an OS call like cursor-position lookup fails, always fall back
            // through primary display -> a fixed coordinate, so showing always lands
            // somewhere on-screen. (Sending only `Focus` while still at
            // `OFFSCREEN_POSITION` would leave an invisible window off-screen that
            // silently steals keyboard focus — once "show" is decided, the window
            // must always move on-screen.) If `ISSEN_DEBUG_FORCE_MONITOR_POINT`
            // (`"x,y"`, test-only) is set, target whichever display that point falls
            // on regardless of the actual cursor position — lets a screen-capture
            // test predict coordinates in advance.
            let target = self.resolve_show_target();
            ctx.send_viewport_cmd(egui::ViewportCommand::OuterPosition(egui::pos2(
                target.0, target.1,
            )));
            ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
        } else {
            ctx.send_viewport_cmd(egui::ViewportCommand::OuterPosition(egui::pos2(
                OFFSCREEN_POSITION.0,
                OFFSCREEN_POSITION.1,
            )));
            // The next show's `OuterPosition` is always computed relative to
            // `MAIN_WINDOW_SIZE` (the zero-results height) — see the
            // `position_on_*_monitor` call sites. Leaving the window at whatever
            // height it grew to from search results would break that assumption, so
            // the search state is folded away along with hiding.
            self.query.clear();
            self.results.clear();
            self.selected = 0;
            self.history_panel_open = false;
            self.set_window_height(ctx, MAIN_WINDOW_SIZE.1);
        }
    }

    /// Computes where on the target display the window should go. Both
    /// `set_visible(true)` and startup layered-window priming
    /// (`tick_layered_prime`'s `MoveOnScreen` step) need "where would we show right
    /// now" computed by the same logic, hence this shared function.
    fn resolve_show_target(&self) -> (f32, f32) {
        // Falls back through primary display -> a fixed coordinate in case an OS
        // call like cursor-position lookup fails. If
        // `ISSEN_DEBUG_FORCE_MONITOR_POINT` (`"x,y"`, test-only) is set, targets
        // whichever display that point falls on regardless of the actual cursor
        // position, so a screen-capture test can predict coordinates in advance.
        let forced_point = std::env::var("ISSEN_DEBUG_FORCE_MONITOR_POINT")
            .ok()
            .and_then(|s| {
                let (x, y) = s.split_once(',')?;
                Some((x.trim().parse::<i32>().ok()?, y.trim().parse::<i32>().ok()?))
            });
        if let Some(point) = forced_point {
            crate::display::position_on_point_monitor(point, MAIN_WINDOW_SIZE)
        } else {
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
        }
        .or_else(|| crate::display::position_on_primary_monitor(MAIN_WINDOW_SIZE))
        .unwrap_or((100.0, 100.0))
    }

    /// Cuts short whatever's left of startup layered-window priming (see
    /// `LayeredPrimeState`) and immediately restores full opacity (alpha 255). An
    /// escape hatch preventing the window from staying invisible if a real show
    /// request (`set_visible(true)`) interrupts priming mid-flight. No-op if
    /// priming has already finished (`Done`).
    fn finish_layered_prime_immediately(&mut self) {
        if !matches!(self.layered_prime, LayeredPrimeState::Done) {
            exit_layered(self.main_hwnd);
            self.layered_prime = LayeredPrimeState::Done;
        }
    }

    /// Advances startup layered-window priming by one frame. Called every frame from
    /// `logic()` (a no-op, immediate return, once `LayeredPrimeState::Done` — no
    /// effect on normal operation at that point).
    fn tick_layered_prime(&mut self, ctx: &egui::Context) {
        match self.layered_prime {
            LayeredPrimeState::Done => {}
            LayeredPrimeState::Waiting(deadline) => {
                if Instant::now() >= deadline {
                    // Go invisible before moving the position. With this order, the
                    // window stays invisible the whole time until the `MoveOnScreen`
                    // move to the real position is actually processed — the worst
                    // case if that's delayed is simply "sits invisible at the old
                    // (off-screen) position a little longer," which is visually a no-op.
                    enter_layered_invisible(self.main_hwnd);
                    self.layered_prime = LayeredPrimeState::MoveOnScreen;
                }
                ctx.request_repaint_after(Duration::from_millis(16));
            }
            LayeredPrimeState::MoveOnScreen => {
                // Test-only hook: if `ISSEN_DEBUG_FORCE_PRIME_POINT` (`"x,y"`) is set,
                // redirect only the priming warp target there. `resolve_show_target()`
                // is shared with the real (user-facing) show target, so
                // `ISSEN_DEBUG_FORCE_MONITOR_POINT` alone can't put the priming target
                // and the real show target on different monitors. This hook made that
                // A/B test possible; pinning the priming target to a different
                // monitor than the real show target did not reproduce the white
                // flicker there either, disconfirming a "each monitor has its own
                // independent first composite" hypothesis. Falls back to
                // `resolve_show_target()` when unset (no effect on real behavior).
                // Kept permanently for future regression testing, same as the other
                // `ISSEN_DEBUG_*` hooks.
                let target = std::env::var("ISSEN_DEBUG_FORCE_PRIME_POINT")
                    .ok()
                    .and_then(|s| {
                        let (x, y) = s.split_once(',')?;
                        Some((x.trim().parse::<i32>().ok()?, y.trim().parse::<i32>().ok()?))
                    })
                    .and_then(|point| {
                        crate::display::position_on_point_monitor(point, MAIN_WINDOW_SIZE)
                    })
                    .unwrap_or_else(|| self.resolve_show_target());
                ctx.send_viewport_cmd(egui::ViewportCommand::OuterPosition(egui::pos2(
                    target.0, target.1,
                )));
                self.layered_prime = LayeredPrimeState::Hold(LAYERED_PRIME_HOLD_FRAMES);
                ctx.request_repaint();
            }
            LayeredPrimeState::Hold(remaining) => {
                if let Some(next) = remaining.checked_sub(1) {
                    self.layered_prime = LayeredPrimeState::Hold(next);
                } else {
                    self.layered_prime = LayeredPrimeState::MoveOffscreen;
                }
                ctx.request_repaint();
            }
            LayeredPrimeState::MoveOffscreen => {
                // The critical ordering: send "move off-screen" before restoring
                // opacity, and only actually restore opacity in the next state
                // (`WaitOffscreen`) once arrival is confirmed. This avoids ever
                // exposing full opacity on-screen between the two operations.
                ctx.send_viewport_cmd(egui::ViewportCommand::OuterPosition(egui::pos2(
                    OFFSCREEN_POSITION.0,
                    OFFSCREEN_POSITION.1,
                )));
                self.layered_prime =
                    LayeredPrimeState::WaitOffscreen(LAYERED_PRIME_OFFSCREEN_TIMEOUT_FRAMES);
                ctx.request_repaint();
            }
            LayeredPrimeState::WaitOffscreen(remaining) => {
                if window_is_offscreen(self.main_hwnd) {
                    exit_layered(self.main_hwnd);
                    self.layered_prime = LayeredPrimeState::Done;
                } else if let Some(next) = remaining.checked_sub(1) {
                    self.layered_prime = LayeredPrimeState::WaitOffscreen(next);
                    ctx.request_repaint();
                } else {
                    // Safety valve: if arrival off-screen still can't be confirmed
                    // within the expected number of frames, restoring opacity is
                    // preferred over leaving the window invisible indefinitely.
                    exit_layered(self.main_hwnd);
                    self.layered_prime = LayeredPrimeState::Done;
                }
            }
        }
    }

    /// The single entry point for sending the window's actual height to the OS, only
    /// when it changes (same "send only on change" guard pattern as `set_visible`).
    /// Used to grow/shrink the window downward, dropdown-style, as results change.
    fn set_window_height(&mut self, ctx: &egui::Context, height: f32) {
        if (self.content_height - height).abs() < f32::EPSILON {
            return;
        }
        self.content_height = height;
        ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(egui::vec2(
            MAIN_WINDOW_SIZE.0,
            height,
        )));
    }

    /// Syncs window height to the current result count. Safe to call every frame
    /// from `ui()` (`set_window_height` only sends a command when the value
    /// changes). Adds `CONTEXT_MENU_HEADROOM` while the right-click context menu is
    /// open, so the menu doesn't get cut off (see `CONTEXT_MENU_HEADROOM`'s comment).
    /// Because `ViewportCommand::InnerSize` doesn't apply until the next frame, the
    /// window can lag one frame behind right after the menu opens — judged low-impact.
    fn sync_window_height(&mut self, ctx: &egui::Context) {
        // While the history panel is open, size to the history row count instead of
        // the search results (`show_history_panel`). Even with zero history entries,
        // one row ("no history yet") is still drawn, hence `max(1)`.
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
        if ctx.any_popup_open() {
            height += CONTEXT_MENU_HEADROOM;
        }
        self.set_window_height(ctx, height);
    }

    /// Number of result rows shown without scrolling. The settings window's
    /// `DragValue` can't set `config.max_results` above `MAX_VISIBLE_ROWS`, but this
    /// still clamps defensively in case an older `config.toml` on disk has a larger
    /// value from before that cap existed.
    fn visible_rows_cap(&self) -> usize {
        (self.config.max_results as usize).min(MAX_VISIBLE_ROWS)
    }

    fn register_selected_as_alias(&mut self) {
        let Some(result) = self.results.get(self.selected) else {
            return;
        };
        let target = crate::search::target_key(&result.action);
        self.settings.prefill_alias(result.title.clone(), target);
    }

    /// Rebuilds runtime state that depends on `language` (display strings, tray
    /// icon) when it changes. A cheap no-op when nothing changed, so it's fine to
    /// call from `logic()` every frame (needed to reflect a settings-window change
    /// immediately).
    fn apply_language(&mut self, ctx: &egui::Context) {
        let lang = i18n::resolve(self.config.language);
        if lang == self.lang {
            return;
        }
        self.lang = lang;
        self.strings = Strings::for_lang(lang);
        // Keep the existing tray icon if rebuilding fails (its labels stay in the
        // old language, but that's better than crashing the whole app).
        if let Some(tray) = TrayHandle::new(ctx, self.strings) {
            self.tray = tray;
            // A freshly rebuilt tray always starts with idle labels/tooltip; if a
            // scan is in progress when the language switch happens, reapply that state.
            if self.pending_scan.is_some() {
                self.tray.set_scanning(self.strings, true);
            }
        } else {
            eprintln!("issen: failed to rebuild tray icon after language change; keeping old one");
        }
    }

    /// The input box's right-click menu. Shared by both the text area itself and the
    /// blank space above/below it (the drag region, `drag-bg`) so either can open
    /// the same menu.
    fn show_main_context_menu(&mut self, ui: &mut egui::Ui) {
        if ui.button(self.strings.tray_settings).clicked() {
            self.settings.open();
            ui.close();
        }
        if ui.button(self.strings.tray_reindex).clicked() {
            self.start_scan();
            ui.close();
        }
        ui.separator();
        if ui.button(self.strings.tray_quit).clicked() {
            // See `handle_tray_action`'s `TrayAction::Quit` doc comment: `ViewportCommand::Close`
            // is unreliable while the main window is hidden, so this exits directly instead.
            std::process::exit(0);
        }
    }

    /// The list of past search queries opened via the history icon (🕘) next to the
    /// input box. Visually and structurally close to the normal result list
    /// (`ui()`), but simplified — no Alt+N hint chips or action menu, and no
    /// keyboard selection yet (mouse-click re-search is enough for now; YAGNI).
    /// Clicking a row replaces `self.query` with that query and re-runs search
    /// (doesn't execute anything directly — it's a "re-search," not a "re-run").
    fn show_history_panel(&mut self, ui: &mut egui::Ui) {
        if self.history.queries.is_empty() {
            ui.horizontal(|ui| {
                ui.set_min_height(RESULT_ROW_HEIGHT);
                ui.add_space(CONTENT_PADDING + 10.0);
                ui.weak(self.strings.history_empty);
            });
            return;
        }

        let visible_rows_cap = self.visible_rows_cap();
        let scroll_height =
            self.history.queries.len().min(visible_rows_cap) as f32 * RESULT_ROW_HEIGHT;
        let mut chosen: Option<usize> = None;
        egui::ScrollArea::vertical()
            .id_salt("history-scroll")
            .max_height(scroll_height)
            .auto_shrink([false, true])
            .show(ui, |ui| {
                for (i, query) in self.history.queries.iter().enumerate() {
                    let row_id = ui.id().with(("history_row", i));
                    let row = egui::Frame::default()
                        .corner_radius(10.0)
                        .inner_margin(egui::Margin::symmetric(CONTENT_PADDING as i8 + 10, 0))
                        .show(ui, |ui| {
                            ui.horizontal(|ui| {
                                ui.set_min_height(RESULT_ROW_HEIGHT);
                                ui.label(query);
                            });
                        })
                        .response;
                    let row = ui
                        .interact(row.rect, row_id, egui::Sense::click())
                        .on_hover_cursor(egui::CursorIcon::PointingHand);
                    if row.clicked() {
                        chosen = Some(i);
                    }
                }
            });

        // Click detection happens outside the scroll area's closure, then
        // `self.history` is read afterward — same reason as `pending_row_action` in
        // the results list (don't mutate data still borrowed by an in-progress loop).
        if let Some(i) = chosen {
            self.query = self.history.queries[i].clone();
            self.history_panel_open = false;
            self.run_search();
        }
    }

    /// Tells `HotkeyListener` to live-reregister when `config.hotkey` changes. A
    /// cheap no-op when nothing changed, so safe to call every frame from `logic()`
    /// (same pattern as `apply_language`). Handling of invalid strings (including
    /// partial input while typing in the settings window) is left to
    /// `HotkeyListener` (see `hotkey.rs::spawn`'s doc comment).
    fn apply_hotkey_change(&mut self) {
        if self.config.hotkey == self.last_hotkey_spec {
            return;
        }
        self.last_hotkey_spec = self.config.hotkey.clone();
        self.hotkey.update_hotkey(self.last_hotkey_spec.clone());
    }

    /// Rescales `egui::Style::text_styles`'s font sizes when `config.font_scale`
    /// changes. A no-op when nothing changed (same guard pattern as
    /// `apply_language`; unlike `apply_theme`, this isn't called unconditionally
    /// every frame, since it triggers font-atlas relayout). Always scales from
    /// `egui::Style::default()`'s base sizes rather than the current style, so
    /// repeated changes don't compound the scale factor.
    fn apply_font_scale(&mut self, ctx: &egui::Context) {
        if (self.config.font_scale - self.applied_font_scale).abs() < f32::EPSILON {
            return;
        }
        self.applied_font_scale = self.config.font_scale;
        let scale = self.applied_font_scale;
        let base = egui::Style::default();
        // Apply the same scale to both the light and dark `Style` (`egui::Context`
        // keeps a separate `Style` per theme).
        ctx.all_styles_mut(|style| {
            for (text_style, base_font_id) in &base.text_styles {
                if let Some(font_id) = style.text_styles.get_mut(text_style) {
                    font_id.size = base_font_id.size * scale;
                }
            }
        });
    }

    /// Re-invokes `crate::fonts::apply_fonts` only when `config.ui_font` actually
    /// changes (same guard pattern as `apply_font_scale`, since it also rebuilds the
    /// font atlas and shouldn't run every frame).
    fn apply_ui_font(&mut self, ctx: &egui::Context) {
        if self.config.ui_font == self.applied_ui_font {
            return;
        }
        self.applied_ui_font = self.config.ui_font;
        crate::fonts::apply_fonts(ctx, self.applied_ui_font);
    }

    /// The tray's "Settings"/"About" actions are the only path that can be triggered
    /// while the main window is still hidden (every other route to settings/about —
    /// the right-click menu, registering an alias — only works while the main window
    /// is already visible). This doesn't call `self.settings.open()`/
    /// `self.about_open = true` immediately because, while the main window is
    /// hidden, `eframe` takes a special path for it (`check_redraw_requests` calls
    /// `ui()` directly for windows that never get a `RedrawRequested` event while
    /// hidden — a workaround for emilk/egui#5229) that lacks the "current event
    /// loop" context a viewport needs to be created. Constructing the settings/about
    /// viewport for the first time via `show_viewport_immediate` from inside that
    /// path crashes with "egui backend is implemented incorrectly - the user
    /// callback was never called" (reproduced on real hardware). Instead,
    /// `set_visible(true)` schedules the main window to become visible, and actually
    /// opening the settings/about window is deferred via `pending_open` to the start
    /// of the *next* frame's `logic()`. By then `Visible(true)` — well, the position
    /// move that makes it visible — has already been applied on the OS side, so
    /// `ui()` runs through its normal path (with a proper event-loop context) and
    /// can safely construct the new viewport.
    fn handle_tray_action(&mut self, ctx: &egui::Context, action: TrayAction) {
        match action {
            TrayAction::Open => {
                self.set_visible(ctx, true);
            }
            TrayAction::Settings => {
                self.set_visible(ctx, true);
                self.pending_open = Some(PendingOpen::Settings);
            }
            TrayAction::Reindex => {
                self.start_scan();
            }
            TrayAction::About => {
                self.set_visible(ctx, true);
                self.pending_open = Some(PendingOpen::About);
            } // Quit has no variant here — see `tray.rs`'s `ensure_event_forwarding` doc
              // comment for why it's handled directly in the `MenuEvent` callback instead.
        }
    }
}

/// Gets the main window's HWND from `CreationContext`. `eframe::CreationContext`
/// implements `raw_window_handle::HasWindowHandle`, and by this point the OS window
/// has already been created (this runs inside `main.rs::run_native`'s callback).
fn window_hwnd(cc: &eframe::CreationContext<'_>) -> Option<HWND> {
    let handle = cc.window_handle().ok()?;
    match handle.as_raw() {
        RawWindowHandle::Win32(handle) => Some(HWND(handle.hwnd.get() as *mut core::ffi::c_void)),
        _ => None,
    }
}

/// Because the main window stays permanently OS-visible under the resident/display
/// model (see `set_visible`'s doc comment), it would otherwise keep showing up as an
/// Alt+Tab candidate (`main.rs`'s `with_taskbar(false)` only removes the taskbar
/// entry, not the Alt+Tab one). Setting the `WS_EX_TOOLWINDOW` extended window style
/// excludes it from Alt+Tab regardless of visibility or on-screen/off-screen
/// position. This bit only needs to be set once after window creation, so it's
/// called once from `new()`.
fn set_tool_window_style(hwnd: HWND) {
    unsafe {
        let current = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
        let _ = SetWindowLongPtrW(hwnd, GWL_EXSTYLE, current | WS_EX_TOOLWINDOW.0 as isize);
    }
}

/// The "go invisible" step of startup layered-window priming (`LayeredPrimeState`).
/// Temporarily sets `WS_EX_LAYERED` and alpha 0.
///
/// An earlier design left `WS_EX_LAYERED` set permanently once turned on, but
/// real-hardware testing found a separate issue that design didn't catch: during the
/// priming window, the real white DWM placeholder was exposed regardless of alpha —
/// static-presence testing alone only covers "how it looks during normal display,"
/// not "the very first real on-screen composite after process start." This function
/// goes back to toggling `WS_EX_LAYERED` on only for the duration it's needed,
/// cleared by `exit_layered` once the window is confirmed off-screen (see
/// `LayeredPrimeState::WaitOffscreen`) — the ordering fix described in
/// `LayeredPrimeState`'s doc comment.
fn enter_layered_invisible(hwnd: HWND) {
    unsafe {
        let current = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
        let _ = SetWindowLongPtrW(hwnd, GWL_EXSTYLE, current | WS_EX_LAYERED.0 as isize);
        let _ = SetLayeredWindowAttributes(hwnd, COLORREF(0), 0, LWA_ALPHA);
    }
}

/// The counterpart to `enter_layered_invisible`: clears the `WS_EX_LAYERED` bit,
/// returning to a normal (always-opaque) window. Callers (`LayeredPrimeState::WaitOffscreen`)
/// only call this after `window_is_offscreen` confirms arrival off-screen, avoiding
/// the ordering bug of restoring opacity while still on-screen (see
/// `LayeredPrimeState`'s doc comment).
fn exit_layered(hwnd: HWND) {
    unsafe {
        let current = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
        let _ = SetWindowLongPtrW(hwnd, GWL_EXSTYLE, current & !(WS_EX_LAYERED.0 as isize));
    }
}

/// Confirms via `GetWindowRect` whether the window has actually (at the OS level)
/// moved to roughly `OFFSCREEN_POSITION`. `ViewportCommand::OuterPosition` applies
/// with a delay through egui/winit's command queue, so the frame right after sending
/// it may still observe the old position (see `LayeredPrimeState::WaitOffscreen`'s
/// doc comment). Rather than requiring an exact match with `OFFSCREEN_POSITION`,
/// this checks against a threshold (`-4000`) that normal monitor layouts would never
/// reach.
fn window_is_offscreen(hwnd: HWND) -> bool {
    let mut rect = RECT::default();
    let ok = unsafe { GetWindowRect(hwnd, &mut rect) }.is_ok();
    ok && rect.left < -4000
}

fn apply_theme(ctx: &egui::Context, theme: Theme) {
    let preference = match theme {
        Theme::Light => egui::ThemePreference::Light,
        Theme::Dark => egui::ThemePreference::Dark,
        Theme::System => egui::ThemePreference::System,
    };
    ctx.set_theme(preference);
}

impl eframe::App for IssenApp {
    /// `eframe`'s default `clear_color` (a translucent solid dark gray) would leave
    /// a faint tint outside `ui_chrome::glass_panel`'s rounded corners, muddying what
    /// should be fully transparent. Using full transparency (alpha 0) instead leaves
    /// the panel's look entirely up to `glass_panel`'s own drawing (verified on real
    /// hardware; see docs/architecture/ui-appearance.md).
    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        [0.0, 0.0, 0.0, 0.0]
    }

    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Consume whatever `handle_tray_action` queued in the previous frame, once,
        // at the very start of this function (before tray-event handling below).
        // Keeping the consume and the enqueue in separate places like this ensures a
        // pending action newly queued in this same frame doesn't get processed
        // within the same frame it was queued (see `handle_tray_action`'s doc comment).
        if let Some(pending) = self.pending_open.take() {
            match pending {
                PendingOpen::Settings => self.settings.open(),
                PendingOpen::About => self.about_open = true,
            }
        }

        if self.hotkey.try_recv_toggle() {
            self.set_visible(ctx, true);
        }

        self.tick_layered_prime(ctx);

        // Test-only auto-show toggle (see `debug_auto_cycle`'s doc comment).
        if let Some((interval, next_toggle)) = self.debug_auto_cycle {
            if Instant::now() >= next_toggle {
                let now_visible = self.visible;
                self.set_visible(ctx, !now_visible);
                self.debug_auto_cycle = Some((interval, Instant::now() + interval));
            }
            ctx.request_repaint_after(Duration::from_millis(16));
        }

        // Test-only delayed first show (see `debug_delayed_show`'s doc comment).
        // Never calls `request_repaint_after` while waiting, so the loop genuinely sleeps.
        if let Some(rx) = &self.debug_delayed_show {
            if rx.try_recv().is_ok() {
                self.set_visible(ctx, true);
                self.debug_delayed_show = None;
            }
        }

        // `i.focused` doesn't update while hidden — it stays at the last visible
        // frame's value (which is itself the reason it went hidden, so checking it
        // here would immediately re-hide the window one frame after showing it). To
        // catch only the actual "moment" focus is lost, this checks the
        // `WindowFocused(false)` event rather than polling.
        let lost_focus = ctx.input(|i| {
            i.events
                .iter()
                .any(|event| matches!(event, egui::Event::WindowFocused(false)))
        });
        if lost_focus && !self.settings.is_open() && !self.about_open && !self.tools.is_open() {
            self.set_visible(ctx, false);
        }

        // Esc closes the main window. Checked before the history-panel early return
        // below so it still works while that panel is open.
        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.set_visible(ctx, false);
        }

        if let Some(action) = self.tray.try_recv_action() {
            self.handle_tray_action(ctx, action);
        }

        self.poll_scan();
        if self.settings.take_rescan_requested() {
            self.start_scan();
        }
        if self.settings.take_save_requested() {
            if let Err(err) = self.config.save(config::APP_NAME) {
                eprintln!("issen: failed to save config.toml: {err}");
            }
        }
        self.apply_language(ctx);
        self.apply_hotkey_change();
        self.apply_font_scale(ctx);
        self.apply_ui_font(ctx);
        if self.pending_scan.is_none() && Instant::now() >= self.next_periodic_scan {
            self.start_scan();
        }
        // Redraw every frame for the duration of the fade-in/scale animation
        // (`SHOW_ANIM_DURATION`). Requesting a short interval here composes fine
        // with the `request_repaint_after(next_wake)` call below, since egui takes
        // the minimum of multiple requests within a frame.
        if self
            .show_anim_start
            .is_some_and(|start| start.elapsed() < SHOW_ANIM_DURATION)
        {
            ctx.request_repaint();
        }

        // Wake up frequently while a scan is running so completion is caught
        // promptly; while idle, sleep in one jump until the next periodic rescan is
        // due (a scheduled wake via a timer, not polling).
        let next_wake = if self.pending_scan.is_some() {
            Duration::from_millis(150)
        } else {
            self.next_periodic_scan
                .saturating_duration_since(Instant::now())
                .max(Duration::from_secs(1))
        };
        ctx.request_repaint_after(next_wake);

        // While the query-history panel (🕘) is open, suspend keyboard handling for
        // the normal search results (up/down, Enter, Alt+digit). `self.results`
        // stays stale (from the last real search) while the panel is open, so acting
        // on it here would run something unrelated to what's actually on screen.
        if self.history_panel_open {
            return;
        }

        let (up, down) = ctx.input(|i| {
            (
                i.key_pressed(egui::Key::ArrowUp),
                i.key_pressed(egui::Key::ArrowDown),
            )
        });
        if down && !self.results.is_empty() {
            self.selected = (self.selected + 1).min(self.results.len() - 1);
            self.pending_scroll_to_selected = true;
        }
        if up {
            self.selected = self.selected.saturating_sub(1);
            self.pending_scroll_to_selected = true;
        }

        let (enter, admin, open_location, copy_path) = ctx.input(|i| {
            let enter_pressed = i.key_pressed(egui::Key::Enter);
            let mods = i.modifiers;
            (
                enter_pressed && !(mods.ctrl && mods.shift),
                enter_pressed && mods.ctrl && mods.shift,
                mods.ctrl && mods.shift && i.key_pressed(egui::Key::E),
                mods.ctrl && !mods.shift && i.key_pressed(egui::Key::C),
            )
        });
        if admin {
            self.run_result_action(ctx, ResultActionKind::RunAsAdmin);
        } else if enter {
            self.run_result_action(ctx, ResultActionKind::Default);
        }
        if open_location {
            self.run_result_action(ctx, ResultActionKind::OpenLocation);
        }
        if copy_path {
            self.run_result_action(ctx, ResultActionKind::CopyPath);
        }

        // Alt+2 through Alt+9 select and run a visible result directly (see
        // docs/architecture/hotkey-input.md). The global hotkey `Alt+Space` is a
        // separate OS-level path via `RegisterHotKey` (`src/hotkey.rs`), so it can't
        // conflict with these in-app `Alt+digit` shortcuts. This range deliberately
        // ignores scroll position, covering only the first `visible_rows_cap()` rows
        // visible without scrolling (following the scrolled viewport would add
        // complexity for little benefit). The first row is already reachable via
        // `Enter` alone, so while pressing `Alt+1` is harmless given how
        // `DIGIT_KEYS` is laid out, no distinct meaning is assigned to it.
        const DIGIT_KEYS: [egui::Key; 9] = [
            egui::Key::Num1,
            egui::Key::Num2,
            egui::Key::Num3,
            egui::Key::Num4,
            egui::Key::Num5,
            egui::Key::Num6,
            egui::Key::Num7,
            egui::Key::Num8,
            egui::Key::Num9,
        ];
        let alt_digit = ctx.input(|i| {
            if !i.modifiers.alt {
                return None;
            }
            DIGIT_KEYS.iter().position(|key| i.key_pressed(*key))
        });
        if let Some(idx) = alt_digit {
            let visible_rows = self.results.len().min(self.visible_rows_cap());
            if idx < visible_rows {
                self.selected = idx;
                self.run_result_action(ctx, ResultActionKind::Default);
            }
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();

        egui::CentralPanel::default()
            .frame(egui::Frame::default().fill(egui::Color32::TRANSPARENT))
            .show(ui, |ui| {
                // The input box always stays on a fixed dark, glass-style background
                // (see docs/architecture/window-lifecycle.md), so text
                // color is forced to a light palette even in light theme — a
                // theme-derived dark text color would blend into the fixed dark
                // background and become unreadable.
                *ui.visuals_mut() = egui::Visuals::dark();

                let full_rect = ui.available_rect_before_wrap();

                // Show animation (fade-in + scale). See `SHOW_ANIM_DURATION`'s doc
                // comment. Because `set_visible` resets search state (and thus zero
                // results) on every show, `full_rect`'s height at this point is
                // always just the input row (`MAIN_WINDOW_SIZE.1`), and the pivot
                // for scaling is its center.
                let anim_t = self.show_anim_start.map(|start| {
                    (start.elapsed().as_secs_f32() / SHOW_ANIM_DURATION.as_secs_f32()).min(1.0)
                });
                if let Some(t) = anim_t {
                    // Ease-out cubic: fast start, smooth settle.
                    let eased = 1.0 - (1.0 - t).powi(3);
                    ui.set_opacity(eased);
                    let scale = SHOW_ANIM_SCALE_FROM + (1.0 - SHOW_ANIM_SCALE_FROM) * eased;
                    let pivot = egui::pos2(
                        full_rect.center().x,
                        full_rect.top() + MAIN_WINDOW_SIZE.1 / 2.0,
                    );
                    let transform =
                        egui::emath::TSTransform::new(pivot.to_vec2() * (1.0 - scale), scale);
                    ctx.set_transform_layer(egui::LayerId::background(), transform);
                    if t >= 1.0 {
                        self.show_anim_start = None;
                    }
                } else {
                    ctx.set_transform_layer(
                        egui::LayerId::background(),
                        egui::emath::TSTransform::IDENTITY,
                    );
                }

                let accent = crate::ui_chrome::accent_color(self.config.accent_color);
                let glass = crate::ui_chrome::palette(true);
                crate::ui_chrome::glass_panel(ui, full_rect, &glass);
                // `sync_window_height` computes window height as input row
                // (`MAIN_WINDOW_SIZE.1`) + row count * `RESULT_ROW_HEIGHT`, without
                // accounting for `item_spacing` between rows. Left at egui's default
                // vertical spacing, that gap would accumulate and push the last row
                // out of the computed height, so vertical spacing alone is zeroed to
                // pack rows with no gap (horizontal spacing is left alone — the
                // toolbar buttons need it).
                ui.spacing_mut().item_spacing.y = 0.0;

                // `with_decorations(false)` (`main.rs`) means there's no OS title
                // bar to drag, so the input row (`MAIN_WINDOW_SIZE.1` tall, acting as
                // a pseudo title bar) doubles as the drag handle wherever it isn't
                // covered by the text field or buttons (the `TextEdit` is vertically
                // centered, leaving thin margins above and below). The search-results
                // dropdown area (the part `sync_window_height` grows) is deliberately
                // excluded — including it would let the background's
                // `Sense::drag()` region claim the same full-row-height space as a
                // result row's `Sense::click()`, resolving as a drag before a click
                // could launch anything. This `interact` call runs before the other
                // widgets placed over the same rect, so it doesn't interfere with
                // their own click handling (egui gives input-handling priority to
                // whichever widget on a layer was added later).
                let drag_rect = egui::Rect::from_min_size(
                    full_rect.min,
                    egui::vec2(full_rect.width(), MAIN_WINDOW_SIZE.1),
                );
                // `Sense::drag()` alone never resolves egui's internal "click" events
                // (including `secondary_clicked()`) — egui only resolves click-type
                // events for widgets whose `Sense` includes clicking. Right-click
                // needs to open the context menu too, so this uses `click_and_drag()`.
                let drag_response = ui
                    .interact(
                        drag_rect,
                        ui.id().with("drag-bg"),
                        egui::Sense::click_and_drag(),
                    )
                    // Switch the cursor to a move icon on hover/drag so the
                    // draggable area is discoverable at a glance — the background
                    // stays a plain black fill, so a static border wasn't used, just
                    // the cursor shape.
                    .on_hover_and_drag_cursor(egui::CursorIcon::Move);
                if drag_response.drag_started_by(egui::PointerButton::Primary) {
                    ui.ctx().send_viewport_cmd(egui::ViewportCommand::StartDrag);
                }
                drag_response.context_menu(|ui| self.show_main_context_menu(ui));

                // Accent bar on the input row's left edge (part of the reference
                // design). The visible fill is drawn via `painter`; layout reserves
                // the same width via `add_space`.
                let accent_bar_w = 3.0;
                let accent_bar_rect = egui::Rect::from_center_size(
                    egui::pos2(
                        full_rect.left() + CONTENT_PADDING + accent_bar_w / 2.0,
                        full_rect.top() + MAIN_WINDOW_SIZE.1 / 2.0,
                    ),
                    egui::vec2(accent_bar_w, 28.0),
                );
                for (expand, alpha) in [(9.0, 24u8), (4.0, 55)] {
                    ui.painter().rect_filled(
                        accent_bar_rect.expand(expand),
                        4.0,
                        egui::Color32::from_rgba_unmultiplied(
                            accent.r(),
                            accent.g(),
                            accent.b(),
                            alpha,
                        ),
                    );
                }
                ui.painter().rect_filled(accent_bar_rect, 1.5, accent);

                let toolbar_width = 3.0 * 28.0 + CONTENT_PADDING;
                let response = ui
                    .horizontal(|ui| {
                        // Expanding the row's minimum height to fill the whole input
                        // area lets `horizontal`'s default vertical `Align::Center`
                        // put the naturally sized `TextEdit` in the row's center
                        // (previously a fixed 40px rect was used inside the full
                        // 60px window, leaving the text drawn too high). Height is
                        // deliberately not fixed via `add_sized` — only
                        // `desired_width` constrains the width.
                        ui.set_min_height(MAIN_WINDOW_SIZE.1);
                        // Left: room for the accent bar. Right: margin from the
                        // window edge (reserved manually here since `CentralPanel`'s
                        // own margin was zeroed to let the glass-panel background
                        // paint edge-to-edge).
                        ui.add_space(CONTENT_PADDING + accent_bar_w + 14.0);
                        let input_font = egui::FontId::proportional(20.0 * self.config.font_scale);
                        let response = ui.add(
                            egui::TextEdit::singleline(&mut self.query)
                                // `hint_text` builds its own Atom for the
                                // placeholder and doesn't inherit the font set via
                                // `TextEdit::font` — egui 0.36's `AtomLayout` only
                                // applies `fallback_font` (the default
                                // `TextStyle::Body`) to an Atom with no font of its
                                // own. Without an explicit `FontId` here, the
                                // placeholder and the actual typed text end up at
                                // different sizes/baselines, so the same `FontId` is
                                // set explicitly via `RichText` to keep them matched.
                                .hint_text(
                                    egui::RichText::new(self.strings.search_hint)
                                        .font(input_font.clone()),
                                )
                                .frame(egui::Frame::NONE)
                                .font(input_font)
                                .desired_width(ui.available_width() - toolbar_width),
                        );
                        if ui
                            .button("🎨")
                            .on_hover_text(self.strings.tool_color_picker)
                            .clicked()
                        {
                            self.tools.toggle(crate::tools::ToolKind::ColorPicker);
                        }
                        if ui
                            .button("📐")
                            .on_hover_text(self.strings.tool_unit_converter)
                            .clicked()
                        {
                            self.tools.toggle(crate::tools::ToolKind::UnitConverter);
                        }
                        if ui
                            .button("🕘")
                            .on_hover_text(self.strings.tool_history)
                            .clicked()
                        {
                            self.history_panel_open = !self.history_panel_open;
                        }
                        ui.add_space(CONTENT_PADDING - 8.0);
                        response
                    })
                    .inner;
                // `TextEdit` normally claims left-drag for text selection (egui's
                // standard `Sense::click_and_drag()`), so the window usually can't be
                // dragged from on top of the text box. When the query is empty (no
                // selectable text, so drag can't conflict with text selection),
                // the `TextEdit`'s own drag detection is repurposed for window
                // dragging instead — while any query is typed, text selection still
                // takes priority as before.
                if self.query.is_empty() && response.drag_started_by(egui::PointerButton::Primary) {
                    ui.ctx().send_viewport_cmd(egui::ViewportCommand::StartDrag);
                }
                // Don't steal focus from a tool popup's (e.g. the unit converter's)
                // own text input while one is open.
                if !self.tools.is_open() {
                    response.request_focus();
                }
                response.context_menu(|ui| self.show_main_context_menu(ui));

                if response.changed() {
                    // Switch back to the normal search results, not the history
                    // panel, as soon as the query is edited (showing both search
                    // results and history at once would be confusing).
                    self.history_panel_open = false;
                    self.run_search();
                }

                if self.history_panel_open {
                    // `sync_window_height`/`apply_theme` still run once, in their
                    // usual place, after this `CentralPanel` closure returns (this
                    // `return` only exits the closure, not the whole `ui()` call).
                    // The early return here is only to skip drawing the normal
                    // search-result list below.
                    self.show_history_panel(ui);
                    return;
                }

                // The dropdown-style results area. The window's own height is sized
                // by `sync_window_height` to fit `visible_rows_cap()` rows without
                // scrolling. `self.results` itself holds more than that (up to
                // `RESULT_RETENTION_CAP`), so anything beyond the visible cap is
                // reachable via the `ScrollArea` below.
                let visible_rows_cap = self.visible_rows_cap();
                let scroll_height =
                    self.results.len().min(visible_rows_cap) as f32 * RESULT_ROW_HEIGHT;
                // `search_generation` (used as `id_salt`) changes on every query
                // change, so egui treats this as a fresh scroll area each time and
                // always starts scrolled to the top rather than keeping the previous
                // manual scroll position (see `run_search`'s comment).
                //
                // Actions triggered by clicking a result or its right-click menu
                // (running it, registering an alias, unpinning) are only queued
                // here, not executed inside this `for` loop. These actions can
                // mutate `self.results`/`self.query` via `self.run_result_action`
                // (e.g. a successful launch resets the whole search state through
                // `set_visible` when the window closes). Since `for i in
                // 0..self.results.len()` iterates up to the count captured when the
                // loop started, a mid-loop change to `self.results`'s length makes
                // the next `self.results[i]` access go out of bounds — this
                // reproduced as an actual panic
                // (`index out of bounds: the len is 0 but the index is 1`) when
                // searching for and clicking a result to launch it.
                let mut pending_row_action: Option<(usize, RowMenuAction)> = None;
                egui::ScrollArea::vertical()
                    .id_salt(("results-scroll", self.search_generation))
                    .max_height(scroll_height)
                    .auto_shrink([false, true])
                    .show(ui, |ui| {
                        for i in 0..self.results.len() {
                            let result = &self.results[i];
                            let row_id = ui.id().with(("result_row", i));
                            let is_selected = i == self.selected;
                            // `run_search` only adds `HISTORY_SCORE_BOOST` to pinned
                            // results (a value ordinary fuzzy-match scores can never
                            // reach), so checking the score magnitude here is enough
                            // — no need to recompute `target_key` and look it up in
                            // `History` again.
                            let is_pinned = result.score >= crate::history::HISTORY_SCORE_BOOST;
                            let bg = if is_selected {
                                RESULT_SELECTED_BG
                            } else {
                                egui::Color32::TRANSPARENT
                            };
                            let row = egui::Frame::default()
                                .fill(bg)
                                .corner_radius(10.0)
                                .inner_margin(egui::Margin::symmetric(
                                    CONTENT_PADDING as i8 + 10,
                                    0,
                                ))
                                .show(ui, |ui| {
                                    ui.horizontal(|ui| {
                                        // `set_min_height` must be called inside
                                        // `ui.horizontal`, not around it as part of
                                        // the outer vertical layout. Called from
                                        // outside, the vertical layout's default
                                        // top alignment (`Align::Min`) leaves the
                                        // extra height as blank space below the text
                                        // row instead, so the row's contents stay
                                        // top-aligned (misaligned against the
                                        // vertically centered selection bar drawn
                                        // through the row's middle). Same pattern as
                                        // the main search input row (this file):
                                        // calling it inside `horizontal` itself lets
                                        // the default `Align::Center` take effect.
                                        ui.set_min_height(RESULT_ROW_HEIGHT);
                                        // Reserve space so this doesn't overlap the
                                        // `Alt+N` hint chip drawn on the right (via
                                        // `painter`, using `row.rect`, after this
                                        // `Frame::show` completes).
                                        ui.set_width(
                                            ui.available_width() - HINT_CHIP_RESERVED_WIDTH,
                                        );
                                        if is_pinned {
                                            ui.label(
                                                egui::RichText::new("\u{1F4CC}")
                                                    .color(accent)
                                                    .size(11.0),
                                            );
                                        }
                                        if is_selected {
                                            ui.colored_label(accent, &result.title);
                                        } else {
                                            ui.label(&result.title);
                                        }
                                        ui.add(egui::Label::new(&result.subtitle).truncate());
                                    });
                                })
                                .response;
                            if is_selected {
                                let bar_rect = egui::Rect::from_center_size(
                                    egui::pos2(
                                        row.rect.left() + CONTENT_PADDING,
                                        row.rect.center().y,
                                    ),
                                    egui::vec2(2.0, RESULT_ROW_HEIGHT - 20.0),
                                );
                                ui.painter().rect_filled(bar_rect, 1.0, accent);
                            }
                            // Hint chip showing that `Alt+2` through `Alt+9` selects
                            // and runs this row directly (matches the Alt+digit
                            // handling in `logic()`). That shortcut ignores scroll
                            // position and only covers the visible-without-scrolling
                            // `visible_rows_cap` rows, so the chip is drawn only
                            // within that same range (see `logic()`'s comment). The
                            // first row (`i == 0`) is already reachable via `Enter`
                            // alone, so it shows `⏎` instead — no distinct `Alt+1`
                            // is assigned since it would just duplicate `Enter`.
                            if i < visible_rows_cap {
                                let hint = if i == 0 {
                                    "\u{23ce}".to_string()
                                } else {
                                    format!("Alt+{}", i + 1)
                                };
                                let chip_w = 14.0 + hint.chars().count() as f32 * 6.0;
                                let chip_rect = egui::Rect::from_center_size(
                                    egui::pos2(
                                        row.rect.right() - CONTENT_PADDING - chip_w / 2.0,
                                        row.rect.center().y,
                                    ),
                                    egui::vec2(chip_w, 20.0),
                                );
                                ui.painter().rect_filled(
                                    chip_rect,
                                    6.0,
                                    egui::Color32::from_rgba_unmultiplied(255, 255, 255, 16),
                                );
                                ui.painter().text(
                                    chip_rect.center(),
                                    egui::Align2::CENTER_CENTER,
                                    hint,
                                    egui::FontId::monospace(10.0),
                                    if is_selected {
                                        glass.text
                                    } else {
                                        glass.subtext
                                    },
                                );
                            }
                            let row = ui.interact(row.rect, row_id, egui::Sense::click());
                            // Right after a key action (up/down) scrolls the
                            // selected row out of view, pull it back into range once
                            // (see `pending_scroll_to_selected`'s doc comment).
                            // Calling this every frame during a manual mouse scroll
                            // would fight the user's own scrolling, so it's limited
                            // to the one frame right after selection changes.
                            if is_selected && self.pending_scroll_to_selected {
                                row.scroll_to_me(Some(egui::Align::Center));
                                self.pending_scroll_to_selected = false;
                            }
                            let is_file_action =
                                matches!(self.results[i].action, Action::Launch { .. });

                            if row.clicked() {
                                self.selected = i;
                                pending_row_action =
                                    Some((i, RowMenuAction::Run(ResultActionKind::Default)));
                            }

                            row.context_menu(|ui| {
                                self.selected = i;
                                if ui.button(self.strings.action_run).clicked() {
                                    pending_row_action =
                                        Some((i, RowMenuAction::Run(ResultActionKind::Default)));
                                    ui.close();
                                }
                                if is_file_action {
                                    if ui.button(self.strings.action_run_as_admin).clicked() {
                                        pending_row_action = Some((
                                            i,
                                            RowMenuAction::Run(ResultActionKind::RunAsAdmin),
                                        ));
                                        ui.close();
                                    }
                                    if ui.button(self.strings.action_open_location).clicked() {
                                        pending_row_action = Some((
                                            i,
                                            RowMenuAction::Run(ResultActionKind::OpenLocation),
                                        ));
                                        ui.close();
                                    }
                                    if ui.button(self.strings.action_copy_path).clicked() {
                                        pending_row_action = Some((
                                            i,
                                            RowMenuAction::Run(ResultActionKind::CopyPath),
                                        ));
                                        ui.close();
                                    }
                                }
                                if ui.button(self.strings.action_register_alias).clicked() {
                                    pending_row_action = Some((i, RowMenuAction::RegisterAlias));
                                    ui.close();
                                }
                                // Show "pin" or "unpin" depending on whether this
                                // result is already pinned (unpin is only offered
                                // for results already boosted from a past selection).
                                if is_pinned {
                                    if ui.button(self.strings.action_unpin).clicked() {
                                        pending_row_action = Some((i, RowMenuAction::Unpin));
                                        ui.close();
                                    }
                                } else if ui.button(self.strings.action_pin).clicked() {
                                    pending_row_action = Some((i, RowMenuAction::Pin));
                                    ui.close();
                                }
                            });
                        }
                    });

                // Only after the whole results list has finished drawing is the one
                // queued action actually run (see `pending_row_action`'s doc
                // comment — by this point, nothing later in this frame still reads
                // `self.results[i]`, so it's safe to mutate `self.results` here).
                if let Some((idx, action)) = pending_row_action {
                    self.selected = idx;
                    match action {
                        RowMenuAction::Run(kind) => self.run_result_action(&ctx, kind),
                        RowMenuAction::RegisterAlias => self.register_selected_as_alias(),
                        RowMenuAction::Pin => {
                            if let Some(result) = self.results.get(idx) {
                                let key = crate::search::target_key(&result.action);
                                self.history.pin(&key);
                                if let Err(err) = self.history.save(config::APP_NAME) {
                                    eprintln!("issen: failed to save history.toml: {err}");
                                }
                            }
                            // Rebuild via `run_search`, same as Unpin, so the new
                            // pinned ranking is reflected correctly right away.
                            self.run_search();
                        }
                        RowMenuAction::Unpin => {
                            if let Some(result) = self.results.get(idx) {
                                let key = crate::search::target_key(&result.action);
                                self.history.remove(&key);
                                if let Err(err) = self.history.save(config::APP_NAME) {
                                    eprintln!("issen: failed to save history.toml: {err}");
                                }
                            }
                            // Rebuild via the normal search path (`run_search`)
                            // rather than patching the score in place, so the
                            // post-unpin ranking is correct. Safe even if the
                            // result count changes, since this runs after the loop.
                            self.run_search();
                        }
                    }
                }
            });

        self.sync_window_height(&ctx);
        apply_theme(&ctx, self.config.theme);

        self.settings.show(
            &ctx,
            self.strings,
            self.lang,
            &mut self.config,
            self.last_scan_finished,
            self.last_scan_count,
            self.pending_scan.is_some(),
        );

        show_about_window(&ctx, self.strings, &mut self.about_open);

        self.tools.show(&ctx, self.strings);
    }
}

fn show_about_window(ctx: &egui::Context, strings: &'static Strings, open: &mut bool) {
    if !*open {
        return;
    }

    let mut still_open = true;
    ctx.show_viewport_immediate(
        egui::ViewportId::from_hash_of("issen-about"),
        egui::ViewportBuilder::default()
            .with_title(strings.about_title)
            .with_inner_size([320.0, 140.0])
            .with_resizable(false)
            // Without this, the about window would always end up behind the main
            // window, since the main window is `with_always_on_top()` (same reason
            // as in `settings_window.rs`).
            .with_always_on_top(),
        |ui, _class| {
            if ui.ctx().input(|i| i.viewport().close_requested()) {
                still_open = false;
            }
            egui::CentralPanel::default().show(ui, |ui| {
                ui.heading(config::APP_NAME);
                ui.label(i18n::about_version_text(
                    i18n::lang_of(strings),
                    env!("CARGO_PKG_VERSION"),
                ));
                ui.add_space(12.0);
                if ui.button(strings.about_close).clicked() {
                    still_open = false;
                }
            });
        },
    );
    *open = still_open;
    if *open {
        ctx.request_repaint();
    }
}
