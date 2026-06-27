// Intentionally (almost) empty: SPM requires a target to have at least one
// source file. The actual client comes from the InjectionNext dependency, pulled
// in whole by the `-all_load` linker flag in Package.swift. There is nothing to
// call — the client wires itself up from an ObjC `+load` when the dylib is
// inserted into the app via DYLD_INSERT_LIBRARIES.
