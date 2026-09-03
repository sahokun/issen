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
mod history;
mod hotkey;
mod i18n;
mod launch;
mod search;
mod single_instance;
mod tray;
// GPUI移行(egui/eframe→GPUI)のPhase 1着手に伴い、egui依存のこれらは一時的に
// コンパイル対象から外している。Phase 1 step3/4でGPUI版に書き直して復活させる
// 予定(docs/architecture配下のGPUI移行計画を参照)。ファイル自体は削除していない。
// mod fonts;
// mod settings_window;
// mod tools;
// mod ui_chrome;

fn main() {
    if single_instance::is_already_running() {
        return;
    }

    let config = config::Config::load_or_default(config::APP_NAME);
    app::run(config);
}
