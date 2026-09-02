use std::path::Path;

use windows::core::{HSTRING, PCWSTR};
use windows::Win32::UI::Shell::ShellExecuteW;
use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

/// Opens a file path, `.lnk`, `ms-settings:` URI, or similar, uniformly.
/// The Windows shell handles link resolution and URI scheme dispatch, so the app
/// doesn't need to branch on target type itself.
pub fn open(target: &str) -> bool {
    shell_execute(None, target, None)
}

/// Runs a command alias's target with its arguments. An empty `args` behaves the same
/// as no arguments.
pub fn open_with_args(target: &str, args: &str) -> bool {
    shell_execute(None, target, non_empty(args))
}

/// Runs as administrator (the `runas` verb, which triggers a UAC elevation prompt).
pub fn open_elevated(target: &str, args: &str) -> bool {
    shell_execute(Some("runas"), target, non_empty(args))
}

fn non_empty(s: &str) -> Option<&str> {
    (!s.is_empty()).then_some(s)
}

/// Opens the target's containing folder in Explorer with it selected.
pub fn open_containing_folder(path: &Path) -> bool {
    let arg = format!("/select,\"{}\"", path.display());
    shell_execute(None, "explorer.exe", Some(&arg))
}

/// Copies the path to the clipboard. Failure isn't fatal — the caller can check the
/// return value, but it's fine to ignore it too.
pub fn copy_to_clipboard(text: &str) -> bool {
    arboard::Clipboard::new()
        .and_then(|mut clipboard| clipboard.set_text(text))
        .is_ok()
}

fn shell_execute(operation: Option<&str>, file: &str, params: Option<&str>) -> bool {
    let operation = operation.map(HSTRING::from);
    let file = HSTRING::from(file);
    let params = params.map(HSTRING::from);

    let hinstance = unsafe {
        ShellExecuteW(
            None,
            operation
                .as_ref()
                .map_or(PCWSTR::null(), |s| PCWSTR(s.as_ptr())),
            &file,
            params
                .as_ref()
                .map_or(PCWSTR::null(), |s| PCWSTR(s.as_ptr())),
            PCWSTR::null(),
            SW_SHOWNORMAL,
        )
    };
    // ShellExecuteW's return type is HINSTANCE, but the value is really a status
    // code — greater than 32 means success.
    hinstance.0 as usize > 32
}
