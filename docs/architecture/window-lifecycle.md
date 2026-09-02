# Resident process & display model

Covers: `src/app.rs`, `src/main.rs`, `src/settings_window.rs`, `src/tools/**/*.rs`, `src/ui_chrome.rs`, `src/single_instance.rs`.

"Fast startup" doesn't mean spawning a new process — it means **making an
already-running hidden window instantly visible**.

- `eframe` creates exactly one viewport at startup, with
  `with_decorations(false)` / `with_always_on_top(true)`. **OS-level
  Show/Hide is never used**: the window is `with_visible(true)` from launch
  onward, and its position alone toggles between the real on-screen
  position and a fixed off-screen coordinate (`app::OFFSCREEN_POSITION` in
  `main.rs`) via `ViewportCommand::OuterPosition`. `ViewportCommand::Visible`
  is never sent.
  - *Rationale:* an earlier version used real Show/Hide
    (`ViewportCommand::Visible(true/false)` on every reveal), which caused
    a visible flicker of an empty white frame (tens of ms, confirmed on
    screen recordings) every time the window reappeared. Three separate
    mitigations were tried — disabling DWM transitions, redrawing a few
    invisible frames before showing, re-invoking the blur-behind call at
    startup — and none helped, because all three left the OS-level
    Show/Hide transition itself in place and only tweaked something inside
    it. Removing the transition entirely (stay visible, move instead) is
    what actually fixed it. Don't reintroduce a real Show/Hide cycle here.
  - This also sidesteps an `eframe` (0.36) quirk where the window is
    force-shown after the first render regardless of
    `with_visible(false)` (`epi_integration::post_rendering`, see
    [emilk/egui#2279](https://github.com/emilk/egui/pull/2279)) — since the
    window is `with_visible(true)` from the start, there's nothing left to
    fight, and no workaround for it exists in the code.
  - Even with this model, the **very first** time the window is actually
    composited onto a monitor after process start (i.e. the first hotkey
    press), DWM shows one frame of an opaque white placeholder before the
    real content — reproducible only on that first composite, never again
    in the same process. Fixed by a "startup priming" step: 800ms after
    launch, the window is invisibly warped to its real position and back
    off-screen, consuming that "first" before the user ever triggers it for
    real (`src/app.rs`). Invisibility is done by toggling the
    `WS_EX_LAYERED` window style on only for that step. Before restoring
    opacity, the code explicitly confirms via `GetWindowRect` that the
    queued `ViewportCommand::OuterPosition` move has actually landed,
    rather than assuming an order between it and the immediate Win32 call —
    these two operations travel through different paths with different
    timing (an immediate Win32 call vs. a queued, delayed `ViewportCommand`),
    and assuming an implicit order between them caused a rare race where
    opacity was restored just before the position actually changed,
    reproducing the exact flicker this step exists to prevent.
- A hotkey press only sends `ViewportCommand::OuterPosition` (to the real
  position) + `ViewportCommand::Focus` — it never recreates the window and
  never sends `Visible`.
- Quitting calls `std::process::exit(0)` directly instead of
  `ctx.send_viewport_cmd(ViewportCommand::Close)`, which turned out to be
  broken two layers deep (both confirmed by instrumenting eframe's/the
  app's `log` output on real hardware):
  - While the main window is hidden, `logic()` runs through eframe 0.36.1's
    invisible-window fallback path (`glow_integration.rs`'s
    `update_logic_only` branch). A `Close` command queued from there is
    applied to the viewport but never reaches the code that would turn it
    into an actual process exit — it vanishes with no further trace.
  - The right-click menu's Quit (`show_main_context_menu`, only reachable
    while the window is visible) only needed `process::exit(0)` to route
    around that. But the tray menu's Quit needed more: `TrayAction`s are
    delivered by `logic()` polling a channel every frame
    (`TrayHandle::try_recv_action`), and that polling itself isn't
    guaranteed to run again once the window is hidden.
    `ctx.request_repaint()` (called from the `MenuEvent` handler to wake
    the loop) schedules a wakeup through the same invisible-window repaint
    machinery, and that wakeup can be silently dropped — observed as
    `logic()` not running again for several seconds in one run and never
    in another, after the Quit click had already been received. So tray
    Quit can't go through the channel/poll path at all: it's handled
    inline inside the `MenuEvent` callback itself (`tray.rs`'s
    `ensure_event_forwarding`), which `muda` calls directly and which
    therefore doesn't depend on `logic()` running again.
  `process::exit(0)` is safe in both places because config/history are
  saved synchronously on every mutation, not on a shutdown hook.
- Losing focus hides the window automatically. `egui::InputState::focused`
  doesn't update while hidden (it stays at the last visible frame's value),
  so this is detected via the `egui::Event::WindowFocused(false)` event,
  not by polling.
- Every place that changes visibility must go through a single entry point,
  `IssenApp::set_visible(ctx, bool)` (`src/app.rs`), which sends the
  `ViewportCommand` only when the value actually changes.
  - *Rationale:* an earlier version compared `visible` at the start and end
    of `logic()` and sent a command if it differed. But running a search
    result (button click, right-click menu, or a mouse click handled in
    `ui()`) could change `visible` *after* that comparison had already run,
    so the command was silently skipped and the window stayed visible. This
    went unnoticed for a while because focus-loss handling covered most
    cases through a separate path. Routing every mutation through one
    function, which sends its command immediately regardless of caller,
    closes this gap for good.
- The settings window is a separate viewport, lazily created on first open.
  - It uses `egui::Context::show_viewport_immediate`, not
    `show_viewport_deferred` (`src/settings_window.rs`): the deferred
    variant only stays alive if called every frame, while the immediate
    variant's lifetime is naturally tied to its caller being invoked every
    frame — which is also how CPU use is kept down while everything is
    hidden. "Lazily created on first open" is implemented by having
    `show()` early-return until `open()` has been called once, so the
    viewport is never actually constructed before that.
  - The settings window (and the color picker / unit converter windows)
    use a sidebar + translucent glass panel layout. Sections aren't
    reduced when reorganized into tabs — everything that existed before
    stays, just regrouped.
  - **Custom chrome**: the settings window, color picker, and unit
    converter are undecorated, transparent windows that share
    `src/ui_chrome.rs`'s `title_bar` / `resize_grips` for a consistent
    look. Dragging uses `ctx.send_viewport_cmd(ViewportCommand::StartDrag)`;
    resizing (east / south / southeast only) delegates to the OS via
    `ViewportCommand::BeginResize(direction)` rather than computing hit
    regions by hand. The drag region's `Sense` is `click_and_drag()`, not
    just `drag()` — egui only resolves click-type events (including
    right-click) for widgets whose `Sense` includes clicking, and a
    drag-only region silently swallows right-clicks.
  - These three windows follow `config.theme` (light/dark/system), unlike
    the main search box, which always stays black regardless of theme (see
    `docs/architecture/ui-appearance.md`).
- Single-instance is enforced with a named mutex (`CreateMutexW`). Its
  `HANDLE` is intentionally never closed — `HANDLE` has no `Drop` impl, and
  holding it open *is* the mutex's lifetime; adding a `CloseHandle` call
  because it "looks discarded" would break single-instance enforcement.
- **Known gap, not yet fixed**: `TrayAction::Open`/`Settings`/`Reindex`/
  `About` all still go through the channel-then-poll path described above
  (`ensure_event_forwarding`'s doc comment), which relies on
  `ctx.request_repaint()` successfully waking `logic()` while the window is
  hidden. That wakeup was observed to be silently dropped for Quit (fixed
  by handling Quit inline instead — see the Quit bullet above), and the
  same underlying eframe/winit scheduling gap plausibly affects these other
  tray actions too: clicking tray → Settings while the window is hidden
  could, in principle, sit unresponsive the same way Quit did. Not
  reproduced or fixed here because it doesn't hang the process the way an
  un-actionable Quit does — worth revisiting if it's ever reported.
