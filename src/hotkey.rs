use std::sync::mpsc::{channel, Receiver, Sender};
use std::thread;

use windows::Win32::Foundation::{LPARAM, WPARAM};
use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    RegisterHotKey, UnregisterHotKey, HOT_KEY_MODIFIERS, MOD_ALT, MOD_CONTROL, MOD_NOREPEAT,
    MOD_SHIFT, MOD_WIN,
};
use windows::Win32::UI::WindowsAndMessaging::{
    GetMessageW, PeekMessageW, PostThreadMessageW, MSG, PM_NOREMOVE, WM_APP, WM_HOTKEY,
};

const HOTKEY_ID: i32 = 1;
const VK_SPACE: u32 = 0x20;
/// Cross-thread message used for live hotkey updates (the new string is
/// pushed onto `update_tx`'s channel first, and this unblocks `GetMessageW`
/// so the listener thread goes and picks it up).
const WM_UPDATE_HOTKEY: u32 = WM_APP + 1;

pub struct HotkeyListener {
    receiver: Receiver<()>,
    /// The thread that update messages get posted to. Sent back by
    /// `spawn`'s thread once it starts (`GetCurrentThreadId()` can only be
    /// read from within the thread itself). `update_hotkey` is never
    /// realistically called before that happens, but `0` is treated as
    /// "not yet received" and ignored just in case.
    thread_id: u32,
    update_tx: Sender<String>,
}

impl HotkeyListener {
    /// Runs its own message loop on a background thread, listening for the
    /// global hotkey. On a hotkey press, it both notifies via the channel
    /// and calls `ctx.request_repaint()` to wake egui's event loop (a
    /// hidden window receives no OS input events, so without this,
    /// `update()` never gets called).
    ///
    /// `hotkey_spec` is `config.hotkey` (e.g. `"Alt+Space"`). If it fails
    /// to parse, this falls back to the default `Alt+Space` — but only for
    /// this initial registration. A live update via `update_hotkey` that
    /// fails to parse instead just keeps the current registration as-is,
    /// because the settings window's hotkey field applies on every
    /// keystroke; if invalid partial input like `"C"` → `"Ct"` → `"Ctrl+"`
    /// fell back to Alt+Space each time, the live global hotkey would keep
    /// changing out from under the user while they're still typing.
    pub fn spawn(ctx: egui::Context, hotkey_spec: String) -> Self {
        let (toggle_tx, toggle_rx) = channel();
        let (update_tx, update_rx) = channel::<String>();
        let (thread_id_tx, thread_id_rx) = channel();

        thread::spawn(move || unsafe {
            let thread_id = GetCurrentThreadId();
            // A thread's message queue is lazily created on its first message-related
            // API call. Call `PeekMessageW` once before sending `thread_id` back to the
            // caller, to guarantee the queue exists first — otherwise, if `update_hotkey`
            // is called right after `spawn` returns, `PostThreadMessageW` could target a
            // queue that doesn't exist yet and fail.
            let mut msg = MSG::default();
            let _ = PeekMessageW(&mut msg, None, 0, 0, PM_NOREMOVE);
            let _ = thread_id_tx.send(thread_id);

            let (modifiers, vk) = parse_hotkey(&hotkey_spec).unwrap_or_else(|| {
                eprintln!(
                    "issen: invalid hotkey {hotkey_spec:?} in config.toml, falling back to Alt+Space"
                );
                (MOD_ALT, VK_SPACE)
            });
            let mut registered =
                RegisterHotKey(None, HOTKEY_ID, modifiers | MOD_NOREPEAT, vk).is_ok();
            if !registered {
                eprintln!("issen: failed to register hotkey {hotkey_spec:?}");
            }

            loop {
                let mut msg = MSG::default();
                if GetMessageW(&mut msg, None, 0, 0).0 == 0 {
                    break;
                }
                match msg.message {
                    WM_HOTKEY => {
                        let _ = toggle_tx.send(());
                        ctx.request_repaint();
                    }
                    WM_UPDATE_HOTKEY => {
                        let Ok(new_spec) = update_rx.try_recv() else {
                            continue;
                        };
                        // Only re-register if it actually parses (see the doc comment
                        // above); otherwise leave the current registration as-is.
                        if let Some((modifiers, vk)) = parse_hotkey(&new_spec) {
                            if registered {
                                let _ = UnregisterHotKey(None, HOTKEY_ID);
                            }
                            registered =
                                RegisterHotKey(None, HOTKEY_ID, modifiers | MOD_NOREPEAT, vk)
                                    .is_ok();
                            if !registered {
                                eprintln!(
                                    "issen: failed to re-register hotkey {new_spec:?}; hotkey is now unregistered until a valid one is set"
                                );
                            }
                        }
                    }
                    _ => {}
                }
            }
        });

        let thread_id = thread_id_rx.recv().unwrap_or(0);
        Self {
            receiver: toggle_rx,
            thread_id,
            update_tx,
        }
    }

    pub fn try_recv_toggle(&self) -> bool {
        self.receiver.try_recv().is_ok()
    }

    /// Re-registers the hotkey while running (reflects a settings-window
    /// change immediately). If `new_spec` doesn't currently parse (e.g. a
    /// partial in-progress string), this doesn't error — the listener
    /// thread checks it and ignores it (see `spawn`'s doc comment).
    pub fn update_hotkey(&self, new_spec: String) {
        if self.thread_id == 0 {
            return;
        }
        if self.update_tx.send(new_spec).is_err() {
            return;
        }
        unsafe {
            if let Err(err) =
                PostThreadMessageW(self.thread_id, WM_UPDATE_HOTKEY, WPARAM(0), LPARAM(0))
            {
                eprintln!("issen: failed to notify hotkey thread of update: {err}");
            }
        }
    }
}

/// Parses a string like `"Ctrl+Shift+K"`. Modifier keys (Ctrl/Alt/Shift/Win)
/// can appear in any order and any combination; the last `+`-separated part
/// is the base key. Case-insensitive.
fn parse_hotkey(spec: &str) -> Option<(HOT_KEY_MODIFIERS, u32)> {
    let parts: Vec<&str> = spec
        .split('+')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .collect();
    let (key_part, mod_parts) = parts.split_last()?;

    let mut modifiers = HOT_KEY_MODIFIERS(0);
    for m in mod_parts {
        modifiers |= match m.to_ascii_lowercase().as_str() {
            "ctrl" | "control" => MOD_CONTROL,
            "alt" => MOD_ALT,
            "shift" => MOD_SHIFT,
            "win" | "windows" => MOD_WIN,
            _ => return None,
        };
    }

    parse_key(key_part).map(|vk| (modifiers, vk))
}

fn parse_key(key: &str) -> Option<u32> {
    if key.chars().count() == 1 {
        let c = key.chars().next()?.to_ascii_uppercase();
        if c.is_ascii_alphanumeric() {
            return Some(c as u32);
        }
    }

    match key.to_ascii_lowercase().as_str() {
        "space" => return Some(0x20),
        "tab" => return Some(0x09),
        "enter" | "return" => return Some(0x0D),
        "escape" | "esc" => return Some(0x1B),
        "backspace" => return Some(0x08),
        _ => {}
    }

    let upper = key.to_ascii_uppercase();
    if let Some(n) = upper.strip_prefix('F') {
        if let Ok(n) = n.parse::<u32>() {
            if (1..=24).contains(&n) {
                return Some(0x70 + (n - 1));
            }
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_default_hotkey() {
        let (modifiers, vk) = parse_hotkey("Alt+Space").unwrap();
        assert_eq!(modifiers, MOD_ALT);
        assert_eq!(vk, VK_SPACE);
    }

    #[test]
    fn parses_multiple_modifiers_case_insensitively() {
        let (modifiers, vk) = parse_hotkey("ctrl+SHIFT+k").unwrap();
        assert_eq!(modifiers, MOD_CONTROL | MOD_SHIFT);
        assert_eq!(vk, 'K' as u32);
    }

    #[test]
    fn parses_function_keys() {
        let (_, vk) = parse_hotkey("Ctrl+F12").unwrap();
        assert_eq!(vk, 0x7B);
    }

    #[test]
    fn rejects_unknown_key() {
        assert!(parse_hotkey("Ctrl+NotAKey").is_none());
    }

    #[test]
    fn rejects_empty_string() {
        assert!(parse_hotkey("").is_none());
    }

    #[test]
    fn rejects_modifier_only() {
        assert!(parse_hotkey("Ctrl+").is_none());
    }
}
