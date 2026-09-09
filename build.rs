// Copies `third_party/Everything64.dll` next to the built exe so
// `LoadLibraryW("Everything64.dll")` (src/search/everything.rs) can find it
// via the exe's own directory, for both `cargo build`/`cargo run` and
// `cargo build --release` (see docs/DEVELOPMENT.md's packaging section).
use std::path::PathBuf;

fn main() {
    println!("cargo:rerun-if-changed=third_party/Everything64.dll");

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
