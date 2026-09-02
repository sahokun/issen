use crate::config::UiFont;

/// egui's `default_fonts` (Ubuntu-Light, etc.) don't include CJK glyphs, so a
/// Windows-bundled font is loaded at runtime as a fallback instead. The font files
/// themselves aren't bundled in the zip distribution.
const CJK_FALLBACK_CANDIDATES: &[&str] = &[
    r"C:\Windows\Fonts\meiryo.ttc",
    r"C:\Windows\Fonts\YuGothR.ttc",
    r"C:\Windows\Fonts\msgothic.ttc",
];

/// The monospace face isn't user-selectable, unlike the proportional UI font below —
/// it's used for structural elements like path display rather than being a matter of
/// preference. Fixed to `Consolas`, which ships with Windows.
const MONOSPACE_CANDIDATE: &str = r"C:\Windows\Fonts\consola.ttf";

const CJK_FALLBACK_KEY: &str = "cjk-fallback";
const UI_FONT_KEY: &str = "ui-font";
const MONOSPACE_KEY: &str = "monospace-font";

/// The actual file path for a given `UiFont` choice. Deliberately not derived by
/// enumerating system fonts (e.g. via DirectWrite) — mapping a family name to an
/// actual file (and, for a multi-weight `.ttc`, which face to use) that way is
/// brittle, so only a short list of verified candidates is supported here.
fn ui_font_path(font: UiFont) -> &'static str {
    match font {
        UiFont::SegoeUi => r"C:\Windows\Fonts\segoeui.ttf",
        UiFont::YuGothic => r"C:\Windows\Fonts\YuGothR.ttc",
        UiFont::Meiryo => r"C:\Windows\Fonts\meiryo.ttc",
    }
}

/// Applies `config.ui_font`: the chosen Latin proportional face goes first, with CJK
/// fallback candidates stacked after it. The monospace family is fixed to `Consolas`
/// plus the same CJK fallback. `egui::Context::set_fonts` replaces the whole
/// `FontDefinitions` each time, so this function rebuilds it from scratch on every
/// call.
pub fn apply_fonts(ctx: &egui::Context, ui_font: UiFont) {
    let mut fonts = egui::FontDefinitions::default();

    if let Ok(bytes) = std::fs::read(ui_font_path(ui_font)) {
        fonts.font_data.insert(
            UI_FONT_KEY.to_owned(),
            std::sync::Arc::new(egui::FontData::from_owned(bytes)),
        );
        fonts
            .families
            .entry(egui::FontFamily::Proportional)
            .or_default()
            .insert(0, UI_FONT_KEY.to_owned());
    }

    if let Ok(bytes) = std::fs::read(MONOSPACE_CANDIDATE) {
        fonts.font_data.insert(
            MONOSPACE_KEY.to_owned(),
            std::sync::Arc::new(egui::FontData::from_owned(bytes)),
        );
        fonts
            .families
            .entry(egui::FontFamily::Monospace)
            .or_default()
            .insert(0, MONOSPACE_KEY.to_owned());
    }

    if let Some(bytes) = CJK_FALLBACK_CANDIDATES
        .iter()
        .find_map(|path| std::fs::read(path).ok())
    {
        fonts.font_data.insert(
            CJK_FALLBACK_KEY.to_owned(),
            std::sync::Arc::new(egui::FontData::from_owned(bytes)),
        );
        for family in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
            fonts
                .families
                .entry(family)
                .or_default()
                .push(CJK_FALLBACK_KEY.to_owned());
        }
    }

    ctx.set_fonts(fonts);
}
