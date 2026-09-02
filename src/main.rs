// Release builds link as a GUI app so no console host window ever appears (see
// be04679 in git history / CHANGELOG.md). Debug builds deliberately keep the default
// console subsystem instead: a GUI-subsystem process isn't part of the console's
// control-event group, so `cargo run`'s terminal can't deliver Ctrl+C to it at all
// (confirmed on real hardware — `SetConsoleCtrlHandler` registers successfully but
// the handler never fires). Losing Ctrl+C during development outweighs the stray
// console window here; use the tray's "Quit" instead if this bothers you.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
mod app;
mod config;
mod display;
mod fonts;
mod history;
mod hotkey;
mod i18n;
mod launch;
mod search;
mod settings_window;
mod single_instance;
mod tools;
mod tray;
mod ui_chrome;

fn main() -> eframe::Result<()> {
    if single_instance::is_already_running() {
        return Ok(());
    }

    let config = config::Config::load_or_default(config::APP_NAME);

    let viewport = egui::ViewportBuilder::default()
        .with_decorations(false)
        .with_transparent(true)
        .with_always_on_top()
        // `app.rs`'s `sync_window_height` sends `ViewportCommand::InnerSize` from code
        // in response to the result count (dragging to resize isn't a use case here).
        .with_resizable(false)
        // No taskbar icon: as a search launcher, the window shouldn't show up in the
        // taskbar or Alt+Tab even while visible (Alt+Tab exclusion is handled separately
        // via `WS_EX_TOOLWINDOW`, see the comment in `app.rs::new`).
        .with_taskbar(false)
        .with_inner_size([app::MAIN_WINDOW_SIZE.0, app::MAIN_WINDOW_SIZE.1])
        // The resident/display model keeps the window always visible at the OS level and
        // moves it between its real position and an off-screen one
        // (`app::OFFSCREEN_POSITION`) instead of using OS-level Show/Hide — see
        // `app.rs::set_visible`. So the window is `with_visible(true)` from the start,
        // parked off-screen by position alone.
        //
        // Rationale: an earlier version used real Show/Hide (`with_visible(false)` at
        // startup, `ViewportCommand::Visible(true/false)` on every reveal), which caused
        // a visible flicker — an empty white frame for tens of ms — on every reappearance.
        // Three mitigations were tried and none helped, because all three left the
        // OS-level Show/Hide transition itself in place and only tweaked something inside
        // it. Removing the transition entirely (stay visible, move instead) is what
        // actually fixed it. Moving the window via `OuterPosition` was already a
        // known-safe path by that point (used for startup-flicker mitigation and
        // tray-click positioning), so this switch didn't introduce a new flicker risk.
        .with_visible(true)
        .with_position([app::OFFSCREEN_POSITION.0, app::OFFSCREEN_POSITION.1]);

    let native_options = eframe::NativeOptions {
        viewport,
        ..Default::default()
    };

    eframe::run_native(
        config::APP_NAME,
        native_options,
        Box::new(|cc| Ok(Box::new(app::IssenApp::new(cc, config)))),
    )
}
