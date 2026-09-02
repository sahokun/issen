# UI / appearance

Covers: `src/app.rs`, `src/ui_chrome.rs`, `src/display.rs`, `src/fonts.rs`.

- **UI font**: `config.toml`'s `ui_font` lets the user pick the Latin
  proportional typeface used for UI chrome — Segoe UI (default), Yu
  Gothic, or Meiryo. The monospace face (used for path display etc.) isn't
  user-selectable and is fixed to Consolas, since it's a structural choice
  rather than a preference. CJK fallback loading (see
  `docs/architecture/i18n.md`) is unconditional regardless of this choice, for
  the same reason.
  - The choice is deliberately limited to a short list of verified,
    always-present Windows typefaces rather than enumerating every
    installed font (e.g. via DirectWrite): enumeration would require
    mapping family names to actual font files/faces (including which face
    of a multi-weight `.ttc` to use) on the host side, which is brittle.
    Bundling a custom font was also considered and rejected in favor of
    sticking to fonts Windows already ships.
  - `app.rs`'s `applied_ui_font` follows the same "apply only when changed"
    guard pattern as `applied_font_scale` (`apply_ui_font`), since
    `ctx.set_fonts` rebuilds the font atlas and shouldn't run every frame.
- The main input box is a single-line, translucent glass-style panel.
  - Input text is vertically centered within its row via a horizontal
    layout's default `Align::Center`; the `TextEdit` itself keeps its
    natural height rather than being height-constrained, and the row
    reserves height via `ui.set_min_height` (`src/app.rs::ui`).
  - The input box always stays dark (`ui_chrome::palette(true)`, ignoring
    `config.theme`) — unlike the settings/tool windows (see
    `docs/architecture/window-lifecycle.md`). Because of that,
    `*ui.visuals_mut() = egui::Visuals::dark()` is set explicitly so
    text color doesn't get pulled toward a lighter theme-derived color that
    would be unreadable against the fixed dark background.
  - **Glass panel implementation**: `main.rs`'s `ViewportBuilder` sets
    `with_transparent(true)`, `IssenApp::clear_color` is overridden to
    alpha 0, and `ui_chrome::glass_panel` paints a rounded, translucent
    rectangle over that. This was verified on real hardware by overlaying
    the window on a solid red background and checking actual pixel alpha
    values, to confirm `with_transparent(true)` + zero-alpha clear actually
    composites at the OS level without relying on Mica/Acrylic APIs — which
    this doesn't use.
  - An accent color (`ui_chrome::ACCENT`) is used for a subtle glow effect
    on the input box's left edge and the selected result row.
- Right-clicking the input box (including the drag/blank area above and
  below the text field, not just the `TextEdit` itself) opens a context
  menu with a way to open settings. The drag area's `Sense` needs to
  include clicking (see `docs/architecture/window-lifecycle.md`'s "Custom
  chrome" note) for this to work at all.
  - The context menu renders in the same OS window (`Order::Foreground`
    layer) as the main box, so it's physically clipped to that window's
    pixels. With an empty query (window height = `MAIN_WINDOW_SIZE.1` = 60px
    only), the menu (up to 5 items + a separator) would otherwise get cut
    off. `ctx.any_popup_open()` detects the open menu and adds
    `CONTEXT_MENU_HEADROOM` extra height while it's open — the same
    dynamic-resize mechanism used for the results dropdown, not a separate
    one.
- Search results render as a dropdown-style area directly under the input
  box, sized to fit the visible row count without scrolling
  (`src/app.rs::sync_window_height`); anything beyond that scrolls (see
  `docs/architecture/hotkey-input.md`). The window's on-screen position is
  computed relative to the zero-results height (`MAIN_WINDOW_SIZE`), so
  hiding the window must reset query, results, and window height together
  (`set_visible`).
- Font scale (`config.font_scale`, default `1.0`) is applied via
  `app.rs::apply_font_scale` (`egui::Context::all_styles_mut`, both
  light/dark `Style::text_styles`) — same "apply only on change" pattern as
  the hotkey and UI font above, since it also touches font layout. Result
  row height and input box height stay fixed in px rather than scaling, so
  the allowed scale range is capped to `0.9`–`1.25` to avoid visibly
  breaking the layout.
- Light/dark mode follow `egui::Context::set_theme` from `config.toml`'s
  `theme` (`ThemePreference::{Light,Dark,System}` maps 1:1 to `Theme`) —
  except the input box itself, which always stays dark regardless (see
  above).
- On multi-monitor setups, the window is repositioned into the display
  chosen by `config.display_target` on every show (`src/display.rs`):
  the display under the mouse cursor (default,
  `GetCursorPos` → `MonitorFromPoint` → `GetMonitorInfoW`), the primary
  display (`MonitorFromPoint` with the origin and
  `MONITOR_DEFAULTTOPRIMARY`), or the currently focused window's display
  (`GetForegroundWindow` → `MonitorFromWindow`).
  - "Focused window" is read via `GetForegroundWindow` at the exact moment
    `app.rs::set_visible` sends the `ViewportCommand` that will make the
    window visible — that command is only queued, not applied
    synchronously, so the read still sees whatever window the user was
    last on rather than Issen's own (still hidden) window. This is less
    precise when triggered from the tray, since the foreground window at
    that point may be the taskbar itself rather than whatever the user was
    last working in (see `DisplayTarget::FocusedWindow`'s doc comment in
    `src/config.rs`).
