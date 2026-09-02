use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Receiver};
use std::thread;

use regex::Regex;

use super::fuzzy::fuzzy_match;
use super::uwp;
use super::{Action, SearchProvider, SearchResult};
use crate::config::Config;

enum Launch {
    Path(PathBuf),
    Uwp { aumid: String },
}

/// Where an entry came from, used to weight result priority (`search()`).
/// Executables in `C:\Windows\System32` and on `PATH` tend to produce a lot
/// of noisy matches on name alone, so they're weighted below Start Menu,
/// custom folders, and UWP.
#[derive(Clone, Copy, PartialEq, Eq)]
enum AppSource {
    StartMenu,
    CustomFolder,
    SystemOrPath,
    Uwp,
}

struct IndexedApp {
    title: String,
    subtitle: String,
    launch: Launch,
    source: AppSource,
}

/// Indexes Start Menu shortcuts (`.lnk`), executables in System32 and on
/// `PATH`, user-added custom folders, and UWP/Store apps
/// (`shell:AppsFolder`).
/// `.lnk` targets aren't resolved (no COM/`IShellLink`) — the shortcut's
/// own file path is passed straight to `ShellExecuteW`, which lets the
/// shell handle resolution, so COM isn't needed just to launch it.
pub struct AppIndexProvider {
    entries: Vec<IndexedApp>,
}

/// A snapshot of the config fields the background scan needs, rather than
/// the whole `Config` — `Config` keeps getting edited on the UI thread
/// while a scan is running.
#[derive(Clone)]
pub struct ScanConfig {
    pub extra_folders: Vec<PathBuf>,
    pub exclude_patterns: Vec<String>,
}

impl ScanConfig {
    pub fn from_config(config: &Config) -> Self {
        Self {
            extra_folders: config.index_folders.iter().map(PathBuf::from).collect(),
            exclude_patterns: config.exclude_patterns.clone(),
        }
    }
}

impl AppIndexProvider {
    pub fn empty() -> Self {
        Self {
            entries: Vec::new(),
        }
    }

    pub fn scan(scan_config: &ScanConfig) -> Self {
        let patterns = compile_patterns(&scan_config.exclude_patterns);
        let mut entries = Vec::new();

        for dir in start_menu_dirs() {
            collect_shortcuts(&dir, &patterns, &mut entries, 0, AppSource::StartMenu);
        }

        // System32/PATH folders never recurse into subfolders — recursively
        // walking all of System32 would be slow and noisy.
        for dir in path_and_system_dirs() {
            collect_executables(&dir, &patterns, &mut entries, 0, 0, AppSource::SystemOrPath);
        }

        // User-added custom folders are treated like a mini Start Menu:
        // both .lnk and .exe recurse into subfolders, capped at
        // MAX_SCAN_DEPTH so a directory-junction cycle can't cause a stack
        // overflow.
        for dir in &scan_config.extra_folders {
            collect_shortcuts(dir, &patterns, &mut entries, 0, AppSource::CustomFolder);
            collect_executables(
                dir,
                &patterns,
                &mut entries,
                0,
                MAX_SCAN_DEPTH,
                AppSource::CustomFolder,
            );
        }

        for app in uwp::scan() {
            if is_excluded(&app.title, &patterns) {
                continue;
            }
            entries.push(IndexedApp {
                title: app.title,
                subtitle: "UWP".to_string(),
                launch: Launch::Uwp { aumid: app.aumid },
                source: AppSource::Uwp,
            });
        }

        Self { entries }
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }
}

/// Fixed penalty subtracted from the score of `AppSource::SystemOrPath`
/// entries (executables in `C:\Windows\System32` / on `PATH`), which are
/// weighted below Start Menu, custom folders, and UWP since they tend to
/// match on name alone very easily. Set well above `fuzzy_match`'s max
/// per-character score (14, for a boundary + consecutive match), so
/// whenever another source also matches, it always ranks above a
/// System32/PATH match — System32/PATH entries only surface (still ranked
/// relative to each other) when nothing else matches at all.
const LOW_PRIORITY_PENALTY: i32 = 1000;

impl SearchProvider for AppIndexProvider {
    fn search(&self, query: &str) -> Vec<SearchResult> {
        if query.is_empty() {
            return Vec::new();
        }

        self.entries
            .iter()
            .filter_map(|app| {
                fuzzy_match(query, &app.title).map(|score| {
                    let score = if app.source == AppSource::SystemOrPath {
                        score - LOW_PRIORITY_PENALTY
                    } else {
                        score
                    };
                    SearchResult {
                        title: app.title.clone(),
                        subtitle: app.subtitle.clone(),
                        action: match &app.launch {
                            Launch::Path(path) => Action::Launch {
                                path: path.clone(),
                                args: String::new(),
                            },
                            Launch::Uwp { aumid } => Action::LaunchUwp {
                                aumid: aumid.clone(),
                            },
                        },
                        score,
                    }
                })
            })
            .collect()
    }
}

/// Runs `AppIndexProvider::scan` on a background thread and returns the
/// result over a channel once it's done, so the UI thread never blocks on
/// it.
pub struct IndexScan {
    receiver: Receiver<AppIndexProvider>,
}

impl IndexScan {
    /// Returns `None` if the thread fails to spawn (a rare case, e.g.
    /// resource exhaustion). This runs on every periodic rescan and manual
    /// reindex, so a failure here just skips that scan rather than
    /// panicking and taking the whole app down.
    pub fn spawn(scan_config: ScanConfig) -> Option<Self> {
        let (sender, receiver) = channel();

        let spawned = thread::Builder::new()
            .name("issen-index-scan".to_string())
            .spawn(move || {
                // Lower the scan thread's priority so it interferes with UI
                // work as little as possible. This is an approximation
                // (there's no true idle detection via GetSystemTimes or
                // similar) of scanning once things have settled down.
                lower_thread_priority();
                let provider = AppIndexProvider::scan(&scan_config);
                let _ = sender.send(provider);
            });

        match spawned {
            Ok(_) => Some(Self { receiver }),
            Err(err) => {
                eprintln!("issen: failed to spawn index scan thread: {err}");
                None
            }
        }
    }

    pub fn try_recv(&self) -> Option<AppIndexProvider> {
        self.receiver.try_recv().ok()
    }
}

fn lower_thread_priority() {
    use windows::Win32::System::Threading::{
        GetCurrentThread, SetThreadPriority, THREAD_PRIORITY_BELOW_NORMAL,
    };
    unsafe {
        let _ = SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_BELOW_NORMAL);
    }
}

fn compile_patterns(patterns: &[String]) -> Vec<Regex> {
    patterns
        .iter()
        .filter_map(|pattern| match Regex::new(pattern) {
            Ok(re) => Some(re),
            Err(err) => {
                eprintln!("issen: invalid exclude pattern {pattern:?}: {err}");
                None
            }
        })
        .collect()
}

fn is_excluded(title: &str, patterns: &[Regex]) -> bool {
    patterns.iter().any(|re| re.is_match(title))
}

fn start_menu_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Some(program_data) = std::env::var_os("ProgramData") {
        dirs.push(PathBuf::from(program_data).join(r"Microsoft\Windows\Start Menu\Programs"));
    }
    if let Some(app_data) = std::env::var_os("APPDATA") {
        dirs.push(PathBuf::from(app_data).join(r"Microsoft\Windows\Start Menu\Programs"));
    }
    dirs
}

fn path_and_system_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Some(windir) = std::env::var_os("WINDIR") {
        dirs.push(PathBuf::from(windir).join("System32"));
    }
    if let Some(path) = std::env::var_os("PATH") {
        dirs.extend(std::env::split_paths(&path));
    }
    dirs
}

/// Maximum subfolder recursion depth. A safety valve against a directory
/// junction/symlink cycle causing a stack overflow (an unrecoverable
/// process abort in Rust) even in a user-added custom folder — ordinary
/// folder structures should never come close to this depth.
const MAX_SCAN_DEPTH: u32 = 8;

fn collect_shortcuts(
    dir: &Path,
    patterns: &[Regex],
    out: &mut Vec<IndexedApp>,
    depth: u32,
    source: AppSource,
) {
    let Ok(read_dir) = std::fs::read_dir(dir) else {
        return;
    };

    for entry in read_dir.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if depth < MAX_SCAN_DEPTH {
                collect_shortcuts(&path, patterns, out, depth + 1, source);
            }
            continue;
        }

        if has_extension(&path, "lnk") {
            if let Some(title) = path.file_stem().and_then(|s| s.to_str()) {
                if !is_excluded(title, patterns) {
                    out.push(IndexedApp {
                        title: title.to_string(),
                        subtitle: path.display().to_string(),
                        launch: Launch::Path(path.clone()),
                        source,
                    });
                }
            }
        }
    }
}

/// `max_depth` of 0 means `dir` itself only (no subfolder recursion).
/// System32/PATH pass 0; user-added custom folders pass `MAX_SCAN_DEPTH`
/// (see `collect_shortcuts` above).
fn collect_executables(
    dir: &Path,
    patterns: &[Regex],
    out: &mut Vec<IndexedApp>,
    depth: u32,
    max_depth: u32,
    source: AppSource,
) {
    let Ok(read_dir) = std::fs::read_dir(dir) else {
        return;
    };

    for entry in read_dir.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if depth < max_depth {
                collect_executables(&path, patterns, out, depth + 1, max_depth, source);
            }
            continue;
        }
        if !has_extension(&path, "exe") {
            continue;
        }

        if let Some(title) = path.file_stem().and_then(|s| s.to_str()) {
            if !is_excluded(title, patterns) {
                out.push(IndexedApp {
                    title: title.to_string(),
                    subtitle: path.display().to_string(),
                    launch: Launch::Path(path.clone()),
                    source,
                });
            }
        }
    }
}

fn has_extension(path: &Path, ext: &str) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case(ext))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn excludes_uninstall_entries_in_both_languages() {
        let patterns = compile_patterns(&["^(Uninstall|アンインストール)".to_string()]);
        assert!(is_excluded("Uninstall Foo", &patterns));
        assert!(is_excluded("アンインストール Foo", &patterns));
        assert!(!is_excluded("Foo", &patterns));
    }

    #[test]
    fn invalid_pattern_is_skipped_not_fatal() {
        let patterns = compile_patterns(&["[unterminated".to_string()]);
        assert!(patterns.is_empty());
    }
}
