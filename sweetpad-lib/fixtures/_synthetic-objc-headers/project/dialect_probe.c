/* BSP per-file dialect probe (see tests/bsp_conformance.rs). Sits next to
   widget.m but is not in the ObjCHeaders sources build phase, so the hermetic
   build oracle ignores it; it exists only so the BSP server can resolve a `.c`
   file's clang dialect (`-x c`, no C++/ObjC flags) for the per-file oracle. */
int sweetpad_dialect_probe_c(void) {
    return 0;
}
