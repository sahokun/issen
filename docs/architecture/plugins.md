# Plugins

Covers: `plugin-api/**/*.rs`, `plugin-example/**/*.rs`, `src/search/plugin.rs`.

Dynamic DLL loading, with the ABI stabilized via `abi_stable` so it doesn't
depend on Rust's unstable ABI — a deliberate choice to keep the door open
to third-party plugin authors, even though only built-in providers use it
today. The plugin API is versioned and checked for compatibility at load
time. Built-in providers implement the same `SearchProvider` trait; the DLL
loader is just a second way to obtain an implementation of it, so built-in
functionality never depends on the plugin machinery being present.

The action vocabulary exposed to plugins (`RAction`) stays deliberately
minimal, driven by concrete use cases (calculator, snippets, clipboard
history, translation) rather than speculative generality: launch-style
actions (`Launch` / `LaunchUwp` / `OpenUri`) plus `CopyToClipboard` for
"copy this computed text to the clipboard." Multi-step actions, per-result
icons, per-result right-click menus, and plugins writing settings back to
the host all exist in other launchers' plugin systems but aren't supported
here — none of the concrete use cases need them yet, and some would need
host UI (per-result icons, per-result right-click) that doesn't exist yet
either. Revisit when a concrete use case actually needs one of these.
