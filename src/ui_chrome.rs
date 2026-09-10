//! Theme constants and chrome shared by the secondary windows (about,
//! settings, tools): a translucent glass panel background plus a custom
//! title bar with drag-to-move and a close button. GPUI's default window
//! decorations are used nowhere in this app (see `docs/architecture/
//! window-lifecycle.md`), so every window paints its own.

use gpui::{
    div, hsla, px, rems, rgba, AnyView, App, AppContext, Context, Div, Hsla, InteractiveElement,
    IntoElement, MouseButton, ParentElement, Render, SharedString, Styled, Window,
    WindowAppearance, WindowControlArea,
};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use windows::Win32::Foundation::HWND;
use windows::Win32::UI::WindowsAndMessaging::{
    SetWindowPos, HWND_TOPMOST, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE,
};

use crate::config::Theme;

/// `Window` has an inherent `window_handle()` method (returns GPUI's own
/// `AnyWindowHandle`, not an HWND) with the same name as the
/// `HasWindowHandle` trait method, so the trait method must be called
/// fully-qualified to get the actual HWND (same gotcha as `app.rs`'s
/// private copy of this for the main window).
pub fn window_hwnd(window: &Window) -> Option<HWND> {
    let handle = HasWindowHandle::window_handle(window).ok()?;
    match handle.as_raw() {
        RawWindowHandle::Win32(handle) => Some(HWND(handle.hwnd.get() as *mut core::ffi::c_void)),
        _ => None,
    }
}

/// Puts a secondary window in the OS topmost band. Needed because the main
/// search window (`app.rs::run`) is itself always-on-top
/// (`WindowKind::PopUp` there doesn't imply topmost on Windows) — without
/// this, a secondary window opened while the main window is visible ends up
/// behind it. GPUI's `WindowOptions` has no cross-platform equivalent, so
/// this goes straight to the same raw `SetWindowPos` call the egui/eframe
/// version used via `.with_always_on_top()`.
pub fn set_topmost(window: &Window) {
    let Some(hwnd) = window_hwnd(window) else {
        return;
    };
    unsafe {
        let _ = SetWindowPos(
            hwnd,
            Some(HWND_TOPMOST),
            0,
            0,
            0,
            0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
        );
    }
}

/// GPUI's default `rem` size — every `.text_size(rems(n))` call in this app
/// (converted from a literal `px` value by dividing by this) is sized
/// relative to it. See [`apply_font_scale`].
const BASE_REM_SIZE: f32 = 16.0;

/// Applies `config.font_scale` to `window` by scaling its `rem` size —
/// `Window::set_rem_size`'s own doc comment describes this as "just like
/// zooming a web page". Every text size in the app is expressed via
/// `.text_size(rems(n))` rather than a literal `px(n)` specifically so this
/// one call scales all of them at once, without threading a scale factor
/// through every widget helper individually. Layout metrics (paddings, row
/// heights, window sizes) stay in literal `px` and don't scale — matching
/// the egui/eframe version's `apply_font_scale`, which only ever rescaled
/// `egui::Style::text_styles` for the same reason, and explains why its
/// slider was clamped to a narrow 0.9-1.25 range: much further outside it
/// and text starts to clip against those fixed-size rows.
///
/// Called at the top of every window's `render()` (not just once at
/// creation) so a live change from the settings window's font-scale
/// stepper is picked up on the next repaint of each open window.
pub fn apply_font_scale(window: &mut Window, font_scale: f32) {
    window.set_rem_size(px(BASE_REM_SIZE * font_scale.clamp(0.5, 2.0)));
}

/// Resolves `config.theme` to an actual dark/light bool: `Light`/`Dark` are
/// explicit overrides, `System` follows the window's real OS appearance.
pub fn resolve_dark(theme: Theme, window: &Window) -> bool {
    match theme {
        Theme::Light => false,
        Theme::Dark => true,
        Theme::System => matches!(
            window.appearance(),
            WindowAppearance::Dark | WindowAppearance::VibrantDark
        ),
    }
}

/// The actual color for a given `config.accent_color`. The default `Lime`
/// is the reference design's `oklch(0.9 0.19 124)` converted to sRGB.
pub fn accent_color(theme: crate::config::AccentColor) -> Hsla {
    use crate::config::AccentColor;
    match theme {
        AccentColor::Lime => rgba(0xC4F252FFu32).into(),
        AccentColor::Red => rgba(0xFF746EFFu32).into(),
        AccentColor::Orange => rgba(0xFFA63DFFu32).into(),
        AccentColor::Blue => rgba(0x6CC3FFFFu32).into(),
        AccentColor::Purple => rgba(0xD798FFFFu32).into(),
    }
}

/// A full glass-panel color set. [`palette`] picks one based on the
/// window's actual OS appearance (`Window::appearance`), same values as
/// the egui/eframe version's `GlassPalette`.
pub struct GlassPalette {
    pub panel_bg: Hsla,
    pub border: Hsla,
    pub text: Hsla,
    pub subtext: Hsla,
    pub divider: Hsla,
    #[allow(dead_code)]
    pub control_bg: Hsla,
    #[allow(dead_code)]
    pub control_border: Hsla,
}

pub fn palette(dark: bool) -> GlassPalette {
    if dark {
        GlassPalette {
            panel_bg: rgba(0x12141A96u32).into(),
            border: rgba(0xFFFFFF24u32).into(),
            text: rgba(0xF2F4F7FFu32).into(),
            subtext: rgba(0xFFFFFF8Fu32).into(),
            divider: rgba(0xFFFFFF1Au32).into(),
            control_bg: rgba(0xFFFFFF0Fu32).into(),
            control_border: rgba(0xFFFFFF1Eu32).into(),
        }
    } else {
        GlassPalette {
            panel_bg: rgba(0xFAFAFCC3u32).into(),
            border: rgba(0x00000012u32).into(),
            text: rgba(0x1E2024FFu32).into(),
            subtext: rgba(0x000000A0u32).into(),
            divider: rgba(0x00000012u32).into(),
            control_bg: rgba(0x0000000Au32).into(),
            control_border: rgba(0x0000001Au32).into(),
        }
    }
}

/// A fully opaque variant of [`palette`]'s `panel_bg` (same hue, alpha
/// forced to 255). For a window that wants the glass chrome's colors but
/// not the see-through background — settings (`settings_window.rs`) and
/// tools (`tools/mod.rs`), whose users found the default translucency too
/// faint to read text against a busy desktop behind it. `palette()` itself
/// stays untouched so about (which didn't ask for this) isn't affected.
pub fn opaque_panel_bg(dark: bool) -> Hsla {
    if dark {
        rgba(0x12141AFFu32).into()
    } else {
        rgba(0xFAFAFCFFu32).into()
    }
}

/// A small text tooltip styled to match the glass chrome, for icon-only
/// buttons that have no visible label (e.g. the color picker's copy/
/// eyedropper buttons in `tools/mod.rs`).
struct SimpleTooltip {
    text: SharedString,
    dark: bool,
}

impl Render for SimpleTooltip {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        let palette = palette(self.dark);
        div()
            .bg(opaque_panel_bg(self.dark))
            .border_1()
            .border_color(palette.border)
            .rounded(px(6.))
            .px(px(8.))
            .py(px(4.))
            .text_size(rems(11. / 16.))
            .text_color(palette.text)
            .child(self.text.clone())
    }
}

/// Builds a `.tooltip()` callback showing `text` in the glass-chrome style.
/// `dark` is snapshotted at call time (same as the rest of this window's
/// palette) rather than re-resolved per hover.
pub fn simple_tooltip(
    text: impl Into<SharedString>,
    dark: bool,
) -> impl Fn(&mut Window, &mut App) -> AnyView {
    let text = text.into();
    move |_window, cx| {
        cx.new(|_| SimpleTooltip {
            text: text.clone(),
            dark,
        })
        .into()
    }
}

/// A window-filling container already styled with the glass panel
/// background and border. Callers stack their content into it.
pub fn glass_container(palette: &GlassPalette) -> Div {
    div()
        .size_full()
        .flex()
        .flex_col()
        .bg(palette.panel_bg)
        .border_1()
        .border_color(palette.border)
}

pub const TITLE_BAR_HEIGHT: f32 = 36.0;

/// The custom title bar shared by every secondary window: title text on
/// the left, a close (×) button on the right, and a drag-to-move region
/// covering the rest. Dragging and closing only touch the window itself
/// (`Window::start_window_move`/`remove_window`), so neither needs access
/// to the root entity's state.
pub fn title_bar(title: impl Into<SharedString>, palette: &GlassPalette) -> Div {
    // The drag region is a separate child from the close button (rather than
    // one mouse-down handler on the whole row) so a click on the close
    // button doesn't also register as a window-move — the two hitboxes
    // would otherwise overlap and both fire.
    let drag_region = div()
        .flex_1()
        .h_full()
        .flex()
        .items_center()
        // `Window::start_window_move` is a no-op on Windows (its own doc
        // comment says "for Linux and macOS") — real Windows window-drag
        // goes through the `WM_NCHITTEST` hit-test system, wired up
        // automatically by GPUI for any element marked as a
        // `WindowControlArea::Drag` region (see `app.rs`'s main search box
        // for the same mechanism). `is_movable: true` must also be set on
        // the window (`WindowOptions`) or Windows ignores this entirely.
        .window_control_area(WindowControlArea::Drag)
        .child(
            div()
                .text_size(rems(11. / 16.))
                .text_color(palette.subtext)
                .child(title.into()),
        );

    div()
        .h(px(TITLE_BAR_HEIGHT))
        .w_full()
        .flex_none()
        .flex()
        .items_center()
        .justify_between()
        .pl(px(16.))
        .pr(px(8.))
        .border_b_1()
        .border_color(palette.divider)
        .child(drag_region)
        .child(close_button(palette))
}

fn close_button(palette: &GlassPalette) -> Div {
    div()
        .flex()
        .items_center()
        .justify_center()
        .size(px(26.))
        .rounded(px(4.))
        .text_size(rems(13. / 16.))
        .text_color(palette.subtext)
        .hover(|d| d.bg(hsla(0., 0., 1., 0.08)).text_color(palette.text))
        .on_mouse_down(MouseButton::Left, |_, window, _cx| {
            window.remove_window();
        })
        .child("\u{2715}")
}
