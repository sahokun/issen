# GUI tool plugins: implementation-approach comparison prototypes (reference for the third-party plugin mechanism)

A minimal spike testing three approaches to the "externalize GUI tools into
plugin DLLs" idea. Contains three standalone
Cargo workspaces (`thread-eframe/`, `setparent/`, `raw-window/`), none of
which are members of Issen's root `Cargo.toml` workspace.

## Conclusion and current status

The investigation settled the following:

- **The built-in color picker and unit converter (`src/tools/`) will not be
  pluginized.** They're already implemented as independent OS top-level
  windows via `ctx.show_viewport_immediate`, so the problem embedding was
  meant to solve (being clipped to the main window's rectangle) doesn't
  exist for them — there's nothing to gain from moving them into a DLL.
  They stay in `src/tools/` on egui/eframe.
- **A separate mechanism for third-party GUI tool plugins is still worth
  building.** When that happens, this prototype's findings (particularly A
  and C below) feed into `plugin-api`'s ABI design.

This directory is kept intentionally, as **reference material for when the
third-party GUI tool plugin mechanism is implemented**, rather than being
throwaway spike code. Delete it once that mechanism exists and these
prototypes are no longer needed as a reference.

Each approach is a `host` crate (an undecorated, always-on-top eframe
window standing in for Issen's main window) plus a `plugin` crate (a
cdylib) that `host` loads dynamically at startup.

## Running

```
cd thread-eframe   # or setparent, raw-window
cargo run --bin host
```

(Each is its own standalone Cargo workspace — run `cargo build`/`cargo run`
from inside that directory.)

## Prototype A: `thread-eframe/` — plugin's own eframe thread

Clicking "open tool" in `host` makes `plugin.dll` spin up its own `eframe`
instance (an independent top-level window with just a button and a
counter) on a new OS thread, using winit's `any_thread` feature.

**Result**: works on real hardware. Opens as a proper independent window
and responds correctly to button input; moving/resizing get standard OS
behavior for free. Downside: statically linking all of `egui`/`eframe`
makes the DLL noticeably larger (see the size comparison below).

## Prototype B: `setparent/` — `SetParent` embedding

At startup, `host` passes its own HWND to `plugin.dll`'s `create_child`.
The plugin builds a child window (a label + button, raw Win32) and embeds
it into the host via `SetParent` + `WS_CHILD`.

**Result**: works on real hardware (button clicks and following the host's
resize both work correctly), but has a **structural flaw**: the host window
it embeds into (standing in for Issen's undecorated main window) has no way
to move or resize itself, so the embedded tool inherits that limitation.
Not viable for tools like the color picker or unit converter that need to
be freely sized and moved.

## Prototype C: `raw-window/` — independent top-level window, raw Win32

The plugin creates its own independent top-level window
(`WS_OVERLAPPEDWINDOW`) via raw Win32 APIs and runs it on a thread with its
own message loop. Same goal as prototype A (an independent window, not
embedded in the host), but built without egui/eframe to see how much
smaller the DLL could get.

**Result**: works on real hardware.
- The window gets a standard OS title bar and minimize/maximize/close
  buttons for free (`WS_OVERLAPPEDWINDOW` means the OS handles
  move/resize without extra code).
- Button clicks (`WM_COMMAND`) correctly update the counter display
  (`clicked N times`).
- Clicking the title bar's close (X) button closes correctly
  (`WM_CLOSE`'s default handling → `DestroyWindow` → `WM_DESTROY` →
  `PostQuitMessage` cleanly exits the message-loop thread; no leftover
  window or thread).
- An independent top-level window with its own message loop needs an
  explicit `PostQuitMessage` call from `WM_DESTROY`, or `GetMessageW` never
  returns and the thread leaks — a concern specific to owning your own
  message loop that prototype B's embedded child window didn't have, since
  it rides on the host's loop.

## DLL size comparison

All three workspaces use the same `[profile.release]` as Issen itself
(`opt-level = "s"`, `lto = true`, `codegen-units = 1`, `strip = true`).
`plugin.dll` size after `cargo build --release`:

| Prototype | Approach | DLL size |
|---|---|---|
| A: `thread-eframe` | Plugin's own eframe thread | ~5.3 MB |
| B: `setparent` | `SetParent` embedding (raw Win32) | ~92 KB |
| C: `raw-window` | Independent top-level window (raw Win32) | ~122 KB |

Approach A's static-linked egui/eframe adds this much size per plugin. B
and C, built on raw Win32 alone, stay small — a meaningful difference if
third-party plugins are expected to be installed several at a time.

## Direction for the third-party GUI tool plugin ABI

Based on this, the GUI tool plugin ABI to add to `plugin-api` should
**center on prototype A/C's approach (independent top-level window), while
also supporting prototype B's approach (`SetParent` embedding)** — plugin
authors get A if they want to build their UI in egui/eframe, C if they want
a smaller DLL, or B if they want to integrate into an existing host screen.
A plugin declares whether it wants its own top-level window
(`Standalone`) or a child window embedded in the host (`Embedded`), and the
host calls either `launch_standalone` or `create_embedded` accordingly.
