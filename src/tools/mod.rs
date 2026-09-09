//! The tools window: a merged color picker (saturation/value square + hue
//! bar + hex/RGB/HSL display + eyedropper) and a unit converter, opened
//! from icon buttons next to the main search box. Ported from the
//! egui/eframe version (see git history) as its own runtime-created GPUI
//! window, using the same glass chrome (`ui_chrome.rs`) and double-open
//! guard as `about_window.rs`/`settings_window.rs`.
//!
//! Unlike the settings window, this needs nothing live from `IssenApp` —
//! just `strings`/`theme`/`accent_color`, snapshotted once at open time
//! (same as `about_window.rs`; see its doc comment for why that sidesteps
//! the entity-reentrancy panic instead of needing settings' `AppSnapshot`+
//! `cx.observe` machinery).
//!
//! Two things don't port over structurally from egui:
//!   - **No HSV wheel widget.** GPUI has no per-pixel canvas/shader, so a
//!     radial hue wheel isn't practical. The saturation/value square is
//!     built from two stacked CSS-style linear-gradient overlays (white→
//!     transparent horizontally, transparent→black vertically) on a solid
//!     hue-colored base, and the hue bar from six 2-stop gradient segments
//!     (red→yellow→green→cyan→blue→magenta→red) — both draggable via a
//!     `canvas()` element that captures the drag surface's window-space
//!     bounds during prepaint, the same bounds-capture need `TextInput`
//!     solves with its own hand-rolled `Element` (not needed here since
//!     these are declarative gradient backgrounds, not shaped text).
//!   - **No per-frame eyedropper poll.** egui's `poll_eyedropper` ran once
//!     per frame because `show()` called `ctx.request_repaint()` every
//!     frame while a tool window was open. GPUI has no such continuous
//!     repaint loop — `render()` only runs on `cx.notify()` — so the
//!     eyedropper instead drives its own poll loop via `cx.spawn` +
//!     `background_executor().timer(...)`, ticking roughly 60 times/sec
//!     until Escape/Enter or the window closes.
pub mod color;
pub mod units;

use std::time::Duration;

use gpui::{
    canvas, div, hsla, linear_color_stop, linear_gradient, point, prelude::*, px, rems, size,
    white, App, AppContext, Bounds, Context, Entity, Hsla, InteractiveElement, IntoElement,
    MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, ParentElement, Pixels, Point,
    Render, SharedString, Styled, Window, WindowBackgroundAppearance, WindowBounds, WindowHandle,
    WindowKind, WindowOptions,
};

use crate::config::{AccentColor, Theme, UiFont};
use crate::i18n::Strings;
use crate::text_input::{self, TextInput};
use crate::ui_chrome::{self, GlassPalette};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolKind {
    ColorPicker,
    UnitConverter,
}

const SV_SIZE: (f32, f32) = (268.0, 160.0);
const HUE_BAR_HEIGHT: f32 = 20.0;
const COLOR_WINDOW_SIZE: (f32, f32) = (300.0, 540.0);
const UNITS_WINDOW_SIZE: (f32, f32) = (380.0, 440.0);
const MIN_WINDOW_SIZE: (f32, f32) = (260.0, 300.0);

/// Opens the tools window on `kind`, or brings an already-open one to the
/// front and switches its content to `kind` in place — matching the
/// egui/eframe version's behavior, where both tools shared a single
/// viewport (clicking the other tool's toolbar button while one is open
/// replaces the content, not a second window).  Closing on a repeat click
/// of the *same* tool is the caller's job (`IssenApp::toggle_tool`), since
/// that's a decision about the toolbar button, not about opening a window.
#[allow(clippy::too_many_arguments)]
pub fn open(
    existing: &mut Option<WindowHandle<ToolsWindow>>,
    kind: ToolKind,
    strings: &'static Strings,
    theme: Theme,
    ui_font: UiFont,
    font_scale: f32,
    accent_color: AccentColor,
    cx: &mut App,
) {
    if let Some(handle) = existing {
        if handle
            .update(cx, |view, window, cx| {
                view.kind = kind;
                window.activate_window();
                cx.notify();
            })
            .is_ok()
        {
            return;
        }
    }

    let window_size = match kind {
        ToolKind::ColorPicker => COLOR_WINDOW_SIZE,
        ToolKind::UnitConverter => UNITS_WINDOW_SIZE,
    };
    let bounds = Bounds {
        origin: point(px(140.), px(140.)),
        size: size(px(window_size.0), px(window_size.1)),
    };
    let handle = cx
        .open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                titlebar: None,
                focus: true,
                show: true,
                kind: WindowKind::Normal,
                is_movable: true,
                is_resizable: true,
                window_min_size: Some(size(px(MIN_WINDOW_SIZE.0), px(MIN_WINDOW_SIZE.1))),
                // 設定画面と同様、半透明背景は背景コンテンツに埋もれて
                // 読みづらいとの指摘を受け不透明化(about共有パレットは変更なし)
                window_background: WindowBackgroundAppearance::Opaque,
                ..Default::default()
            },
            move |window, cx| {
                ui_chrome::set_topmost(window);
                cx.new(|cx| {
                    ToolsWindow::new(kind, strings, theme, ui_font, font_scale, accent_color, cx)
                })
            },
        )
        .expect("failed to open tools window");
    *existing = Some(handle);
}

pub struct ToolsWindow {
    kind: ToolKind,
    strings: &'static Strings,
    theme: Theme,
    ui_font: UiFont,
    font_scale: f32,
    accent_color: AccentColor,

    // Color picker state. Hue/saturation/value are kept as their own
    // persistent fields (HSV, not derived fresh from RGB every render) for
    // the same reason the egui version kept a separate editable `hsl`
    // field: recomputing from RGB loses hue/saturation whenever the color
    // passes through zero saturation (gray) or zero value (black), and
    // once lost it can't be recovered — dragging the square straight up
    // from black would otherwise reset the hue bar to an arbitrary value.
    hue: f32,
    sat: f32,
    val: f32,
    hex_input: Entity<TextInput>,
    picking_color: bool,
    eyedropper_prev_confirm_key: bool,
    sv_dragging: bool,
    hue_dragging: bool,
    /// Window-space bounds of the SV square / hue bar's drag surface,
    /// captured via a `canvas()` overlay's `prepaint` each render. `None`
    /// only before the very first paint (never observed in practice, since
    /// a click can't happen before the window has painted once).
    sv_bounds: Option<Bounds<Pixels>>,
    hue_bounds: Option<Bounds<Pixels>>,

    // Unit converter state.
    unit_category: usize,
    unit_from: usize,
    unit_to: usize,
    unit_input: Entity<TextInput>,
}

impl ToolsWindow {
    fn new(
        kind: ToolKind,
        strings: &'static Strings,
        theme: Theme,
        ui_font: UiFont,
        font_scale: f32,
        accent_color: AccentColor,
        cx: &mut Context<Self>,
    ) -> Self {
        let (init_r, init_g, init_b) = (0xF2u8, 0xA9u8, 0x3Bu8);
        let (hue, sat, val) = color::rgb_to_hsv(init_r, init_g, init_b);

        let hex_input = cx.new(|cx| TextInput::new(cx, color::to_hex(init_r, init_g, init_b), ""));
        cx.observe(&hex_input, |this, entity, cx| {
            // Only reached for a genuine edit to the hex field itself
            // (typing, or a future direct `set_text` call) — programmatic
            // sync from the wheel/eyedropper uses `set_text_silent`, which
            // doesn't notify, specifically so it doesn't loop back here.
            let text = entity.read(cx).text().to_string();
            if let Some((r, g, b)) = color::parse_hex(&text) {
                let (h, s, v) = color::rgb_to_hsv(r, g, b);
                this.hue = h;
                this.sat = s;
                this.val = v;
            }
            cx.notify();
        })
        .detach();

        let unit_input = cx.new(|cx| TextInput::new(cx, "1", ""));
        cx.observe(&unit_input, |_, _, cx| cx.notify()).detach();

        Self {
            kind,
            strings,
            theme,
            ui_font,
            font_scale,
            accent_color,
            hue,
            sat,
            val,
            hex_input,
            picking_color: false,
            eyedropper_prev_confirm_key: false,
            sv_dragging: false,
            hue_dragging: false,
            sv_bounds: None,
            hue_bounds: None,
            unit_category: 0,
            unit_from: 0,
            unit_to: 1,
            unit_input,
        }
    }

    /// Which tool is currently showing — read by `IssenApp::toggle_tool` to
    /// decide whether a toolbar click should close this window (clicking
    /// the same tool again) or switch its content (clicking the other one).
    pub(crate) fn kind(&self) -> ToolKind {
        self.kind
    }

    // --- Color picker ---

    fn rgb(&self) -> (u8, u8, u8) {
        color::hsv_to_rgb(self.hue, self.sat, self.val)
    }

    /// Pushes the current color into the hex field's display text without
    /// treating it as a user edit (see `hex_input`'s `cx.observe` above and
    /// `TextInput::set_text_silent`'s doc comment for why).
    fn sync_hex(&mut self, cx: &mut Context<Self>) {
        let (r, g, b) = self.rgb();
        let hex = color::to_hex(r, g, b);
        self.hex_input.update(cx, |ti, _| ti.set_text_silent(hex));
    }

    fn set_sat_val_from_position(&mut self, position: Point<Pixels>, cx: &mut Context<Self>) {
        let Some(bounds) = self.sv_bounds else {
            return;
        };
        let width = f32::from(bounds.size.width);
        let height = f32::from(bounds.size.height);
        if width <= 0.0 || height <= 0.0 {
            return;
        }
        let local_x = f32::from(position.x - bounds.origin.x);
        let local_y = f32::from(position.y - bounds.origin.y);
        self.sat = (local_x / width).clamp(0.0, 1.0);
        self.val = (1.0 - local_y / height).clamp(0.0, 1.0);
        self.sync_hex(cx);
        cx.notify();
    }

    fn set_hue_from_position(&mut self, position: Point<Pixels>, cx: &mut Context<Self>) {
        let Some(bounds) = self.hue_bounds else {
            return;
        };
        let width = f32::from(bounds.size.width);
        if width <= 0.0 {
            return;
        }
        let local_x = f32::from(position.x - bounds.origin.x);
        self.hue = ((local_x / width) * 360.0).clamp(0.0, 360.0);
        self.sync_hex(cx);
        cx.notify();
    }

    fn on_sv_mouse_down(&mut self, event: &MouseDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        self.sv_dragging = true;
        self.set_sat_val_from_position(event.position, cx);
    }

    fn on_hue_mouse_down(
        &mut self,
        event: &MouseDownEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.hue_dragging = true;
        self.set_hue_from_position(event.position, cx);
    }

    /// Registered on the whole window content, not just the SV square/hue
    /// bar, so a fast drag that briefly leaves the small drag surface
    /// (but stays inside the window) keeps tracking — `div`'s own
    /// `on_mouse_move` only fires while the cursor is over that specific
    /// element's hitbox.
    fn on_root_mouse_move(
        &mut self,
        event: &MouseMoveEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.sv_dragging {
            self.set_sat_val_from_position(event.position, cx);
        } else if self.hue_dragging {
            self.set_hue_from_position(event.position, cx);
        }
    }

    fn on_root_mouse_up(&mut self, _: &MouseUpEvent, _: &mut Window, cx: &mut Context<Self>) {
        if self.sv_dragging || self.hue_dragging {
            self.sv_dragging = false;
            self.hue_dragging = false;
            cx.notify();
        }
    }

    fn start_eyedropper(&mut self, cx: &mut Context<Self>) {
        if self.picking_color {
            return;
        }
        self.picking_color = true;
        self.eyedropper_prev_confirm_key = false;
        cx.notify();

        cx.spawn(async move |this, cx| loop {
            cx.background_executor()
                .timer(Duration::from_millis(16))
                .await;
            let Ok(still_picking) = this.update(cx, |view, cx| view.poll_eyedropper(cx)) else {
                break;
            };
            if !still_picking {
                break;
            }
        })
        .detach();
    }

    /// One eyedropper poll tick: samples the pixel under the OS cursor and
    /// checks Escape/Enter directly via Win32 (not GPUI input events), so
    /// this tracks the cursor across the whole screen even though this
    /// window never has OS focus while picking. Returns whether the caller
    /// should keep polling. See the module doc comment for why this is a
    /// background-timer loop rather than a per-frame poll.
    fn poll_eyedropper(&mut self, cx: &mut Context<Self>) -> bool {
        if !self.picking_color {
            return false;
        }
        if key_down(VK_ESCAPE) {
            self.picking_color = false;
            cx.notify();
            return false;
        }

        unsafe {
            let mut cursor = windows::Win32::Foundation::POINT::default();
            if windows::Win32::UI::WindowsAndMessaging::GetCursorPos(&mut cursor).is_ok() {
                if let Some((r, g, b)) = screen_pixel_color(cursor.x, cursor.y) {
                    let (h, s, v) = color::rgb_to_hsv(r, g, b);
                    self.hue = h;
                    self.sat = s;
                    self.val = v;
                    self.sync_hex(cx);
                }
            }
        }

        // Confirm on the rising edge (the moment Enter is pressed), not
        // while held — otherwise the Enter/Space keypress that activated
        // the eyedropper button itself would be picked up and confirm
        // immediately.
        let confirm_down = key_down(VK_RETURN);
        if confirm_down && !self.eyedropper_prev_confirm_key {
            self.picking_color = false;
        }
        self.eyedropper_prev_confirm_key = confirm_down;
        cx.notify();
        self.picking_color
    }

    fn sv_square(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let base = hsla(self.hue / 360.0, 1.0, 0.5, 1.0);
        let this = cx.entity();
        div()
            .relative()
            .w(px(SV_SIZE.0))
            .h(px(SV_SIZE.1))
            .rounded(px(6.))
            .overflow_hidden()
            .bg(base)
            .child(div().absolute().inset_0().bg(linear_gradient(
                90.,
                linear_color_stop(hsla(0., 0., 1., 1.), 0.0),
                linear_color_stop(hsla(0., 0., 1., 0.), 1.0),
            )))
            .child(div().absolute().inset_0().bg(linear_gradient(
                180.,
                linear_color_stop(hsla(0., 0., 0., 0.), 0.0),
                linear_color_stop(hsla(0., 0., 0., 1.), 1.0),
            )))
            .child(
                canvas(
                    move |bounds, _, cx| {
                        this.update(cx, |view, _| view.sv_bounds = Some(bounds));
                    },
                    |_, _, _, _| {},
                )
                .absolute()
                .inset_0(),
            )
            .child(
                div()
                    .absolute()
                    .left(px(self.sat * SV_SIZE.0 - 6.))
                    .top(px((1.0 - self.val) * SV_SIZE.1 - 6.))
                    .size(px(12.))
                    .rounded_full()
                    .border_1()
                    .border_color(white()),
            )
            .on_mouse_down(MouseButton::Left, cx.listener(Self::on_sv_mouse_down))
    }

    fn hue_bar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let this = cx.entity();
        const STOPS: [f32; 7] = [0., 60., 120., 180., 240., 300., 360.];
        let segments = (0..6).map(|i| {
            let from = hsla(STOPS[i] / 360., 1.0, 0.5, 1.0);
            let to = hsla(STOPS[i + 1] / 360., 1.0, 0.5, 1.0);
            div().flex_1().h_full().bg(linear_gradient(
                90.,
                linear_color_stop(from, 0.0),
                linear_color_stop(to, 1.0),
            ))
        });

        div()
            .relative()
            .w(px(SV_SIZE.0))
            .h(px(HUE_BAR_HEIGHT))
            .rounded(px(4.))
            .overflow_hidden()
            .flex()
            .children(segments)
            .child(
                canvas(
                    move |bounds, _, cx| {
                        this.update(cx, |view, _| view.hue_bounds = Some(bounds));
                    },
                    |_, _, _, _| {},
                )
                .absolute()
                .inset_0(),
            )
            .child(
                div()
                    .absolute()
                    .left(px((self.hue / 360.0) * SV_SIZE.0 - 2.))
                    .top(px(0.))
                    .w(px(4.))
                    .h(px(HUE_BAR_HEIGHT))
                    .bg(white())
                    .border_1()
                    .border_color(hsla(0., 0., 0., 0.4)),
            )
            .on_mouse_down(MouseButton::Left, cx.listener(Self::on_hue_mouse_down))
    }

    fn icon_button(
        key: &'static str,
        glyph: &'static str,
        palette: &GlassPalette,
        on_click: impl Fn(&MouseDownEvent, &mut Window, &mut App) + 'static,
    ) -> impl IntoElement {
        div()
            .id(key)
            .flex()
            .items_center()
            .justify_center()
            .size(px(28.))
            .rounded(px(6.))
            .cursor_pointer()
            .text_size(rems(14. / 16.))
            .hover(|d| d.bg(palette.control_bg))
            .on_mouse_down(MouseButton::Left, on_click)
            .child(glyph)
    }

    fn render_color_tab(
        &self,
        palette: &GlassPalette,
        accent: Hsla,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let (r, g, b) = self.rgb();
        let packed = ((r as u32) << 24) | ((g as u32) << 16) | ((b as u32) << 8) | 0xFF;
        let swatch_color: Hsla = gpui::rgba(packed).into();
        let (hsl_h, hsl_s, hsl_l) = color::rgb_to_hsl(r, g, b);
        let hex_for_copy = color::to_hex(r, g, b);
        let strings = self.strings;

        div()
            .flex()
            .flex_col()
            .gap_3()
            .child(self.sv_square(cx))
            .child(self.hue_bar(cx))
            .child(
                div()
                    .w_full()
                    .h(px(28.))
                    .rounded(px(6.))
                    .border_1()
                    .border_color(palette.border)
                    .bg(swatch_color),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(div().flex_1().child(text_input::text_field(
                        &self.hex_input,
                        palette.text,
                        accent,
                        palette.control_bg,
                        palette.control_border,
                        cx,
                    )))
                    .child(Self::icon_button(
                        "tool-copy-hex",
                        "\u{1F4CB}",
                        palette,
                        move |_, _, _cx| {
                            crate::launch::copy_to_clipboard(&hex_for_copy);
                        },
                    ))
                    .child(Self::icon_button(
                        "tool-eyedropper",
                        "\u{1F4A7}",
                        palette,
                        cx.listener(|this, _, _, cx| this.start_eyedropper(cx)),
                    )),
            )
            .when(self.picking_color, |d| {
                d.child(
                    div()
                        .text_size(rems(11. / 16.))
                        .text_color(palette.subtext)
                        .child(SharedString::from(strings.eyedropper_hint)),
                )
            })
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(
                        div()
                            .w(px(32.))
                            .text_size(rems(12. / 16.))
                            .text_color(palette.subtext)
                            .child(SharedString::from(strings.label_rgb)),
                    )
                    .child(
                        div()
                            .text_size(rems(12. / 16.))
                            .text_color(palette.text)
                            .child(format!("{r}, {g}, {b}")),
                    ),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(
                        div()
                            .w(px(32.))
                            .text_size(rems(12. / 16.))
                            .text_color(palette.subtext)
                            .child(SharedString::from(strings.label_hsl)),
                    )
                    .child(
                        div()
                            .text_size(rems(12. / 16.))
                            .text_color(palette.text)
                            .child(format!("{hsl_h:.0}\u{b0}, {hsl_s:.0}%, {hsl_l:.0}%")),
                    ),
            )
    }

    // --- Unit converter ---

    fn chip(
        key: impl std::fmt::Display,
        label: SharedString,
        selected: bool,
        palette: &GlassPalette,
        accent: Hsla,
        on_click: impl Fn(&MouseDownEvent, &mut Window, &mut App) + 'static,
    ) -> impl IntoElement {
        div()
            .id(SharedString::from(format!("tool-chip-{key}")))
            .px(px(10.))
            .h(px(26.))
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(6.))
            .cursor_pointer()
            .text_size(rems(12. / 16.))
            .text_color(if selected { accent } else { palette.subtext })
            .when(selected, |d| d.bg(palette.control_bg))
            .when(!selected, |d| {
                d.border_1().border_color(palette.control_border)
            })
            .on_mouse_down(MouseButton::Left, on_click)
            .child(label)
    }

    fn chip_group_label(label: &'static str, palette: &GlassPalette) -> impl IntoElement {
        div()
            .text_size(rems(11. / 16.))
            .text_color(palette.subtext)
            .child(SharedString::from(label))
    }

    fn render_units_tab(
        &self,
        palette: &GlassPalette,
        accent: Hsla,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let strings = self.strings;
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

        let category = categories[self.unit_category];
        let unit_list = category.units();

        let result_text = self
            .unit_input
            .read(cx)
            .text()
            .parse::<f64>()
            .ok()
            .and_then(|value| units::convert(category, value, self.unit_from, self.unit_to))
            .map(|r| format!("= {r:.6}"));

        div()
            .flex()
            .flex_col()
            .gap_3()
            .child(text_input::text_field(
                &self.unit_input,
                palette.text,
                accent,
                palette.control_bg,
                palette.control_border,
                cx,
            ))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(Self::chip_group_label(strings.label_unit_category, palette))
                    .child(div().flex().flex_wrap().gap_1().children(
                        categories.iter().enumerate().map(|(i, c)| {
                            let selected = i == self.unit_category;
                            Self::chip(
                                format!("cat-{i}"),
                                SharedString::from(category_label(*c)),
                                selected,
                                palette,
                                accent,
                                cx.listener(move |this, _, _, cx| {
                                    this.unit_category = i;
                                    this.unit_from = 0;
                                    this.unit_to =
                                        1.min(units::CATEGORIES[i].units().len().saturating_sub(1));
                                    cx.notify();
                                }),
                            )
                        }),
                    )),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(Self::chip_group_label(strings.label_unit_from, palette))
                    .child(div().flex().flex_wrap().gap_1().children(
                        unit_list.iter().enumerate().map(|(i, u)| {
                            let selected = i == self.unit_from;
                            Self::chip(
                                format!("from-{i}"),
                                SharedString::from(u.symbol),
                                selected,
                                palette,
                                accent,
                                cx.listener(move |this, _, _, cx| {
                                    this.unit_from = i;
                                    cx.notify();
                                }),
                            )
                        }),
                    )),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(Self::chip_group_label(strings.label_unit_to, palette))
                    .child(div().flex().flex_wrap().gap_1().children(
                        unit_list.iter().enumerate().map(|(i, u)| {
                            let selected = i == self.unit_to;
                            Self::chip(
                                format!("to-{i}"),
                                SharedString::from(u.symbol),
                                selected,
                                palette,
                                accent,
                                cx.listener(move |this, _, _, cx| {
                                    this.unit_to = i;
                                    cx.notify();
                                }),
                            )
                        }),
                    )),
            )
            .child(
                div()
                    .text_size(rems(18. / 16.))
                    .text_color(accent)
                    .child(result_text.unwrap_or_default()),
            )
    }
}

impl Render for ToolsWindow {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        ui_chrome::apply_font_scale(window, self.font_scale);
        let dark = ui_chrome::resolve_dark(self.theme, window);
        let palette = ui_chrome::palette(dark);
        let accent = ui_chrome::accent_color(self.accent_color);
        let title = match self.kind {
            ToolKind::ColorPicker => self.strings.tool_color_picker,
            ToolKind::UnitConverter => self.strings.tool_unit_converter,
        };

        let content = match self.kind {
            ToolKind::ColorPicker => self
                .render_color_tab(&palette, accent, cx)
                .into_any_element(),
            ToolKind::UnitConverter => self
                .render_units_tab(&palette, accent, cx)
                .into_any_element(),
        };

        ui_chrome::glass_container(&palette)
            .font_family(crate::fonts::ui_font_family(self.ui_font))
            .bg(ui_chrome::opaque_panel_bg(dark))
            .on_mouse_move(cx.listener(Self::on_root_mouse_move))
            .on_mouse_up(MouseButton::Left, cx.listener(Self::on_root_mouse_up))
            .on_mouse_up_out(MouseButton::Left, cx.listener(Self::on_root_mouse_up))
            .child(ui_chrome::title_bar(title, &palette))
            .child(div().flex_1().p(px(16.)).child(content))
    }
}

// --- Win32 eyedropper helpers (ported verbatim; no egui dependency) ---

const VK_RETURN: i32 = 0x0D;
const VK_ESCAPE: i32 = 0x1B;

fn key_down(vk: i32) -> bool {
    use windows::Win32::UI::Input::KeyboardAndMouse::GetAsyncKeyState;
    unsafe { (GetAsyncKeyState(vk) as u16) & 0x8000 != 0 }
}

/// Reads the pixel color at the given screen coordinates from the
/// whole-desktop device context. `GetDC(None)` returns the whole-desktop
/// DC when hWnd is NULL (per MSDN). Returns `None` on failure.
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
