//! Capture the next screen click as an eyedropper confirmation. The hook
//! lives only for one sampling session and consumes its down/up pair so
//! selecting a color cannot activate or click the application underneath.
use std::cell::RefCell;
use std::sync::mpsc::{channel, Receiver, Sender};

use windows::Win32::Foundation::{LPARAM, LRESULT, WPARAM};
use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, GetMessageW, PeekMessageW, PostQuitMessage, PostThreadMessageW,
    SetWindowsHookExW, UnhookWindowsHookEx, MSG, MSLLHOOKSTRUCT, PM_NOREMOVE, WH_MOUSE_LL, WM_APP,
    WM_LBUTTONDOWN, WM_LBUTTONUP,
};

const WM_STOP: u32 = WM_APP + 17;
type Sample = Option<(u8, u8, u8)>;

thread_local! {
    static STATE: RefCell<Option<HookState>> = const { RefCell::new(None) };
}

struct HookState {
    samples: Sender<Sample>,
    confirmed: bool,
    waiting_release: bool,
    stopping: bool,
}

pub(super) struct ScreenClick {
    thread_id: u32,
    samples: Receiver<Sample>,
}

impl ScreenClick {
    pub fn start() -> windows::core::Result<Self> {
        let (samples_tx, samples) = channel();
        let (ready_tx, ready_rx) = channel();
        std::thread::spawn(move || unsafe {
            let mut msg = MSG::default();
            let _ = PeekMessageW(&mut msg, None, 0, 0, PM_NOREMOVE);
            STATE.with(|state| {
                *state.borrow_mut() = Some(HookState {
                    samples: samples_tx,
                    confirmed: false,
                    waiting_release: false,
                    stopping: false,
                });
            });
            let hook = match SetWindowsHookExW(WH_MOUSE_LL, Some(mouse_hook), None, 0) {
                Ok(hook) => hook,
                Err(error) => {
                    let _ = ready_tx.send(Err(error));
                    return;
                }
            };
            if ready_tx.send(Ok(GetCurrentThreadId())).is_ok() {
                while GetMessageW(&mut msg, None, 0, 0).0 > 0 {
                    if msg.message == WM_STOP {
                        let waiting = STATE.with(|state| {
                            let mut state = state.borrow_mut();
                            let state = state.as_mut().unwrap();
                            state.stopping = true;
                            state.waiting_release
                        });
                        if !waiting {
                            break;
                        }
                    }
                }
            }
            let _ = UnhookWindowsHookEx(hook);
            STATE.with(|state| *state.borrow_mut() = None);
        });
        let thread_id = ready_rx.recv().map_err(|_| {
            windows::core::Error::new(
                windows::Win32::Foundation::E_FAIL,
                "Eyedropper hook thread stopped before initialization",
            )
        })??;
        Ok(Self { thread_id, samples })
    }

    pub fn take_sample(&self) -> Option<Sample> {
        self.samples.try_recv().ok()
    }
}

impl Drop for ScreenClick {
    fn drop(&mut self) {
        unsafe {
            let _ = PostThreadMessageW(self.thread_id, WM_STOP, WPARAM(0), LPARAM(0));
        }
    }
}

unsafe extern "system" fn mouse_hook(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code >= 0 {
        let consumed = STATE.with(|state| {
            let mut state = state.borrow_mut();
            let Some(state) = state.as_mut() else {
                return false;
            };
            match wparam.0 as u32 {
                WM_LBUTTONDOWN if !state.confirmed && !state.stopping => {
                    let event = unsafe { &*(lparam.0 as *const MSLLHOOKSTRUCT) };
                    // Sample before the target application receives the click,
                    // so a hover/activation change cannot alter the chosen pixel.
                    let sample = super::screen_pixel_color(event.pt.x, event.pt.y);
                    state.confirmed = true;
                    state.waiting_release = true;
                    let _ = state.samples.send(sample);
                    true
                }
                WM_LBUTTONUP if state.waiting_release => {
                    state.waiting_release = false;
                    unsafe { PostQuitMessage(0) };
                    true
                }
                _ => false,
            }
        });
        if consumed {
            return LRESULT(1);
        }
    }
    unsafe { CallNextHookEx(None, code, wparam, lparam) }
}
