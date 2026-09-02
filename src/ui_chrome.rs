//! Theme constants for the translucent glass panel shared by the settings
//! window, color picker, unit converter, and main search box. Implemented
//! via `with_transparent(true)` + zero-alpha `clear_color` + a rounded,
//! translucent rectangle painted by `painter` — verified on real hardware,
//! not dependent on DWM composition APIs like Mica/Acrylic (see
//! docs/architecture/ui-appearance.md).

/// The actual color for a given `config.accent_color`. The default `Lime`
/// is the reference design's `oklch(0.9 0.19 124)` converted to sRGB (same
/// value as the old fixed `ACCENT` constant). The other colors are
/// pre-converted the same way, keeping roughly the same OKLCH lightness and
/// chroma and only varying hue, and are shared across light/dark (the
/// accent stays fixed as a brand color regardless of theme).
pub fn accent_color(theme: crate::config::AccentColor) -> egui::Color32 {
    use crate::config::AccentColor;
    match theme {
        AccentColor::Lime => egui::Color32::from_rgb(196, 242, 82),
        AccentColor::Red => egui::Color32::from_rgb(255, 116, 110),
        AccentColor::Orange => egui::Color32::from_rgb(255, 166, 61),
        AccentColor::Blue => egui::Color32::from_rgb(108, 195, 255),
        AccentColor::Purple => egui::Color32::from_rgb(215, 152, 255),
    }
}

/// Set to `0.0` (square) rather than rounded. Painting this panel with
/// rounded corners caused a visible seam on dark wallpaper, where the
/// corner cutout (the part of the panel rect outside the rounded arc, left
/// transparent via zero-alpha `clear_color`) showed up as a hard edge; the
/// root cause was never pinned down, so this was dropped to `0.0` as a
/// workaround instead. On real hardware the window still ends up with a
/// slight, seam-free rounding regardless of this constant — that's Windows
/// 11's own automatic corner rounding, a separate layer from this panel's
/// paint call — so the final look is unaffected by keeping this at `0.0`,
/// and there's no need to revisit rounding it here again.
pub const PANEL_ROUNDING: f32 = 0.0;

/// A full glass-panel color set. [`palette`] picks one based on
/// `config.theme`'s light/dark resolution (`ui.visuals().dark_mode`).
/// Unlike the main search box, which always stays dark regardless of theme
/// (see docs/architecture/ui-appearance.md), the settings window and tool windows follow both
/// light and dark.
pub struct GlassPalette {
    pub panel_bg: egui::Color32,
    pub border: egui::Color32,
    pub text: egui::Color32,
    pub subtext: egui::Color32,
    pub divider: egui::Color32,
    pub control_bg: egui::Color32,
    /// Intended for the color picker's/unit converter's input field borders
    /// (not wired up yet).
    #[allow(dead_code)]
    pub control_border: egui::Color32,
}

pub fn palette(dark: bool) -> GlassPalette {
    if dark {
        GlassPalette {
            panel_bg: egui::Color32::from_rgba_unmultiplied(18, 20, 26, 150),
            border: egui::Color32::from_rgba_unmultiplied(255, 255, 255, 36),
            text: egui::Color32::from_rgb(242, 244, 247),
            subtext: egui::Color32::from_rgba_unmultiplied(255, 255, 255, 97),
            divider: egui::Color32::from_rgba_unmultiplied(255, 255, 255, 26),
            control_bg: egui::Color32::from_rgba_unmultiplied(255, 255, 255, 15),
            control_border: egui::Color32::from_rgba_unmultiplied(255, 255, 255, 30),
        }
    } else {
        GlassPalette {
            panel_bg: egui::Color32::from_rgba_unmultiplied(250, 250, 252, 195),
            border: egui::Color32::from_rgba_unmultiplied(0, 0, 0, 18),
            text: egui::Color32::from_rgb(30, 32, 36),
            subtext: egui::Color32::from_rgba_unmultiplied(0, 0, 0, 115),
            divider: egui::Color32::from_rgba_unmultiplied(0, 0, 0, 18),
            control_bg: egui::Color32::from_rgba_unmultiplied(0, 0, 0, 10),
            control_border: egui::Color32::from_rgba_unmultiplied(0, 0, 0, 26),
        }
    }
}

/// Paints the rounded, translucent panel background and border covering
/// the whole window. Callers should layer their content on top with a
/// backgroundless frame, e.g.
/// `egui::CentralPanel::default().frame(egui::Frame::NONE)`.
pub fn glass_panel(ui: &egui::Ui, rect: egui::Rect, palette: &GlassPalette) {
    let painter = ui.painter();
    painter.rect_filled(rect, PANEL_ROUNDING, palette.panel_bg);
    painter.rect_stroke(
        rect,
        PANEL_ROUNDING,
        egui::Stroke::new(1.0, palette.border),
        egui::StrokeKind::Inside,
    );
}

pub const TITLE_BAR_HEIGHT: f32 = 36.0;
const RESIZE_MARGIN: f32 = 6.0;

/// The custom title bar shared by the settings window, color picker, and
/// unit converter. Handles window drag-to-move and the close (×) button.
/// Returns whether the close button was clicked (the caller closes the
/// window itself).
///
/// The drag region uses `Sense::click_and_drag()` — a drag-only `Sense`
/// silently drops click-type events like a right-click context menu (see
/// docs/architecture/window-lifecycle.md), so this includes clicking too.
pub fn title_bar(ui: &mut egui::Ui, rect: egui::Rect, title: &str, palette: &GlassPalette) -> bool {
    let close_size = 26.0;
    let close_rect = egui::Rect::from_min_size(
        egui::pos2(
            rect.right() - close_size - 8.0,
            rect.top() + (TITLE_BAR_HEIGHT - close_size) / 2.0,
        ),
        egui::vec2(close_size, close_size),
    );
    let drag_rect = egui::Rect::from_min_max(
        rect.min,
        egui::pos2(close_rect.left() - 4.0, rect.top() + TITLE_BAR_HEIGHT),
    );

    let drag_id = ui.id().with("chrome-drag");
    let drag_response = ui.interact(drag_rect, drag_id, egui::Sense::click_and_drag());
    if drag_response.drag_started() {
        ui.ctx().send_viewport_cmd(egui::ViewportCommand::StartDrag);
    }

    ui.painter().text(
        egui::pos2(rect.left() + 16.0, rect.top() + TITLE_BAR_HEIGHT / 2.0),
        egui::Align2::LEFT_CENTER,
        title,
        egui::FontId::monospace(11.0),
        palette.subtext,
    );
    ui.painter().hline(
        rect.left() + 4.0..=rect.right() - 4.0,
        rect.top() + TITLE_BAR_HEIGHT,
        egui::Stroke::new(1.0, palette.divider),
    );

    let close_id = ui.id().with("chrome-close");
    let close_response = ui.interact(close_rect, close_id, egui::Sense::click());
    let close_color = if close_response.hovered() {
        palette.text
    } else {
        palette.subtext
    };
    ui.painter().text(
        close_rect.center(),
        egui::Align2::CENTER_CENTER,
        "\u{2715}",
        egui::FontId::proportional(13.0),
        close_color,
    );

    close_response.clicked()
}

/// Resize handles on the east edge, south edge, and southeast corner.
/// Starting a drag inside a hit region delegates to the OS via
/// `ViewportCommand::BeginResize` rather than computing resize behavior by
/// hand. Only these three directions are handled, as the minimum needed
/// for windows (like the color picker) whose height varies with content —
/// north/west aren't used anywhere currently.
pub fn resize_grips(ui: &mut egui::Ui, rect: egui::Rect) {
    let m = RESIZE_MARGIN;
    let corner = m * 2.5;

    let east = egui::Rect::from_min_max(
        egui::pos2(rect.right() - m, rect.top() + TITLE_BAR_HEIGHT),
        egui::pos2(rect.right(), rect.bottom() - corner),
    );
    let south = egui::Rect::from_min_max(
        egui::pos2(rect.left() + corner, rect.bottom() - m),
        egui::pos2(rect.right() - corner, rect.bottom()),
    );
    let south_east = egui::Rect::from_min_max(
        egui::pos2(rect.right() - corner, rect.bottom() - corner),
        rect.max,
    );

    resize_grip(
        ui,
        east,
        "east",
        egui::CursorIcon::ResizeEast,
        egui::viewport::ResizeDirection::East,
    );
    resize_grip(
        ui,
        south,
        "south",
        egui::CursorIcon::ResizeSouth,
        egui::viewport::ResizeDirection::South,
    );
    resize_grip(
        ui,
        south_east,
        "south-east",
        egui::CursorIcon::ResizeSouthEast,
        egui::viewport::ResizeDirection::SouthEast,
    );
}

fn resize_grip(
    ui: &mut egui::Ui,
    rect: egui::Rect,
    salt: &str,
    cursor: egui::CursorIcon,
    direction: egui::viewport::ResizeDirection,
) {
    let id = ui.id().with(("chrome-resize", salt));
    let response = ui.interact(rect, id, egui::Sense::drag());
    if response.hovered() || response.dragged() {
        ui.ctx().set_cursor_icon(cursor);
    }
    if response.drag_started() {
        ui.ctx()
            .send_viewport_cmd(egui::ViewportCommand::BeginResize(direction));
    }
}
