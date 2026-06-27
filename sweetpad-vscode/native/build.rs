fn main() {
    // N-API linker setup: `-undefined dynamic_lookup` on macOS so the cdylib
    // resolves N-API symbols from the host (node/Electron) at load time.
    napi_build::setup();
}
