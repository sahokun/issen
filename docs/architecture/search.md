# Search index targets

Covers: `src/search/**/*.rs`, `src/app.rs`.

All of the following are indexed:

- Start Menu `.lnk` shortcuts, from both
  `%ProgramData%\Microsoft\Windows\Start Menu\Programs` and
  `%AppData%\Microsoft\Windows\Start Menu\Programs`. Shortcut targets are
  not resolved — the `.lnk` path itself is passed to `ShellExecuteW`, which
  lets the shell handle resolution, so `IShellLink` (COM) isn't needed for
  launching.
- Executables in `%Windows%\System32` and on `PATH` (for full-path launches
  and bare-filename fallback).
- UWP / Store apps: `shell:AppsFolder`
  (`SHCreateItemInKnownFolder(FOLDERID_AppsFolder)` → `IEnumShellItems`) is
  enumerated, `IShellItem2::GetString(PKEY_AppUserModel_ID)` gets each
  item's AUMID, and launching goes through
  `IApplicationActivationManager::ActivateApplication` (COM).
  - `shell:AppsFolder` mixes actual packaged apps with ordinary desktop
    apps that only registered an AppUserModelID for taskbar notifications
    (e.g. Chrome, Discord). Packaged AUMIDs are always
    `PackageFamilyName!RelativeAppId` (exactly one `!`), so that shape is
    used to filter (`src/search/uwp.rs::is_packaged_aumid`). The `windows`
    crate doesn't expose a typed `PKEY_AppUserModel_PackageFamilyName`, and
    hand-rolling that `PROPERTYKEY` risks getting the value wrong, so this
    filter was preferred over reading the family name property directly.
- Any folders added via settings.

Anything that doesn't match the index is tried as-is, as a filename or full
path.

## Index updates

Three mechanisms:

- A background scan shortly after startup, once CPU usage has settled.
  There's no true idle detection (e.g. via `GetSystemTimes`) — the scan
  thread's priority is simply lowered to `THREAD_PRIORITY_BELOW_NORMAL`
  (`src/search/app_index.rs::IndexScan`) as an approximation that keeps it
  from competing with UI work. `IssenApp::start_scan` is the single entry
  point for every scan (startup, periodic, and manual — see below), and
  its completion is awaited by a `cx.spawn` task blocking on the scan's
  channel from a background executor thread, waking the view exactly once
  — unlike the egui/eframe version's `poll_scan`, which was called every
  frame from its render loop.
- **Known gap, currently open:** periodic rescans (intended: every 30
  minutes, `app.rs::PERIODIC_RESCAN_INTERVAL`) are not actually wired up
  in the GPUI port. `IssenApp::next_periodic_scan` still exists and is
  updated on every scan completion, but nothing currently reads or
  compares it against `Instant::now()` to trigger a rescan when it
  elapses — the egui/eframe version's timer (`egui::Context::
  request_repaint_after`, which woke a hidden window's render loop
  without burning CPU every frame) was never re-implemented as a GPUI
  equivalent (e.g. a `cx.spawn` loop using `cx.background_executor()
  .timer(...)`, the same pattern this app already uses elsewhere for the
  tools window's eyedropper poll). Until this is fixed, the index only
  updates via the startup scan and manual reindex.
- Manual rescan from the tray menu, the input box's right-click menu, or
  the settings window — all three call the same `start_scan`, which also
  guards against overlapping scans.

## Search provider architecture

Built around a `SearchProvider` trait, with these built-in providers:

- `AppIndexProvider` — the index described above.
- `EverythingProvider` — delegates to a running Everything (voidtools)
  instance via `Everything64.dll` (the SDK client DLL, MIT-licensed,
  bundled at `third_party/Everything64.dll`; see
  `licenses/Everything-SDK.txt`), loaded at runtime.
  - Because the DLL is bundled, "can the DLL be loaded" no longer tells you
    whether Everything integration actually works (it's always true).
    Instead, showing/enabling the Everything settings section requires an
    actual round-trip IPC probe (`Everything_GetMajorVersion`,
    `src/search/everything.rs::is_available`) each time. Running Everything
    as a Windows service puts it in session 0, which can't respond to the
    interactive session's window-message IPC (a Windows session-isolation
    limitation, not a bug here) — in that configuration the integration
    stays unavailable and hidden. The lightweight probe returns in a few ms
    even when unreachable, but the real query call (`Everything_QueryW`)
    takes roughly 1.2s to time out when unreachable — so `search()` always
    probes first rather than calling `Everything_QueryW` directly on every
    keystroke.
  - `build.rs` copies `third_party/Everything64.dll` next to the built exe
    (`target/<profile>/`) on every `cargo build`/`cargo run`, since
    `LoadLibraryW("Everything64.dll")` looks in the exe's own directory
    first. Without this, a plain dev build has no way to find the DLL at
    all — `is_available()` always returns `false` and the settings tab
    stays hidden, regardless of whether Everything is actually running
    (this was an open bug in the GPUI-era dev workflow, fixed once
    `build.rs` was added; see `docs/DEVELOPMENT.md`'s packaging section).
- `AliasProvider` — user-defined command name ↔ launch target aliases.
- `WindowsSettingsProvider` — predefined commands that open specific
  `ms-settings:` pages.
- The color picker and unit converter are implemented as separate
  top-level windows rather than as `SearchProvider` results — see
  `docs/architecture/tools.md`.
