//! Prototype A: the plugin DLL launches its own `eframe` instance on its
//! own thread. Every call to `open_tool()` spawns a new thread and creates
//! an event loop off the main thread using winit's `any_thread` feature.

use std::sync::atomic::{AtomicBool, Ordering};

static OPENING: AtomicBool = AtomicBool::new(false);

struct ToolApp {
    count: i32,
}

impl eframe::App for ToolApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        egui::CentralPanel::default().show(ui, |ui| {
            ui.heading("Prototype A");
            ui.label("eframe window opened from a plugin-owned thread");
            if ui.button(format!("clicked {} times", self.count)).clicked() {
                self.count += 1;
            }
        });
    }
}

/// Entry point called by the host. Reopening while already open is
/// prevented with a simple flag (this is comparison spike code, so there's
/// no strict state management to allow reopening after close).
#[no_mangle]
pub extern "C" fn open_tool() {
    if OPENING.swap(true, Ordering::SeqCst) {
        return;
    }

    std::thread::spawn(|| {
        // Whether a second event loop on another thread even works at all
        // is exactly what this prototype tests, so `run_native` failing or
        // panicking is never swallowed silently — always print it to
        // stderr. Otherwise "nothing happens" would be impossible to tell
        // apart from "the button just doesn't respond" versus "this
        // approach doesn't work at all."
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            use winit::platform::windows::EventLoopBuilderExtWindows;

            let viewport = egui::ViewportBuilder::default()
                .with_title("Prototype A Tool")
                .with_inner_size([300.0, 160.0])
                .with_always_on_top();

            let native_options = eframe::NativeOptions {
                viewport,
                event_loop_builder: Some(Box::new(|builder| {
                    builder.with_any_thread(true);
                })),
                ..Default::default()
            };

            eframe::run_native(
                "prototype-a-tool",
                native_options,
                Box::new(|_cc| Ok(Box::new(ToolApp { count: 0 }))),
            )
        }));

        match result {
            Ok(Ok(())) => {}
            Ok(Err(err)) => eprintln!("[prototype-a] eframe::run_native failed: {err}"),
            Err(panic) => {
                let message = panic
                    .downcast_ref::<&str>()
                    .map(|s| s.to_string())
                    .or_else(|| panic.downcast_ref::<String>().cloned())
                    .unwrap_or_else(|| "unknown panic".to_string());
                eprintln!("[prototype-a] eframe::run_native panicked: {message}");
            }
        }

        OPENING.store(false, Ordering::SeqCst);
    });
}
