//! Prototype C: the plugin creates an independent top-level window
//! (`WS_OVERLAPPEDWINDOW`) via raw Win32 APIs and shows it on a thread
//! with its own message loop. Same goal as prototype A (`thread-eframe`)
//! — an independent window, not embedded in the host — but built without
//! egui/eframe to see how much smaller the DLL could get.
//! `WS_OVERLAPPEDWINDOW` gets a standard OS title bar and frame, so
//! move/resize come from the OS for free (freedom prototype B couldn't
//! have, structurally).

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Once;

use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    GetStockObject, COLOR_BTNFACE, DEFAULT_GUI_FONT, HBRUSH, HFONT,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DispatchMessageW, GetMessageW, GetWindowLongPtrW,
    PostQuitMessage, RegisterClassW, SendMessageW, SetWindowLongPtrW, SetWindowTextW, ShowWindow,
    TranslateMessage, CW_USEDEFAULT, GWLP_USERDATA, HMENU, MSG, SW_SHOW, WM_COMMAND, WM_DESTROY,
    WM_SETFONT, WNDCLASSW, WS_CHILD, WS_OVERLAPPEDWINDOW, WS_VISIBLE,
};

static OPENING: AtomicBool = AtomicBool::new(false);
static REGISTER_CLASS: Once = Once::new();

/// The button's control ID, matched against `WM_COMMAND`'s `wparam` low
/// word to identify the click.
const IDC_BUTTON: usize = 1001;
/// The `BN_CLICKED` (button-pressed notification) value. Hardcoded rather
/// than pulling in another `windows` crate feature flag for it — it's a
/// fixed `0` in WinUser.h.
const BN_CLICKED: u32 = 0;

struct CountState {
    count: i32,
    label: HWND,
}

fn to_wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        WM_COMMAND => {
            let control_id = wparam.0 & 0xffff;
            let notify_code = ((wparam.0 >> 16) & 0xffff) as u32;
            if control_id == IDC_BUTTON && notify_code == BN_CLICKED {
                let state = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut CountState;
                if let Some(state) = state.as_mut() {
                    state.count += 1;
                    let text = to_wide(&format!("clicked {} times", state.count));
                    let _ = SetWindowTextW(state.label, PCWSTR(text.as_ptr()));
                }
            }
            LRESULT(0)
        }
        WM_DESTROY => {
            let state = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut CountState;
            if !state.is_null() {
                drop(Box::from_raw(state));
            }
            // An independent top-level window runs its own message loop
            // (`run_window`), so without calling `PostQuitMessage` from
            // `WM_DESTROY`, `GetMessageW` never returns and the thread
            // blocks forever. Prototype B's embedded child window didn't
            // need this since it rides on the host's message loop, but an
            // independent window must do it.
            PostQuitMessage(0);
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

fn run_window() {
    unsafe {
        let Ok(hinstance) = GetModuleHandleW(None) else {
            return;
        };
        let class_name = w!("IssenProtoCWindow");

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

        let Ok(window) = CreateWindowExW(
            Default::default(),
            class_name,
            w!("Prototype C Tool"),
            WS_OVERLAPPEDWINDOW | WS_VISIBLE,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            300,
            160,
            None,
            None,
            Some(hinstance.into()),
            None,
        ) else {
            return;
        };

        // Same reason as `setparent/plugin/src/lib.rs`: the default bitmap
        // font has no Japanese glyphs, so `DEFAULT_GUI_FONT` is sent explicitly.
        let font = HFONT(GetStockObject(DEFAULT_GUI_FONT).0);

        let Ok(label) = CreateWindowExW(
            Default::default(),
            w!("STATIC"),
            w!("clicked 0 times"),
            WS_CHILD | WS_VISIBLE,
            8,
            8,
            240,
            20,
            Some(window),
            None,
            Some(hinstance.into()),
            None,
        ) else {
            return;
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
            Some(window),
            Some(HMENU(IDC_BUTTON as *mut _)),
            Some(hinstance.into()),
            None,
        ) else {
            return;
        };
        SendMessageW(
            button,
            WM_SETFONT,
            Some(WPARAM(font.0 as usize)),
            Some(LPARAM(1)),
        );

        let state = Box::new(CountState { count: 0, label });
        SetWindowLongPtrW(window, GWLP_USERDATA, Box::into_raw(state) as isize);

        let _ = ShowWindow(window, SW_SHOW);

        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
}

/// Entry point called by the host. Reopening while already open is
/// prevented with a simple flag (this is comparison spike code, so there's
/// no strict state management to allow reopening after close).
#[no_mangle]
pub extern "C" fn open_tool() {
    if OPENING.swap(true, Ordering::SeqCst) {
        return;
    }
    std::thread::spawn(|| {
        run_window();
        OPENING.store(false, Ordering::SeqCst);
    });
}
