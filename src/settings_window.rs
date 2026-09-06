//! The settings window: a sidebar-tabbed panel over the same glass chrome
//! (`ui_chrome.rs`) as the about window. Ported from the egui/eframe
//! version's `SettingsWindow` (see git history) with two structural
//! changes:
//!   - **Live mutation through `WeakEntity<IssenApp>`.** The egui version
//!     mutated `&mut Config` directly every frame; this does the same thing
//!     event-driven — each control's click/edit handler updates
//!     `IssenApp::config` straight away (`cx.notify()` on the app entity),
//!     and this window re-renders in response via `cx.observe`. The old
//!     `rescan_requested`/`save_requested` flags existed only because egui
//!     polled every frame; there's no polling here; the Rescan/Save buttons
//!     call `IssenApp::start_scan`/`Config::save` directly.
//!   - **No dropdowns/sliders.** GPUI core has no combo-box or drag-value
//!     widget. The handful of options each dropdown offered (language,
//!     theme, display target, UI font) are laid out as segmented button
//!     rows instead (theme already did this in the egui version); the two
//!     numeric ranges (`max_results`, `font_scale`) are steppers.
use std::time::Instant;

use gpui::{
    div, point, prelude::*, px, size, App, AppContext, Bounds, Context, Div, Entity, Hsla,
    InteractiveElement, MouseButton, ParentElement, ScrollHandle, SharedString, Styled, Window,
    WindowBackgroundAppearance, WindowBounds, WindowHandle, WindowKind, WindowOptions,
};

use crate::app::{IssenApp, MAX_VISIBLE_ROWS};
use crate::config::{
    self, AccentColor, AliasEntry, Config, DisplayTarget, Language, Theme, UiFont,
    WindowsShortcutEntry,
};
use crate::i18n::{self, Strings};
use crate::text_input::{self, TextInput};
use crate::ui_chrome::{self, GlassPalette};
use gpui::WeakEntity;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SettingsTab {
    General,
    Appearance,
    Index,
    Aliases,
    WindowsShortcuts,
    Everything,
}

const WINDOW_SIZE: (f32, f32) = (640.0, 680.0);
const MIN_WINDOW_SIZE: (f32, f32) = (480.0, 420.0);

/// A plain-data snapshot of the `IssenApp` fields the settings window
/// needs to render. Passed in at construction and refreshed afterward via
/// `cx.observe` — never fetched with `app.upgrade()?.read(cx)` directly
/// from `render()`, since `open()` (and therefore the settings window's
/// first render, which happens synchronously as part of window creation)
/// always runs from *inside* an active `IssenApp` update
/// (`handle_tray_action`), and reading an entity while it's mid-update
/// panics at runtime (confirmed on real hardware: "cannot read
/// issen::app::IssenApp while it is already being updated").
pub struct AppSnapshot {
    pub config: Config,
    pub strings: &'static Strings,
    pub scanning: bool,
    pub last_scan_finished: Option<Instant>,
    pub last_scan_count: usize,
}

pub struct SettingsWindow {
    app: WeakEntity<IssenApp>,
    tab: SettingsTab,
    everything_available: bool,
    content_scroll: ScrollHandle,
    pending_folder_pick: bool,

    cached: AppSnapshot,

    hotkey_input: Entity<TextInput>,
    new_exclude_pattern_input: Entity<TextInput>,
    new_alias_name_input: Entity<TextInput>,
    new_alias_target_input: Entity<TextInput>,
    new_alias_args_input: Entity<TextInput>,
    new_shortcut_label_input: Entity<TextInput>,
    new_shortcut_uri_input: Entity<TextInput>,
}

/// Opens the settings window, or brings an already-open one to front (and
/// re-checks Everything's availability either way — it can change while
/// Issen is running, same as the egui version's `SettingsWindow::open`).
///
/// Takes `initial` as a plain snapshot rather than reading `app` itself —
/// see `AppSnapshot`'s doc comment for why. The caller (`handle_tray_action`)
/// already has direct field access (`self.config.clone()` etc.), no entity
/// read needed.
pub fn open(
    existing: &mut Option<WindowHandle<SettingsWindow>>,
    app: WeakEntity<IssenApp>,
    initial: AppSnapshot,
    cx: &mut App,
) {
    if let Some(handle) = existing {
        if handle
            .update(cx, |view, window, cx| {
                view.everything_available = crate::search::everything::is_available();
                if !view.everything_available && view.tab == SettingsTab::Everything {
                    view.tab = SettingsTab::General;
                }
                window.activate_window();
                cx.notify();
            })
            .is_ok()
        {
            return;
        }
    }

    let bounds = Bounds {
        origin: point(px(120.), px(120.)),
        size: size(px(WINDOW_SIZE.0), px(WINDOW_SIZE.1)),
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
                window_background: WindowBackgroundAppearance::Transparent,
                ..Default::default()
            },
            move |window, cx| {
                ui_chrome::set_topmost(window);
                cx.new(|cx| SettingsWindow::new(app, initial, cx))
            },
        )
        .expect("failed to open settings window");
    *existing = Some(handle);
}

fn set_config(app: &WeakEntity<IssenApp>, cx: &mut App, f: impl FnOnce(&mut Config) + 'static) {
    if let Some(app) = app.upgrade() {
        app.update(cx, |app, cx| {
            f(&mut app.config);
            cx.notify();
        });
    }
}

impl SettingsWindow {
    fn new(app: WeakEntity<IssenApp>, initial: AppSnapshot, cx: &mut Context<Self>) -> Self {
        let everything_available = crate::search::everything::is_available();

        let hotkey_input = cx.new(|cx| TextInput::new(cx, initial.config.hotkey.clone(), ""));
        cx.observe(&hotkey_input, {
            let app = app.clone();
            move |_, entity, cx| {
                let text = entity.read(cx).text().to_string();
                if let Some(app_entity) = app.upgrade() {
                    app_entity.update(cx, |app, cx| {
                        app.config.hotkey = text.clone();
                        app.hotkey.update_hotkey(text);
                        cx.notify();
                    });
                }
            }
        })
        .detach();

        // Refreshes the cached snapshot whenever `IssenApp` notifies (config
        // changed elsewhere, a scan finished, …). Safe to `.read()` here —
        // unlike a direct read from `render()` — because an observer
        // callback only runs once the notifying update has fully returned
        // (see `AppSnapshot`'s doc comment).
        if let Some(app_entity) = app.upgrade() {
            cx.observe(&app_entity, |this, entity, cx| {
                let app = entity.read(cx);
                this.cached = AppSnapshot {
                    config: app.config.clone(),
                    strings: app.strings,
                    scanning: app.scanning,
                    last_scan_finished: app.last_scan_finished,
                    last_scan_count: app.last_scan_count,
                };
                cx.notify();
            })
            .detach();
        }

        // `TextInput` is a plain data entity, not a `Render`-backed view, so
        // GPUI's per-view `accessed_entities` auto-tracking (what makes
        // reading an entity during render implicitly subscribe to it)
        // doesn't apply to it — without an explicit observe per field, a
        // keystroke would update the entity's state but never trigger this
        // window to repaint it. `hotkey_input` gets this for free as a
        // side effect of the write-back observe above (it re-notifies via
        // the app entity); every other field needs its own plain forward.
        let new_exclude_pattern_input = cx.new(|cx| TextInput::new(cx, "", ""));
        let new_alias_name_input = cx.new(|cx| TextInput::new(cx, "", ""));
        let new_alias_target_input = cx.new(|cx| TextInput::new(cx, "", ""));
        let new_alias_args_input = cx.new(|cx| TextInput::new(cx, "", ""));
        let new_shortcut_label_input = cx.new(|cx| TextInput::new(cx, "", ""));
        let new_shortcut_uri_input = cx.new(|cx| TextInput::new(cx, "", ""));
        for entity in [
            &new_exclude_pattern_input,
            &new_alias_name_input,
            &new_alias_target_input,
            &new_alias_args_input,
            &new_shortcut_label_input,
            &new_shortcut_uri_input,
        ] {
            cx.observe(entity, |_, _, cx| cx.notify()).detach();
        }

        Self {
            app,
            tab: SettingsTab::General,
            everything_available,
            content_scroll: ScrollHandle::new(),
            pending_folder_pick: false,
            cached: initial,
            hotkey_input,
            new_exclude_pattern_input,
            new_alias_name_input,
            new_alias_target_input,
            new_alias_args_input,
            new_shortcut_label_input,
            new_shortcut_uri_input,
        }
    }

    fn add_folder_clicked(&mut self, cx: &mut Context<Self>) {
        if self.pending_folder_pick {
            return;
        }
        self.pending_folder_pick = true;
        cx.notify();
        let app = self.app.clone();
        cx.spawn(async move |this, cx| {
            let picked = cx
                .background_executor()
                .spawn(async { rfd::FileDialog::new().pick_folder() })
                .await;
            let _ = this.update(cx, |view, cx| {
                view.pending_folder_pick = false;
                if let Some(dir) = picked {
                    set_config(&app, cx, move |c| {
                        c.index_folders.push(dir.display().to_string())
                    });
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn nav(
        &self,
        strings: &'static Strings,
        palette: &GlassPalette,
        accent: Hsla,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
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

        div()
            .w(px(180.))
            .flex_none()
            .h_full()
            .flex()
            .flex_col()
            .gap_1()
            .p(px(10.))
            .border_r_1()
            .border_color(palette.divider)
            .children(items.into_iter().map(|(tab, label)| {
                let selected = self.tab == tab;
                div()
                    .id(SharedString::from(format!("settings-nav-{label}")))
                    .h(px(36.))
                    .px(px(12.))
                    .flex()
                    .items_center()
                    .rounded(px(8.))
                    .cursor_pointer()
                    .text_size(px(13.))
                    .text_color(if selected { accent } else { palette.subtext })
                    .when(selected, |d| d.bg(palette.control_bg))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, _, _, cx| {
                            this.tab = tab;
                            cx.notify();
                        }),
                    )
                    .child(SharedString::from(label))
            }))
    }

    fn footer(
        &self,
        strings: &'static Strings,
        palette: &GlassPalette,
        accent: Hsla,
    ) -> impl IntoElement {
        let app = self.app.clone();
        div()
            .h(px(52.))
            .flex_none()
            .flex()
            .items_center()
            .gap_2()
            .px(px(18.))
            .border_t_1()
            .border_color(palette.divider)
            .child(
                div()
                    .id("settings-save")
                    .px(px(16.))
                    .h(px(32.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(8.))
                    .cursor_pointer()
                    .bg(accent)
                    .text_size(px(13.))
                    .text_color(gpui::rgba(0x0A1002FFu32))
                    .on_mouse_down(MouseButton::Left, move |_, _window, cx| {
                        if let Some(app_entity) = app.upgrade() {
                            app_entity.update(cx, |app, _cx| {
                                if let Err(err) = app.config.save(config::APP_NAME) {
                                    eprintln!("issen: failed to save config.toml: {err}");
                                }
                            });
                        }
                    })
                    .child(SharedString::from(strings.button_save)),
            )
            .child(
                div()
                    .id("settings-close")
                    .px(px(16.))
                    .h(px(32.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(8.))
                    .cursor_pointer()
                    .text_size(px(13.))
                    .text_color(palette.text)
                    .hover(|d| d.bg(palette.control_bg))
                    .on_mouse_down(MouseButton::Left, |_, window, _cx| {
                        window.remove_window();
                    })
                    .child(SharedString::from(strings.button_close)),
            )
    }

    // --- Shared row widgets ---

    fn checkbox_row(
        label: &'static str,
        checked: bool,
        palette: &GlassPalette,
        accent: Hsla,
        on_toggle: impl Fn(&mut App) + 'static,
    ) -> impl IntoElement {
        div()
            .id(SharedString::from(format!("chk-{label}")))
            .flex()
            .items_center()
            .gap_2()
            .cursor_pointer()
            .on_mouse_down(MouseButton::Left, move |_, _window, cx| on_toggle(cx))
            .child(
                div()
                    .size(px(16.))
                    .rounded(px(4.))
                    .border_1()
                    .border_color(if checked {
                        accent
                    } else {
                        palette.control_border
                    })
                    .when(checked, |d| d.bg(accent))
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_size(px(11.))
                    .text_color(gpui::rgba(0x0A1002FFu32))
                    .child(if checked { "\u{2713}" } else { "" }),
            )
            .child(
                div()
                    .text_size(px(13.))
                    .text_color(palette.text)
                    .child(SharedString::from(label)),
            )
    }

    fn segment_button(
        label: &'static str,
        selected: bool,
        palette: &GlassPalette,
        accent: Hsla,
        on_click: impl Fn(&mut App) + 'static,
    ) -> impl IntoElement {
        div()
            .id(SharedString::from(format!("seg-{label}")))
            .px(px(10.))
            .h(px(28.))
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(6.))
            .cursor_pointer()
            .text_size(px(12.))
            .text_color(if selected { accent } else { palette.subtext })
            .when(selected, |d| d.bg(palette.control_bg))
            .when(!selected, |d| {
                d.border_1().border_color(palette.control_border)
            })
            .on_mouse_down(MouseButton::Left, move |_, _window, cx| on_click(cx))
            .child(SharedString::from(label))
    }

    fn stepper_row(
        label: &'static str,
        value_text: String,
        palette: &GlassPalette,
        on_dec: impl Fn(&mut App) + 'static,
        on_inc: impl Fn(&mut App) + 'static,
    ) -> impl IntoElement {
        div()
            .flex()
            .items_center()
            .justify_between()
            .child(
                div()
                    .text_size(px(13.))
                    .text_color(palette.text)
                    .child(SharedString::from(label)),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(Self::stepper_button("-", palette, on_dec))
                    .child(
                        div()
                            .w(px(40.))
                            .text_size(px(13.))
                            .text_color(palette.text)
                            .child(SharedString::from(value_text)),
                    )
                    .child(Self::stepper_button("+", palette, on_inc)),
            )
    }

    fn stepper_button(
        symbol: &'static str,
        palette: &GlassPalette,
        on_click: impl Fn(&mut App) + 'static,
    ) -> impl IntoElement {
        div()
            .id(SharedString::from(format!("stepper-{symbol}")))
            .size(px(22.))
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(5.))
            .border_1()
            .border_color(palette.control_border)
            .cursor_pointer()
            .text_size(px(13.))
            .text_color(palette.text)
            .hover(|d| d.bg(palette.control_bg))
            .on_mouse_down(MouseButton::Left, move |_, _window, cx| on_click(cx))
            .child(symbol)
    }

    fn row_label(label: &'static str, palette: &GlassPalette) -> impl IntoElement {
        div()
            .text_size(px(13.))
            .text_color(palette.text)
            .child(SharedString::from(label))
    }

    fn section_heading(label: &'static str, palette: &GlassPalette) -> impl IntoElement {
        div()
            .text_size(px(16.))
            .text_color(palette.text)
            .child(SharedString::from(label))
    }

    fn list_row(
        key: impl std::fmt::Display,
        primary: String,
        secondary: Option<String>,
        strings: &'static Strings,
        palette: &GlassPalette,
        on_remove: impl Fn(&mut App) + 'static,
    ) -> impl IntoElement {
        div()
            .flex()
            .items_center()
            .gap_2()
            .py(px(4.))
            .child(
                div()
                    .flex_1()
                    .text_size(px(13.))
                    .text_color(palette.text)
                    .child(primary),
            )
            .children(secondary.map(|s| {
                div()
                    .flex_1()
                    .text_size(px(12.))
                    .text_color(palette.subtext)
                    .child(s)
            }))
            .child(
                div()
                    .id(SharedString::from(format!("remove-{key}")))
                    .px(px(8.))
                    .h(px(22.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(5.))
                    .cursor_pointer()
                    .text_size(px(11.))
                    .text_color(palette.subtext)
                    .hover(|d| d.bg(palette.control_bg).text_color(palette.text))
                    .on_mouse_down(MouseButton::Left, move |_, _window, cx| on_remove(cx))
                    .child(SharedString::from(strings.button_delete)),
            )
    }

    // --- Tabs ---

    fn tab_general(
        &self,
        config: &Config,
        strings: &'static Strings,
        palette: &GlassPalette,
        accent: Hsla,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let app = self.app.clone();
        div()
            .flex()
            .flex_col()
            .gap_4()
            .child(Self::section_heading(strings.section_general, palette))
            .child(Self::checkbox_row(
                strings.label_autostart,
                config.autostart,
                palette,
                accent,
                {
                    let app = app.clone();
                    move |cx| set_config(&app, cx, |c| c.autostart = !c.autostart)
                },
            ))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(Self::row_label(strings.label_hotkey, palette))
                    .child(text_input::text_field(
                        &self.hotkey_input,
                        palette.text,
                        accent,
                        palette.control_bg,
                        palette.control_border,
                        cx,
                    )),
            )
            .child(Self::stepper_row(
                strings.label_max_results,
                config.max_results.to_string(),
                palette,
                {
                    let app = app.clone();
                    move |cx| {
                        set_config(&app, cx, |c| {
                            c.max_results = c.max_results.saturating_sub(1).max(1)
                        })
                    }
                },
                {
                    let app = app.clone();
                    move |cx| {
                        set_config(&app, cx, |c| {
                            c.max_results = (c.max_results + 1).min(MAX_VISIBLE_ROWS as u32)
                        })
                    }
                },
            ))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(Self::row_label(strings.label_language, palette))
                    .child(
                        div()
                            .flex()
                            .gap_1()
                            .child(Self::segment_button(
                                strings.language_system,
                                config.language == Language::System,
                                palette,
                                accent,
                                {
                                    let app = app.clone();
                                    move |cx| set_language(&app, Language::System, cx)
                                },
                            ))
                            .child(Self::segment_button(
                                strings.language_en,
                                config.language == Language::En,
                                palette,
                                accent,
                                {
                                    let app = app.clone();
                                    move |cx| set_language(&app, Language::En, cx)
                                },
                            ))
                            .child(Self::segment_button(
                                strings.language_ja,
                                config.language == Language::Ja,
                                palette,
                                accent,
                                {
                                    let app = app.clone();
                                    move |cx| set_language(&app, Language::Ja, cx)
                                },
                            )),
                    ),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(Self::row_label(strings.label_display_target, palette))
                    .child(
                        div()
                            .flex()
                            .flex_wrap()
                            .gap_1()
                            .child(Self::segment_button(
                                strings.display_target_cursor,
                                config.display_target == DisplayTarget::Cursor,
                                palette,
                                accent,
                                {
                                    let app = app.clone();
                                    move |cx| {
                                        set_config(&app, cx, |c| {
                                            c.display_target = DisplayTarget::Cursor
                                        })
                                    }
                                },
                            ))
                            .child(Self::segment_button(
                                strings.display_target_primary,
                                config.display_target == DisplayTarget::Primary,
                                palette,
                                accent,
                                {
                                    let app = app.clone();
                                    move |cx| {
                                        set_config(&app, cx, |c| {
                                            c.display_target = DisplayTarget::Primary
                                        })
                                    }
                                },
                            ))
                            .child(Self::segment_button(
                                strings.display_target_focused,
                                config.display_target == DisplayTarget::FocusedWindow,
                                palette,
                                accent,
                                move |cx| {
                                    set_config(&app, cx, |c| {
                                        c.display_target = DisplayTarget::FocusedWindow
                                    })
                                },
                            )),
                    ),
            )
    }

    fn tab_appearance(
        &self,
        config: &Config,
        strings: &'static Strings,
        palette: &GlassPalette,
        accent: Hsla,
    ) -> impl IntoElement {
        let app = self.app.clone();
        div()
            .flex()
            .flex_col()
            .gap_4()
            .child(Self::section_heading(strings.section_appearance, palette))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(Self::row_label(strings.label_theme, palette))
                    .child(
                        div()
                            .flex()
                            .gap_1()
                            .child(Self::segment_button(
                                strings.theme_system,
                                config.theme == Theme::System,
                                palette,
                                accent,
                                {
                                    let app = app.clone();
                                    move |cx| set_config(&app, cx, |c| c.theme = Theme::System)
                                },
                            ))
                            .child(Self::segment_button(
                                strings.theme_light,
                                config.theme == Theme::Light,
                                palette,
                                accent,
                                {
                                    let app = app.clone();
                                    move |cx| set_config(&app, cx, |c| c.theme = Theme::Light)
                                },
                            ))
                            .child(Self::segment_button(
                                strings.theme_dark,
                                config.theme == Theme::Dark,
                                palette,
                                accent,
                                {
                                    let app = app.clone();
                                    move |cx| set_config(&app, cx, |c| c.theme = Theme::Dark)
                                },
                            )),
                    ),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(Self::row_label(strings.label_accent_color, palette))
                    .child(
                        div().flex().gap_2().children(
                            [
                                (AccentColor::Lime, strings.accent_color_lime),
                                (AccentColor::Red, strings.accent_color_red),
                                (AccentColor::Orange, strings.accent_color_orange),
                                (AccentColor::Blue, strings.accent_color_blue),
                                (AccentColor::Purple, strings.accent_color_purple),
                            ]
                            .into_iter()
                            .map(|(color, name)| {
                                let selected = config.accent_color == color;
                                let app = app.clone();
                                div()
                                    .id(SharedString::from(format!("accent-{name}")))
                                    .size(px(26.))
                                    .rounded(px(13.))
                                    .bg(ui_chrome::accent_color(color))
                                    .cursor_pointer()
                                    .when(selected, |d| d.border_2().border_color(palette.text))
                                    .on_mouse_down(MouseButton::Left, move |_, _window, cx| {
                                        set_config(&app, cx, move |c| c.accent_color = color)
                                    })
                            }),
                        ),
                    ),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(Self::row_label(strings.label_font, palette))
                    .child(
                        div()
                            .flex()
                            .gap_1()
                            .child(Self::segment_button(
                                strings.font_segoe_ui,
                                config.ui_font == UiFont::SegoeUi,
                                palette,
                                accent,
                                {
                                    let app = app.clone();
                                    move |cx| set_config(&app, cx, |c| c.ui_font = UiFont::SegoeUi)
                                },
                            ))
                            .child(Self::segment_button(
                                strings.font_yu_gothic,
                                config.ui_font == UiFont::YuGothic,
                                palette,
                                accent,
                                {
                                    let app = app.clone();
                                    move |cx| set_config(&app, cx, |c| c.ui_font = UiFont::YuGothic)
                                },
                            ))
                            .child(Self::segment_button(
                                strings.font_meiryo,
                                config.ui_font == UiFont::Meiryo,
                                palette,
                                accent,
                                {
                                    let app = app.clone();
                                    move |cx| set_config(&app, cx, |c| c.ui_font = UiFont::Meiryo)
                                },
                            )),
                    ),
            )
            .child(Self::stepper_row(
                strings.label_font_size,
                format!("{:.2}", config.font_scale),
                palette,
                {
                    let app = app.clone();
                    move |cx| {
                        set_config(&app, cx, |c| {
                            c.font_scale = ((c.font_scale - 0.05) * 100.).round() / 100.
                        })
                    }
                },
                move |cx| {
                    set_config(&app, cx, |c| {
                        c.font_scale = ((c.font_scale + 0.05) * 100.).round() / 100.
                    })
                },
            ))
    }

    #[allow(clippy::too_many_arguments)]
    fn tab_index(
        &self,
        config: &Config,
        strings: &'static Strings,
        palette: &GlassPalette,
        scanning: bool,
        last_scan_finished: Option<std::time::Instant>,
        last_scan_count: usize,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let app = self.app.clone();
        div()
            .flex()
            .flex_col()
            .gap_4()
            .child(Self::section_heading(strings.section_index, palette))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(Self::row_label(strings.label_custom_folders, palette))
                    .children(config.index_folders.iter().cloned().enumerate().map(
                        |(i, folder)| {
                            let app = app.clone();
                            Self::list_row(
                                format!("folder-{i}"),
                                folder,
                                None,
                                strings,
                                palette,
                                move |cx| {
                                    set_config(&app, cx, move |c| {
                                        if i < c.index_folders.len() {
                                            c.index_folders.remove(i);
                                        }
                                    })
                                },
                            )
                        },
                    ))
                    .child(
                        div()
                            .id("add-folder")
                            .h(px(28.))
                            .px(px(12.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .w(px(140.))
                            .rounded(px(6.))
                            .border_1()
                            .border_color(palette.control_border)
                            .text_size(px(12.))
                            .text_color(if self.pending_folder_pick {
                                palette.subtext
                            } else {
                                palette.text
                            })
                            .when(!self.pending_folder_pick, |d| {
                                d.cursor_pointer()
                                    .hover(|d| d.bg(palette.control_bg))
                                    .on_mouse_down(
                                        MouseButton::Left,
                                        cx.listener(|this, _, _, cx| this.add_folder_clicked(cx)),
                                    )
                            })
                            .child(SharedString::from(strings.button_add_folder)),
                    ),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(Self::row_label(strings.label_exclude_patterns, palette))
                    .children(config.exclude_patterns.iter().cloned().enumerate().map(
                        |(i, pattern)| {
                            let app = app.clone();
                            Self::list_row(
                                format!("pattern-{i}"),
                                pattern,
                                None,
                                strings,
                                palette,
                                move |cx| {
                                    set_config(&app, cx, move |c| {
                                        if i < c.exclude_patterns.len() {
                                            c.exclude_patterns.remove(i);
                                        }
                                    })
                                },
                            )
                        },
                    ))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(div().flex_1().child(text_input::text_field(
                                &self.new_exclude_pattern_input,
                                palette.text,
                                palette.text,
                                palette.control_bg,
                                palette.control_border,
                                cx,
                            )))
                            .child({
                                let app = app.clone();
                                let input = self.new_exclude_pattern_input.clone();
                                div()
                                    .id("add-exclude-pattern")
                                    .px(px(12.))
                                    .h(px(28.))
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .rounded(px(6.))
                                    .border_1()
                                    .border_color(palette.control_border)
                                    .cursor_pointer()
                                    .text_size(px(12.))
                                    .text_color(palette.text)
                                    .hover(|d| d.bg(palette.control_bg))
                                    .on_mouse_down(MouseButton::Left, move |_, _window, cx| {
                                        let text = input.read(cx).text().trim().to_string();
                                        if text.is_empty() {
                                            return;
                                        }
                                        set_config(&app, cx, move |c| {
                                            c.exclude_patterns.push(text)
                                        });
                                        input.update(cx, |ti, cx| ti.set_text("", cx));
                                    })
                                    .child(SharedString::from(strings.button_add))
                            }),
                    ),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .child({
                        let app = app.clone();
                        div()
                            .id("rescan-now")
                            .px(px(14.))
                            .h(px(30.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded(px(6.))
                            .border_1()
                            .border_color(palette.control_border)
                            .text_size(px(12.))
                            .text_color(if scanning {
                                palette.subtext
                            } else {
                                palette.text
                            })
                            .when(!scanning, |d| {
                                d.cursor_pointer()
                                    .hover(|d| d.bg(palette.control_bg))
                                    .on_mouse_down(MouseButton::Left, move |_, _window, cx| {
                                        if let Some(app_entity) = app.upgrade() {
                                            app_entity.update(cx, |app, cx| app.start_scan(cx));
                                        }
                                    })
                            })
                            .child(SharedString::from(strings.button_rescan_now))
                    })
                    .child(if scanning {
                        div()
                            .text_size(px(12.))
                            .text_color(palette.subtext)
                            .child(SharedString::from(strings.scanning))
                    } else if let Some(finished) = last_scan_finished {
                        let minutes = finished.elapsed().as_secs() / 60;
                        div().text_size(px(12.)).text_color(palette.subtext).child(
                            i18n::last_scan_text(i18n::lang_of(strings), minutes, last_scan_count),
                        )
                    } else {
                        div()
                    }),
            )
    }

    fn tab_aliases(
        &self,
        config: &Config,
        strings: &'static Strings,
        palette: &GlassPalette,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let app = self.app.clone();
        div()
            .flex()
            .flex_col()
            .gap_3()
            .child(Self::section_heading(strings.section_aliases, palette))
            .children(
                config
                    .aliases
                    .iter()
                    .cloned()
                    .enumerate()
                    .map(|(i, alias)| {
                        let app = app.clone();
                        let secondary = if alias.args.is_empty() {
                            alias.target.clone()
                        } else {
                            format!("{}  {}", alias.target, alias.args)
                        };
                        Self::list_row(
                            format!("alias-{i}"),
                            alias.name.clone(),
                            Some(secondary),
                            strings,
                            palette,
                            move |cx| {
                                set_config(&app, cx, move |c| {
                                    if i < c.aliases.len() {
                                        c.aliases.remove(i);
                                    }
                                })
                            },
                        )
                    }),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(
                        div()
                            .flex()
                            .gap_2()
                            .child(labeled_field(
                                strings.label_alias_name,
                                &self.new_alias_name_input,
                                palette,
                                cx,
                            ))
                            .child(labeled_field(
                                strings.label_alias_target,
                                &self.new_alias_target_input,
                                palette,
                                cx,
                            ))
                            .child(labeled_field(
                                strings.label_alias_args,
                                &self.new_alias_args_input,
                                palette,
                                cx,
                            )),
                    )
                    .child({
                        let app = app.clone();
                        let name_input = self.new_alias_name_input.clone();
                        let target_input = self.new_alias_target_input.clone();
                        let args_input = self.new_alias_args_input.clone();
                        div()
                            .id("add-alias")
                            .px(px(14.))
                            .h(px(28.))
                            .w(px(140.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded(px(6.))
                            .border_1()
                            .border_color(palette.control_border)
                            .cursor_pointer()
                            .text_size(px(12.))
                            .text_color(palette.text)
                            .hover(|d| d.bg(palette.control_bg))
                            .on_mouse_down(MouseButton::Left, move |_, _window, cx| {
                                let name = name_input.read(cx).text().trim().to_string();
                                let target = target_input.read(cx).text().trim().to_string();
                                let args = args_input.read(cx).text().trim().to_string();
                                if name.is_empty() || target.is_empty() {
                                    return;
                                }
                                set_config(&app, cx, move |c| {
                                    c.aliases.push(AliasEntry { name, target, args })
                                });
                                name_input.update(cx, |ti, cx| ti.set_text("", cx));
                                target_input.update(cx, |ti, cx| ti.set_text("", cx));
                                args_input.update(cx, |ti, cx| ti.set_text("", cx));
                            })
                            .child(SharedString::from(strings.button_add_alias))
                    }),
            )
    }

    fn tab_windows_shortcuts(
        &self,
        config: &Config,
        strings: &'static Strings,
        palette: &GlassPalette,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let app = self.app.clone();
        div()
            .flex()
            .flex_col()
            .gap_3()
            .child(Self::section_heading(
                strings.section_windows_shortcuts,
                palette,
            ))
            .children(
                config
                    .custom_windows_shortcuts
                    .iter()
                    .cloned()
                    .enumerate()
                    .map(|(i, entry)| {
                        let app = app.clone();
                        Self::list_row(
                            format!("shortcut-{i}"),
                            entry.label.clone(),
                            Some(entry.uri.clone()),
                            strings,
                            palette,
                            move |cx| {
                                set_config(&app, cx, move |c| {
                                    if i < c.custom_windows_shortcuts.len() {
                                        c.custom_windows_shortcuts.remove(i);
                                    }
                                })
                            },
                        )
                    }),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(
                        div()
                            .flex()
                            .gap_2()
                            .child(labeled_field(
                                strings.label_shortcut_label,
                                &self.new_shortcut_label_input,
                                palette,
                                cx,
                            ))
                            .child(labeled_field(
                                strings.label_shortcut_uri,
                                &self.new_shortcut_uri_input,
                                palette,
                                cx,
                            )),
                    )
                    .child({
                        let app = app.clone();
                        let label_input = self.new_shortcut_label_input.clone();
                        let uri_input = self.new_shortcut_uri_input.clone();
                        div()
                            .id("add-shortcut")
                            .px(px(14.))
                            .h(px(28.))
                            .w(px(160.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded(px(6.))
                            .border_1()
                            .border_color(palette.control_border)
                            .cursor_pointer()
                            .text_size(px(12.))
                            .text_color(palette.text)
                            .hover(|d| d.bg(palette.control_bg))
                            .on_mouse_down(MouseButton::Left, move |_, _window, cx| {
                                let label = label_input.read(cx).text().trim().to_string();
                                let uri = uri_input.read(cx).text().trim().to_string();
                                if label.is_empty() || uri.is_empty() {
                                    return;
                                }
                                set_config(&app, cx, move |c| {
                                    c.custom_windows_shortcuts
                                        .push(WindowsShortcutEntry { label, uri })
                                });
                                label_input.update(cx, |ti, cx| ti.set_text("", cx));
                                uri_input.update(cx, |ti, cx| ti.set_text("", cx));
                            })
                            .child(SharedString::from(strings.button_add_shortcut))
                    }),
            )
    }

    fn tab_everything(
        &self,
        config: &Config,
        strings: &'static Strings,
        palette: &GlassPalette,
        accent: Hsla,
    ) -> impl IntoElement {
        let app = self.app.clone();
        div()
            .flex()
            .flex_col()
            .gap_3()
            .child(Self::section_heading(strings.section_everything, palette))
            .child(
                div()
                    .text_size(px(12.))
                    .text_color(palette.subtext)
                    .child(SharedString::from(strings.everything_connected)),
            )
            .child(Self::checkbox_row(
                strings.label_everything_enabled,
                config.everything_enabled,
                palette,
                accent,
                move |cx| set_config(&app, cx, |c| c.everything_enabled = !c.everything_enabled),
            ))
    }
}

fn set_language(app: &WeakEntity<IssenApp>, language: Language, cx: &mut App) {
    if let Some(app_entity) = app.upgrade() {
        app_entity.update(cx, |app, cx| {
            app.config.language = language;
            app.apply_language(cx);
        });
    }
}

fn labeled_field(
    label: &'static str,
    input: &Entity<TextInput>,
    palette: &GlassPalette,
    cx: &mut App,
) -> Div {
    div()
        .flex_1()
        .flex()
        .flex_col()
        .gap_1()
        .child(
            div()
                .text_size(px(11.))
                .text_color(palette.subtext)
                .child(SharedString::from(label)),
        )
        .child(text_input::text_field(
            input,
            palette.text,
            palette.text,
            palette.control_bg,
            palette.control_border,
            cx,
        ))
}

impl Render for SettingsWindow {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // Reads the cached snapshot (see `AppSnapshot`'s doc comment) —
        // never `self.app.upgrade()?.read(cx)` directly here.
        let config = self.cached.config.clone();
        let strings = self.cached.strings;
        let scanning = self.cached.scanning;
        let last_scan_finished = self.cached.last_scan_finished;
        let last_scan_count = self.cached.last_scan_count;

        let dark = ui_chrome::resolve_dark(config.theme, window);
        let palette = ui_chrome::palette(dark);
        let accent = ui_chrome::accent_color(config.accent_color);

        let content: gpui::AnyElement = match self.tab {
            SettingsTab::General => self
                .tab_general(&config, strings, &palette, accent, cx)
                .into_any_element(),
            SettingsTab::Appearance => self
                .tab_appearance(&config, strings, &palette, accent)
                .into_any_element(),
            SettingsTab::Index => self
                .tab_index(
                    &config,
                    strings,
                    &palette,
                    scanning,
                    last_scan_finished,
                    last_scan_count,
                    cx,
                )
                .into_any_element(),
            SettingsTab::Aliases => self
                .tab_aliases(&config, strings, &palette, cx)
                .into_any_element(),
            SettingsTab::WindowsShortcuts => self
                .tab_windows_shortcuts(&config, strings, &palette, cx)
                .into_any_element(),
            SettingsTab::Everything => self
                .tab_everything(&config, strings, &palette, accent)
                .into_any_element(),
        };

        ui_chrome::glass_container(&palette)
            .child(ui_chrome::title_bar(strings.settings_title, &palette))
            .child(
                div()
                    .flex_1()
                    .min_h(px(0.))
                    .flex()
                    .child(self.nav(strings, &palette, accent, cx))
                    .child(
                        div()
                            .id("settings-content-scroll")
                            .flex_1()
                            .min_w(px(0.))
                            .h_full()
                            .overflow_y_scroll()
                            .track_scroll(&self.content_scroll)
                            .p(px(20.))
                            .child(content),
                    ),
            )
            .child(self.footer(strings, &palette, accent))
    }
}
