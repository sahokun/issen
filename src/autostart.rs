use std::path::Path;

use windows::core::{w, HSTRING, PCWSTR};
use windows::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_SUCCESS, WIN32_ERROR};
use windows::Win32::System::Registry::{
    RegDeleteKeyValueW, RegSetKeyValueW, HKEY_CURRENT_USER, REG_SZ,
};

use crate::config::APP_NAME;

const RUN_KEY: PCWSTR = w!("Software\\Microsoft\\Windows\\CurrentVersion\\Run");

/// Registers (`true`) or unregisters (`false`) Issen under the per-user
/// `HKCU\...\CurrentVersion\Run` key, value name [`APP_NAME`].
///
/// HKCU Run needs no elevation and no installer, which suits a zip-distributed
/// app. Because the exe can be moved freely, `main.rs` calls this on every
/// (release) startup as well as on settings save, so the stored path always
/// tracks the exe's current location.
///
/// Task Manager's "Disable" on the Startup tab is recorded separately under
/// `Explorer\StartupApproved\Run` and is deliberately left untouched here: if
/// the user disabled Issen there, that choice still wins over this value.
pub fn apply(enabled: bool) -> std::io::Result<()> {
    let name = HSTRING::from(APP_NAME);
    let result = if enabled {
        let exe = std::env::current_exe()?;
        let data: Vec<u16> = run_value(&exe)
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect();
        unsafe {
            RegSetKeyValueW(
                HKEY_CURRENT_USER,
                RUN_KEY,
                &name,
                REG_SZ.0,
                Some(data.as_ptr().cast()),
                (data.len() * std::mem::size_of::<u16>()) as u32,
            )
        }
    } else {
        match unsafe { RegDeleteKeyValueW(HKEY_CURRENT_USER, RUN_KEY, &name) } {
            ERROR_FILE_NOT_FOUND => ERROR_SUCCESS,
            other => other,
        }
    };
    to_io_result(result)
}

/// The Run value's command line: the exe path, quoted so paths containing
/// spaces aren't split into a program and arguments.
fn run_value(exe: &Path) -> String {
    format!("\"{}\"", exe.display())
}

fn to_io_result(err: WIN32_ERROR) -> std::io::Result<()> {
    if err == ERROR_SUCCESS {
        Ok(())
    } else {
        Err(std::io::Error::from_raw_os_error(err.0 as i32))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn run_value_quotes_path_with_spaces() {
        let exe = Path::new(r"C:\Program Files\Issen\issen.exe");
        assert_eq!(run_value(exe), r#""C:\Program Files\Issen\issen.exe""#);
    }
}
