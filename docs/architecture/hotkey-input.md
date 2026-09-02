# Hotkey & input

Covers: `src/hotkey.rs`, `src/app.rs`.

- Default global hotkey: `Alt+Space` (`Win+Space` collides with the
  Japanese IME's input-language switch, so it's avoided). Configurable.
  - `HotkeyListener` (`src/hotkey.rs`) owns a single background thread with
    its own message loop, which calls `RegisterHotKey(None, ...)` —
    registration is tied to the calling thread's message queue. Changing
    the hotkey at runtime (`app.rs::apply_hotkey_change`) posts a custom
    message to that same thread via `PostThreadMessageW`, which then does
    `UnregisterHotKey` → `RegisterHotKey` itself, because
    `RegisterHotKey(None, ...)` can only be unregistered/re-registered from
    the thread that registered it.
  - The settings window's hotkey field applies on every keystroke (e.g.
    `"C"` → `"Ct"` → `"Ctrl+"` as the user types), so `apply_hotkey_change`
    runs on every partial string. Falling back to `Alt+Space` whenever
    parsing fails would make the live hotkey flicker mid-typing, so the
    fallback to a default only applies at startup's initial registration;
    live updates simply keep the current registration whenever the new
    string doesn't parse yet.
- Incremental search uses fuzzy matching.
- `Alt+2`–`Alt+9` jump straight to a visible result by position. The range
  is capped at `visible_rows_cap()` (`config.max_results`, itself capped at
  `MAX_VISIBLE_ROWS`) — i.e. only rows visible without scrolling, and this
  stays true even after scroll support was added (see below); following
  the scrolled viewport would add complexity for little benefit. `Alt+1` is
  intentionally unassigned since it would duplicate `Enter`. The global
  hotkey doesn't conflict with these since `RegisterHotKey` is a distinct
  OS-level path.
- Up to `RESULT_RETENTION_CAP` (50) results are kept, but the window only
  grows to fit `config.max_results` rows (default 6, capped at
  `MAX_VISIBLE_ROWS` = 8 in the settings UI) without scrolling; anything
  beyond that scrolls inside the results area. The `egui::ScrollArea`'s ID
  changes with every new query (`IssenApp::search_generation`) so a new
  query always starts scrolled to the top rather than inheriting the
  previous scroll position. Arrow-key selection moving out of the visible
  range triggers `Response::scroll_to_me` once
  (`IssenApp::pending_scroll_to_selected`), rather than every frame, so it
  doesn't fight a manual mouse scroll in progress.
- Japanese IME preedit text relies on `egui`/`egui-winit`'s `Ime::Preedit`
  support.
