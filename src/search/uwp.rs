use windows::core::{Interface, PCWSTR, PWSTR};
use windows::Win32::Storage::EnhancedStorage::PKEY_AppUserModel_ID;
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CoTaskMemFree, CoUninitialize, CLSCTX_INPROC_SERVER,
    COINIT_APARTMENTTHREADED,
};
use windows::Win32::UI::Shell::{
    ApplicationActivationManager, BHID_EnumItems, FOLDERID_AppsFolder,
    IApplicationActivationManager, IEnumShellItems, IShellItem, IShellItem2,
    SHCreateItemInKnownFolder, AO_NONE, KF_FLAG_DEFAULT, SIGDN_NORMALDISPLAY,
};

pub struct UwpApp {
    pub title: String,
    pub aumid: String,
}

/// Enumerates `shell:AppsFolder` and collects the display name and AppUserModelID of
/// each UWP/Store app. COM requires `CoInitializeEx` per thread, so this runs on
/// whatever thread calls it (expected to be the background scan thread).
pub fn scan() -> Vec<UwpApp> {
    let mut apps = Vec::new();
    unsafe {
        let com = ComGuard::init();
        if com.is_ok() {
            if let Err(err) = scan_inner(&mut apps) {
                eprintln!("issen: UWP app scan failed: {err}");
            }
        }
    }
    apps
}

/// Launches a UWP/Store app from its `aumid` (AppUserModelID).
pub fn launch(aumid: &str) -> bool {
    unsafe {
        let com = ComGuard::init();
        if !com.is_ok() {
            return false;
        }
        match launch_inner(aumid) {
            Ok(()) => true,
            Err(err) => {
                eprintln!("issen: UWP app launch failed: {err}");
                false
            }
        }
    }
}

unsafe fn scan_inner(apps: &mut Vec<UwpApp>) -> windows::core::Result<()> {
    let folder: IShellItem =
        SHCreateItemInKnownFolder(&FOLDERID_AppsFolder, KF_FLAG_DEFAULT, PCWSTR::null())?;
    let items: IEnumShellItems = folder.BindToHandler(None, &BHID_EnumItems)?;

    loop {
        let mut buf = [None::<IShellItem>; 1];
        let fetched = items.Next(&mut buf, None);
        let Some(item) = buf[0].take() else {
            break;
        };
        if fetched.is_err() {
            break;
        }

        let title = item
            .GetDisplayName(SIGDN_NORMALDISPLAY)
            .ok()
            .map(|p| unsafe { pwstr_to_string_and_free(p) })
            .unwrap_or_default();

        if let Ok(item2) = item.cast::<IShellItem2>() {
            if let Ok(aumid_pwstr) = item2.GetString(&PKEY_AppUserModel_ID) {
                let aumid = pwstr_to_string_and_free(aumid_pwstr);
                // `shell:AppsFolder` mixes actual UWP/MSIX packaged apps with ordinary
                // desktop apps that only registered an AppUserModelID for the taskbar
                // (e.g. Chrome, Discord). A packaged app's AUMID is always shaped
                // `PackageFamilyName!AppId` (contains "!"), while a desktop app's
                // self-registered ID never does — so that shape is what distinguishes
                // them. (`windows-rs` 0.62 has no typed
                // `PKEY_AppUserModel_PackageFamilyName`, and hand-rolling that
                // `PROPERTYKEY` risks getting the value wrong, so this shape check is
                // used instead of reading the family name property directly.)
                if is_packaged_aumid(&aumid) && !title.is_empty() {
                    apps.push(UwpApp { title, aumid });
                }
            }
        }
    }

    Ok(())
}

/// A packaged app's AUMID is shaped `PackageFamilyName!RelativeAppId`, containing
/// exactly one `!` (a spec-level constraint). A desktop app's self-registered AUMID is
/// a free-form string that in practice never contains "!".
fn is_packaged_aumid(aumid: &str) -> bool {
    !aumid.is_empty() && aumid.matches('!').count() == 1
}

unsafe fn launch_inner(aumid: &str) -> windows::core::Result<()> {
    let mgr: IApplicationActivationManager =
        CoCreateInstance(&ApplicationActivationManager, None, CLSCTX_INPROC_SERVER)?;
    let aumid_wide: Vec<u16> = aumid.encode_utf16().chain(std::iter::once(0)).collect();
    mgr.ActivateApplication(PCWSTR(aumid_wide.as_ptr()), PCWSTR::null(), AO_NONE)?;
    Ok(())
}

unsafe fn pwstr_to_string_and_free(pwstr: PWSTR) -> String {
    // `PWSTR::to_string()` walks the pointer internally (like `wcslen`), so check for
    // null first — calling it on a null pointer is undefined behavior. Some
    // `shell:AppsFolder` items really do return empty/null string properties.
    if pwstr.is_null() {
        return String::new();
    }
    let s = pwstr.to_string().unwrap_or_default();
    CoTaskMemFree(Some(pwstr.0 as *const _));
    s
}

/// RAII wrapper for per-thread COM initialization. Every successful
/// `CoInitializeEx` (S_OK/S_FALSE = already initialized) needs a matching
/// `CoUninitialize`, so `Drop` only calls it when initialization succeeded.
struct ComGuard {
    initialized: bool,
}

impl ComGuard {
    unsafe fn init() -> Self {
        let hr = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        Self {
            initialized: hr.is_ok(),
        }
    }

    fn is_ok(&self) -> bool {
        self.initialized
    }
}

impl Drop for ComGuard {
    fn drop(&mut self) {
        if self.initialized {
            unsafe { CoUninitialize() };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn packaged_aumid_has_exactly_one_bang() {
        assert!(is_packaged_aumid(
            "Microsoft.WindowsCalculator_8wekyb3d8bbwe!App"
        ));
        assert!(!is_packaged_aumid("Docker.DockerForWindows.Settings"));
        assert!(!is_packaged_aumid("https://gitforwindows.org/faq"));
        assert!(!is_packaged_aumid(""));
    }
}
