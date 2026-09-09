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
  `MAX_VISIBLE_ROWS`) regardless of scroll position — deliberately not
  scroll-aware (see the scroll container below): the target row is always
  "the Nth row from the top of the *original*, unscrolled view," matching
  the egui/eframe version's own documented simplification ("following the
  scrolled viewport would add complexity for little benefit"). `Alt+1` is
  intentionally unassigned since it would duplicate `Enter`. The global
  hotkey doesn't conflict with these since `RegisterHotKey` is a distinct
  OS-level path.
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
- Results (and query-history rows) beyond `visible_rows_cap()` are
  reachable by scrolling, not just rendered up to the cap and dropped: all
  of `self.results` (up to `RESULT_RETENTION_CAP`, 50) and all of
  `self.history.queries` (up to `QUERY_HISTORY_CAP`, 20) are rendered, but
  into a fixed-height wrapper div (`render`'s `rows_list`, height =
  `visible_row_count() * RESULT_ROW_HEIGHT` — the same height
  `sync_window_height` already gives the window) with `.id(...)` +
  `.overflow_y_scroll()` + `.track_scroll(&self.scroll_handle)`. The fixed
  height is what keeps `flex_col`'s default flex-shrink from compressing
  every row to fit as the count crosses `visible_rows_cap` — GPUI's
  `.overflow_y_scroll()`/`ScrollHandle`/`.track_scroll()` only work on a
  `Stateful<Div>` (i.e. after `.id(...)`), not a bare `Div`.
  - `IssenApp::scroll_handle` (`gpui::ScrollHandle`) is shared between the
    result list and the query-history panel, since only one is ever shown
    at a time. Reset to the top (`set_offset(Point::default())`) on every
    new search (`run_search`) and on opening/closing the history panel
    (`toggle_history_panel`), matching the egui/eframe version's
    `search_generation`-keyed `egui::ScrollArea` id reset — a new query
    (or switching lists) always starts scrolled to the top rather than
    inheriting the previous scroll position.
  - Keyboard Up/Down (`move_up`/`move_down`) calls
    `self.scroll_handle.scroll_to_item(self.selected)` once per move,
    which scrolls the minimal amount to bring that row fully into view
    (GPUI's default `FirstVisible` strategy) — matching the egui/eframe
    version's one-shot `Response::scroll_to_me` rather than fighting an
    in-progress manual mouse scroll every frame.
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
