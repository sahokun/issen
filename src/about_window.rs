//! The "About" window: app name, version, and a close button. The smallest
//! secondary window, ported first (see `app.rs`'s module doc comment) to
//! prove out opening/reusing/closing a runtime-created GPUI window before
//! settings and tools — the bigger, stateful secondary windows — build on
//! the same mechanism.

use gpui::{
    div, point, px, rems, size, App, AppContext, Bounds, Context, IntoElement, ParentElement,
    Render, SharedString, Styled, Window, WindowBackgroundAppearance, WindowBounds, WindowHandle,
    WindowKind, WindowOptions,
};

use crate::config::{self, Theme, UiFont};
use crate::i18n::{self, Strings};
use crate::ui_chrome;

const WINDOW_SIZE: (f32, f32) = (320.0, 160.0);

pub struct AboutWindow {
    strings: &'static Strings,
    /// Snapshotted from the live config when the window is opened, not
    /// re-read afterward — About is short-lived, and re-resolving on every
    /// repaint would mean re-reading `config.toml` from disk every frame.
    theme: Theme,
    ui_font: UiFont,
    font_scale: f32,
}

/// Opens the about window, or brings an already-open one to the front.
/// Mirrors the double-open guard every secondary window needs: `handle`
/// only stays valid while its window is open, so `update` fails (and this
/// falls through to opening a fresh window) once the user has closed it.
pub fn open(
    existing: &mut Option<WindowHandle<AboutWindow>>,
    strings: &'static Strings,
    theme: Theme,
    ui_font: UiFont,
    font_scale: f32,
    cx: &mut App,
) {
    if let Some(handle) = existing {
        if handle
            .update(cx, |_, window, _| window.activate_window())
            .is_ok()
        {
            return;
        }
    }

    let bounds = Bounds {
        origin: point(px(100.), px(100.)),
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
                is_resizable: false,
                window_background: WindowBackgroundAppearance::Transparent,
                ..Default::default()
            },
            move |window, cx| {
                ui_chrome::set_topmost(window);
                cx.new(|_cx| AboutWindow {
                    strings,
                    theme,
                    ui_font,
                    font_scale,
                })
            },
        )
        .expect("failed to open about window");
    *existing = Some(handle);
}

impl Render for AboutWindow {
    fn render(&mut self, window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        ui_chrome::apply_font_scale(window, self.font_scale);
        let dark = ui_chrome::resolve_dark(self.theme, window);
        let palette = ui_chrome::palette(dark);

        let version =
            i18n::about_version_text(i18n::lang_of(self.strings), env!("CARGO_PKG_VERSION"));

        ui_chrome::glass_container(&palette)
            .font_family(crate::fonts::ui_font_family(self.ui_font))
            .child(ui_chrome::title_bar(self.strings.about_title, &palette))
            .child(
                div()
                    .flex_1()
                    .flex()
                    .flex_col()
                    .items_center()
                    .justify_center()
                    .gap_2()
                    .child(
                        div()
                            .text_size(rems(18. / 16.))
                            .text_color(palette.text)
                            .child(SharedString::from(config::APP_NAME)),
                    )
                    .child(
                        div()
                            .text_size(rems(13. / 16.))
                            .text_color(palette.subtext)
                            .child(SharedString::from(version)),
                    ),
            )
    }
}
