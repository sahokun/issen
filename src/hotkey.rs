use std::sync::mpsc::{channel, Sender};
use std::thread;

use futures::channel::mpsc::{unbounded, UnboundedReceiver};
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
/// Cross-thread message used for live hotkey updates (the command is
/// pushed onto `update_tx`'s channel first, and this unblocks `GetMessageW`
/// so the listener thread goes and picks it up).
const WM_UPDATE_HOTKEY: u32 = WM_APP + 1;

pub struct HotkeyListener {
    /// The thread that update messages get posted to. Sent back by
    /// `spawn`'s thread once it starts (`GetCurrentThreadId()` can only be
    /// read from within the thread itself). `update_hotkey` is never
    /// realistically called before that happens, but `0` is treated as
    /// "not yet received" and ignored just in case.
    thread_id: u32,
    update_tx: Sender<HotkeyCommand>,
}

enum HotkeyCommand {
    Set(String),
    Suspend(bool),
}

impl HotkeyListener {
    /// Runs its own message loop on a background thread, listening for the
    /// global hotkey. On a hotkey press, it notifies via the returned
    /// `UnboundedReceiver` — GPUI's `AsyncApp` is `!Send` and can't be
    /// touched from this background thread directly, so the caller is
    /// expected to `.await` the receiver from a `cx.spawn` foreground task
    /// and wake the window from there (see `app.rs`).
    ///
    /// `hotkey_spec` is `config.hotkey` (e.g. `"Alt+Space"`). If it fails
    /// to parse, this falls back to the default `Alt+Space` — but only for
    /// this initial registration. Live updates that fail to parse keep the
    /// current registration. The settings recorder suspends registration
    /// while capturing, then resumes it on commit, cancellation, or blur.
    pub fn spawn(hotkey_spec: String) -> (Self, UnboundedReceiver<()>) {
        let (toggle_tx, toggle_rx) = unbounded();
        let (update_tx, update_rx) = channel::<HotkeyCommand>();
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
            let mut current = (modifiers, vk);
            let mut suspended = false;

            loop {
                let mut msg = MSG::default();
                if GetMessageW(&mut msg, None, 0, 0).0 == 0 {
                    break;
                }
                match msg.message {
                    WM_HOTKEY => {
                        let _ = toggle_tx.unbounded_send(());
                    }
                    WM_UPDATE_HOTKEY => {
                        let Ok(command) = update_rx.try_recv() else {
                            continue;
                        };
                        match command {
                            HotkeyCommand::Set(spec) => {
                                let Some(parsed) = parse_hotkey(&spec) else {
                                    continue;
                                };
                                current = parsed;
                            }
                            HotkeyCommand::Suspend(value) => suspended = value,
                        }
                        if registered {
                            let _ = UnregisterHotKey(None, HOTKEY_ID);
                        }
                        registered = !suspended
                            && RegisterHotKey(None, HOTKEY_ID, current.0 | MOD_NOREPEAT, current.1)
                                .is_ok();
                        if !suspended && !registered {
                            eprintln!("issen: failed to register hotkey");
                        }
                    }
                    _ => {}
                }
            }
        });

        let thread_id = thread_id_rx.recv().unwrap_or(0);
        let listener = Self {
            thread_id,
            update_tx,
        };
        (listener, toggle_rx)
    }

    /// Re-registers the hotkey while running (reflects a settings-window
    /// change immediately). If `new_spec` doesn't currently parse (e.g. a
    /// unsupported key specification), this doesn't error — the listener
    /// thread checks it and ignores it (see `spawn`'s doc comment).
    pub fn update_hotkey(&self, new_spec: String) {
        self.send(HotkeyCommand::Set(new_spec));
    }

    /// Let the settings recorder receive even the currently registered chord.
    pub fn suspend(&self, suspended: bool) {
        self.send(HotkeyCommand::Suspend(suspended));
    }

    fn send(&self, command: HotkeyCommand) {
        if self.thread_id == 0 {
            return;
        }
        if self.update_tx.send(command).is_err() {
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

/// Canonical config syntax from a physical key chord, never from typed text.
/// Ignore modifiers alone and keys our Win32 registration cannot represent.
pub fn recorded_hotkey(key: &gpui::Keystroke) -> Option<String> {
    let vk = parse_key(&key.key)?;
    let base = match vk {
        0x20 => "Space".into(),
        0x09 => "Tab".into(),
        0x0D => "Enter".into(),
        0x1B => "Escape".into(),
        0x08 => "Backspace".into(),
        0x21 => "PageUp".into(),
        0x22 => "PageDown".into(),
        0x23 => "End".into(),
        0x24 => "Home".into(),
        0x25 => "Left".into(),
        0x26 => "Up".into(),
        0x27 => "Right".into(),
        0x28 => "Down".into(),
        0x2D => "Insert".into(),
        0x2E => "Delete".into(),
        0x70..=0x87 => format!("F{}", vk - 0x70 + 1),
        _ => char::from_u32(vk)?.to_string(),
    };
    let mut parts = Vec::new();
    if key.modifiers.control {
        parts.push("Ctrl");
    }
    if key.modifiers.alt {
        parts.push("Alt");
    }
    if key.modifiers.shift {
        parts.push("Shift");
    }
    if key.modifiers.platform {
        parts.push("Win");
    }
    parts.push(&base);
    Some(parts.join("+"))
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
        "pageup" => return Some(0x21),
        "pagedown" => return Some(0x22),
        "end" => return Some(0x23),
        "home" => return Some(0x24),
        "left" => return Some(0x25),
        "up" => return Some(0x26),
        "right" => return Some(0x27),
        "down" => return Some(0x28),
        "insert" => return Some(0x2D),
        "delete" => return Some(0x2E),
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
    fn captured_chords_roundtrip_to_win32_keys() {
        for (key, name, vk) in [
            ("space", "Space", 0x20),
            ("k", "K", 0x4B),
            ("f12", "F12", 0x7B),
            ("left", "Left", 0x25),
            ("delete", "Delete", 0x2E),
        ] {
            let stroke = gpui::Keystroke {
                modifiers: gpui::Modifiers {
                    control: true,
                    shift: true,
                    ..Default::default()
                },
                key: key.into(),
                key_char: None,
            };
            let spec = recorded_hotkey(&stroke).unwrap();
            assert_eq!(spec, format!("Ctrl+Shift+{name}"));
            assert_eq!(parse_hotkey(&spec), Some((MOD_CONTROL | MOD_SHIFT, vk)));
        }
    }

    #[test]
    fn capture_ignores_modifiers_and_unsupported_keys() {
        for key in ["control", "alt", "shift", "platform", "+", "あ"] {
            assert!(recorded_hotkey(&gpui::Keystroke {
                modifiers: gpui::Modifiers::default(),
                key: key.into(),
                key_char: None,
            })
            .is_none());
        }
    }

    #[test]
    fn capture_preserves_alt_and_windows_modifiers() {
        let spec = recorded_hotkey(&gpui::Keystroke {
            modifiers: gpui::Modifiers {
                alt: true,
                platform: true,
                ..Default::default()
            },
            key: "enter".into(),
            key_char: None,
        })
        .unwrap();
        assert_eq!(spec, "Alt+Win+Enter");
        assert_eq!(parse_hotkey(&spec), Some((MOD_ALT | MOD_WIN, 0x0D)));
    }

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
