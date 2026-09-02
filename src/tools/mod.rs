pub mod color;
pub mod units;

use crate::i18n::Strings;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolKind {
    /// Combined color tool: HSV wheel, eyedropper, and hex/RGB/HSL display in one
    /// (merged from a formerly separate "color code conversion" tool, `ColorCode`).
    ColorPicker,
    UnitConverter,
}

/// Single-purpose tools opened from icon buttons next to the input box. Implemented as
/// separate viewports via `ctx.show_viewport_immediate`, the same as the settings window and
/// the about window (see `src/settings_window.rs`).
///
/// Rationale: these were originally an `egui::Window` popup floating over the main window, to
/// avoid the cost of spawning a separate viewport for something meant to open and close
/// quickly. But `egui::Window` renders inside the same viewport as its parent, so it inherited
/// the main window's small, undecorated rectangle — the popup was clipped to it and couldn't
/// extend past the input box at all. Moving to a separate viewport makes it an independent
/// OS-level window, no longer bound by the main window's rectangle.
pub struct ToolsState {
    active: Option<ToolKind>,
    color: egui::Color32,
    unit_category: usize,
    unit_from: usize,
    unit_to: usize,
    unit_input: String,
    code_hex: String,
    /// Editable buffer for the HSL fields (hue/saturation/lightness). Recomputing this every
    /// frame from `self.color` via `rgb_to_hsl` would lose hue/saturation information whenever
    /// the color passes through zero saturation (gray) or lightness 0/100 (black/white) — and
    /// once lost, it can't be recovered from lightness alone (e.g. red → L=100 (white) → L=50
    /// comes back gray, not red). Same reasoning and pattern as the hex field (`code_hex`)
    /// below: the HSL values are kept as their own editable state, updated only when the HSL
    /// fields themselves are edited (changes from the wheel, hex, or eyedropper resync it from
    /// `self.color` instead).
    hsl: (f32, f32, f32),
    /// Whether the eyedropper is currently picking a color from the screen. While true,
    /// `show()` calls `poll_eyedropper` every frame, which polls the OS cursor position and the
    /// Escape key directly (the cursor keeps moving across the whole screen even while this
    /// tool window doesn't have OS focus, so this needs `GetCursorPos`/`GetAsyncKeyState`
    /// rather than egui's input events).
    picking_color: bool,
    /// Previous frame's Enter-key state, used by `poll_eyedropper` to detect the key being
    /// pressed (not held).
    eyedropper_prev_confirm_key: bool,
}

impl Default for ToolsState {
    fn default() -> Self {
        let color = egui::Color32::from_rgb(0xF2, 0xA9, 0x3B);
        Self {
            active: None,
            color,
            unit_category: 0,
            unit_from: 0,
            unit_to: 1,
            unit_input: "1".to_string(),
            code_hex: "#F2A93B".to_string(),
            hsl: color::rgb_to_hsl(color.r(), color.g(), color.b()),
            picking_color: false,
            eyedropper_prev_confirm_key: false,
        }
    }
}

const VK_RETURN: i32 = 0x0D;
const VK_ESCAPE: i32 = 0x1B;

fn key_down(vk: i32) -> bool {
    use windows::Win32::UI::Input::KeyboardAndMouse::GetAsyncKeyState;
    unsafe { (GetAsyncKeyState(vk) as u16) & 0x8000 != 0 }
}

/// Eyedropper: reads the pixel color at the given screen coordinates from the whole-desktop
/// device context. `GetDC(None)` returns the whole-desktop DC when hWnd is NULL (per MSDN).
/// Returns `None` on failure.
fn screen_pixel_color(x: i32, y: i32) -> Option<(u8, u8, u8)> {
    use windows::Win32::Graphics::Gdi::{GetDC, GetPixel, ReleaseDC};
    unsafe {
        let hdc = GetDC(None);
        if hdc.is_invalid() {
            return None;
        }
        let color = GetPixel(hdc, x, y);
        ReleaseDC(None, hdc);
        // CLR_INVALID: the coordinates were invalid, or reading the pixel failed.
        if color.0 == 0xFFFF_FFFF {
            return None;
        }
        // COLORREF is in 0x00BBGGRR format.
        let r = (color.0 & 0xFF) as u8;
        let g = ((color.0 >> 8) & 0xFF) as u8;
        let b = ((color.0 >> 16) & 0xFF) as u8;
        Some((r, g, b))
    }
}

impl ToolsState {
    pub fn is_open(&self) -> bool {
        self.active.is_some()
    }

    pub fn toggle(&mut self, kind: ToolKind) {
        // Don't carry an in-progress eyedropper pick across a tool switch/close (it would
        // otherwise keep polling after switching to a different tool).
        self.picking_color = false;
        self.active = if self.active == Some(kind) {
            None
        } else {
            Some(kind)
        };
    }

    pub fn show(&mut self, ctx: &egui::Context, strings: &'static Strings) {
        let Some(active) = self.active else {
            return;
        };

        if self.picking_color {
            self.poll_eyedropper();
        }

        let title = match active {
            ToolKind::ColorPicker => strings.tool_color_picker,
            ToolKind::UnitConverter => strings.tool_unit_converter,
        };
        // Each tool's viewport is sized individually since their content differs. `ColorPicker`
        // is taller than a bare color picker would be, since it also holds the merged color-code
        // display (wheel + swatch + hex row + RGB row + HSL row + eyedropper hint row). In case
        // that still doesn't fit, `ui_chrome::resize_grips` always provides east/south/southeast
        // resize handles — these windows are undecorated, so the OS's own resize border isn't
        // available and this replaces it. The sizes below add 36px for the custom title bar
        // (`TITLE_BAR_HEIGHT`), which eats into the content area.
        let size = match active {
            ToolKind::ColorPicker => [300.0, 516.0],
            ToolKind::UnitConverter => [380.0, 196.0],
        };

        let mut still_open = true;
        ctx.show_viewport_immediate(
            egui::ViewportId::from_hash_of("issen-tool-window"),
            egui::ViewportBuilder::default()
                .with_title(title)
                .with_decorations(false)
                .with_transparent(true)
                .with_inner_size(size)
                // The main window is `with_always_on_top()`; without this, this window would
                // always end up behind it (same reason as `settings_window.rs`).
                .with_always_on_top(),
            |ui, _class| {
                if ui.ctx().input(|i| i.viewport().close_requested()) {
                    still_open = false;
                }

                let dark = ui.visuals().dark_mode;
                let glass = crate::ui_chrome::palette(dark);
                let full_rect = ui.max_rect();
                crate::ui_chrome::glass_panel(ui, full_rect, &glass);
                if crate::ui_chrome::title_bar(ui, full_rect, title, &glass) {
                    still_open = false;
                }
                crate::ui_chrome::resize_grips(ui, full_rect);
                ui.add_space(crate::ui_chrome::TITLE_BAR_HEIGHT);

                egui::CentralPanel::default()
                    .frame(egui::Frame::NONE.inner_margin(egui::Margin::same(16)))
                    .show(ui, |ui| match active {
                        ToolKind::ColorPicker => self.show_color_picker(ui, strings),
                        ToolKind::UnitConverter => self.show_unit_converter(ui, strings),
                    });
            },
        );

        self.active = if still_open { Some(active) } else { None };
        if self.active.is_none() {
            self.picking_color = false;
        }

        // Same reason as the settings window (`src/settings_window.rs`): while this viewport is
        // open, keep requesting repaints explicitly so it doesn't lag behind input as a side
        // effect of the main window's idle-CPU suppression while hidden.
        if self.active.is_some() {
            ctx.request_repaint();
        }
    }

    /// Picks up the color under the cursor (eyedropper). Called every frame from `show()` while
    /// `picking_color` is true. Uses `GetCursorPos`/`GetAsyncKeyState` directly rather than
    /// egui's input events, so cursor movement across the whole screen is tracked even while
    /// this tool window doesn't have OS focus. Confirming with the Enter key rather than a mouse
    /// click is deliberate: a click would also land on whatever window is under the cursor,
    /// potentially opening a link or pressing a button unintentionally. Intercepting mouse input
    /// globally would need a system-wide hook, which is expensive to implement and verify, so
    /// keyboard confirmation was chosen instead.
    fn poll_eyedropper(&mut self) {
        if key_down(VK_ESCAPE) {
            self.picking_color = false;
            return;
        }

        use windows::Win32::Foundation::POINT;
        use windows::Win32::UI::WindowsAndMessaging::GetCursorPos;
        unsafe {
            let mut cursor = POINT::default();
            if GetCursorPos(&mut cursor).is_ok() {
                if let Some((r, g, b)) = screen_pixel_color(cursor.x, cursor.y) {
                    self.color = egui::Color32::from_rgb(r, g, b);
                    // Keep the hex/HSL fields following the eyedropper too (see the sync rule
                    // documented on `show_color_picker`).
                    self.code_hex = color::to_hex(r, g, b);
                    self.hsl = color::rgb_to_hsl(r, g, b);
                }
            }
        }

        // Confirm on the rising edge (the moment the key is pressed), not while held — otherwise
        // the Enter/Space keypress that activated the eyedropper button itself would be picked
        // up and confirm immediately.
        let confirm_down = key_down(VK_RETURN);
        if confirm_down && !self.eyedropper_prev_confirm_key {
            self.picking_color = false;
        }
        self.eyedropper_prev_confirm_key = confirm_down;
    }

    /// The merged color picker (HSV wheel + eyedropper) and color-code conversion (hex/RGB/HSL)
    /// tool. `self.color` is the single source of truth; whichever of the wheel, hex field, or
    /// eyedropper changes it syncs the others, but only at the moment its own `.changed()`
    /// fires. This change-event-driven sync avoids needing to track "which one was edited most
    /// recently" every frame, which would risk another sync pass overwriting the hex field's
    /// string mid-edit.
    fn show_color_picker(&mut self, ui: &mut egui::Ui, strings: &'static Strings) {
        let wheel_changed = egui::widgets::color_picker::color_picker_color32(
            ui,
            &mut self.color,
            egui::widgets::color_picker::Alpha::Opaque,
        );
        if wheel_changed {
            self.code_hex = color::to_hex(self.color.r(), self.color.g(), self.color.b());
            self.hsl = color::rgb_to_hsl(self.color.r(), self.color.g(), self.color.b());
        }

        ui.add_space(4.0);
        let (_, swatch_rect) = ui.allocate_space(egui::vec2(ui.available_width(), 24.0));
        ui.painter().rect_filled(swatch_rect, 2.0, self.color);

        ui.horizontal(|ui| {
            ui.label(strings.label_hex);
            // Reserve width for the copy and eyedropper buttons up front (same pattern as
            // `app.rs`'s `toolbar_width`). Without this, the hex field claims all the
            // remaining width and pushes the buttons off the right edge, making them
            // unreachable at the window's default size.
            let hex_toolbar_width = 2.0 * 28.0;
            let hex_response = ui.add(
                egui::TextEdit::singleline(&mut self.code_hex)
                    .desired_width(ui.available_width() - hex_toolbar_width),
            );
            if hex_response.changed() {
                if let Some((r, g, b)) = color::parse_hex(&self.code_hex) {
                    self.color = egui::Color32::from_rgb(r, g, b);
                    self.hsl = color::rgb_to_hsl(r, g, b);
                }
            }
            if ui.button("📋").clicked() {
                crate::launch::copy_to_clipboard(&color::to_hex(
                    self.color.r(),
                    self.color.g(),
                    self.color.b(),
                ));
            }
            let eyedropper_button = ui.button("💧").on_hover_text(strings.tool_eyedropper);
            if eyedropper_button.clicked() {
                self.picking_color = true;
                self.eyedropper_prev_confirm_key = false;
                // If this button kept focus, the confirming Enter keypress would also be
                // interpreted as a click on it (`.clicked()`), fighting within the same frame
                // against `poll_eyedropper` having just set `picking_color` back to false.
                // Give up focus so that can't happen.
                eyedropper_button.surrender_focus();
            }
        });
        if self.picking_color {
            ui.weak(strings.eyedropper_hint);
        }

        ui.horizontal(|ui| {
            ui.label(strings.label_rgb);
            ui.label(format!(
                "{}, {}, {}",
                self.color.r(),
                self.color.g(),
                self.color.b()
            ));
        });
        let (mut h, mut s, mut l) = self.hsl;
        ui.horizontal(|ui| {
            ui.label(strings.label_hsl);
            let h_resp = ui.add(
                egui::DragValue::new(&mut h)
                    .range(0.0..=360.0)
                    .suffix("°")
                    .speed(1.0),
            );
            let s_resp = ui.add(
                egui::DragValue::new(&mut s)
                    .range(0.0..=100.0)
                    .suffix("%")
                    .speed(0.5),
            );
            let l_resp = ui.add(
                egui::DragValue::new(&mut l)
                    .range(0.0..=100.0)
                    .suffix("%")
                    .speed(0.5),
            );
            if h_resp.changed() || s_resp.changed() || l_resp.changed() {
                self.hsl = (h, s, l);
                let (r, g, b) = color::hsl_to_rgb(h, s, l);
                self.color = egui::Color32::from_rgb(r, g, b);
                self.code_hex = color::to_hex(r, g, b);
            }
        });
    }

    fn show_unit_converter(&mut self, ui: &mut egui::Ui, strings: &'static Strings) {
        let categories = units::CATEGORIES;
        let category_label = |c: units::Category| match c {
            units::Category::Length => strings.unit_length,
            units::Category::Mass => strings.unit_mass,
            units::Category::Temperature => strings.unit_temperature,
            units::Category::Area => strings.unit_area,
            units::Category::Volume => strings.unit_volume,
            units::Category::Speed => strings.unit_speed,
            units::Category::Time => strings.unit_time,
            units::Category::Data => strings.unit_data,
        };

        egui::ComboBox::from_id_salt("issen-tool-unit-category")
            .selected_text(category_label(categories[self.unit_category]))
            .show_ui(ui, |ui| {
                for (i, category) in categories.iter().enumerate() {
                    if ui
                        .selectable_label(self.unit_category == i, category_label(*category))
                        .clicked()
                    {
                        self.unit_category = i;
                        self.unit_from = 0;
                        self.unit_to = 1.min(category.units().len().saturating_sub(1));
                    }
                }
            });

        let category = categories[self.unit_category];
        let unit_list = category.units();

        ui.horizontal(|ui| {
            ui.text_edit_singleline(&mut self.unit_input);
            egui::ComboBox::from_id_salt("issen-tool-unit-from")
                .selected_text(unit_list[self.unit_from].symbol)
                .show_ui(ui, |ui| {
                    for (i, unit) in unit_list.iter().enumerate() {
                        ui.selectable_value(&mut self.unit_from, i, unit.symbol);
                    }
                });
            ui.label("→");
            egui::ComboBox::from_id_salt("issen-tool-unit-to")
                .selected_text(unit_list[self.unit_to].symbol)
                .show_ui(ui, |ui| {
                    for (i, unit) in unit_list.iter().enumerate() {
                        ui.selectable_value(&mut self.unit_to, i, unit.symbol);
                    }
                });
        });

        if let Ok(value) = self.unit_input.parse::<f64>() {
            if let Some(result) = units::convert(category, value, self.unit_from, self.unit_to) {
                ui.label(format!("= {result:.6}"));
            }
        }
    }
}
