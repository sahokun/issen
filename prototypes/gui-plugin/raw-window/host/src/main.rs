//! Host for prototype C. A minimal eframe app standing in for issen's main
//! window (undecorated, always-on-top, small). Clicking the button
//! dynamically loads `plugin.dll` and calls `open_tool()`.

use libloading::Library;

struct HostApp {
    lib: Option<Library>,
}

impl HostApp {
    fn open_tool(&self) {
        let Some(lib) = &self.lib else {
            eprintln!("[host] plugin.dll isn't loaded, can't call open_tool");
            return;
        };
        unsafe {
            match lib.get::<unsafe extern "C" fn()>(b"open_tool") {
                Ok(func) => func(),
                Err(err) => eprintln!("[host] failed to get the open_tool function: {err}"),
            }
        }
    }
}

impl eframe::App for HostApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        egui::CentralPanel::default().show(ui, |ui| {
            if ui.button("Open tool (prototype C)").clicked() {
                self.open_tool();
            }
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
        .with_resizable(false)
        .with_inner_size([260.0, 60.0]);

    let native_options = eframe::NativeOptions {
        viewport,
        ..Default::default()
    };

    eframe::run_native(
        "prototype-c-host",
        native_options,
        Box::new(move |_cc| Ok(Box::new(HostApp { lib }))),
    )
}
