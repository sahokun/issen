# Resident process & display model

Covers: `src/app.rs`, `src/main.rs`, `src/settings_window.rs`, `src/about_window.rs`, `src/tools/**/*.rs`, `src/ui_chrome.rs`, `src/tray.rs`, `src/single_instance.rs`.

"Fast startup" doesn't mean spawning a new process — it means **making an
already-running hidden window instantly visible**.

- GPUI opens exactly one window at startup (`kind: WindowKind::PopUp`,
  `titlebar: None`, `show: true`, `window_background:
  WindowBackgroundAppearance::Transparent` — `app.rs`'s `run`).
  **OS-level Show/Hide is never used**: the window stays OS-visible for the
  whole process lifetime, and "hidden" means parked at a fixed off-screen
  coordinate (`OFFSCREEN_POSITION`), moved back to its real on-screen
  position to "show" it (`IssenApp::show`/`hide`, raw `SetWindowPos` via
  `move_window` — not any GPUI-level visibility API).
  - *Rationale, carried over unchanged from this app's original egui/eframe
    implementation:* an early version used real OS Show/Hide on every
    reveal, which caused a visible flicker of an empty white frame (tens of
    ms) every time the window reappeared. Several narrower mitigations were
    tried and none helped, because they all left the OS-level Show/Hide
    transition itself in place. Removing the transition entirely (stay
    visible, move instead) is what actually fixed it, and the GPUI port
    kept the same model rather than re-litigating it.
  - Because the window is permanently OS-visible, it's excluded from
    Alt+Tab and the taskbar via a raw `WS_EX_TOOLWINDOW` style bit
    (`set_tool_window_style`, called once from `IssenApp::new`) — GPUI's
    `WindowOptions` has no cross-platform equivalent for this.
  - `WindowOptions.window_bounds`'s origin/size aren't reliably honored
    when the window is created off-screen at a large negative origin
    (observed on real hardware: the window came up full-monitor-width and
    on-screen instead of at `OFFSCREEN_POSITION`) — `IssenApp::new` forces
    both explicitly via the same raw Win32 calls `hide()` uses, rather than
    trusting `WindowOptions` for the off-screen case.
  - Unlike the egui/eframe version, **no startup "priming" step exists** to
    pre-consume DWM's first-ever-composite white flash. Phase 0 of the
    GPUI migration spiked repeated offscreen show/hide toggling
    specifically looking for this, found none, and the framework switch
    was later confirmed (by the user, on real hardware) to have made the
    issue moot on its own — don't reintroduce the egui-era
    `LayeredPrimeState` machinery pre-emptively; there's nothing here for
    it to fix.
- A hotkey press only calls `IssenApp::show()` (`app.rs`'s `cx.spawn` task
  awaiting the hotkey channel — see "Event-driven wake-up" below) — it
  never recreates the window and never touches OS-level visibility.
- Quitting calls `std::process::exit(0)` directly rather than going through
  any window-close path, in two places: the right-click context menu's
  Quit item (`app.rs`'s `render_context_menu`) and the tray menu's Quit
  item, which is special-cased *inside* `tray.rs`'s `MenuEvent` handler
  itself (`ensure_event_forwarding`) rather than routed through the
  `TrayAction` channel like every other tray action — `muda` calls that
  handler directly from its own message handling, so it's guaranteed to
  run regardless of anything else in the app, and Quit can't tolerate ever
  being missed. `process::exit(0)` is safe in both places because
  config/history are saved synchronously on every mutation, not on a
  shutdown hook.
- Losing OS focus hides the window automatically, unless a secondary
  window (about/settings/tools) is why focus moved — those are opened
  *over* the main window and shouldn't cause it to vanish out from under
  them. Implemented via `cx.observe_window_activation` (fires on both
  activate and deactivate; the deactivate branch checks
  `has_open_secondary_window()`), guarded by a `seen_active_since_show`
  flag that's reset on every `show()` and only set once a real OS
  activation is observed afterward — see `app.rs`'s doc comments on
  `seen_active_since_show`, `show()`, and `sync_window_height()` for two
  distinct races this guards against (an `activate_window()` call that
  turns out to be a no-op because the window was already OS-foreground;
  `Window::resize`'s own `SetWindowPos` silently reclaiming OS-thread
  activation on the way into `hide()`). Both were found and fixed by
  comparing real-hotkey behavior against the egui/eframe version's
  (`ISSEN_DEBUG_FOCUS_TRACE` file-based tracing, since a release-shaped run
  may have no console).
- Every place that changes visibility must go through `IssenApp::show`/
  `hide` (`app.rs`), which only acts when the value actually changes and
  is the single source of truth for `self.visible`.
- **Event-driven wake-up, not polling.** The egui/eframe version drove
  tray-action delivery off a per-frame channel poll inside its render
  loop, which went silently unresponsive while the window was hidden (a
  documented, never-fully-fixed gap for actions other than Quit). GPUI has
  no such per-frame loop to hang a poll off in the first place: hotkey
  presses, tray menu clicks, and tray icon double-clicks each arrive
  through their own `futures::channel::mpsc` receiver, and `app.rs`'s
  `run` spawns one `cx.spawn` task per channel that simply
  `.await`s `rx.next()` in a loop and calls `window_handle.update(...)`
  when woken (see `app.rs`, right after the main window opens). This
  isn't a workaround for the old gap — it structurally can't have it,
  since nothing here depends on a repaint happening while hidden.
  `tray.rs`'s module doc comment covers the two separate `muda`/
  `tray-icon` `OnceCell` event-handler mechanisms these channels forward
  from.
- The about/settings/tools windows are each a separate GPUI window
  (`kind: WindowKind::Normal`), lazily created on first open and cached as
  `Option<WindowHandle<T>>` on `IssenApp`. `open()` (each module's own
  free function) checks the existing handle first — `handle.update(cx,
  ...).is_ok()` succeeds and just re-activates/refreshes if the window is
  still open, and only calls `cx.open_window` on `Err` (closed or never
  created). Real Win32 always-on-top (`HWND_TOPMOST` via `SetWindowPos`,
  `ui_chrome::set_topmost`) is applied explicitly to each, since only the
  main window's `WindowKind::PopUp` gets that treatment from GPUI's
  Windows backend automatically — without it, a secondary window would
  end up behind the always-on-top main window.
  - **Entity-reentrancy hazard**: `cx.open_window` runs the new window's
    first `Render::render()` synchronously, inside whatever call opened
    it — which itself often runs nested inside the main window's own
    `Context<IssenApp>::update` (e.g. a tray-action or context-menu
    handler). Reading a cross-window entity synchronously from code that
    might still be nested inside that entity's own update panics at
    runtime (`"cannot read ... while it is already being updated"`). Two
    patterns avoid this: pass plain owned data as parameters when the
    callee needs nothing *live* afterward (about, tools), or cache a
    plain snapshot struct refreshed via `cx.observe(&app_entity, ...)`
    when it does (settings' `AppSnapshot`) — an observer callback only
    runs after the notifying update has fully returned, so reading inside
    one is always safe.
  - The same synchronous-open behavior can deactivate the main window
    *before* the caller has recorded the new window's handle back onto
    `IssenApp`, making the entity-based "is a secondary window open?"
    check briefly see `None` mid-open and hide the main window out from
    under the one that's opening — guarded by the separate
    `opening_secondary_window` flag (set immediately before each of the
    three `open()` call sites, cleared immediately after).
  - The settings window, color picker, and unit converter windows use a
    sidebar + translucent glass panel layout, sharing `ui_chrome.rs`'s
    `title_bar`/`glass_container`/`palette`. **Custom chrome**: all three
    (plus about) are undecorated (`titlebar: None`), so `title_bar`
    paints its own title text and close (×) button. Dragging uses
    `.window_control_area(WindowControlArea::Drag)` (GPUI's `WM_NCHITTEST`
    integration — see `ui_chrome::title_bar`'s doc comment; `Window::
    start_window_move()` is a documented no-op on Windows) with
    `is_movable: true` also set on the window, since Windows ignores drag
    regions entirely without it. Resizing is native OS resize (`
    is_resizable: true` + `window_min_size` in `WindowOptions`), not the
    egui/eframe version's hand-computed east/south/southeast resize grips
    — GPUI's Windows backend handles resize hit-testing itself once the
    window opts in.
  - These windows follow `config.theme` (light/dark/system via `Window::
    appearance()`), unlike the main search box, which always stays black
    regardless of theme (see `docs/architecture/ui-appearance.md`).
- Single-instance is enforced with a named mutex (`CreateMutexW`,
  `single_instance.rs`, unchanged by the GPUI migration). Its `HANDLE` is
  intentionally never closed — `HANDLE` has no `Drop` impl, and holding it
  open *is* the mutex's lifetime; adding a `CloseHandle` call because it
  "looks discarded" would break single-instance enforcement.
