use std::path::Path;

fn main() {
    emit_lib_dir();
}

/// The shared test corpus (`fixtures/`, `corpus/`, `xcspec-cache/`) lives in the
/// sibling `sweetpad-lib` crate. Emit its canonical path so integration tests
/// locate it via `env!("SWEETPAD_LIB_DIR")` without a `..` segment — the BSP
/// server canonicalizes paths, so test URIs must be canonical to compare equal.
fn emit_lib_dir() {
    let manifest = std::env::var("CARGO_MANIFEST_DIR").unwrap();
    let lib = Path::new(&manifest).parent().unwrap().join("sweetpad-lib");
    let lib = std::fs::canonicalize(&lib).unwrap_or(lib);
    println!("cargo:rustc-env=SWEETPAD_LIB_DIR={}", lib.display());
    println!("cargo:rerun-if-changed=build.rs");
}
