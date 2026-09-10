// Copies `third_party/Everything64.dll` next to the built exe so
// `LoadLibraryW("Everything64.dll")` (src/search/everything.rs) can find it
// via the exe's own directory, for both `cargo build`/`cargo run` and
// `cargo build --release` (see docs/DEVELOPMENT.md's packaging section).
//
// Also embeds `assets/icon.ico` as the exe's Win32 icon resource (id 1) —
// gpui_windows's `load_icon()` does `LoadImageW(module, PCWSTR(1), ...)`,
// so this one resource covers both the Explorer/taskbar exe icon and every
// gpui window's title-bar icon; `tray.rs` pulls the same resource via
// `Icon::from_resource(1, ..)` for the tray icon.
use std::path::PathBuf;

fn main() {
    println!("cargo:rerun-if-changed=third_party/Everything64.dll");
    println!("cargo:rerun-if-changed=assets/icon.ico");

    winresource::WindowsResource::new()
        .set_icon("assets/icon.ico")
        .compile()
        .expect("failed to embed assets/icon.ico as a Win32 resource");

    let src = PathBuf::from("third_party/Everything64.dll");
    let out_dir = PathBuf::from(std::env::var("OUT_DIR").unwrap());
    // OUT_DIR is target/<profile>/build/<pkg>-<hash>/out; the exe sits
    // three levels up, directly in target/<profile>/.
    let dest = out_dir
        .ancestors()
        .nth(3)
        .expect("OUT_DIR shallower than expected")
        .join("Everything64.dll");

    // issen.exe never releases the DLL handle (see everything.rs) and may
    // still be running from a previous build, so overwriting it can fail
    // with a sharing violation. Skip the copy when a same-size file is
    // already there, and warn (don't fail the build) on any other error.
    let already_up_to_date = dest
        .metadata()
        .and_then(|dest_meta| src.metadata().map(|src_meta| (src_meta, dest_meta)))
        .is_ok_and(|(src_meta, dest_meta)| src_meta.len() == dest_meta.len());

    if !already_up_to_date {
        if let Err(err) = std::fs::copy(&src, &dest) {
            println!(
                "cargo:warning=failed to copy Everything64.dll next to the built exe \
                 (Everything integration will be unavailable until issen.exe is closed \
                 and rebuilt): {err}"
            );
        }
    }
}
