# Development

## Tech stack

- Language: Rust
- GUI: egui / eframe (glow backend) — chosen to avoid an external runtime
  dependency and keep startup fast. Being immediate-mode, animations and
  similar effects need to be hand-rolled.
- Windows API integration: the `windows` crate (COM / Shell interfaces)
- Config: `serde` + `toml`

## Prerequisites

### 1. Rust (via rustup)

```powershell
winget install --id Rustlang.Rustup -e
```

Restart your terminal (or VS Code) afterward so `PATH` picks up the new
toolchain, then verify:

```powershell
rustc --version
cargo --version
```

The default target is `x86_64-pc-windows-msvc`, which needs the MSVC build
tools below.

### 2. MSVC build tools (linker)

The default (MSVC) Rust toolchain needs `link.exe`. If you already have
Visual Studio installed but not the C++ workload:

1. Launch "Visual Studio Installer" from the Start menu.
2. Click "Modify" on your Visual Studio installation.
3. Check "Desktop development with C++" in the workload list.
4. Click "Modify" to install it (this downloads a few GB).

Restart your terminal afterward.

### 3. Build and run

```powershell
cargo build
cargo run
```

Debug builds (`cargo run`, plain `cargo build`) open a console window alongside
the app — expected, not a bug. `src/main.rs` only sets
`windows_subsystem = "windows"` on release builds; debug builds keep the
default console subsystem specifically so `cargo run`'s terminal can send
Ctrl+C to the process (a GUI-subsystem process isn't part of the console's
control-event group at all, so Ctrl+C can't reach it — confirmed on real
hardware). To stop a debug run without the console, use the tray icon's
"Quit" instead.

## Building, linting, testing

Before committing, all of these should pass:

```powershell
cargo build
cargo clippy -- -D warnings
cargo fmt --check
cargo test
```

Keep commits scoped to a single logical change rather than bundling
unrelated work together — it keeps `git log` useful for tracing why a
specific change was made.

`main` is protected: changes land via PR, squash-merge only, no direct
pushes.

## Project layout

- `src/` — the application. `app.rs` holds the main window's state machine
  (show/hide, search, results); `search/` holds the `SearchProvider`
  implementations (app index, Everything, aliases, Windows Settings
  shortcuts, plugins); `tools/` holds the color picker and unit converter;
  `settings_window.rs` and `ui_chrome.rs` are the settings UI and its
  shared custom window chrome.
- `plugin-api/` — the stable ABI (via `abi_stable`) that third-party
  plugins link against. Not part of the root Cargo workspace's runtime
  dependency graph beyond being the interface `src/search/plugin.rs` loads.
- `plugin-example/` — a minimal reference plugin implementing that ABI.
- `prototypes/gui-plugin/` — standalone spike code comparing three ways to
  expose GUI tools (not just text results) through the plugin system; kept
  as reference for when that host-side work is picked up.
  Each subdirectory is its own Cargo workspace — `cd` into one before
  running `cargo build`/`cargo run`.
- `third_party/Everything64.dll` — the Everything SDK client DLL, loaded at
  runtime by `src/search/everything.rs`.

See [`docs/architecture/`](architecture/) for the rationale behind these
pieces (one doc per area).

## Packaging a release zip

There's no build script for this yet. For now: build in
release mode, and copy `third_party/Everything64.dll` next to `issen.exe`
in the zip (`LoadLibraryW` looks in the executable's own folder first).

```powershell
cargo build --release
```

Once a build script exists, consider fetching `Everything64.dll` at build
time instead of committing the binary to the repo (it's currently tracked
directly under `third_party/`).
