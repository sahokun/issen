use crate::config::WindowsShortcutEntry;
use crate::i18n::Lang;

use super::fuzzy::fuzzy_match;
use super::{Action, SearchProvider, SearchResult};

struct Entry {
    label_en: &'static str,
    label_ja: &'static str,
    keywords: &'static [&'static str],
    uri: &'static str,
}

impl Entry {
    fn label(&self, lang: Lang) -> &'static str {
        match lang {
            Lang::En => self.label_en,
            Lang::Ja => self.label_ja,
        }
    }
}

/// The predefined set of built-in Windows Settings shortcuts.
/// `keywords` intentionally keeps both languages regardless of the display
/// language, so an English keyword still matches under a Japanese UI and
/// vice versa.
const ENTRIES: &[Entry] = &[
    Entry {
        label_en: "About",
        label_ja: "このPCについて",
        keywords: &["about"],
        uri: "ms-settings:about",
    },
    Entry {
        label_en: "Display",
        label_ja: "ディスプレイ",
        keywords: &["display", "画面"],
        uri: "ms-settings:display",
    },
    Entry {
        label_en: "Sound",
        label_ja: "サウンド",
        keywords: &["sound", "音量"],
        uri: "ms-settings:sound",
    },
    Entry {
        label_en: "Power & sleep",
        label_ja: "電源とスリープ",
        keywords: &["power", "sleep", "電源"],
        uri: "ms-settings:powersleep",
    },
    Entry {
        label_en: "Storage",
        label_ja: "ストレージ",
        keywords: &["storage", "容量"],
        uri: "ms-settings:storagesense",
    },
    Entry {
        label_en: "Bluetooth",
        label_ja: "Bluetooth",
        keywords: &["bluetooth"],
        uri: "ms-settings:bluetooth",
    },
    Entry {
        label_en: "Printers & scanners",
        label_ja: "プリンターとスキャナー",
        keywords: &["printer", "印刷"],
        uri: "ms-settings:printers",
    },
    Entry {
        label_en: "Mouse & touchpad",
        label_ja: "マウスとタッチパッド",
        keywords: &["mouse", "touchpad"],
        uri: "ms-settings:mousetouchpad",
    },
    Entry {
        label_en: "Connected devices",
        label_ja: "接続デバイス",
        keywords: &["device", "デバイス"],
        uri: "ms-settings:connecteddevices",
    },
    Entry {
        label_en: "Wi-Fi",
        label_ja: "Wi-Fi",
        keywords: &["wifi", "無線"],
        uri: "ms-settings:network-wifi",
    },
    Entry {
        label_en: "Ethernet",
        label_ja: "イーサネット",
        keywords: &["ethernet", "有線"],
        uri: "ms-settings:network-ethernet",
    },
    Entry {
        label_en: "VPN",
        label_ja: "VPN",
        keywords: &["vpn"],
        uri: "ms-settings:network-vpn",
    },
    Entry {
        label_en: "Airplane mode",
        label_ja: "機内モード",
        keywords: &["airplane", "機内"],
        uri: "ms-settings:network-airplanemode",
    },
    Entry {
        label_en: "Background",
        label_ja: "背景",
        keywords: &["background", "壁紙"],
        uri: "ms-settings:personalization-background",
    },
    Entry {
        label_en: "Colors",
        label_ja: "色",
        keywords: &["colors", "テーマカラー"],
        uri: "ms-settings:personalization-colors",
    },
    Entry {
        label_en: "Lock screen",
        label_ja: "ロック画面",
        keywords: &["lockscreen"],
        uri: "ms-settings:lockscreen",
    },
    Entry {
        label_en: "Themes",
        label_ja: "テーマ",
        keywords: &["themes"],
        uri: "ms-settings:themes",
    },
    Entry {
        label_en: "Taskbar",
        label_ja: "タスクバー",
        keywords: &["taskbar"],
        uri: "ms-settings:taskbar",
    },
    Entry {
        label_en: "Apps & features",
        label_ja: "アプリと機能",
        keywords: &["apps", "uninstall", "アンインストール"],
        uri: "ms-settings:appsfeatures",
    },
    Entry {
        label_en: "Default apps",
        label_ja: "既定のアプリ",
        keywords: &["defaultapps", "既定"],
        uri: "ms-settings:defaultapps",
    },
    Entry {
        label_en: "Startup",
        label_ja: "スタートアップ",
        keywords: &["startup", "自動起動"],
        uri: "ms-settings:startupapps",
    },
    Entry {
        label_en: "Your info",
        label_ja: "ユーザーの情報",
        keywords: &["account", "アカウント"],
        uri: "ms-settings:yourinfo",
    },
    Entry {
        label_en: "Sign-in options",
        label_ja: "サインインオプション",
        keywords: &["signin", "パスワード", "pin"],
        uri: "ms-settings:signinoptions",
    },
    Entry {
        label_en: "Date & time",
        label_ja: "日付と時刻",
        keywords: &["date", "time", "時刻"],
        uri: "ms-settings:dateandtime",
    },
    Entry {
        label_en: "Language",
        label_ja: "言語",
        keywords: &["language", "言語", "ime"],
        uri: "ms-settings:regionlanguage",
    },
    Entry {
        label_en: "Game Mode",
        label_ja: "ゲームモード",
        keywords: &["gamemode"],
        uri: "ms-settings:gaming-gamemode",
    },
    Entry {
        label_en: "Xbox Game Bar",
        label_ja: "Xbox Game Bar",
        keywords: &["gamebar"],
        uri: "ms-settings:gaming-gamebar",
    },
    Entry {
        label_en: "Narrator",
        label_ja: "ナレーター",
        keywords: &["narrator"],
        uri: "ms-settings:easeofaccess-narrator",
    },
    Entry {
        label_en: "Magnifier",
        label_ja: "拡大鏡",
        keywords: &["magnifier"],
        uri: "ms-settings:easeofaccess-magnifier",
    },
    Entry {
        label_en: "High contrast",
        label_ja: "ハイコントラスト",
        keywords: &["highcontrast"],
        uri: "ms-settings:easeofaccess-highcontrast",
    },
    Entry {
        label_en: "Camera",
        label_ja: "カメラ",
        keywords: &["camera", "webcam"],
        uri: "ms-settings:privacy-webcam",
    },
    Entry {
        label_en: "Microphone",
        label_ja: "マイク",
        keywords: &["microphone", "mic"],
        uri: "ms-settings:privacy-microphone",
    },
    Entry {
        label_en: "Location",
        label_ja: "位置情報",
        keywords: &["location"],
        uri: "ms-settings:privacy-location",
    },
    Entry {
        label_en: "Windows Update",
        label_ja: "Windows Update",
        keywords: &["update", "更新"],
        uri: "ms-settings:windowsupdate",
    },
    Entry {
        label_en: "Windows Security",
        label_ja: "Windowsセキュリティ",
        keywords: &["security", "defender", "ウイルス"],
        uri: "ms-settings:windowsdefender",
    },
    Entry {
        label_en: "Recovery",
        label_ja: "回復",
        keywords: &["recovery", "リセット"],
        uri: "ms-settings:recovery",
    },
];

pub struct WindowsSettingsProvider<'a> {
    lang: Lang,
    custom: &'a [WindowsShortcutEntry],
}

impl<'a> WindowsSettingsProvider<'a> {
    pub fn with_custom(lang: Lang, custom: &'a [WindowsShortcutEntry]) -> Self {
        Self { lang, custom }
    }
}

impl SearchProvider for WindowsSettingsProvider<'_> {
    fn search(&self, query: &str) -> Vec<SearchResult> {
        if query.is_empty() {
            return Vec::new();
        }

        let subtitle = crate::i18n::Strings::for_lang(self.lang).windows_settings_subtitle;

        let builtin = ENTRIES.iter().filter_map(|entry| {
            let label = entry.label(self.lang);
            let label_score = fuzzy_match(query, label);
            let keyword_score = entry
                .keywords
                .iter()
                .filter_map(|k| fuzzy_match(query, k))
                .max();
            label_score
                .into_iter()
                .chain(keyword_score)
                .max()
                .map(|score| SearchResult {
                    title: label.to_string(),
                    subtitle: subtitle.to_string(),
                    action: Action::OpenUri(entry.uri.to_string()),
                    score,
                })
        });

        let custom = self.custom.iter().filter_map(|entry| {
            fuzzy_match(query, &entry.label).map(|score| SearchResult {
                title: entry.label.clone(),
                subtitle: subtitle.to_string(),
                action: Action::OpenUri(entry.uri.clone()),
                score,
            })
        });

        builtin.chain(custom).collect()
    }
}
