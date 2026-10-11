# Color picker & unit converter

Covers: `src/tools/**/*.rs`.

The color picker and unit converter, launched from icon buttons next to the
input box, are implemented as separate top-level windows (same custom
chrome as the settings window; see `docs/architecture/window-lifecycle.md`)
rather than as `SearchProvider` results.

- The color picker and the earlier separate "color code conversion"
  (hex/RGB/HSL display) tool were merged into one: once an eyedropper was
  added, its output duplicated the color-code tool's display, and running
  two color-related tools side by side in a 640px window was redundant.
  All representations (HSV wheel, eyedropper, hex, RGB/HSL, swatch) stay
  synced to a single `Color32`, event-driven — changing any one updates
  the rest immediately.
- The HSL fields keep their own editable state (`ToolsState::hsl`) instead
  of being recomputed from `self.color` every frame via `rgb_to_hsl`. A
  pure RGB→HSL recompute loses information at the boundaries (saturation
  0, or lightness 0/100): red → lightness 100 (white) → lightness 50
  becomes gray, not red, because hue/saturation can't be recovered from a
  fully desaturated RGB value. So HSL fields only get overwritten by
  `rgb_to_hsl` when a *different* input (hex, wheel, eyedropper) changed
  the color; editing HSL directly updates `self.color` without going
  through that recompute.

- Screen sampling previews on a GPUI timer, but mouse confirmation uses a
  session-scoped `WH_MOUSE_LL` hook on a dedicated Win32 message-loop
  thread (`src/tools/eyedropper.rs`). Polling `GetAsyncKeyState` can miss
  clicks between timer ticks and does not stop the target app from receiving
  them. The hook captures the pixel at mouse-down and consumes both down
  and up; only that selected click is intercepted. Closing/cancelling
  releases the hook, with cleanup deferred until mouse-up if confirmation
  is still held. Enter confirms the preview and Escape restores the color
  from before sampling.

- The color input accepts six-digit hex and the 148 fixed CSS color names.
  The typed name stays in the input; its canonical hex appears inside the
  swatch only while a valid name is present. The label uses black or white
  according to relative luminance. Copy always copies the canonical hex.
  Wheel and eyedropper edits replace the input with hex, clearing the extra
  label. Incomplete/invalid input preserves the last valid preview.
  `svgtypes` provides CSS Color 3 names; `rebeccapurple` is handled explicitly.
  `transparent` and contextual keywords are outside this opaque RGB picker.
