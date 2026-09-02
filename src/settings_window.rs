use std::path::PathBuf;
use std::sync::mpsc::Receiver;
use std::time::Instant;

use crate::config::{
    AccentColor, AliasEntry, Config, DisplayTarget, Language, Theme, UiFont, WindowsShortcutEntry,
};
use crate::i18n::{self, Lang, Strings};
use crate::ui_chrome::{self, GlassPalette};

/// Sidebar tabs. Each tab's content reuses the original `show_general` and similar functions
/// as-is; only the layout and tab-switching navigation changed (no settings item was dropped
/// when this moved from a single scrolling page to tabs).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SettingsTab {
    General,
    Appearance,
    Index,
    Aliases,
    WindowsShortcuts,
    Everything,
}

/// The settings window (a separate viewport). Per
/// docs/architecture/window-lifecycle.md, it's lazily created on first open
/// (`show` never constructs the viewport until `open()` has been called).
pub struct SettingsWindow {
    open: bool,
    everything_available: bool,
    tab: SettingsTab,

    new_exclude_pattern: String,
    new_alias_name: String,
    new_alias_target: String,
    new_alias_args: String,
    new_shortcut_label: String,
    new_shortcut_uri: String,

    /// The folder-picker dialog (`rfd`) is a synchronous, blocking OS call, so it runs on a
    /// separate thread with the result received over a channel. Blocking the settings window's
    /// UI thread while the dialog is open would also stall background rescan completion
    /// detection and tray menu interaction for as long as it stayed open.
    pending_folder_pick: Option<Receiver<Option<PathBuf>>>,

    pub rescan_requested: bool,
    pub save_requested: bool,
}

impl SettingsWindow {
    pub fn new(_config: &Config) -> Self {
        Self {
            open: false,
            everything_available: false,
            tab: SettingsTab::General,
            new_exclude_pattern: String::new(),
            new_alias_name: String::new(),
            new_alias_target: String::new(),
            new_alias_args: String::new(),
            new_shortcut_label: String::new(),
            new_shortcut_uri: String::new(),
            pending_folder_pick: None,
            rescan_requested: false,
            save_requested: false,
        }
    }

    /// Whether Everything is actually reachable can change while Issen is running (Everything
    /// itself starting or quitting), so this is re-checked every time the window opens (see
    /// `crate::search::everything::is_available`'s doc comment).
    pub fn open(&mut self) {
        self.everything_available = crate::search::everything::is_available();
        if !self.everything_available && self.tab == SettingsTab::Everything {
            self.tab = SettingsTab::General;
        }
        self.open = true;
    }

    pub fn is_open(&self) -> bool {
        self.open
    }

    /// Called from a search result's action menu ("Register as alias"). Opens the settings
    /// window with the alias-creation form pre-filled (nothing is saved until the user clicks
    /// "Add alias").
    pub fn prefill_alias(&mut self, name: String, target: String) {
        self.new_alias_name = name;
        self.new_alias_target = target;
        self.new_alias_args.clear();
        self.open = true;
    }

    pub fn take_rescan_requested(&mut self) -> bool {
        std::mem::take(&mut self.rescan_requested)
    }

    pub fn take_save_requested(&mut self) -> bool {
        std::mem::take(&mut self.save_requested)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn show(
        &mut self,
        ctx: &egui::Context,
        strings: &'static Strings,
        lang: Lang,
        config: &mut Config,
        last_scan_finished: Option<Instant>,
        last_scan_count: usize,
        scanning: bool,
    ) {
        if !self.open {
            return;
        }

        if let Some(receiver) = &self.pending_folder_pick {
            if let Ok(picked) = receiver.try_recv() {
                if let Some(dir) = picked {
                    config.index_folders.push(dir.display().to_string());
                }
                self.pending_folder_pick = None;
            }
        }

        let mut still_open = true;

        ctx.show_viewport_immediate(
            egui::ViewportId::from_hash_of("issen-settings"),
            egui::ViewportBuilder::default()
                .with_title(strings.settings_title)
                .with_decorations(false)
                .with_transparent(true)
                .with_inner_size([640.0, 680.0])
                .with_min_inner_size([480.0, 420.0])
                // The main window (`main.rs`) is `with_always_on_top()`; without this, this
                // window sits outside the OS's topmost band and always ends up behind it.
                .with_always_on_top(),
            |ui, _class| {
                if ui.ctx().input(|i| i.viewport().close_requested()) {
                    still_open = false;
                }

                // `config.theme`'s light/dark choice is already resolved by
                // `app.rs::apply_theme` (`ctx.set_theme`), so it's enough to read
                // `ui.visuals().dark_mode` here — unlike the main search box, this window is
                // meant to follow the theme.
                let dark = ui.visuals().dark_mode;
                let glass = ui_chrome::palette(dark);
                let accent = ui_chrome::accent_color(config.accent_color);
                let full_rect = ui.max_rect();
                ui_chrome::glass_panel(ui, full_rect, &glass);
                if ui_chrome::title_bar(ui, full_rect, strings.settings_title, &glass) {
                    still_open = false;
                }
                ui_chrome::resize_grips(ui, full_rect);

                // Reserve vertical space for the custom title bar up front, so the
                // SidePanel/Panel/CentralPanel below use only the remaining area.
                ui.add_space(ui_chrome::TITLE_BAR_HEIGHT);

                egui::Panel::bottom("issen-settings-footer")
                    .frame(egui::Frame::NONE)
                    .show(ui, |ui| {
                        ui.add_space(6.0);
                        ui.horizontal(|ui| {
                            ui.add_space(18.0);
                            let save = egui::Button::new(
                                egui::RichText::new(strings.button_save)
                                    .color(egui::Color32::from_rgb(10, 16, 2)),
                            )
                            .fill(accent)
                            .corner_radius(9.0);
                            if ui.add(save).clicked() {
                                self.save_requested = true;
                            }
                            if ui.button(strings.button_close).clicked() {
                                still_open = false;
                            }
                        });
                        ui.add_space(10.0);
                    });

                egui::Panel::left("issen-settings-nav")
                    .frame(
                        egui::Frame::NONE
                            .inner_margin(egui::Margin::symmetric(10, 14))
                            .stroke(egui::Stroke::new(1.0, glass.divider)),
                    )
                    .resizable(false)
                    .exact_size(180.0)
                    .show(ui, |ui| {
                        self.show_nav(ui, strings, &glass, accent);
                    });

                egui::CentralPanel::default()
                    .frame(egui::Frame::NONE.inner_margin(egui::Margin::symmetric(24, 18)))
                    .show(ui, |ui| {
                        egui::ScrollArea::vertical().show(ui, |ui| match self.tab {
                            SettingsTab::General => self.show_general(ui, strings, config),
                            SettingsTab::Appearance => self.show_appearance(ui, strings, config),
                            SettingsTab::Index => self.show_index(
                                ui,
                                strings,
                                config,
                                last_scan_finished,
                                last_scan_count,
                                scanning,
                            ),
                            SettingsTab::Aliases => self.show_aliases(ui, strings, config),
                            SettingsTab::WindowsShortcuts => {
                                self.show_windows_shortcuts(ui, strings, config)
                            }
                            SettingsTab::Everything => self.show_everything(ui, strings, config),
                        });
                    });
            },
        );

        self.open = still_open;

        // Keep requesting a repaint every frame while this viewport is open. The main window
        // throttles egui's overall repaint frequency while hidden (via
        // `request_repaint_after` in app.rs), so this window needs to compensate explicitly or
        // it stops following input.
        if self.open {
            ctx.request_repaint();
        }

        let _ = lang; // Reserved for lang-dependent branching beyond section headings
    }

    fn show_nav(
        &mut self,
        ui: &mut egui::Ui,
        strings: &'static Strings,
        glass: &GlassPalette,
        accent: egui::Color32,
    ) {
        let mut items = vec![
            (SettingsTab::General, strings.section_general),
            (SettingsTab::Appearance, strings.section_appearance),
            (SettingsTab::Index, strings.section_index),
            (SettingsTab::Aliases, strings.section_aliases),
            (
                SettingsTab::WindowsShortcuts,
                strings.section_windows_shortcuts,
            ),
        ];
        if self.everything_available {
            items.push((SettingsTab::Everything, strings.section_everything));
        }
        for (tab, label) in items {
            self.nav_button(ui, tab, label, glass, accent);
        }
    }

    fn nav_button(
        &mut self,
        ui: &mut egui::Ui,
        tab: SettingsTab,
        label: &str,
        glass: &GlassPalette,
        accent: egui::Color32,
    ) {
        let selected = self.tab == tab;
        let color = if selected { accent } else { glass.subtext };
        let button = egui::Button::new(egui::RichText::new(label).color(color))
            .fill(if selected {
                glass.control_bg
            } else {
                egui::Color32::TRANSPARENT
            })
            .corner_radius(10.0)
            .min_size(egui::vec2(ui.available_width(), 38.0));
        if ui.add(button).clicked() {
            self.tab = tab;
        }
    }

    fn show_general(&mut self, ui: &mut egui::Ui, strings: &'static Strings, config: &mut Config) {
        ui.heading(strings.section_general);
        ui.checkbox(&mut config.autostart, strings.label_autostart);

        ui.horizontal(|ui| {
            ui.label(strings.label_hotkey);
            ui.text_edit_singleline(&mut config.hotkey);
        });

        ui.horizontal(|ui| {
            ui.label(strings.label_max_results);
            // This caps how many results are visible without scrolling, itself capped at
            // `crate::app::MAX_VISIBLE_ROWS` — anything beyond that is still reachable by
            // scrolling the results area.
            ui.add(
                egui::DragValue::new(&mut config.max_results)
                    .range(1..=crate::app::MAX_VISIBLE_ROWS as u32),
            );
        });

        ui.horizontal(|ui| {
            ui.label(strings.label_language);
            egui::ComboBox::from_id_salt("issen-settings-language")
                .selected_text(language_label(strings, config.language))
                .show_ui(ui, |ui| {
                    ui.selectable_value(
                        &mut config.language,
                        Language::System,
                        strings.language_system,
                    );
                    ui.selectable_value(&mut config.language, Language::En, strings.language_en);
                    ui.selectable_value(&mut config.language, Language::Ja, strings.language_ja);
                });
        });

        // This dropdown's label is longer than the others; laid out horizontally like the rest
        // of the "General" tab's rows, it overflows the content width once narrowed by the
        // sidebar. Stacked vertically (label above, dropdown below) instead, matching how the
        // reference design also singles this item out.
        ui.vertical(|ui| {
            ui.label(strings.label_display_target);
            egui::ComboBox::from_id_salt("issen-settings-display-target")
                .selected_text(display_target_label(strings, config.display_target))
                .show_ui(ui, |ui| {
                    ui.selectable_value(
                        &mut config.display_target,
                        DisplayTarget::Cursor,
                        strings.display_target_cursor,
                    );
                    ui.selectable_value(
                        &mut config.display_target,
                        DisplayTarget::Primary,
                        strings.display_target_primary,
                    );
                    ui.selectable_value(
                        &mut config.display_target,
                        DisplayTarget::FocusedWindow,
                        strings.display_target_focused,
                    );
                });
        });
    }

    fn show_appearance(
        &mut self,
        ui: &mut egui::Ui,
        strings: &'static Strings,
        config: &mut Config,
    ) {
        ui.heading(strings.section_appearance);
        ui.horizontal(|ui| {
            ui.label(strings.label_theme);
            ui.selectable_value(&mut config.theme, Theme::System, strings.theme_system);
            ui.selectable_value(&mut config.theme, Theme::Light, strings.theme_light);
            ui.selectable_value(&mut config.theme, Theme::Dark, strings.theme_dark);
        });
        ui.horizontal(|ui| {
            ui.label(strings.label_accent_color);
            // An axis independent of light/dark (`config.theme`). Uses color swatches you can
            // actually see rather than a text toggle, since here the visual appearance itself
            // is what's being chosen.
            for (color, name) in [
                (AccentColor::Lime, strings.accent_color_lime),
                (AccentColor::Red, strings.accent_color_red),
                (AccentColor::Orange, strings.accent_color_orange),
                (AccentColor::Blue, strings.accent_color_blue),
                (AccentColor::Purple, strings.accent_color_purple),
            ] {
                let selected = config.accent_color == color;
                let swatch = egui::Button::new("")
                    .fill(ui_chrome::accent_color(color))
                    .corner_radius(14.0)
                    .min_size(egui::vec2(28.0, 28.0))
                    .stroke(if selected {
                        ui.visuals().selection.stroke
                    } else {
                        egui::Stroke::NONE
                    });
                if ui.add(swatch).on_hover_text(name).clicked() {
                    config.accent_color = color;
                }
            }
        });
        ui.horizontal(|ui| {
            ui.label(strings.label_font);
            egui::ComboBox::from_id_salt("issen-settings-ui-font")
                .selected_text(ui_font_label(strings, config.ui_font))
                .show_ui(ui, |ui| {
                    ui.selectable_value(
                        &mut config.ui_font,
                        UiFont::SegoeUi,
                        strings.font_segoe_ui,
                    );
                    ui.selectable_value(
                        &mut config.ui_font,
                        UiFont::YuGothic,
                        strings.font_yu_gothic,
                    );
                    ui.selectable_value(&mut config.ui_font, UiFont::Meiryo, strings.font_meiryo);
                });
        });
        ui.horizontal(|ui| {
            ui.label(strings.label_font_size);
            // The range is capped to stay within what `src/app.rs::apply_font_scale`'s layout
            // math (fixed-px result row height, window height) can handle without breaking.
            // `step_by` quantizes to 0.05 steps so dragging doesn't produce a continuous stream
            // of distinct values — each one would trigger `apply_font_scale`'s font atlas
            // rebuild, defeating its "only apply on change" guard.
            ui.add(egui::Slider::new(&mut config.font_scale, 0.9..=1.25).step_by(0.05));
        });
    }

    fn show_index(
        &mut self,
        ui: &mut egui::Ui,
        strings: &'static Strings,
        config: &mut Config,
        last_scan_finished: Option<Instant>,
        last_scan_count: usize,
        scanning: bool,
    ) {
        ui.heading(strings.section_index);

        ui.label(strings.label_custom_folders);
        let mut remove_folder: Option<usize> = None;
        for (i, folder) in config.index_folders.iter().enumerate() {
            ui.horizontal(|ui| {
                ui.label(folder.as_str());
                if ui.small_button(strings.button_remove).clicked() {
                    remove_folder = Some(i);
                }
            });
        }
        if let Some(i) = remove_folder {
            config.index_folders.remove(i);
        }
        ui.add_enabled_ui(self.pending_folder_pick.is_none(), |ui| {
            if ui.button(strings.button_add_folder).clicked() {
                let (sender, receiver) = std::sync::mpsc::channel();
                std::thread::spawn(move || {
                    let picked = rfd::FileDialog::new().pick_folder();
                    let _ = sender.send(picked);
                });
                self.pending_folder_pick = Some(receiver);
            }
        });

        ui.add_space(8.0);
        ui.label(strings.label_exclude_patterns);
        let mut remove_pattern: Option<usize> = None;
        for (i, pattern) in config.exclude_patterns.iter().enumerate() {
            ui.horizontal(|ui| {
                ui.label(pattern.as_str());
                if ui.small_button(strings.button_remove).clicked() {
                    remove_pattern = Some(i);
                }
            });
        }
        if let Some(i) = remove_pattern {
            config.exclude_patterns.remove(i);
        }
        ui.horizontal(|ui| {
            ui.add(
                egui::TextEdit::singleline(&mut self.new_exclude_pattern)
                    .hint_text(strings.exclude_pattern_placeholder),
            );
            if ui.button(strings.button_add).clicked()
                && !self.new_exclude_pattern.trim().is_empty()
            {
                config
                    .exclude_patterns
                    .push(std::mem::take(&mut self.new_exclude_pattern));
            }
        });

        ui.add_space(8.0);
        ui.horizontal(|ui| {
            ui.add_enabled_ui(!scanning, |ui| {
                if ui.button(strings.button_rescan_now).clicked() {
                    self.rescan_requested = true;
                }
            });
            if scanning {
                ui.label(strings.scanning);
            } else if let Some(finished) = last_scan_finished {
                let minutes = finished.elapsed().as_secs() / 60;
                ui.weak(i18n::last_scan_text(
                    i18n::lang_of(strings),
                    minutes,
                    last_scan_count,
                ));
            }
        });
    }

    fn show_aliases(&mut self, ui: &mut egui::Ui, strings: &'static Strings, config: &mut Config) {
        ui.heading(strings.section_aliases);

        let mut remove_alias: Option<usize> = None;
        for (i, alias) in config.aliases.iter().enumerate() {
            ui.horizontal(|ui| {
                ui.label(&alias.name);
                ui.weak(&alias.target);
                if !alias.args.is_empty() {
                    ui.weak(&alias.args);
                }
                if ui.small_button(strings.button_delete).clicked() {
                    remove_alias = Some(i);
                }
            });
        }
        if let Some(i) = remove_alias {
            config.aliases.remove(i);
        }

        ui.horizontal(|ui| {
            // Sized as a fraction of the remaining width rather than fixed px, so these fields
            // don't overflow the window once the sidebar narrows the content width.
            let remaining = (ui.available_width() - 110.0 - 24.0).max(0.0);
            ui.add(
                egui::TextEdit::singleline(&mut self.new_alias_name)
                    .hint_text(strings.label_alias_name)
                    .desired_width(remaining * 0.25),
            );
            ui.add(
                egui::TextEdit::singleline(&mut self.new_alias_target)
                    .hint_text(strings.label_alias_target)
                    .desired_width(remaining * 0.5),
            );
            ui.add(
                egui::TextEdit::singleline(&mut self.new_alias_args)
                    .hint_text(strings.label_alias_args)
                    .desired_width(remaining * 0.25),
            );
            if ui.button(strings.button_add_alias).clicked()
                && !self.new_alias_name.trim().is_empty()
                && !self.new_alias_target.trim().is_empty()
            {
                config.aliases.push(AliasEntry {
                    name: std::mem::take(&mut self.new_alias_name),
                    target: std::mem::take(&mut self.new_alias_target),
                    args: std::mem::take(&mut self.new_alias_args),
                });
            }
        });
    }

    fn show_windows_shortcuts(
        &mut self,
        ui: &mut egui::Ui,
        strings: &'static Strings,
        config: &mut Config,
    ) {
        ui.heading(strings.section_windows_shortcuts);

        let mut remove_shortcut: Option<usize> = None;
        for (i, entry) in config.custom_windows_shortcuts.iter().enumerate() {
            ui.horizontal(|ui| {
                ui.label(&entry.label);
                ui.weak(&entry.uri);
                if ui.small_button(strings.button_delete).clicked() {
                    remove_shortcut = Some(i);
                }
            });
        }
        if let Some(i) = remove_shortcut {
            config.custom_windows_shortcuts.remove(i);
        }

        ui.horizontal(|ui| {
            // Same reason as the alias fields above: sized as a fraction of the remaining width
            // rather than fixed px.
            let remaining = (ui.available_width() - 140.0 - 16.0).max(0.0);
            ui.add(
                egui::TextEdit::singleline(&mut self.new_shortcut_label)
                    .hint_text(strings.label_shortcut_label)
                    .desired_width(remaining * 0.35),
            );
            ui.add(
                egui::TextEdit::singleline(&mut self.new_shortcut_uri)
                    .hint_text(strings.label_shortcut_uri)
                    .desired_width(remaining * 0.65),
            );
            if ui.button(strings.button_add_shortcut).clicked()
                && !self.new_shortcut_label.trim().is_empty()
                && !self.new_shortcut_uri.trim().is_empty()
            {
                config.custom_windows_shortcuts.push(WindowsShortcutEntry {
                    label: std::mem::take(&mut self.new_shortcut_label),
                    uri: std::mem::take(&mut self.new_shortcut_uri),
                });
            }
        });
    }

    fn show_everything(
        &mut self,
        ui: &mut egui::Ui,
        strings: &'static Strings,
        config: &mut Config,
    ) {
        ui.heading(strings.section_everything);
        ui.weak(strings.everything_connected);
        ui.checkbox(
            &mut config.everything_enabled,
            strings.label_everything_enabled,
        );
    }
}

fn language_label(strings: &'static Strings, language: Language) -> &'static str {
    match language {
        Language::System => strings.language_system,
        Language::En => strings.language_en,
        Language::Ja => strings.language_ja,
    }
}

fn ui_font_label(strings: &'static Strings, font: UiFont) -> &'static str {
    match font {
        UiFont::SegoeUi => strings.font_segoe_ui,
        UiFont::YuGothic => strings.font_yu_gothic,
        UiFont::Meiryo => strings.font_meiryo,
    }
}

fn display_target_label(strings: &'static Strings, target: DisplayTarget) -> &'static str {
    match target {
        DisplayTarget::Cursor => strings.display_target_cursor,
        DisplayTarget::Primary => strings.display_target_primary,
        DisplayTarget::FocusedWindow => strings.display_target_focused,
    }
}
