# Hotkey & input

Covers: `src/hotkey.rs`, `src/app.rs`.

- Default global hotkey: `Alt+Space` (`Win+Space` collides with the
  Japanese IME's input-language switch, so it's avoided). Configurable.
  - `HotkeyListener` (`src/hotkey.rs`) owns a single background thread with
    its own message loop, which calls `RegisterHotKey(None, ...)` —
    registration is tied to the calling thread's message queue. Changing
    the hotkey at runtime (`app.rs`'s hotkey field's `cx.observe`
    handler, `settings_window.rs`) posts a custom message to that same
    thread via `PostThreadMessageW`, which then does `UnregisterHotKey` →
    `RegisterHotKey` itself, because `RegisterHotKey(None, ...)` can only
    be unregistered/re-registered from the thread that registered it.
  - `HotkeyListener::spawn` returns a `futures::channel::mpsc::
    UnboundedReceiver<()>` rather than taking a repaint callback the way
    the egui/eframe version's `spawn(ctx: egui::Context, ...)` did —
    GPUI's `AsyncApp` is `!Send` and can't be touched from this background
    thread directly. `app.rs`'s `run` hands the receiver to a `cx.spawn`
    foreground task that `.await`s it in a loop and calls `IssenApp::show`
    on each wakeup (see `docs/architecture/window-lifecycle.md`'s
    "Event-driven wake-up" section — the same pattern used for tray
    events).
  - The settings window's hotkey field applies on every keystroke (e.g.
    `"C"` → `"Ct"` → `"Ctrl+"` as the user types). Falling back to
    `Alt+Space` whenever parsing fails would make the live hotkey flicker
    mid-typing, so the fallback to a default only applies at startup's
    initial registration; live updates simply keep the current
    registration whenever the new string doesn't parse yet.
- Incremental search uses fuzzy matching.
- `Alt+2`–`Alt+9` jump straight to a visible result by position
  (`AltNum2`..`AltNum9` `KeyBinding`s, `IssenApp::run`). The range is
  capped at `visible_rows_cap()` (`config.max_results`, itself capped at
  `MAX_VISIBLE_ROWS`) — i.e. only rows actually rendered (see the scroll
  gap below). `Alt+1` is intentionally unassigned since it would
  duplicate `Enter`. The global hotkey doesn't conflict with these since
  `RegisterHotKey` is a distinct OS-level path.
- Right-click context menus (search box background → Settings/Reindex/
  Quit; a result row → Run/Run as Admin/Open Location/Copy Path/Register
  as Alias/Pin·Unpin) are GPUI's `anchored()`/`deferred()` primitives —
  `gpui` core has no built-in `ContextMenu` widget (unlike zed's own `ui`
  crate, not a dependency here). `anchored()` keeps the child inside the
  window's own pixel bounds (`.snap_to_window()`); `deferred()` delays
  its paint until after every sibling so it draws on top regardless of
  tree position. It's dismissed via `on_mouse_down_out` +
  `cx.stop_propagation()` (fires during the capture phase, before the
  same click's bubble-phase handlers run — e.g. a result row's own
  `on_mouse_down`), plus `.occlude()` on the menu itself so a click *on*
  the menu doesn't fall through to whatever's behind it.
- **Known gap, currently open**: results beyond `visible_rows_cap()` are
  not reachable at all right now — up to `RESULT_RETENTION_CAP` (50)
  results are still scored and kept in `self.results` (for stable
  relative ranking), but rendering `.take(visible_rows_cap)`s them with no
  scroll container, unlike the egui/eframe version's `egui::ScrollArea`.
  Since keyboard Down (`app.rs`'s selection-move handler) still clamps
  only to `self.results.len() - 1`, not to the rendered cap, it's
  possible to move `self.selected` onto a row that exists in `self.results`
  but was never actually rendered — the selection highlight silently
  disappears with no way to bring it back into view except moving back
  up. Not yet fixed; a `ScrollHandle`/`.track_scroll()`-based results
  area (GPUI's `.overflow_y_scroll()` needs a `Stateful<Div>`, i.e.
  `.id(...)` first — confirmed working elsewhere in this codebase) is the
  likely fix when this is picked up.
- Japanese IME preedit is handled by the search box's own
  `EntityInputHandler` implementation (`app.rs`, `impl EntityInputHandler
  for IssenApp`) rather than a framework-level `Ime::Preedit` event like
  the egui/eframe version relied on (`egui`/`egui-winit`). GPUI has no
  built-in text-input widget with IME support — this app implements the
  search box itself, including `marked_range` tracking
  (`replace_and_mark_text_in_range`/`unmark_text`) and drawing an
  underline under the IME composition span during render. Verified
  working on real hardware during the GPUI migration's Phase 0 spike
  (typing a Japanese query end-to-end) before the rest of the app was
  ported.
