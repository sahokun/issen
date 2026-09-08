use std::ffi::CStr;
use std::sync::OnceLock;

use windows::core::{BOOL, HSTRING, PCSTR, PCWSTR};
use windows::Win32::Foundation::{HMODULE, TRUE};
use windows::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};

use super::fuzzy::fuzzy_match;
use super::{Action, SearchProvider, SearchResult};

/// Everything (voidtools) integration. Loads `Everything64.dll` (the SDK
/// client DLL; MIT-licensed and redistributable, bundled as
/// `third_party/Everything64.dll` — see `licenses/Everything-SDK.txt`) at
/// runtime and delegates search to a running `Everything.exe` through its
/// exported functions (`Everything_SetSearchW`, etc.). The IPC itself is
/// entirely internal to `Everything64.dll` (it sends/receives messages to a
/// hidden window under the hood); callers only need to call the DLL's
/// exported functions.
///
/// Bundling the DLL means "can the DLL load" no longer tells you whether
/// Everything integration actually works (it's always true). So this
/// always confirms reachability with a real `Everything_GetMajorVersion`
/// IPC round-trip instead (`is_running`), including the case where
/// Everything isn't running at all. When Everything runs as a Windows
/// service, the service runs in session 0 and can't respond to the
/// interactive session's window-message IPC (a Windows session-isolation
/// constraint, not a bug here) — `is_running` also returns false in that
/// case, and the feature is silently disabled. `Everything_GetMajorVersion`
/// returns an error in a few ms when unreachable, while
/// `Everything_QueryW` takes roughly 1.2s to time out — noticeable as
/// input lag if called on every keystroke — so `search()` always probes
/// with the lightweight `is_running` first before sending the real query.
pub struct EverythingProvider;

type SetSearchW = unsafe extern "system" fn(PCWSTR);
type SetMatchPath = unsafe extern "system" fn(BOOL);
type SetMax = unsafe extern "system" fn(u32);
type QueryW = unsafe extern "system" fn(BOOL) -> BOOL;
type GetLastError = unsafe extern "system" fn() -> u32;
type GetMajorVersion = unsafe extern "system" fn() -> u32;
type GetNumResults = unsafe extern "system" fn() -> u32;
type GetResultFullPathNameW = unsafe extern "system" fn(u32, *mut u16, u32) -> u32;

struct Api {
    set_search_w: SetSearchW,
    set_match_path: SetMatchPath,
    set_max: SetMax,
    query_w: QueryW,
    get_last_error: GetLastError,
    get_major_version: GetMajorVersion,
    get_num_results: GetNumResults,
    get_result_full_path_name_w: GetResultFullPathNameW,
}

/// The loaded function pointers are reused for the process's whole
/// lifetime. `Everything64.dll` is deliberately never `FreeLibrary`'d
/// (avoids paying load/unload cost on every search; function pointers are
/// plain address values that are `Send`/`Sync`, and since the `HMODULE`
/// itself isn't held onto, this can sit directly in a `OnceLock`).
static API: OnceLock<Option<Api>> = OnceLock::new();

/// Max results to request from Everything. Unbounded, a 1-2 character
/// query could match tens of thousands of files, and building a full-path
/// string for each on every keystroke isn't free — this caps that cost.
/// (The final displayed count is further limited by `config.max_results`,
/// so this only needs to keep enough candidates for fair score comparison
/// against other providers.)
const QUERY_MAX_RESULTS: u32 = 200;
/// Buffer size (in wchars) for retrieving a full path. Fixed-length, with
/// enough headroom for ordinary file paths (Windows' extended path length
/// limit is 32767 characters, but nothing realistically indexed here
/// approaches that).
const PATH_BUFFER_LEN: usize = 4096;

impl SearchProvider for EverythingProvider {
    fn search(&self, query: &str) -> Vec<SearchResult> {
        if query.is_empty() {
            return Vec::new();
        }

        let Some(api) = API.get_or_init(load_api) else {
            return Vec::new();
        };

        unsafe { run_query(api, query) }
    }
}

unsafe fn run_query(api: &Api, query: &str) -> Vec<SearchResult> {
    if !is_running(api) {
        return Vec::new();
    }

    let wide: Vec<u16> = query.encode_utf16().chain(std::iter::once(0)).collect();
    (api.set_search_w)(PCWSTR(wide.as_ptr()));
    (api.set_match_path)(BOOL(0));
    (api.set_max)(QUERY_MAX_RESULTS);

    if !(api.query_w)(TRUE).as_bool() {
        eprintln!(
            "issen: Everything query failed (error code {})",
            (api.get_last_error)()
        );
        return Vec::new();
    }

    let num_results = (api.get_num_results)();
    let mut results = Vec::with_capacity(num_results as usize);
    let mut buf = vec![0u16; PATH_BUFFER_LEN];

    for index in 0..num_results {
        let len =
            (api.get_result_full_path_name_w)(index, buf.as_mut_ptr(), buf.len() as u32) as usize;
        let full_path = String::from_utf16_lossy(&buf[..len.min(buf.len())]);
        if full_path.is_empty() {
            continue;
        }

        let path = std::path::PathBuf::from(&full_path);
        let title = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or(&full_path)
            .to_string();

        if let Some(score) = fuzzy_match(query, &title) {
            results.push(SearchResult {
                title,
                subtitle: full_path,
                action: Action::Launch {
                    path,
                    args: String::new(),
                },
                score,
            });
        }
    }

    results
}

/// Calls `Everything_GetMajorVersion` once and checks reachability via
/// `Everything_GetLastError`. The version number itself isn't used — this
/// is purely a lightweight liveness check (the settings UI only shows
/// whether Everything is detected, not its version).
unsafe fn is_running(api: &Api) -> bool {
    (api.get_major_version)();
    (api.get_last_error)() == 0
}

fn load_api() -> Option<Api> {
    unsafe {
        let name = HSTRING::from("Everything64.dll");
        let module = LoadLibraryW(&name).ok()?;
        Some(Api {
            set_search_w: proc(module, c"Everything_SetSearchW")?,
            set_match_path: proc(module, c"Everything_SetMatchPath")?,
            set_max: proc(module, c"Everything_SetMax")?,
            query_w: proc(module, c"Everything_QueryW")?,
            get_last_error: proc(module, c"Everything_GetLastError")?,
            get_major_version: proc(module, c"Everything_GetMajorVersion")?,
            get_num_results: proc(module, c"Everything_GetNumResults")?,
            get_result_full_path_name_w: proc(module, c"Everything_GetResultFullPathNameW")?,
        })
    }
}

/// Casts an address from `GetProcAddress` to the target function pointer
/// type. Function pointers are always exactly one pointer wide (as long as
/// the calling convention matches), so this transmute is valid even
/// between function pointer types with different signatures.
unsafe fn proc<F: Copy>(module: HMODULE, name: &CStr) -> Option<F> {
    let addr = GetProcAddress(module, PCSTR(name.as_ptr().cast()))?;
    debug_assert_eq!(
        std::mem::size_of_val(&addr),
        std::mem::size_of::<F>(),
        "function pointer size mismatch"
    );
    Some(std::mem::transmute_copy(&addr))
}

/// Checks whether Everything is usable right now. Everything integration
/// is a soft dependency — the app works fine without it — so this is used
/// to hide the Everything settings section entirely when it isn't
/// available. Since the DLL is bundled with the app, "can the DLL load" is
/// no longer a meaningful check on its own; this confirms actual IPC
/// reachability with the running Everything instance. Called each time the
/// settings window opens rather than polled continuously, so the few-ms
/// IPC round-trip cost is acceptable.
pub fn is_available() -> bool {
    match API.get_or_init(load_api) {
        Some(api) => unsafe { is_running(api) },
        None => false,
    }
}
