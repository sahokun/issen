use std::path::PathBuf;

use serde::{Deserialize, Serialize};

pub const APP_NAME: &str = "Issen";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub hotkey: String,
    pub autostart: bool,
    pub max_results: u32,
    pub theme: Theme,
    /// The accent color theme, independent of the light/dark `theme` axis.
    /// `src/ui_chrome.rs::accent_color` converts this to an actual color.
    pub accent_color: AccentColor,
    /// UI-wide font size multiplier, default `1.0`. `src/app.rs`'s
    /// `apply_font_scale` scales `egui::Style::text_styles` by this.
    pub font_scale: f32,
    pub language: Language,
    /// Which display the search results window shows on.
    pub display_target: DisplayTarget,
    pub everything_enabled: bool,
    pub index_folders: Vec<String>,
    pub exclude_patterns: Vec<String>,
    pub aliases: Vec<AliasEntry>,
    /// User-added entries layered on top of the built-in Windows Settings
    /// shortcut set (`search/windows_settings.rs`'s `ENTRIES`). The
    /// built-in ones can't be edited or deleted.
    pub custom_windows_shortcuts: Vec<WindowsShortcutEntry>,
    /// The Latin proportional typeface used for UI chrome. Doesn't affect
    /// the CJK fallback fonts (Meiryo etc., see `src/fonts.rs`).
    pub ui_font: UiFont,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AliasEntry {
    pub name: String,
    pub target: String,
    pub args: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WindowsShortcutEntry {
    pub label: String,
    pub uri: String,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            hotkey: "Alt+Space".to_string(),
            autostart: true,
            max_results: 6,
            theme: Theme::System,
            accent_color: AccentColor::Lime,
            font_scale: 1.0,
            language: Language::System,
            display_target: DisplayTarget::Cursor,
            everything_enabled: false,
            index_folders: Vec::new(),
            exclude_patterns: vec!["(?i)^(Uninstall|アンインストール)".to_string()],
            aliases: Vec::new(),
            custom_windows_shortcuts: Vec::new(),
            ui_font: UiFont::SegoeUi,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Theme {
    Light,
    Dark,
    #[default]
    System,
}

/// Accent color choices. Each keeps roughly the same OKLCH lightness and
/// chroma as the original default (`Lime`) and only varies hue, so they
/// all read as similarly light colors with comparable contrast against the
/// Save button's near-black text (`settings_window.rs`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AccentColor {
    #[default]
    Lime,
    Red,
    Orange,
    Blue,
    Purple,
}

/// Which display the search results window shows on. `FocusedWindow` reads
/// `GetForegroundWindow` at the moment the hotkey is pressed
/// (`src/display.rs::position_on_foreground_window_monitor`). When shown
/// from the tray icon instead, the foreground window at that moment may be
/// the taskbar itself, so this option reflects "whatever window was last
/// active" less reliably than it does for a hotkey-triggered show.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DisplayTarget {
    #[default]
    Cursor,
    Primary,
    FocusedWindow,
}

/// Latin proportional typeface choices for UI chrome. Deliberately not
/// enumerated from the system (e.g. via DirectWrite) — mapping family
/// names to actual font files is brittle — so this is a fixed, verified
/// list of typefaces Windows always ships with (see `src/fonts.rs`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum UiFont {
    #[default]
    SegoeUi,
    YuGothic,
    Meiryo,
}

/// Display language. `System` resolves from Windows' UI display language
/// ([`crate::i18n::resolve`]). The resolved value isn't written back to
/// the config file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Language {
    #[default]
    System,
    En,
    Ja,
}

impl Config {
    pub fn load_or_default(app_name: &str) -> Self {
        Self::config_path(app_name)
            .and_then(|path| std::fs::read_to_string(path).ok())
            .and_then(|text| toml::from_str(&text).ok())
            .unwrap_or_default()
    }

    pub fn save(&self, app_name: &str) -> std::io::Result<()> {
        let path = Self::config_path(app_name).ok_or_else(|| {
            std::io::Error::new(std::io::ErrorKind::NotFound, "%APPDATA% is not set")
        })?;
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let text = toml::to_string_pretty(self)
            .map_err(|err| std::io::Error::new(std::io::ErrorKind::InvalidData, err))?;
        std::fs::write(path, text)
    }

    fn config_path(app_name: &str) -> Option<PathBuf> {
        let appdata = std::env::var_os("APPDATA")?;
        Some(PathBuf::from(appdata).join(app_name).join("config.toml"))
    }
}
