# UI / appearance

Covers: `src/app.rs`, `src/ui_chrome.rs`, `src/display.rs`, `src/fonts.rs`.

- **UI font**: `config.toml`'s `ui_font` lets the user pick the Latin
  proportional typeface used for UI chrome — Segoe UI (default), Yu
  Gothic, or Meiryo. The monospace face (used for path display etc.) isn't
  user-selectable and is fixed to Consolas, since it's a structural choice
  rather than a preference. CJK fallback (see `docs/architecture/i18n.md`)
  applies unconditionally regardless of this choice, for the same reason.
  - The choice is deliberately limited to a short list of verified,
    always-present Windows typefaces rather than enumerating every
    installed font (e.g. via DirectWrite): enumeration would require
    mapping family names to actual font files/faces (including which face
    of a multi-weight `.ttc` to use) on the host side, which is brittle.
    Bundling a custom font was also considered and rejected in favor of
    sticking to fonts Windows already ships.
  - Applying it is much simpler under GPUI than it was under egui/eframe:
    `fonts::ui_font_family(config.ui_font)` just returns a Win32 family
    name string (`"Segoe UI"` etc.), passed straight to `.font_family(...)`
    on the relevant text elements (`app.rs`'s search box,
    `settings_window.rs`'s own UI text). GPUI's Windows text system shapes
    through DirectWrite, which resolves an installed family by name on its
    own — unlike egui, which did its own text shaping and needed
    `.ttf`/`.ttc` bytes loaded by hand and handed to `egui::Context::
    set_fonts` (a font-atlas rebuild) every time the choice changed. No
    "apply only when changed" guard is needed any more for this reason.
- The main input box is a single-line, translucent glass-style panel,
  `flex_none`/`h(px(MAIN_WINDOW_SIZE.1))` (`app.rs::render`'s
  `search_box`).
  - The input box always renders in the search box's own fixed dark
    scheme (`text_color(white())` against a fixed dark background) —
    unlike the settings/tools windows (see
    `docs/architecture/window-lifecycle.md`), it never follows
    `config.theme`.
  - **Glass panel implementation**: the main window is opened with
    `window_background: WindowBackgroundAppearance::Transparent`
    (`app.rs`'s `run`) — a first-class GPUI API, not the egui/eframe
    version's manual `with_transparent(true)` + zero-alpha `clear_color`
    combination that had to be verified pixel-by-pixel against a solid
    background to confirm it actually composited. On top of that OS-level
    transparency, `render()` paints a single fixed translucent color
    (`glass_bg = hsla(220./360., 0.12, 0.09, 0.80)`) as the panel
    background — the `0.80` alpha was tuned to the user's preference via
    real-hardware comparison, replacing an earlier, less opaque value.
    This app doesn't use Mica/Acrylic APIs either way.
  - An accent color (`ui_chrome::accent_color(config.accent_color)`, one
    of five presets) is used for a bar on the input box's left edge and
    the selected result row's highlight.
- Right-clicking anywhere in the input row — including the background
  drag area, not just the text field itself — opens a context menu
  (Settings/Reindex/Quit). This works because the row's absolutely
  positioned drag background (`WindowControlArea::Drag`) and the text
  field div both carry their own `Right`-button mouse-down handler; see
  `docs/architecture/window-lifecycle.md`'s note on `.occlude()` for why
  the text field needs it to keep this from double-firing as a
  window-drag.
  - The context menu is GPUI's `anchored()`/`deferred()` overlay (no
    built-in `ContextMenu` widget in GPUI core — see
    `docs/architecture/hotkey-input.md` for the interaction details),
    painted inside the same OS window as the main box, so it's physically
    clipped to that window's pixels. With an empty query (window height =
    `MAIN_WINDOW_SIZE.1`, 60px, only) the menu would otherwise be cut off:
    `sync_window_height` adds a fixed `CONTEXT_MENU_HEADROOM` (220px)
    whenever `self.context_menu.is_some()`, on top of whatever height the
    result rows already need — the same resize call, not a separate
    mechanism.
- Search results render as a dropdown-style area directly under the input
  box, sized to fit the visible row count without scrolling
  (`app.rs::sync_window_height`); anything beyond
  `visible_rows_cap()` scrolls instead (see
  `docs/architecture/hotkey-input.md`). The window's on-screen position is
  computed relative to the zero-results height (`MAIN_WINDOW_SIZE`), so
  `hide()` must reset query, results, and window height together.
- Font scale (`config.font_scale`, default `1.0`) is applied via
  `ui_chrome::apply_font_scale`, called at the top of every window's own
  `render()` (main, settings, tools). It scales GPUI's window-wide `rem`
  size (`Window::set_rem_size`, described by GPUI's own doc comment as
  "just like zooming a web page") rather than touching individual text
  styles — every text size in this app is expressed as `.text_size(rems(n))`
  specifically so this one call rescales all of them at once. Layout
  metrics (paddings, row heights, window sizes) stay in literal `px` and
  don't scale, which is why the allowed range is clamped narrowly
  (`0.5`–`2.0` at the `set_rem_size` call site; the settings UI's slider
  itself uses a still narrower range) to avoid visibly breaking the fixed-
  size rows — same reasoning the egui/eframe version had for its own
  narrow clamp, just applied through GPUI's mechanism instead of
  `egui::Style::text_styles`.
- Light/dark mode follows `config.theme` (`Theme::{Light,Dark,System}`) via
  `ui_chrome::resolve_dark`, which reads the window's real OS appearance
  (`Window::appearance()`) for the `System` case — except the input box
  itself, which always stays dark regardless (see above).
- On multi-monitor setups, the window is repositioned into the display
  chosen by `config.display_target` on every `show()`
  (`IssenApp::resolve_show_target`, `src/display.rs`): the display under
  the mouse cursor (default, `GetCursorPos` → `MonitorFromPoint` →
  `GetMonitorInfoW`), the primary display (`MonitorFromPoint` with the
  origin and `MONITOR_DEFAULTTOPRIMARY`), or the currently focused
  window's display (`GetForegroundWindow` → `MonitorFromWindow`). This
  Win32-level logic is unchanged from the egui/eframe version — the GPUI
  migration only touched how `show()` consumes the result (a direct
  `move_window` call instead of a queued `ViewportCommand`).
  - "Focused window" is read via `GetForegroundWindow` synchronously
    inside `show()`, *before* `move_window`/`window.activate_window()` run
    — so it still sees whatever window the user was last on, not Issen's
    own (about-to-be-shown) window. This is less precise when triggered
    from the tray, since the foreground window at that point may be the
    taskbar itself rather than whatever the user was last working in (see
    `DisplayTarget::FocusedWindow`'s doc comment in `src/config.rs`).
  - `display.rs`'s `position_on_point_monitor` (an
    `ISSEN_DEBUG_FORCE_MONITOR_POINT`-driven variant used by verification
    scripts to target an arbitrary monitor deterministically) hasn't been
    wired back into `app.rs`'s GPUI build yet — currently `#[allow(dead_code)]`
    and unused; port it if that debug hook is needed again.
