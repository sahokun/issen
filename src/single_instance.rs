use windows::core::w;
use windows::Win32::Foundation::{GetLastError, ERROR_ALREADY_EXISTS};
use windows::Win32::System::Threading::CreateMutexW;

/// Detects a second instance via a named mutex. Returns true if one is already
/// running.
///
/// The `HANDLE` returned by `CreateMutexW` is deliberately discarded. The `windows`
/// crate's `HANDLE` has no `Drop` impl (so `CloseHandle` is never called on it),
/// meaning it stays held until the process exits. Single-instance enforcement relies
/// on exactly that — the mutex staying open for the resident process's whole
/// lifetime — so adding a `CloseHandle` call here later because it "looks discarded"
/// would break single-instance enforcement.
pub fn is_already_running() -> bool {
    unsafe {
        let _ = CreateMutexW(None, true, w!("Local\\IssenSingleInstanceMutex"));
        GetLastError() == ERROR_ALREADY_EXISTS
    }
}
