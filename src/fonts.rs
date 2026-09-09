use crate::config::UiFont;

/// The Win32 family name for a given `UiFont` choice. Unlike the egui/eframe
/// version (which had to load `.ttf`/`.ttc` bytes by hand and hand them to
/// `egui::Context::set_fonts`, since egui does its own text shaping), GPUI's
/// Windows text system shapes text through DirectWrite, which resolves an
/// installed family by name on its own — no file loading needed here.
///
/// CJK glyphs aren't handled here either: DirectWrite performs its own
/// system font-fallback for glyphs missing from the chosen family whenever a
/// `Font`'s `fallbacks` field is left unset (confirmed against
/// `gpui_windows`'s `direct_write.rs`), so Japanese text already renders
/// correctly regardless of which of these three Latin-oriented families is
/// selected.
pub fn ui_font_family(font: UiFont) -> &'static str {
    match font {
        UiFont::SegoeUi => "Segoe UI",
        UiFont::YuGothic => "Yu Gothic",
        UiFont::Meiryo => "Meiryo",
    }
}
