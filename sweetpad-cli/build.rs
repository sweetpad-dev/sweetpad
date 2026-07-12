use std::{env, fs, path::Path};

fn main() {
    embed_injection_client();
    emit_lib_dir();
}

/// Stage the bundled hot-reload injection clients (CLI_DESIGN §9d) for the
/// `include_bytes!` in `cli::inject::client` — one per supported SDK (iOS
/// simulator + macOS). The dylibs are produced by
/// `vendor/injection-client/build.sh` (macOS + Xcode) and are intentionally not
/// committed, so copy each into `OUT_DIR` when present and otherwise stage an
/// empty placeholder — every build then compiles, and the CLI falls back at
/// runtime when a client wasn't bundled. CI and release builds run `build.sh`
/// first.
fn embed_injection_client() {
    stage_client("SweetpadInjectionClient.dylib", "injection-client.dylib");
    stage_client(
        "SweetpadInjectionClientMac.dylib",
        "injection-client-mac.dylib",
    );
}

fn stage_client(prebuilt: &str, staged: &str) {
    let src = Path::new(&env::var("CARGO_MANIFEST_DIR").unwrap())
        .join("vendor/injection-client/prebuilt")
        .join(prebuilt);
    let dest = Path::new(&env::var("OUT_DIR").unwrap()).join(staged);
    println!("cargo:rerun-if-changed={}", src.display());
    if src.exists() {
        fs::copy(&src, &dest).expect("stage bundled injection client into OUT_DIR");
    } else {
        fs::write(&dest, []).expect("stage empty injection-client placeholder into OUT_DIR");
    }
}

/// The CLI's SPM oracle test reads fixtures from the sibling `sweetpad-lib`
/// crate; expose its canonical path the same way `sweetpad-core` does.
fn emit_lib_dir() {
    let manifest = env::var("CARGO_MANIFEST_DIR").unwrap();
    let lib = Path::new(&manifest).parent().unwrap().join("sweetpad-lib");
    let lib = fs::canonicalize(&lib).unwrap_or(lib);
    println!("cargo:rustc-env=SWEETPAD_LIB_DIR={}", lib.display());
}
