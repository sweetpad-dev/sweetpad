//! Compiler argument generation: resolved build settings → the per-tool argv
//! `xcodebuild` would invoke. The layer directly above
//! [`crate::build_context::BuildContext::resolve`].
//!
//! Today this generates the `swiftc` module invocation. It emits the flags that
//! are a function of the resolved settings (module name, optimization, target
//! triple, search paths, active-compilation `-D`s, …) plus the build-system
//! defaults a current Xcode always passes (`-enable-batch-mode`,
//! `-explicit-module-build`, `-no-color-diagnostics`, …). It does **not** emit
//! the per-build output/intermediate geometry (`-o`, `-output-file-map`,
//! `-emit-module-path`, header maps, the module cache): those are validation-
//! out-of-scope (see the oracle comparator) and have no consumer here.
//!
//! Mappings are grounded against the captured oracle
//! (`fixtures/<slug>/.../compiler-args/`). Where a flag is a fixed build-system
//! default rather than a function of a setting, that is noted at the emit site.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Path, PathBuf};

use crate::xcspec::{CliArgs, CompilerOption};

type Settings = BTreeMap<String, String>;

/// One generated tool invocation: the tool name, its argument vector, and the
/// input files it consumes (`.swift` for swiftc, the C-family sources for
/// clang; empty for the linker, which takes object files via geometry).
#[derive(Debug, Clone)]
pub struct ToolInvocation {
    pub tool: String,
    pub arguments: Vec<String>,
    pub input_files: Vec<String>,
}

/// The per-tool argv a target compiles + links with. A tool is `None` when the
/// target has no inputs for it (no `.swift` → no `swift`; no C-family source →
/// no `clang`; a non-linking product → no `link`).
#[derive(Debug, Clone)]
pub struct TargetCompilerArguments {
    pub target: String,
    pub swift: Option<ToolInvocation>,
    pub clang: Option<ToolInvocation>,
    pub link: Option<ToolInvocation>,
}

/// C-family source extensions a `clang`/`clang++` invocation compiles.
const CLANG_EXTS: &[&str] = &["c", "m", "mm", "cc", "cpp", "cxx", "C"];

/// The xcspec `FileTypes` language a C-family source extension belongs to. This
/// is the value Apple's options gate on (`CLANG_CXX_*` → `sourcecode.cpp.*`,
/// the ObjC warnings → `sourcecode.c.objc`/`sourcecode.cpp.objcpp`), so it lets
/// us emit only the flags a file's language actually accepts.
fn source_file_type(ext: &str) -> Option<&'static str> {
    Some(match ext {
        "c" => "sourcecode.c.c",
        "m" => "sourcecode.c.objc",
        "mm" => "sourcecode.cpp.objcpp",
        "cc" | "cpp" | "cxx" | "C" => "sourcecode.cpp.cpp",
        _ => return None,
    })
}

/// The clang `-x <dialect>` token for an xcspec source `FileTypes` value.
fn dialect_for(file_type: &str) -> Option<&'static str> {
    Some(match file_type {
        "sourcecode.c.c" => "c",
        "sourcecode.c.objc" => "objective-c",
        "sourcecode.cpp.cpp" => "c++",
        "sourcecode.cpp.objcpp" => "objective-c++",
        _ => return None,
    })
}

/// The set of source `FileTypes` a list of C-family inputs covers — the
/// languages a target's clang invocation compiles. Used to gate
/// language-specific flags (a C++ `-std` never reaches an ObjC `.m`).
#[must_use]
pub fn clang_languages(paths: &[String]) -> BTreeSet<String> {
    paths
        .iter()
        .filter_map(|p| Path::new(p).extension().and_then(|e| e.to_str()))
        .filter_map(source_file_type)
        .map(String::from)
        .collect()
}

/// Assemble the full per-tool argv for one target from its resolved settings,
/// arch, product type, and source files. The entry point the bindings call.
///
/// The inputs are distinct resolved facts (settings, arch, product type, sources,
/// the swift/clang spec option sets, the toolchain version), so they stay
/// positional rather than bundled into a one-use struct.
#[allow(clippy::too_many_arguments)]
#[must_use]
pub fn target_arguments(
    target: &str,
    settings: &Settings,
    arch: &str,
    product_type: Option<&str>,
    sources: &[PathBuf],
    frameworks: &[String],
    swift_options: &[CompilerOption],
    clang_options: &[CompilerOption],
    xcode_version: &str,
    has_package_products: bool,
    macro_plugins: &[PathBuf],
) -> TargetCompilerArguments {
    let mut swift_inputs = Vec::new();
    let mut clang_inputs = Vec::new();
    for src in sources {
        let path = src.to_string_lossy().into_owned();
        match src.extension().and_then(|e| e.to_str()) {
            Some("swift") => swift_inputs.push(path),
            Some(ext) if CLANG_EXTS.contains(&ext) => clang_inputs.push(path),
            _ => {}
        }
    }
    let swift = (!swift_inputs.is_empty()).then(|| ToolInvocation {
        tool: "swiftc".to_string(),
        arguments: swift_arguments(
            settings,
            arch,
            swift_options,
            xcode_version,
            has_package_products,
            macro_plugins,
        ),
        input_files: swift_inputs,
    });
    let clang_langs = clang_languages(&clang_inputs);
    let clang = (!clang_inputs.is_empty()).then(|| ToolInvocation {
        tool: "clang".to_string(),
        arguments: clang_arguments(settings, arch, clang_options, &clang_langs),
        input_files: clang_inputs,
    });
    // A target links when it produces a binary from compiled sources. A
    // static library is assembled by libtool; everything else (framework, app,
    // tool, bundle, test, extension) links through the clang driver.
    let has_sources = swift.is_some() || clang.is_some();
    let tool = link_tool(settings, product_type);
    let link = (has_sources && links(product_type)).then(|| ToolInvocation {
        tool: tool.to_string(),
        arguments: if tool == "libtool" {
            static_lib_arguments(settings, arch)
        } else {
            link_arguments(settings, arch, frameworks)
        },
        input_files: Vec::new(),
    });
    TargetCompilerArguments {
        target: target.to_string(),
        swift,
        clang,
        link,
    }
}

/// Whether a product type links a binary (vs. an aggregate / legacy target that
/// runs scripts only). Unknown / absent product types are assumed to link.
fn links(product_type: Option<&str>) -> bool {
    !matches!(product_type, Some(pt) if pt.contains("bundle") && !pt.contains("unit-test"))
}

/// `libtool` for a static library, the `clang` driver otherwise.
fn link_tool(settings: &Settings, product_type: Option<&str>) -> &'static str {
    let static_lib = product_type.is_some_and(|pt| pt.contains("library.static"))
        || settings.get("MACH_O_TYPE").map(String::as_str) == Some("staticlib");
    if static_lib { "libtool" } else { "clang" }
}

/// Build the `swiftc` module argv for one target+arch from its resolved
/// settings, routing the options the compiler xcspec cleanly encodes
/// (`options`) through that data and hand-coding the computed/build-system flags
/// it doesn't (the target triple, search paths, driver defaults, …). Pass `&[]`
/// for `options` to fall back to the hand-coded heuristic for every option.
///
/// Order is not significant — the oracle comparator scores argv as a multiset —
/// so flags are grouped by concern for readability.
#[must_use]
pub fn swift_arguments(
    settings: &Settings,
    arch: &str,
    options: &[CompilerOption],
    xcode_version: &str,
    has_package_products: bool,
    macro_plugins: &[PathBuf],
) -> Vec<String> {
    let mut a = ArgBuilder::default();
    let get = |k: &str| settings.get(k).map(String::as_str);
    let by_name: HashMap<&str, &CompilerOption> =
        options.iter().map(|o| (o.name.as_str(), o)).collect();
    // Spec-driven emit for one named option: its encoding applied to the
    // resolved value, or `None` when the spec lacks it / it didn't resolve.
    // An empty or still-unexpanded (`$(…)`) value emits nothing — it must not
    // fall through to an Enumeration's `<<otherwise>>` branch.
    let spec = |name: &str| -> Option<Vec<String>> {
        let opt = by_name.get(name)?;
        let value = settings.get(name)?;
        if value.is_empty() || value.contains("$(") {
            return None;
        }
        Some(emit_option(opt, value, settings))
    };

    // --- Identity + language -----------------------------------------------
    if let Some(module) = get("PRODUCT_MODULE_NAME").or_else(|| get("PRODUCT_NAME")) {
        a.pair("-module-name", module);
    }
    // SWIFT_OPTIMIZATION_LEVEL: the spec enum maps `-Owholemodule` specially and
    // passes everything else (`-Onone`/`-O`/`-Osize`) through as the flag itself.
    if let Some(args) = spec("SWIFT_OPTIMIZATION_LEVEL") {
        a.extend(args);
    } else if let Some(opt) = get("SWIFT_OPTIMIZATION_LEVEL") {
        a.flag(opt);
    }
    // SWIFT_VERSION has no swift-spec encoding — the build system passes it,
    // collapsing `5.0` → `5`.
    if let Some(v) = get("SWIFT_VERSION") {
        a.pair("-swift-version", swift_version(v));
    }

    // --- Conditional compilation -------------------------------------------
    // SWIFT_ACTIVE_COMPILATION_CONDITIONS is a StringList encoded `-D$(value)`.
    if let Some(args) = spec("SWIFT_ACTIVE_COMPILATION_CONDITIONS") {
        a.extend(args);
    } else {
        for cond in ws(get("SWIFT_ACTIVE_COMPILATION_CONDITIONS")) {
            a.flag(&format!("-D{cond}"));
        }
    }
    emit_feature_flags(&mut a, settings, &by_name);
    // SWIFT_STRICT_CONCURRENCY (`complete` → -enable-upcoming-feature
    // StrictConcurrency) and code-coverage instrumentation are both spec enums
    // keyed on the resolved value.
    if let Some(args) = spec("SWIFT_STRICT_CONCURRENCY") {
        a.extend(args);
    }
    if let Some(args) = spec("CLANG_COVERAGE_MAPPING") {
        a.extend(args);
    }

    // --- Platform ----------------------------------------------------------
    if let Some(sdk) = get("SDKROOT") {
        a.pair("-sdk", sdk);
    }
    if let Some(triple) = target_triple(settings, arch) {
        a.pair("-target", &triple);
    }

    // --- Debugging + testing -----------------------------------------------
    if debug_info_enabled(get("DEBUG_INFORMATION_FORMAT")) {
        a.flag("-g");
        // Xcode serializes the debug options into the module for a debuggable build.
        a.pair("-Xfrontend", "-serialize-debugging-options");
    }
    if is_yes(get("ENABLE_TESTABILITY").unwrap_or("")) {
        a.flag("-enable-testing");
    }
    if is_yes(get("APPLICATION_EXTENSION_API_ONLY").unwrap_or("")) {
        a.flag("-application-extension");
    }

    // --- Search paths (structural; geometry-independent) -------------------
    if let Some(products) = get("BUILT_PRODUCTS_DIR") {
        a.pair("-I", products);
        // Swift-package products Xcode links dynamically build into a
        // `PackageFrameworks` subdir of the products dir (static products land
        // in the products dir itself, covered by `-I` above). Xcode emits this
        // `-F` only for targets that consume package products, so gate on it.
        if has_package_products {
            a.pair("-F", &format!("{products}/PackageFrameworks"));
        }
    }
    for p in ws_paths(get("SWIFT_INCLUDE_PATHS")) {
        a.pair("-I", &p);
    }
    for p in ws_paths(get("FRAMEWORK_SEARCH_PATHS")) {
        a.pair("-F", &p);
    }
    emit_unit_test_search_paths(&mut a, settings);

    // Swift macros a package vends are out-of-process executable plugins. A
    // plugin *search path* doesn't discover executables, so the frontend
    // resolves a `#externalMacro(module:)` reference only when handed each
    // plugin explicitly — `-load-plugin-executable <exe>#<module>`, exactly as
    // Xcode emits it. The caller passes the plugins built into the host products
    // dir (empty unless the target consumes package products).
    for plugin in macro_plugins {
        if let Some(module) = plugin.file_name().and_then(|n| n.to_str()) {
            a.pair("-Xfrontend", "-load-plugin-executable");
            a.pair("-Xfrontend", &format!("{}#{module}", plugin.display()));
        }
    }

    // --- Clang importer flags ----------------------------------------------
    // GCC_PREPROCESSOR_DEFINITIONS reach the embedded clang importer as -Xcc -D.
    for def in ws(get("GCC_PREPROCESSOR_DEFINITIONS")) {
        a.pair("-Xcc", &format!("-D{def}"));
    }
    // The products `include` dir, where Xcode drops generated module maps.
    if let Some(products) = get("BUILT_PRODUCTS_DIR") {
        a.pair("-Xcc", &format!("-I{products}/include"));
    }

    // --- Module / header emission ------------------------------------------
    a.flag("-emit-module");
    a.flag("-emit-dependencies");
    // swiftc always emits the generated ObjC interface header into the build dir;
    // SWIFT_INSTALL_OBJC_HEADER only controls whether it is *installed* publicly.
    a.flag("-emit-objc-header");
    // SWIFT_OBJC_BRIDGING_HEADER imports a target's ObjC declarations into Swift;
    // the build system passes it as `-import-objc-header <path>` (the Swift
    // xcspec carries the setting but not the flag mapping), resolved against
    // SRCROOT when the project gives it relative.
    if let Some(header) = get("SWIFT_OBJC_BRIDGING_HEADER").filter(|h| !h.is_empty()) {
        let path = match get("SRCROOT") {
            Some(root) if !Path::new(header).is_absolute() => format!("{root}/{header}"),
            _ => header.to_string(),
        };
        a.pair("-import-objc-header", &path);
    }

    // --- User passthrough --------------------------------------------------
    for f in ws(get("OTHER_SWIFT_FLAGS")) {
        a.flag(f);
    }

    // --- Compilation mode + build-system defaults --------------------------
    emit_compilation_defaults(&mut a, settings, xcode_major(xcode_version));

    a.into_vec()
}

/// A unit-test target compiles against the framework-under-test (the products
/// dir) and the platform's test frameworks: XCTest's ObjC API via `-F`, and its
/// Swift overlay — the `XCTAssert*` functions in `Developer/usr/lib/
/// XCTest.swiftmodule` — via `-I`. Without the `-I`, `import XCTest` in a test
/// file resolves only the ObjC half and the assertion functions are out of scope.
fn emit_unit_test_search_paths(a: &mut ArgBuilder, settings: &Settings) {
    if !settings
        .get("PRODUCT_TYPE")
        .is_some_and(|p| p.contains("unit-test"))
    {
        return;
    }
    if let Some(products) = settings.get("BUILT_PRODUCTS_DIR") {
        a.pair("-F", products);
    }
    if let Some(platform) = settings.get("PLATFORM_DIR") {
        a.pair("-F", &format!("{platform}/Developer/Library/Frameworks"));
        a.pair("-I", &format!("{platform}/Developer/usr/lib"));
    }
}

/// Major version from an Xcode version string (`26.5.0` → 26), or 0 if absent /
/// unparseable — callers treat 0 as the current (modern) toolchain.
fn xcode_major(version: &str) -> u32 {
    version
        .split('.')
        .next()
        .and_then(|s| s.parse().ok())
        .unwrap_or(0)
}

/// Emit the compilation-mode flags (incremental batch vs whole-module) and the
/// driver / clang-importer defaults a current Xcode always passes to swiftc. A
/// few defaults turned over at the Xcode 26 explicit-modules cutover, so they are
/// gated on the toolchain major (an unknown version is treated as modern, since
/// the bindings always supply one).
fn emit_compilation_defaults(a: &mut ArgBuilder, settings: &Settings, major: u32) {
    let get = |k: &str| settings.get(k).map(String::as_str);
    let modern = major == 0 || major >= 26;
    a.flag("-c");
    // Debug builds compile incrementally in batches; a whole-module build
    // (explicit mode, or implied by -O/-Osize/-Owholemodule) compiles the module
    // at once and emits it in-process rather than separately.
    let mode_wmo =
        get("SWIFT_COMPILATION_MODE").is_some_and(|m| m.eq_ignore_ascii_case("wholemodule"));
    let opt_wmo =
        get("SWIFT_OPTIMIZATION_LEVEL").is_some_and(|o| o.eq_ignore_ascii_case("-Owholemodule"));
    let whole_module = mode_wmo
        || opt_wmo
        || get("SWIFT_OPTIMIZATION_LEVEL").is_some_and(|o| o == "-O" || o == "-Osize");
    if whole_module {
        // -Owholemodule already carries -whole-module-optimization via the
        // opt-level spec; an explicit compilation mode supplies it otherwise.
        if mode_wmo && !opt_wmo {
            a.flag("-whole-module-optimization");
        }
        a.flag("-no-emit-module-separately-wmo");
    } else {
        a.flag("-enable-batch-mode");
        a.flag("-incremental");
        a.flag("-disable-cmo");
    }
    // Pre-26 Swift drivers pass exclusivity enforcement explicitly; 26+ implies it.
    if !modern {
        a.flag("-enforce-exclusivity=checked");
    }
    // Defaults a current Xcode (16+/26) always passes to the Swift driver;
    // -experimental-emit-module-separately is incremental-only (a whole-module
    // build emits the module in-process, via -no-emit-module-separately-wmo).
    for flag in MODERN_DRIVER_DEFAULTS {
        if whole_module && *flag == "-experimental-emit-module-separately" {
            continue;
        }
        a.flag(flag);
    }
    // C++ standard-library hardening: the 26 toolchain's libc++ injects this into
    // the clang importer for a Debug build (earlier toolchains don't).
    if modern && get("CONFIGURATION").is_some_and(|c| c.eq_ignore_ascii_case("Debug")) {
        a.pair(
            "-Xcc",
            "-D_LIBCPP_HARDENING_MODE=_LIBCPP_HARDENING_MODE_DEBUG",
        );
    }
}

/// Build the `clang`/`clang++` per-target compile argv (the shared flag set, not
/// the per-file `-c <src>`/`-o <obj>` geometry) from resolved settings. The
/// warning/codegen flags are routed through the Clang xcspec's option encodings
/// (`options`, the `com.apple.compilers.llvm.clang.1_0` set); the platform flags
/// are computed.
///
/// `file_types` are the source languages the target compiles (see
/// [`clang_languages`]). An option is emitted only when its xcspec `FileTypes`
/// include one of them — so a C++ dialect/warning flag never lands on an ObjC
/// `.m`, and the ObjC warnings never land on a `.c`. A single-language target
/// also gets the leading `-x <dialect>` clang passes per file.
#[must_use]
pub fn clang_arguments(
    settings: &Settings,
    arch: &str,
    options: &[CompilerOption],
    file_types: &BTreeSet<String>,
) -> Vec<String> {
    let mut a = ArgBuilder::default();
    let get = |k: &str| settings.get(k).map(String::as_str);
    // A homogeneous target compiles every file as one dialect, which clang
    // selects with a leading `-x`; a mixed target sets `-x` per file (geometry).
    if file_types.len() == 1
        && let Some(dialect) = file_types.iter().next().and_then(|ft| dialect_for(ft))
    {
        a.pair("-x", dialect);
    }
    if let Some(triple) = target_triple(settings, arch) {
        a.pair("-target", &triple);
    }
    if let Some(sdk) = get("SDKROOT") {
        a.pair("-isysroot", sdk);
    }
    // Header/framework search paths live in the core build-settings spec, not the
    // Clang tool spec, so the option loop below never emits them — the editor's
    // front end needs them to find project headers and framework modules.
    emit_clang_search_paths(&mut a, settings);
    // An option applies when its language, arch, and condition gates all pass. A
    // `FileTypes` is satisfied by a language the target compiles (or is empty, =
    // every C-family input; or `file_types` is empty = no info, gate nothing); an
    // `Architectures` is satisfied by the build arch (or is empty = every arch); a
    // `Condition` is the xcspec predicate evaluated against the settings (e.g.
    // `-fsanitize=integer` is gated on `$(CLANG_UNDEFINED_BEHAVIOR_SANITIZER)`, so
    // its sub-setting being `YES` doesn't emit the flag when the sanitizer is off).
    let applies = |opt: &CompilerOption| {
        let lang_ok = file_types.is_empty()
            || opt.file_types.is_empty()
            || opt.file_types.iter().any(|ft| file_types.contains(ft));
        let arch_ok = opt.architectures.is_empty() || opt.architectures.iter().any(|a| a == arch);
        let cond_ok = match &opt.condition {
            None => true,
            Some(c) => condition_holds(c, settings),
        };
        lang_ok && arch_ok && cond_ok
    };
    // Every applicable Clang-spec option whose value resolved (and isn't a
    // no-op default) contributes its encoded `-W…` / `-f…` flags.
    for opt in options {
        let Some(value) = settings.get(&opt.name) else {
            continue;
        };
        if value.is_empty() || value.contains("$(") || !applies(opt) {
            continue;
        }
        if opt.args.is_some() || opt.flag.is_some() || opt.prefix_flag.is_some() {
            let toks = emit_option(opt, value, settings);
            // The platform triple and sysroot are computed above; an option that
            // re-encodes them (CLANG_TARGET_TRIPLE_ARCHS → `-target`, the header
            // symlink dir → `-isysroot`) would duplicate them, often with an
            // unresolved `$(CURRENT_ARCH)`.
            if toks
                .first()
                .is_some_and(|t| t == "-target" || t == "-isysroot")
            {
                continue;
            }
            a.extend(toks);
        }
    }
    // The compile action every per-file clang runs (compile, don't link).
    a.flag("-c");
    a.into_vec()
}

/// Emit the clang header/framework search paths. The build system adds these
/// from the core build settings (not the Clang xcspec): the products dir's
/// generated-headers `include` subdir + `HEADER_SEARCH_PATHS` as `-I`, the
/// products dir + `FRAMEWORK_SEARCH_PATHS` as `-F`, and the user/system header
/// paths as `-iquote`/`-isystem`. Each list is de-duplicated (a setting often
/// re-inherits the products dir), as `emit_library_paths` does for `-L`.
fn emit_clang_search_paths(a: &mut ArgBuilder, settings: &Settings) {
    let get = |k: &str| settings.get(k).map(String::as_str);
    let products = get("BUILT_PRODUCTS_DIR");

    // `-I`: the products generated-headers dir, then HEADER_SEARCH_PATHS. (Bare
    // `BUILT_PRODUCTS_DIR` is a Swift-module path, not a clang header path.)
    let mut includes: Vec<String> = Vec::new();
    if let Some(p) = products {
        includes.push(format!("{p}/include"));
    }
    includes.extend(ws_paths(get("HEADER_SEARCH_PATHS")));
    emit_unique_pairs(a, "-I", &includes);

    // `-F`: the products dir, then FRAMEWORK_SEARCH_PATHS.
    let mut frameworks: Vec<String> = products.map(str::to_string).into_iter().collect();
    frameworks.extend(ws_paths(get("FRAMEWORK_SEARCH_PATHS")));
    emit_unique_pairs(a, "-F", &frameworks);

    for p in ws_paths(get("USER_HEADER_SEARCH_PATHS")) {
        a.pair("-iquote", &p);
    }
    for p in ws_paths(get("SYSTEM_HEADER_SEARCH_PATHS")) {
        a.pair("-isystem", &p);
    }
}

/// Emit `flag value` for each path, skipping duplicates (order preserved).
fn emit_unique_pairs(a: &mut ArgBuilder, flag: &str, paths: &[String]) {
    let mut seen: Vec<&str> = Vec::new();
    for p in paths {
        if !seen.contains(&p.as_str()) {
            a.pair(flag, p);
            seen.push(p);
        }
    }
}

/// Emit one `-L` per unique library search path: the products dir (passed
/// explicitly) plus LIBRARY_SEARCH_PATHS, which usually inherits the products
/// dir — the build system de-duplicates, so we do too.
fn emit_library_paths(a: &mut ArgBuilder, settings: &Settings) {
    let get = |k: &str| settings.get(k).map(String::as_str);
    let mut paths: Vec<String> = Vec::new();
    if let Some(products) = get("BUILT_PRODUCTS_DIR") {
        paths.push(products.to_string());
    }
    for p in ws_paths(get("LIBRARY_SEARCH_PATHS")) {
        if !paths.contains(&p) {
            paths.push(p);
        }
    }
    for p in &paths {
        a.flag(&format!("-L{p}"));
    }
}

/// Build the link argv (`clang`-driver invoked) from resolved settings: the
/// platform triple, SDK, dylib/runpath/search-path flags, and version stamps.
/// The auto-linked framework imports, the `-add_ast_path` debug-info plumbing,
/// and the object filelist are out of scope (geometry / autolink), tracked by
/// the comparator tally rather than generated here.
#[must_use]
pub fn link_arguments(settings: &Settings, arch: &str, frameworks: &[String]) -> Vec<String> {
    let mut a = ArgBuilder::default();
    let get = |k: &str| settings.get(k).map(String::as_str);
    if let Some(triple) = target_triple(settings, arch) {
        a.pair("-target", &triple);
    }
    if let Some(sdk) = get("SDKROOT") {
        a.pair("-isysroot", sdk);
    }
    let mach_o = get("MACH_O_TYPE");
    if mach_o == Some("mh_dylib") {
        a.flag("-dynamiclib");
    }
    // A loadable bundle (a unit-test `.xctest`, a plug-in) links with -bundle.
    if mach_o == Some("mh_bundle") {
        a.flag("-bundle");
    }
    if let Some(level) = get("GCC_OPTIMIZATION_LEVEL") {
        a.flag(&format!("-O{level}"));
    }
    emit_library_paths(&mut a, settings);
    for p in ws_paths(get("FRAMEWORK_SEARCH_PATHS")) {
        a.flag(&format!("-F{p}"));
    }
    // Swift-runtime stdlib search paths the driver adds for a Swift link.
    if let (Some(toolchain), Some(platform)) = (get("TOOLCHAIN_DIR"), get("PLATFORM_NAME")) {
        a.flag(&format!("-L{toolchain}/usr/lib/swift/{platform}"));
    }
    a.flag("-L/usr/lib/swift");
    // A unit-test bundle links XCTest from the products + platform test paths.
    if get("PRODUCT_TYPE").is_some_and(|p| p.contains("unit-test")) {
        if let Some(products) = get("BUILT_PRODUCTS_DIR") {
            a.flag(&format!("-F{products}"));
        }
        if let Some(platform) = get("PLATFORM_DIR") {
            a.flag(&format!("-L{platform}/Developer/usr/lib"));
        }
        a.pair("-framework", "XCTest");
    }
    for p in ws_paths(get("LD_RUNPATH_SEARCH_PATHS")) {
        a.pair("-Xlinker", "-rpath");
        a.pair("-Xlinker", &p);
    }
    if is_yes(get("APPLICATION_EXTENSION_API_ONLY").unwrap_or("")) {
        a.flag("-fapplication-extension");
    }
    // The dylib identity + version stamps only apply to a dylib link; an
    // executable or bundle never carries them.
    if mach_o == Some("mh_dylib") {
        if let Some(name) = get("LD_DYLIB_INSTALL_NAME") {
            a.pair("-install_name", name);
        }
        if let Some(v) = get("DYLIB_COMPATIBILITY_VERSION") {
            a.pair("-compatibility_version", v);
        }
        if let Some(v) = get("DYLIB_CURRENT_VERSION") {
            a.pair("-current_version", v);
        }
    }
    // Modern Xcode link-driver defaults, grounded across every captured macOS
    // link: a reproducible link, dead-code stripping (on unless disabled), and
    // the ObjC runtime (Swift uses ObjC interop on Apple platforms). A Debug
    // build also disables dedup for link speed.
    a.pair("-Xlinker", "-reproducible");
    if get("DEAD_CODE_STRIPPING") != Some("NO") {
        a.pair("-Xlinker", "-dead_strip");
    }
    if get("CONFIGURATION").is_some_and(|c| c.eq_ignore_ascii_case("Debug")) {
        a.pair("-Xlinker", "-no_deduplicate");
        // A Debug link keeps dynamic symbols exported (for the debugger / dlopen).
        a.flag("-rdynamic");
    }
    a.flag("-fobjc-link-runtime");
    // A coverage-instrumented build links the profiling runtime.
    if get("CLANG_COVERAGE_MAPPING").is_some_and(is_yes) {
        a.flag("-fprofile-instr-generate");
    }
    // Frameworks the target links explicitly (its Frameworks build phase); the
    // ones the sources autolink via `import` are encoded in the objects, not here.
    for fw in frameworks {
        a.pair("-framework", fw);
    }
    for f in ws(get("OTHER_LDFLAGS")) {
        a.flag(f);
    }
    a.into_vec()
}

/// The `libtool -static` argv for a static-library link. Far smaller than the
/// clang-driver link — the archive carries no triple, runpaths, or dylib flags,
/// just the arch, the deterministic-mode `-D`, the SDK (`-syslibroot`), and the
/// library search paths. The object filelist and `-o` / `-dependency_info` are
/// geometry, scored out by the comparator.
#[must_use]
pub fn static_lib_arguments(settings: &Settings, arch: &str) -> Vec<String> {
    let mut a = ArgBuilder::default();
    let get = |k: &str| settings.get(k).map(String::as_str);
    a.flag("-static");
    if !arch.is_empty() {
        a.pair("-arch_only", arch);
    }
    a.flag("-D");
    if let Some(sdk) = get("SDKROOT") {
        a.pair("-syslibroot", sdk);
    }
    emit_library_paths(&mut a, settings);
    a.into_vec()
}

/// Driver flags a current Xcode emits unconditionally for a Swift target. These
/// are build-system defaults, not functions of any user setting; they are
/// calibrated against the Xcode 26 oracle and revisited per-version in Phase 5.
const MODERN_DRIVER_DEFAULTS: &[&str] = &[
    "-enable-bare-slash-regex",
    "-no-color-diagnostics",
    "-use-frontend-parseable-output",
    "-save-temps",
    "-explicit-module-build",
    "-validate-clang-modules-once",
    "-emit-const-values",
    "-experimental-emit-module-separately",
];

// ----- helpers -------------------------------------------------------------

/// Ordered argv accumulator. A standalone flag is one token; a pair is two
/// (`-flag value`). Order isn't scored, but pairs must stay adjacent.
#[derive(Default)]
struct ArgBuilder {
    out: Vec<String>,
}

impl ArgBuilder {
    fn flag(&mut self, f: &str) {
        self.out.push(f.to_string());
    }
    fn pair(&mut self, flag: &str, value: &str) {
        self.out.push(flag.to_string());
        self.out.push(value.to_string());
    }
    fn extend(&mut self, items: Vec<String>) {
        self.out.extend(items);
    }
    fn into_vec(self) -> Vec<String> {
        self.out
    }
}

/// Emit the `-enable-upcoming-feature` / `-enable-experimental-feature` flags
/// for the resolved `SWIFT_UPCOMING_FEATURE_*` / `SWIFT_EXPERIMENTAL_FEATURE_*`
/// settings. The xcspec option (when present) carries the authoritative feature
/// name and the `YES`/`MIGRATE`/`NO` encoding; a setting the spec doesn't model
/// falls back to Title-casing its suffix.
fn emit_feature_flags(a: &mut ArgBuilder, settings: &Settings, by_name: &SpecIndex) {
    for (key, val) in settings {
        for (prefix, flag) in [
            ("SWIFT_UPCOMING_FEATURE_", "-enable-upcoming-feature"),
            (
                "SWIFT_EXPERIMENTAL_FEATURE_",
                "-enable-experimental-feature",
            ),
        ] {
            let Some(suffix) = key.strip_prefix(prefix) else {
                continue;
            };
            if let Some(opt) = by_name.get(key.as_str()) {
                a.extend(emit_option(opt, val, settings));
            } else if is_yes(val) {
                a.pair(flag, &feature_name(suffix));
            }
        }
    }
}

type SpecIndex<'a> = HashMap<&'a str, &'a CompilerOption>;

/// Apply one option's xcspec command-line encoding to its resolved `value`.
fn emit_option(opt: &CompilerOption, value: &str, settings: &Settings) -> Vec<String> {
    if let Some(args) = &opt.args {
        return match args {
            CliArgs::ByValue { map, otherwise } => map
                .get(value)
                .or(otherwise.as_ref())
                .map(|tokens| tokens.iter().map(|t| subst(t, value, settings)).collect())
                .unwrap_or_default(),
            CliArgs::List(tokens) => {
                let mut out = Vec::new();
                if opt.is_list {
                    for elem in value.split_whitespace() {
                        for t in tokens {
                            out.push(subst(t, elem, settings));
                        }
                    }
                } else {
                    for t in tokens {
                        out.push(subst(t, value, settings));
                    }
                }
                out
            }
        };
    }
    if let Some(flag) = &opt.flag {
        if value.eq_ignore_ascii_case("YES") {
            return vec![flag.clone()];
        }
        if value.is_empty() || value.eq_ignore_ascii_case("NO") {
            return Vec::new();
        }
        if opt.is_list {
            let mut out = Vec::new();
            for elem in value.split_whitespace() {
                out.push(flag.clone());
                out.push(elem.to_string());
            }
            return out;
        }
        return vec![flag.clone(), value.to_string()];
    }
    if let Some(prefix) = &opt.prefix_flag {
        return value
            .split_whitespace()
            .map(|elem| format!("{prefix}{elem}"))
            .collect();
    }
    Vec::new()
}

/// Substitute `$(value)` (and any `$(OTHER_SETTING)`) inside one xcspec arg
/// token, single-pass so a substituted value can't recurse.
fn subst(token: &str, value: &str, settings: &Settings) -> String {
    let mut out = String::with_capacity(token.len());
    let mut rest = token;
    while let Some(i) = rest.find("$(") {
        out.push_str(&rest[..i]);
        let after = &rest[i + 2..];
        let Some(j) = after.find(')') else {
            out.push_str(&rest[i..]);
            return out;
        };
        let var = &after[..j];
        if var == "value" {
            out.push_str(value);
        } else {
            out.push_str(settings.get(var).map_or("", String::as_str));
        }
        rest = &after[j + 1..];
    }
    out.push_str(rest);
    out
}

fn is_yes(v: &str) -> bool {
    v.eq_ignore_ascii_case("YES")
}

/// Whether an xcspec option's `Condition` predicate holds against the resolved
/// settings, deciding if the option contributes its argv. The grammar mirrors
/// Apple's macro-condition language: `$(VAR)` references, bare/quoted literals,
/// `==` / `!=` comparisons, `&&` / `||` / `!`, and parentheses. An undefined
/// `$(VAR)` is the empty string (Apple's semantics). Anything we cannot tokenize
/// or parse evaluates to `true`, so a malformed condition never silently drops a
/// real flag.
fn condition_holds(cond: &str, settings: &Settings) -> bool {
    let Some(toks) = tokenize_condition(cond) else {
        return true;
    };
    if toks.is_empty() {
        return true;
    }
    let mut p = CondParser {
        toks: &toks,
        pos: 0,
        settings,
    };
    match p.parse_or() {
        Some(v) if p.pos == toks.len() => v,
        _ => true,
    }
}

/// One lexeme of a `Condition` string.
#[derive(Debug, PartialEq, Eq)]
enum CondTok {
    LParen,
    RParen,
    Not,
    And,
    Or,
    Eq,
    Ne,
    /// `$(NAME)` — resolved against the settings when evaluated.
    Macro(String),
    /// A bare word or quoted string (`mh_object`, `'5'`, `""`), quotes stripped.
    Lit(String),
}

/// Lex a `Condition`. Returns `None` on a stray operator char so an
/// unrecognized condition falls back to always-applies rather than mis-parsing.
fn tokenize_condition(cond: &str) -> Option<Vec<CondTok>> {
    let bytes = cond.as_bytes();
    let mut toks = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b' ' | b'\t' | b'\n' | b'\r' => i += 1,
            b'(' => {
                toks.push(CondTok::LParen);
                i += 1;
            }
            b')' => {
                toks.push(CondTok::RParen);
                i += 1;
            }
            b'&' if bytes.get(i + 1) == Some(&b'&') => {
                toks.push(CondTok::And);
                i += 2;
            }
            b'|' if bytes.get(i + 1) == Some(&b'|') => {
                toks.push(CondTok::Or);
                i += 2;
            }
            b'=' if bytes.get(i + 1) == Some(&b'=') => {
                toks.push(CondTok::Eq);
                i += 2;
            }
            b'!' if bytes.get(i + 1) == Some(&b'=') => {
                toks.push(CondTok::Ne);
                i += 2;
            }
            b'!' => {
                toks.push(CondTok::Not);
                i += 1;
            }
            b'$' if bytes.get(i + 1) == Some(&b'(') => {
                let start = i + 2;
                let end = cond[start..].find(')')? + start;
                toks.push(CondTok::Macro(cond[start..end].to_string()));
                i = end + 1;
            }
            q @ (b'\'' | b'"') => {
                let start = i + 1;
                let end = bytes[start..].iter().position(|&b| b == q)? + start;
                toks.push(CondTok::Lit(cond[start..end].to_string()));
                i = end + 1;
            }
            _ => {
                let start = i;
                while i < bytes.len()
                    && !matches!(
                        bytes[i],
                        b' ' | b'\t'
                            | b'\n'
                            | b'\r'
                            | b'('
                            | b')'
                            | b'!'
                            | b'='
                            | b'&'
                            | b'|'
                            | b'\''
                            | b'"'
                            | b'$'
                    )
                {
                    i += 1;
                }
                if i == start {
                    return None;
                }
                toks.push(CondTok::Lit(cond[start..i].to_string()));
            }
        }
    }
    Some(toks)
}

/// Recursive-descent evaluator over [`CondTok`]s. Precedence: `!` binds tighter
/// than `&&`, which binds tighter than `||`; comparisons sit between a pair of
/// terms. Both sides of `&&` / `||` are always evaluated (no short-circuit) so
/// the whole token stream is consumed and validated.
struct CondParser<'a> {
    toks: &'a [CondTok],
    pos: usize,
    settings: &'a Settings,
}

impl CondParser<'_> {
    fn parse_or(&mut self) -> Option<bool> {
        let mut v = self.parse_and()?;
        while matches!(self.toks.get(self.pos), Some(CondTok::Or)) {
            self.pos += 1;
            v = self.parse_and()? || v;
        }
        Some(v)
    }

    fn parse_and(&mut self) -> Option<bool> {
        let mut v = self.parse_unary()?;
        while matches!(self.toks.get(self.pos), Some(CondTok::And)) {
            self.pos += 1;
            v = self.parse_unary()? && v;
        }
        Some(v)
    }

    fn parse_unary(&mut self) -> Option<bool> {
        let toks = self.toks;
        match toks.get(self.pos)? {
            CondTok::Not => {
                self.pos += 1;
                Some(!self.parse_unary()?)
            }
            CondTok::LParen => {
                self.pos += 1;
                let v = self.parse_or()?;
                if matches!(toks.get(self.pos), Some(CondTok::RParen)) {
                    self.pos += 1;
                    Some(v)
                } else {
                    None
                }
            }
            CondTok::Macro(_) | CondTok::Lit(_) => {
                let (lval, ltruthy) = self.take_term()?;
                match toks.get(self.pos) {
                    Some(CondTok::Eq) => {
                        self.pos += 1;
                        let (rval, _) = self.take_term()?;
                        Some(lval == rval)
                    }
                    Some(CondTok::Ne) => {
                        self.pos += 1;
                        let (rval, _) = self.take_term()?;
                        Some(lval != rval)
                    }
                    _ => Some(ltruthy),
                }
            }
            CondTok::RParen | CondTok::And | CondTok::Or | CondTok::Eq | CondTok::Ne => None,
        }
    }

    /// Consume a term, returning its resolved string value and truthiness (a
    /// `$(VAR)` resolves via the settings; a literal is its own text).
    fn take_term(&mut self) -> Option<(String, bool)> {
        let toks = self.toks;
        let settings = self.settings;
        let val = match toks.get(self.pos)? {
            CondTok::Macro(name) => settings.get(name).cloned().unwrap_or_default(),
            CondTok::Lit(s) => s.clone(),
            _ => return None,
        };
        self.pos += 1;
        let truthy = is_yes(&val);
        Some((val, truthy))
    }
}

/// Whitespace-split a setting value into non-empty tokens.
fn ws(v: Option<&str>) -> Vec<&str> {
    v.map(|s| s.split_whitespace().collect())
        .unwrap_or_default()
}

/// Split a **search-path** setting into the tokens the build system forms for
/// argv: whitespace-separated, but respecting double quotes (a path may contain
/// spaces) and with the quotes stripped. xcconfigs quote each path — CocoaPods
/// writes `FRAMEWORK_SEARCH_PATHS = $(inherited) "${PODS_CONFIGURATION_BUILD_DIR}/Pod"` —
/// and if the quotes survive into a `-F`/`-I` token, swiftc/clang read the quoted
/// string as a relative path and never find the framework/header. (Distinct from
/// [`ws`], which leaves quotes intact for value lists like preprocessor defines.)
fn ws_paths(v: Option<&str>) -> Vec<String> {
    let Some(s) = v else {
        return Vec::new();
    };
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut in_quote = false;
    let mut started = false;
    for ch in s.chars() {
        match ch {
            '"' => {
                in_quote = !in_quote;
                started = true;
            }
            c if c.is_whitespace() && !in_quote => {
                if started {
                    out.push(std::mem::take(&mut cur));
                    started = false;
                }
            }
            c => {
                cur.push(c);
                started = true;
            }
        }
    }
    if started {
        out.push(cur);
    }
    out.retain(|t| !t.is_empty());
    out
}

/// `5.0` → `5`, `6.0` → `6`; anything else (e.g. `4.2`) is passed through.
fn swift_version(v: &str) -> &str {
    v.strip_suffix(".0").unwrap_or(v)
}

/// Debug info is on unless the format is empty / explicitly none.
fn debug_info_enabled(fmt: Option<&str>) -> bool {
    matches!(fmt, Some(f) if !f.is_empty() && !f.eq_ignore_ascii_case("none"))
}

/// `SWIFT_UPCOMING_FEATURE_EXISTENTIAL_ANY` → `ExistentialAny`: split the
/// screaming-snake suffix and Title-case each word. Exact for the common
/// features; the xcspec ingest (Phase 4) supplies the authoritative names where
/// this heuristic (acronyms, digits) would diverge.
fn feature_name(suffix: &str) -> String {
    suffix
        .split('_')
        .map(|word| {
            let mut c = word.chars();
            match c.next() {
                Some(first) => {
                    first.to_ascii_uppercase().to_string() + &c.as_str().to_ascii_lowercase()
                }
                None => String::new(),
            }
        })
        .collect()
}

/// The `swiftc -target` triple: `<arch>-<vendor>-<os><suffix>`. Prefers the
/// resolved `LLVM_TARGET_TRIPLE_OS_VERSION` (e.g. `macos10.12`); falls back to
/// composing `SWIFT_PLATFORM_TARGET_PREFIX` + the platform's deployment target.
fn target_triple(settings: &Settings, arch: &str) -> Option<String> {
    let get = |k: &str| settings.get(k).map(String::as_str);
    let vendor = get("LLVM_TARGET_TRIPLE_VENDOR").unwrap_or("apple");
    let suffix = get("LLVM_TARGET_TRIPLE_SUFFIX").unwrap_or("");
    let os = if let Some(os) = get("LLVM_TARGET_TRIPLE_OS_VERSION") {
        os.to_string()
    } else {
        let prefix = get("SWIFT_PLATFORM_TARGET_PREFIX")?;
        let dep_key = get("DEPLOYMENT_TARGET_SETTING_NAME")?;
        let dep = get(dep_key)?;
        format!("{prefix}{dep}")
    };
    if arch.is_empty() || os.is_empty() {
        return None;
    }
    Some(format!("{arch}-{vendor}-{os}{suffix}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn swift_version_strips_dot_zero() {
        assert_eq!(swift_version("5.0"), "5");
        assert_eq!(swift_version("6.0"), "6");
        assert_eq!(swift_version("4.2"), "4.2");
    }

    #[test]
    fn feature_name_title_cases() {
        assert_eq!(feature_name("EXISTENTIAL_ANY"), "ExistentialAny");
        assert_eq!(
            feature_name("FORWARD_TRAILING_CLOSURES"),
            "ForwardTrailingClosures"
        );
    }

    #[test]
    fn triple_prefers_resolved_os_version() {
        let mut s = Settings::new();
        s.insert("LLVM_TARGET_TRIPLE_OS_VERSION".into(), "macos10.12".into());
        assert_eq!(
            target_triple(&s, "arm64").as_deref(),
            Some("arm64-apple-macos10.12")
        );
    }

    #[test]
    fn triple_composes_from_prefix_and_deployment() {
        let mut s = Settings::new();
        s.insert("SWIFT_PLATFORM_TARGET_PREFIX".into(), "ios".into());
        s.insert(
            "DEPLOYMENT_TARGET_SETTING_NAME".into(),
            "IPHONEOS_DEPLOYMENT_TARGET".into(),
        );
        s.insert("IPHONEOS_DEPLOYMENT_TARGET".into(), "17.0".into());
        s.insert("LLVM_TARGET_TRIPLE_SUFFIX".into(), "-simulator".into());
        assert_eq!(
            target_triple(&s, "arm64").as_deref(),
            Some("arm64-apple-ios17.0-simulator")
        );
    }

    #[test]
    fn emits_core_semantic_flags() {
        let mut s = Settings::new();
        s.insert("PRODUCT_MODULE_NAME".into(), "Alamofire".into());
        s.insert("SWIFT_OPTIMIZATION_LEVEL".into(), "-Onone".into());
        s.insert("SWIFT_VERSION".into(), "5.0".into());
        s.insert("SWIFT_ACTIVE_COMPILATION_CONDITIONS".into(), "DEBUG".into());
        // No spec options: exercises the hand-coded fallback path.
        let args = swift_arguments(&s, "arm64", &[], "26.5.0", false, &[]);
        let joined = args.join(" ");
        assert!(joined.contains("-module-name Alamofire"));
        assert!(joined.contains("-Onone"));
        assert!(joined.contains("-swift-version 5"));
        assert!(joined.contains("-DDEBUG"));
        assert!(args.contains(&"-c".to_string()));
    }

    #[test]
    fn spec_driven_optimization_and_conditions() {
        use crate::xcspec::CliArgs;
        let opt_level = CompilerOption {
            name: "SWIFT_OPTIMIZATION_LEVEL".into(),
            is_list: false,
            flag: None,
            prefix_flag: None,
            args: Some(CliArgs::ByValue {
                map: BTreeMap::from([(
                    "-Owholemodule".into(),
                    vec!["-O".into(), "-whole-module-optimization".into()],
                )]),
                otherwise: Some(vec!["$(value)".into()]),
            }),
            file_types: vec![],
            architectures: vec![],
            condition: None,
        };
        let conds = CompilerOption {
            name: "SWIFT_ACTIVE_COMPILATION_CONDITIONS".into(),
            is_list: true,
            flag: None,
            prefix_flag: None,
            args: Some(CliArgs::List(vec!["-D$(value)".into()])),
            file_types: vec![],
            architectures: vec![],
            condition: None,
        };
        let mut s = Settings::new();
        s.insert("SWIFT_OPTIMIZATION_LEVEL".into(), "-Owholemodule".into());
        s.insert(
            "SWIFT_ACTIVE_COMPILATION_CONDITIONS".into(),
            "DEBUG COCOAPODS".into(),
        );
        let args = swift_arguments(&s, "arm64", &[opt_level, conds], "26.5.0", false, &[]);
        // The enum's special-cased `-Owholemodule` expands; conditions become -D.
        assert!(
            args.windows(2)
                .any(|w| w == ["-O", "-whole-module-optimization"])
        );
        assert!(args.contains(&"-DDEBUG".to_string()));
        assert!(args.contains(&"-DCOCOAPODS".to_string()));
    }

    #[test]
    fn condition_grammar() {
        let mut s = Settings::new();
        s.insert("ON".into(), "YES".into());
        s.insert("OFF".into(), "NO".into());
        s.insert("DRIVER".into(), "clang".into());
        s.insert("MACH".into(), "mh_execute".into());
        s.insert("PKG".into(), String::new());
        let h = |c: &str| condition_holds(c, &s);

        // Bare `$(VAR)` truthiness; an undefined macro is empty (false).
        assert!(h("$(ON)"));
        assert!(!h("$(OFF)"));
        assert!(!h("$(UNDEFINED)"));
        // Negation.
        assert!(h("!$(OFF)"));
        assert!(!h("! $(ON)"));
        // Comparisons against bare and quoted literals.
        assert!(h("$(DRIVER) == clang"));
        assert!(!h("$(DRIVER) == swiftc"));
        assert!(h("$(MACH) != mh_object"));
        assert!(h("$(PKG) == \"\""));
        assert!(h("$(UNDEFINED) == ''"));
        // Boolean composition and precedence (`&&` over `||`, parens override).
        assert!(h("$(ON) && $(DRIVER) == clang"));
        assert!(!h("$(ON) && $(OFF)"));
        assert!(h("$(OFF) || $(ON)"));
        assert!(h("$(OFF) || ($(ON) && $(DRIVER) == clang)"));
        assert!(!h("($(OFF) || $(UNDEFINED)) && $(ON)"));
        // The real UBSan gate and a constant-false literal.
        assert!(!h("$(CLANG_UNDEFINED_BEHAVIOR_SANITIZER)"));
        assert!(!h("NO"));
        // An unparseable condition falls back to applies (never drops a flag).
        assert!(h("$(ON) ^^ garbage"));
        assert!(h(""));
    }

    #[test]
    fn clang_skips_condition_failed_option() {
        use crate::xcspec::CliArgs;
        // `CLANG_UNDEFINED_BEHAVIOR_SANITIZER_INTEGER = YES` would emit
        // `-fsanitize=integer`, but the option is gated on the parent sanitizer.
        let ubsan_integer = CompilerOption {
            name: "CLANG_UNDEFINED_BEHAVIOR_SANITIZER_INTEGER".into(),
            is_list: false,
            flag: None,
            prefix_flag: None,
            args: Some(CliArgs::ByValue {
                map: BTreeMap::from([("YES".into(), vec!["-fsanitize=integer".into()])]),
                otherwise: Some(vec![]),
            }),
            file_types: vec![],
            architectures: vec![],
            condition: Some("$(CLANG_UNDEFINED_BEHAVIOR_SANITIZER)".into()),
        };
        let langs = BTreeSet::from(["sourcecode.c.objc".to_string()]);

        let mut off = Settings::new();
        off.insert(
            "CLANG_UNDEFINED_BEHAVIOR_SANITIZER_INTEGER".into(),
            "YES".into(),
        );
        off.insert("CLANG_UNDEFINED_BEHAVIOR_SANITIZER".into(), "NO".into());
        let args = clang_arguments(&off, "arm64", std::slice::from_ref(&ubsan_integer), &langs);
        assert!(
            !args.contains(&"-fsanitize=integer".to_string()),
            "condition off must drop the flag: {args:?}"
        );

        // Turning the parent sanitizer on lets the same option through.
        let mut on = off.clone();
        on.insert("CLANG_UNDEFINED_BEHAVIOR_SANITIZER".into(), "YES".into());
        let args = clang_arguments(&on, "arm64", std::slice::from_ref(&ubsan_integer), &langs);
        assert!(args.contains(&"-fsanitize=integer".to_string()), "{args:?}");
    }

    #[test]
    fn macro_plugins_emit_load_plugin_executable() {
        // A package's executable macro plugin resolves only via the explicit
        // `-Xfrontend -load-plugin-executable -Xfrontend <plugin>#<module>` form.
        let s = Settings::new();
        let plugins = [PathBuf::from("/dd/Build/Products/Debug/MyMacros")];
        let args = swift_arguments(&s, "arm64", &[], "26.5.0", false, &plugins);
        let joined = args.join(" ");
        assert!(
            args.iter().any(|a| a == "-load-plugin-executable"),
            "macro plugin not loaded: {args:?}"
        );
        assert!(
            joined.contains("/dd/Build/Products/Debug/MyMacros#MyMacros"),
            "plugin#module form missing: {joined}"
        );
    }

    #[test]
    fn package_products_emit_packageframeworks_search_path() {
        // Dynamic Swift-package products build into a `PackageFrameworks` subdir;
        // a target consuming package products needs that `-F` to import them.
        let mut s = Settings::new();
        s.insert(
            "BUILT_PRODUCTS_DIR".into(),
            "/dd/Build/Products/Debug".into(),
        );
        let args = swift_arguments(&s, "arm64", &[], "26.5.0", true, &[]);
        assert!(
            args.windows(2)
                .any(|w| w[0] == "-F" && w[1] == "/dd/Build/Products/Debug/PackageFrameworks"),
            "no -F PackageFrameworks: {args:?}"
        );
    }
}
