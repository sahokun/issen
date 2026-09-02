use windows::Win32::Globalization::GetUserDefaultUILanguage;

use crate::config::Language;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lang {
    En,
    Ja,
}

/// Resolves the config's `Language` setting to the `Lang` actually used. `System` looks at
/// Windows' UI display language (not the region/format locale — so a Japanese-region,
/// English-UI environment doesn't get wrongly Japanized).
pub fn resolve(setting: Language) -> Lang {
    match setting {
        Language::En => Lang::En,
        Language::Ja => Lang::Ja,
        Language::System => detect_system_lang(),
    }
}

fn detect_system_lang() -> Lang {
    const LANG_JAPANESE: u16 = 0x11;
    let langid = unsafe { GetUserDefaultUILanguage() };
    if langid & 0x3FF == LANG_JAPANESE {
        Lang::Ja
    } else {
        Lang::En
    }
}

pub struct Strings {
    pub tray_open: &'static str,
    pub tray_settings: &'static str,
    pub tray_reindex: &'static str,
    /// Swapped in for `tray_reindex` as the menu item's label while a scan is running
    /// (`TrayHandle::set_scanning`). The item itself is also disabled to prevent overlapping
    /// scans, but the label alone wouldn't tell the user a scan is in progress.
    pub tray_reindex_scanning: &'static str,
    pub tray_about: &'static str,
    pub tray_quit: &'static str,
    /// Shown as the tray icon's tooltip while a scan is running (`TrayHandle::set_scanning`).
    /// Stays the default "Issen" while idle.
    pub tray_tooltip_scanning: &'static str,
    pub search_hint: &'static str,
    pub windows_settings_subtitle: &'static str,

    pub action_run: &'static str,
    pub action_run_as_admin: &'static str,
    pub action_open_location: &'static str,
    pub action_copy_path: &'static str,
    pub action_register_alias: &'static str,
    pub action_pin: &'static str,
    pub action_unpin: &'static str,

    pub settings_title: &'static str,
    pub section_general: &'static str,
    pub section_appearance: &'static str,
    pub section_index: &'static str,
    pub section_aliases: &'static str,
    pub section_windows_shortcuts: &'static str,
    pub section_everything: &'static str,

    pub label_autostart: &'static str,
    pub label_hotkey: &'static str,
    pub label_language: &'static str,
    pub language_system: &'static str,
    pub language_en: &'static str,
    pub language_ja: &'static str,
    pub label_max_results: &'static str,
    pub label_display_target: &'static str,
    pub display_target_cursor: &'static str,
    pub display_target_primary: &'static str,
    pub display_target_focused: &'static str,

    pub label_theme: &'static str,
    pub theme_light: &'static str,
    pub theme_dark: &'static str,
    pub theme_system: &'static str,
    pub label_accent_color: &'static str,
    pub accent_color_lime: &'static str,
    pub accent_color_red: &'static str,
    pub accent_color_orange: &'static str,
    pub accent_color_blue: &'static str,
    pub accent_color_purple: &'static str,
    pub label_font_size: &'static str,
    pub label_font: &'static str,
    pub font_segoe_ui: &'static str,
    pub font_yu_gothic: &'static str,
    pub font_meiryo: &'static str,

    pub label_custom_folders: &'static str,
    pub button_add_folder: &'static str,
    pub button_remove: &'static str,
    pub label_exclude_patterns: &'static str,
    pub exclude_pattern_placeholder: &'static str,
    pub button_add: &'static str,
    pub button_rescan_now: &'static str,
    pub scanning: &'static str,

    pub label_alias_name: &'static str,
    pub label_alias_target: &'static str,
    pub label_alias_args: &'static str,
    pub button_add_alias: &'static str,
    pub button_delete: &'static str,

    pub label_shortcut_label: &'static str,
    pub label_shortcut_uri: &'static str,
    pub button_add_shortcut: &'static str,

    pub label_everything_enabled: &'static str,
    pub everything_connected: &'static str,

    pub button_save: &'static str,
    pub button_close: &'static str,

    pub about_title: &'static str,
    pub about_close: &'static str,

    pub tool_color_picker: &'static str,
    pub tool_unit_converter: &'static str,
    pub tool_history: &'static str,
    pub history_empty: &'static str,
    pub tool_eyedropper: &'static str,
    pub eyedropper_hint: &'static str,
    pub label_hex: &'static str,
    pub label_rgb: &'static str,
    pub label_hsl: &'static str,

    pub unit_length: &'static str,
    pub unit_mass: &'static str,
    pub unit_temperature: &'static str,
    pub unit_area: &'static str,
    pub unit_volume: &'static str,
    pub unit_speed: &'static str,
    pub unit_time: &'static str,
    pub unit_data: &'static str,
}

const EN: Strings = Strings {
    tray_open: "Open Issen",
    tray_settings: "Settings",
    tray_reindex: "Rebuild Index",
    tray_reindex_scanning: "Rebuilding Index…",
    tray_about: "About",
    tray_quit: "Quit",
    tray_tooltip_scanning: "Issen - Rebuilding index…",
    search_hint: "Search apps, files, commands…",
    windows_settings_subtitle: "Windows Settings",

    action_run: "Run",
    action_run_as_admin: "Run as administrator",
    action_open_location: "Open file location",
    action_copy_path: "Copy path",
    action_register_alias: "Register as alias",
    action_pin: "Pin to top results",
    action_unpin: "Unpin from top results",

    settings_title: "Issen Settings",
    section_general: "General",
    section_appearance: "Appearance",
    section_index: "Index",
    section_aliases: "Command Aliases",
    section_windows_shortcuts: "Windows Settings Shortcuts",
    section_everything: "Everything Integration",

    label_autostart: "Launch Issen when Windows starts",
    label_hotkey: "Global hotkey",
    label_language: "Display language",
    language_system: "Match Windows",
    language_en: "English",
    language_ja: "日本語",
    label_max_results: "Maximum results shown",
    label_display_target: "Show launcher on",
    display_target_cursor: "The display the mouse cursor is on",
    display_target_primary: "The primary display",
    display_target_focused: "The display of the focused window",

    label_theme: "Theme",
    theme_light: "Light",
    theme_dark: "Dark",
    theme_system: "Match Windows",
    label_accent_color: "Accent color",
    accent_color_lime: "Lime",
    accent_color_red: "Red",
    accent_color_orange: "Orange",
    accent_color_blue: "Blue",
    accent_color_purple: "Purple",
    label_font_size: "Font size",
    label_font: "Font",
    font_segoe_ui: "Segoe UI (default)",
    font_yu_gothic: "Yu Gothic",
    font_meiryo: "Meiryo",

    label_custom_folders: "Custom folders to index",
    button_add_folder: "Add folder…",
    button_remove: "Remove",
    label_exclude_patterns: "Exclude patterns (regular expressions)",
    exclude_pattern_placeholder: "e.g. ^(Uninstall|Update)",
    button_add: "Add",
    button_rescan_now: "Rescan now",
    scanning: "Scanning…",

    label_alias_name: "Name",
    label_alias_target: "Target",
    label_alias_args: "Arguments",
    button_add_alias: "Add alias",
    button_delete: "Delete",

    label_shortcut_label: "Display name",
    label_shortcut_uri: "ms-settings: URI",
    button_add_shortcut: "Add shortcut",

    label_everything_enabled: "Enable Everything search",
    everything_connected: "Connected to Everything",

    button_save: "Save",
    button_close: "Close",

    about_title: "About Issen",
    about_close: "Close",

    tool_color_picker: "Color Picker",
    tool_unit_converter: "Unit Converter",
    tool_history: "Search history",
    history_empty: "No search history yet",
    tool_eyedropper: "Eyedropper",
    eyedropper_hint:
        "Move the cursor over any pixel on screen, then press Enter to pick it (Esc to cancel)",
    label_hex: "Hex",
    label_rgb: "RGB",
    label_hsl: "HSL",

    unit_length: "Length",
    unit_mass: "Mass",
    unit_temperature: "Temperature",
    unit_area: "Area",
    unit_volume: "Volume",
    unit_speed: "Speed",
    unit_time: "Time",
    unit_data: "Data size",
};

const JA: Strings = Strings {
    tray_open: "Issenを開く",
    tray_settings: "設定",
    tray_reindex: "インデックスを再構築",
    tray_reindex_scanning: "インデックスを再構築中…",
    tray_about: "バージョン情報",
    tray_quit: "終了",
    tray_tooltip_scanning: "Issen - インデックスを再構築中…",
    search_hint: "アプリケーション、ファイル、コマンドを検索…",
    windows_settings_subtitle: "Windows設定",

    action_run: "実行",
    action_run_as_admin: "管理者として実行",
    action_open_location: "ファイルの場所を開く",
    action_copy_path: "パスをコピー",
    action_register_alias: "エイリアスとして登録",
    action_pin: "上位表示にピン留め",
    action_unpin: "上位表示のピン留めを解除",

    settings_title: "Issenの設定",
    section_general: "一般",
    section_appearance: "外観",
    section_index: "インデックス",
    section_aliases: "コマンドエイリアス",
    section_windows_shortcuts: "Windows設定ショートカット",
    section_everything: "Everything連携",

    label_autostart: "Windowsスタートアップ時に自動起動する",
    label_hotkey: "グローバルホットキー",
    label_language: "表示言語",
    language_system: "システムに合わせる",
    language_en: "English",
    language_ja: "日本語",
    label_max_results: "検索結果の最大表示件数",
    label_display_target: "結果ウィンドウを表示するディスプレイ",
    display_target_cursor: "マウスカーソルのあるディスプレイ",
    display_target_primary: "プライマリディスプレイ",
    display_target_focused: "フォーカス中のウィンドウのディスプレイ",

    label_theme: "テーマ",
    theme_light: "ライト",
    theme_dark: "ダーク",
    theme_system: "システムに合わせる",
    label_accent_color: "アクセントカラー",
    accent_color_lime: "ライム",
    accent_color_red: "レッド",
    accent_color_orange: "オレンジ",
    accent_color_blue: "ブルー",
    accent_color_purple: "パープル",
    label_font_size: "フォントサイズ",
    label_font: "フォント",
    font_segoe_ui: "Segoe UI(既定)",
    font_yu_gothic: "游ゴシック",
    font_meiryo: "メイリオ",

    label_custom_folders: "インデックス対象のカスタムフォルダ",
    button_add_folder: "フォルダを追加…",
    button_remove: "削除",
    label_exclude_patterns: "除外パターン(正規表現)",
    exclude_pattern_placeholder: "例: ^(Uninstall|Update)",
    button_add: "追加",
    button_rescan_now: "今すぐ再構築",
    scanning: "スキャン中…",

    label_alias_name: "名前",
    label_alias_target: "実行対象",
    label_alias_args: "引数",
    button_add_alias: "エイリアスを追加",
    button_delete: "削除",

    label_shortcut_label: "表示名",
    label_shortcut_uri: "ms-settings: のURI",
    button_add_shortcut: "ショートカットを追加",

    label_everything_enabled: "Everything検索を有効にする",
    everything_connected: "Everythingに接続しました",

    button_save: "保存",
    button_close: "閉じる",

    about_title: "Issenについて",
    about_close: "閉じる",

    tool_color_picker: "カラーピッカー",
    tool_unit_converter: "単位コンバーター",
    tool_history: "検索履歴",
    history_empty: "検索履歴はまだありません",
    tool_eyedropper: "スポイト",
    eyedropper_hint: "画面上の好きな場所にカーソルを合わせてEnterで確定(Escでキャンセル)",
    label_hex: "Hex",
    label_rgb: "RGB",
    label_hsl: "HSL",

    unit_length: "長さ",
    unit_mass: "質量",
    unit_temperature: "温度",
    unit_area: "面積",
    unit_volume: "体積",
    unit_speed: "速度",
    unit_time: "時間",
    unit_data: "情報量",
};

impl Strings {
    pub fn for_lang(lang: Lang) -> &'static Strings {
        match lang {
            Lang::En => &EN,
            Lang::Ja => &JA,
        }
    }
}

/// Determines which language table a `Strings` reference is, by comparing it against the
/// known `JA` instance (`Strings` itself carries no language tag).
pub fn lang_of(strings: &'static Strings) -> Lang {
    if std::ptr::eq(strings, &JA) {
        Lang::Ja
    } else {
        Lang::En
    }
}

/// Read-only "last index update" display text shown in the settings window. Implemented as
/// one function rather than concatenated string fragments, since sentence structure differs
/// between languages.
pub fn last_scan_text(lang: Lang, minutes_ago: u64, count: usize) -> String {
    match lang {
        Lang::En => format!("Last updated {minutes_ago} min ago · {count} apps found"),
        Lang::Ja => format!("最終更新: {minutes_ago}分前・{count}件のアプリを検出"),
    }
}

pub fn about_version_text(lang: Lang, version: &str) -> String {
    match lang {
        Lang::En => format!("Version {version}"),
        Lang::Ja => format!("バージョン {version}"),
    }
}
