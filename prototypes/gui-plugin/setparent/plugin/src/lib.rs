//! Prototype B: the plugin creates its own HWND via raw Win32 APIs, and
//! the host embeds it into its own window via `SetParent`. Since the goal
//! is to test the embedding mechanism itself, this is built entirely from
//! standard Win32 controls (STATIC/BUTTON), no GUI toolkit.

use std::sync::Once;
use windows::core::w;
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    GetStockObject, COLOR_BTNFACE, DEFAULT_GUI_FONT, HBRUSH, HFONT,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, GetWindowLongPtrW, RegisterClassW, SendMessageW, SetParent,
    SetWindowLongPtrW, SetWindowPos, ShowWindow, GWL_STYLE, SWP_FRAMECHANGED, SWP_NOACTIVATE,
    SWP_NOMOVE, SWP_NOSIZE, SWP_NOZORDER, SW_SHOW, WM_DESTROY, WM_SETFONT, WNDCLASSW, WS_CHILD,
    WS_POPUP, WS_VISIBLE,
};

static REGISTER_CLASS: Once = Once::new();

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if msg == WM_DESTROY {
        return LRESULT(0);
    }
    DefWindowProcW(hwnd, msg, wparam, lparam)
}

/// Takes the host's HWND (as `usize`), builds a container window holding a
/// label + button, embeds it, and returns the container's HWND (as
/// `usize`).
///
/// The sequence is "create standalone as `WS_POPUP`" → `SetParent` →
/// "switch the style to `WS_CHILD`", deliberately keeping open the
/// possibility that a plugin could also run as a standalone window. That's
/// more steps than just passing the host HWND straight into
/// `CreateWindowExW`'s `hwndparent` to create it as a child from the
/// start, but what's actually being compared is post-embedding behavior,
/// which doesn't change either way.
#[no_mangle]
pub extern "C" fn create_child(parent_hwnd: usize) -> usize {
    unsafe {
        let Ok(hinstance) = GetModuleHandleW(None) else {
            return 0;
        };
        let class_name = w!("IssenProtoBContainer");

        REGISTER_CLASS.call_once(|| {
            let wc = WNDCLASSW {
                lpfnWndProc: Some(wndproc),
                hInstance: hinstance.into(),
                lpszClassName: class_name,
                hbrBackground: HBRUSH((COLOR_BTNFACE.0 + 1) as isize as *mut _),
                ..Default::default()
            };
            RegisterClassW(&wc);
        });

        let Ok(container) = CreateWindowExW(
            Default::default(),
            class_name,
            w!(""),
            WS_POPUP,
            0,
            0,
            260,
            80,
            None,
            None,
            Some(hinstance.into()),
            None,
        ) else {
            return 0;
        };

        // A standard control created via raw `CreateWindowExW` stays on
        // the default `SYSTEM_FONT` (an old bitmap font) unless
        // `WM_SETFONT` is sent explicitly. That font has no Japanese
        // glyphs, which is why the label below renders as garbled boxes
        // without this. Sending `DEFAULT_GUI_FONT` (the standard dialog
        // font) explicitly gets TrueType system font linking to kick in
        // instead.
        let font = HFONT(GetStockObject(DEFAULT_GUI_FONT).0);

        let Ok(label) = CreateWindowExW(
            Default::default(),
            w!("STATIC"),
            w!("プロトタイプB: SetParent埋め込み"),
            WS_CHILD | WS_VISIBLE,
            8,
            8,
            240,
            20,
            Some(container),
            None,
            Some(hinstance.into()),
            None,
        ) else {
            return 0;
        };
        SendMessageW(
            label,
            WM_SETFONT,
            Some(WPARAM(font.0 as usize)),
            Some(LPARAM(1)),
        );

        let Ok(button) = CreateWindowExW(
            Default::default(),
            w!("BUTTON"),
            w!("Click me"),
            WS_CHILD | WS_VISIBLE,
            8,
            36,
            100,
            28,
            Some(container),
            None,
            Some(hinstance.into()),
            None,
        ) else {
            return 0;
        };
        SendMessageW(
            button,
            WM_SETFONT,
            Some(WPARAM(font.0 as usize)),
            Some(LPARAM(1)),
        );

        let parent = HWND(parent_hwnd as *mut _);
        let _ = SetParent(container, Some(parent));

        // `SetParent` doesn't change the window style, so drop `WS_POPUP`
        // and switch to `WS_CHILD` explicitly.
        let style = GetWindowLongPtrW(container, GWL_STYLE);
        let new_style =
            (style & !(WS_POPUP.0 as isize)) | (WS_CHILD.0 as isize) | (WS_VISIBLE.0 as isize);
        SetWindowLongPtrW(container, GWL_STYLE, new_style);
        let _ = SetWindowPos(
            container,
            None,
            0,
            0,
            0,
            0,
            SWP_NOSIZE | SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE | SWP_FRAMECHANGED,
        );
        let _ = ShowWindow(container, SW_SHOW);

        container.0 as usize
    }
}
