use std::{env, fs, path::Path};

fn main() {
    embed_injection_client();

    // Only the node addon needs N-API linker setup (`-undefined dynamic_lookup`
    // on macOS so the cdylib resolves N-API symbols from the host at load time).
    // Gated so plain `cargo build`/`cargo test` and the CLI bin are unaffected.
    #[cfg(feature = "node")]
    napi_build::setup();
}

/// Stage the bundled hot-reload injection client (CLI_DESIGN §9d) for the
/// `include_bytes!` in `cli::inject::client`. The dylib is produced by
/// `vendor/injection-client/build.sh` (macOS + Xcode) and is intentionally not
/// committed, so copy it into `OUT_DIR` when present and otherwise stage an empty
/// placeholder — every build then compiles, and the CLI falls back at runtime
/// when the client wasn't bundled. CI and release builds run `build.sh` first.
fn embed_injection_client() {
    let src = Path::new(&env::var("CARGO_MANIFEST_DIR").unwrap())
        .join("vendor/injection-client/prebuilt/SweetpadInjectionClient.dylib");
    let dest = Path::new(&env::var("OUT_DIR").unwrap()).join("injection-client.dylib");
    println!("cargo:rerun-if-changed={}", src.display());
    if src.exists() {
        fs::copy(&src, &dest).expect("stage bundled injection client into OUT_DIR");
    } else {
        fs::write(&dest, []).expect("stage empty injection-client placeholder into OUT_DIR");
    }
}
