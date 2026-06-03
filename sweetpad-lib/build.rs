fn main() {
    // Only the node addon needs N-API linker setup (`-undefined dynamic_lookup`
    // on macOS so the cdylib resolves N-API symbols from the host at load time).
    // Gated so plain `cargo build`/`cargo test` and the CLI bin are unaffected.
    #[cfg(feature = "node")]
    napi_build::setup();
}
