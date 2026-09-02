# Issen (いっせん)

A Windows-only command launcher. Issen sits in the tray, and a global hotkey
pops up a single-line black input box where incremental fuzzy search narrows
down applications, files, and built-in tools to launch.

## Features

- **Global hotkey** (default `Alt+Space`, configurable) shows/hides the input
  box instantly — the app is already resident, so there's no process to spin
  up.
- **Fuzzy incremental search** across:
  - Start Menu shortcuts (`.lnk`)
  - Executables on `PATH` and in `System32`
  - UWP / Store apps (via `shell:AppsFolder`)
  - Any custom folders you add
  - Anything else you type is tried directly as a filename or path
- **Result actions** beyond the default launch: run as administrator, open
  the containing folder, copy the path, or register the result as a command
  alias — from a right-click menu or keyboard shortcuts.
- **Usage-based pinning**: results you've picked before float back to the
  top the next time the same query matches them.
- **Query history**: revisit and re-run a recent search from the history
  icon next to the input box.
- **Everything (voidtools) integration** — if Everything is installed and
  running, Issen can delegate to it for full filesystem search. The feature
  is hidden automatically when Everything isn't reachable.
- **Command aliases** — map a short name you choose to any launch target.
- **Windows Settings shortcuts** — jump straight to a `ms-settings:` page
  (Bluetooth, Display, Windows Update, ...) with ~35 pages predefined and
  more addable.
- **Built-in tools**, launched from icon buttons next to the input box:
  a color picker (HSV wheel, eyedropper, hex/RGB/HSL) and a unit converter
  (length, mass, temperature, area, volume, speed, time, data size).
- **Plugins** — third-party search providers can be loaded from a DLL
  through a version-checked, stable ABI. See [`plugin-api/`](plugin-api) and
  the sample in [`plugin-example/`](plugin-example).
- Dark/light/system theme, English/Japanese/system language, adjustable
  result count and font size, and per-monitor placement (cursor, primary,
  or the currently focused window's display).

## Installation

Download the latest release archive from the
[Releases](../../releases) page, extract it anywhere, and run `issen.exe`.
There's no installer — Issen is distributed as a zip, and its settings live
in `%APPDATA%\Issen`, independent of where you unzip it. Enable "start with
Windows" from the settings window if you want it always running.

To build from source instead, see [`docs/DEVELOPMENT.md`](docs/DEVELOPMENT.md).

## Usage

Press `Alt+Space` (or your configured hotkey), start typing, and press
`Enter` to launch the top (or selected) result. The box hides itself again
as soon as it loses focus.

| Key | Action |
| --- | --- |
| `Alt+Space` (default) | Show / hide the launcher |
| `↑` / `↓` | Move selection |
| `Enter` | Run the selected result |
| `Alt+2`–`Alt+9` | Jump straight to the Nth visible result |
| `Ctrl+Shift+Enter` | Run as administrator (exe / shortcut only) |
| `Ctrl+Shift+E` | Open the containing folder |
| `Ctrl+C` | Copy the result's path |
| Right-click a result | Open its action menu |
| Right-click the input box | Open settings / rebuild index / quit |
| `Esc` | Hide the launcher |

## Configuration

Everything is configurable from the settings window (right-click the input
box or the tray icon → Settings): hotkey, theme, language, UI font, result
count, indexed folders and exclusion patterns, Everything integration,
aliases, and Windows Settings shortcuts. Settings are stored as TOML at
`%APPDATA%\Issen\config.toml`; usage history (for pinning and query history)
is stored separately at `%APPDATA%\Issen\history.toml`.

## License

Issen is dual-licensed under [MIT](LICENSE-MIT) or
[Apache-2.0](LICENSE-APACHE), at your option. It bundles
`Everything64.dll` (the Everything SDK client library) under its own MIT
license — see [`licenses/Everything-SDK.txt`](licenses/Everything-SDK.txt).
