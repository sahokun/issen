//! Host for prototype B. A minimal eframe app standing in for issen's main
//! window (undecorated, always-on-top). Gets its own HWND via
//! `raw_window_handle` and passes it to `plugin.dll`'s `create_child` to
//! have it embedded via `SetParent`. Once embedded, the child HWND is kept
//! matched to the remaining area below the top bar via `MoveWindow`, every
//! frame.

use libloading::Library;
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use windows::Win32::Foundation::HWND;
use windows::Win32::UI::WindowsAndMessaging::MoveWindow;

const TOP_BAR_HEIGHT: f32 = 28.0;

struct HostApp {
    lib: Option<Library>,
    child_hwnd: Option<isize>,
    /// The last rect actually passed to `MoveWindow`; skipped again unless
    /// it changes (calling `MoveWindow` every frame forces a repaint,
    /// which would add noise to the flicker comparison this prototype is
    /// meant to inform).
    last_child_rect: Option<(i32, i32, i32, i32)>,
}

impl HostApp {
    fn ensure_child(&mut self, frame: &eframe::Frame) {
        if self.child_hwnd.is_some() {
            return;
        }
        let Some(lib) = &self.lib else {
            return;
        };
        let Ok(window_handle) = frame.window_handle() else {
            return;
        };
        let RawWindowHandle::Win32(handle) = window_handle.as_raw() else {
            return;
        };
        let host_hwnd = handle.hwnd.get() as usize;

        unsafe {
            let create_child =
                match lib.get::<unsafe extern "C" fn(usize) -> usize>(b"create_child") {
                    Ok(func) => func,
                    Err(err) => {
                        eprintln!("[host] failed to get the create_child function: {err}");
                        return;
                    }
                };
            let child = create_child(host_hwnd);
            if child != 0 {
                self.child_hwnd = Some(child as isize);
            } else {
                eprintln!("[host] create_child returned 0 (failure)");
            }
        }
    }

    /// Converts egui's logical size (points) to physical pixels and keeps
    /// the child HWND matched to the whole area below the top bar. Because
    /// of `with_decorations(false)`, egui coordinates are the same as
    /// client-area coordinates.
    fn reposition_child(&mut self, ctx: &egui::Context) {
        let Some(child) = self.child_hwnd else {
            return;
        };
        let scale = ctx.pixels_per_point();
        let screen = ctx.content_rect();
        let x = 0;
        let y = (TOP_BAR_HEIGHT * scale).round() as i32;
        let w = (screen.width() * scale).round() as i32;
        let h = ((screen.height() - TOP_BAR_HEIGHT).max(0.0) * scale).round() as i32;

        let rect = (x, y, w, h);
        if self.last_child_rect == Some(rect) {
            return;
        }
        self.last_child_rect = Some(rect);

        unsafe {
            let hwnd = HWND(child as *mut _);
            let _ = MoveWindow(hwnd, x, y, w, h, true);
        }
    }
}

impl eframe::App for HostApp {
    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        self.ensure_child(frame);
        self.reposition_child(ui.ctx());

        egui::Panel::top("top_bar")
            .exact_size(TOP_BAR_HEIGHT)
            .show(ui, |ui| {
                // eframe's default_fonts has no CJK glyphs (see the
                // comment in issen's own src/fonts.rs), so this stays
                // ASCII rather than rendering as tofu. Runtime font
                // loading is a separate, already-solved problem on the
                // main app's side and isn't relevant to comparing
                // embedding approaches here.
                ui.centered_and_justified(|ui| {
                    ui.label("issen host (prototype B) - embedded child HWND below");
                });
            });
    }
}

fn plugin_dll_path() -> std::path::PathBuf {
    let mut path = std::env::current_exe().expect("current_exe");
    path.pop();
    path.push("plugin.dll");
    path
}

fn main() -> eframe::Result<()> {
    let lib = unsafe { Library::new(plugin_dll_path()) }.ok();
    if lib.is_none() {
        eprintln!(
            "failed to load plugin.dll: {}",
            plugin_dll_path().display()
        );
    }

    let viewport = egui::ViewportBuilder::default()
        .with_decorations(false)
        .with_always_on_top()
        .with_inner_size([320.0, 140.0]);

    let native_options = eframe::NativeOptions {
        viewport,
        ..Default::default()
    };

    eframe::run_native(
        "prototype-b-host",
        native_options,
        Box::new(move |_cc| {
            Ok(Box::new(HostApp {
                lib,
                child_hwnd: None,
                last_child_rect: None,
            }))
        }),
    )
}
