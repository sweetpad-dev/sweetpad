//! Build-settings resolution orchestration, shared by the CLI (`main.rs`) and
//! the N-API node bindings (`node.rs`). Takes a project/workspace selector plus
//! a scheme or target and returns each resolved target's settings — the same
//! pipeline `xcodebuild -showBuildSettings` drives. Output formatting and
//! destination-string parsing stay with the callers.

use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};

use crate::build_context::{BuildContext, MacVariant, ResolveQuery};
use sweetpad_lib::destination::RunDestination;
use sweetpad_lib::xcspec::Catalog;
use sweetpad_lib::{catalog_cache, compiler_args, project, scheme, workspace, xcode};

/// Inputs for a `build-settings` run — the resolved form of the CLI's
/// `BuildSettingsArgs` / a `xcodebuild -showBuildSettings` invocation. The
/// `destination` is pre-parsed by the caller so each surface keeps its own
/// error wording.
#[derive(Default)]
pub struct BuildSettingsOptions {
    /// `.xcodeproj` path. Mutually exclusive with `workspace`.
    pub project: Option<PathBuf>,
    /// `.xcworkspace` path. Mutually exclusive with `project`.
    pub workspace: Option<PathBuf>,
    /// Scheme name — resolves every buildable in its BuildAction.
    pub scheme: Option<String>,
    /// Target name. Mutually exclusive with `scheme`.
    pub target: Option<String>,
    /// Configuration (e.g. `Debug`, `Release`).
    pub configuration: String,
    /// SDK to bind conditionals to (ignored when `destination` is set).
    pub sdk: String,
    /// Arch to bind conditionals to (ignored when `destination` is set).
    pub arch: String,
    /// Pre-parsed run destination (the destination's platform/arch win over
    /// `sdk`/`arch` when present).
    pub destination: Option<RunDestination>,
    /// Extra `.xcconfig` overlay (`xcodebuild -xcconfig`).
    pub xcconfig: Option<PathBuf>,
    /// Resolve against a specific `Xcode.app` / `Contents/Developer`.
    pub xcode: Option<PathBuf>,
    /// Directory of Apple's `*.xcspec` files (low-level alternative to `xcode`).
    pub xcspec_root: Option<PathBuf>,
    /// Directory of per-SDK `SDKSettings.plist`.
    pub sdksettings_root: Option<PathBuf>,
    /// Cache file for a parsed `xcspec_root` catalog.
    pub catalog_cache: Option<PathBuf>,
    /// `xcodebuild -derivedDataPath` override.
    pub derived_data_path: Option<PathBuf>,
    /// `KEY=VALUE` build settings from the `xcodebuild` command line, applied
    /// above every other layer, in order, as `xcodebuild` applies them.
    pub overrides: Vec<(String, String)>,
    /// Place build output the way this machine's Xcode is configured to,
    /// instead of assuming the stock DerivedData layout (see
    /// [`sweetpad_lib::derived_data`]). Interfaces that go on to install,
    /// launch, or index the product set this so they look where `xcodebuild`
    /// actually wrote; the capture-backed suites leave it off so resolution
    /// stays independent of the host.
    pub read_xcode_locations: bool,
    /// When set, restrict each target's returned settings to these keys.
    /// Resolution is unchanged — the whole map is still computed, since settings
    /// reference each other via `$(…)` — but the *output* is trimmed, so a
    /// caller that needs a handful of keys doesn't carry the full (~1.4k-entry)
    /// map. `None` returns every resolved key.
    pub keys: Option<Vec<String>>,
}

/// One target's resolved build settings.
#[derive(Debug, Clone)]
pub struct TargetSettings {
    pub target: String,
    pub settings: BTreeMap<String, String>,
}

/// What the `scheme` / `target` selector resolved to, decided once up front
/// (see [`resolve_selection`]) and then planned against each project.
enum Selection {
    /// A parsed `.xcscheme` found in the workspace bundle or a member project
    /// (shared or per-user). Plans one query per BuildAction buildable, each
    /// resolved in the member project that owns it.
    Scheme {
        name: String,
        // Boxed: a parsed Scheme dwarfs the other variants' Strings.
        parsed: Box<scheme::Scheme>,
    },
    /// An explicit `target` query.
    Target(String),
    /// `scheme` was requested but no `.xcscheme` file exists anywhere —
    /// Xcode's autocreated per-target scheme, which builds the same-named
    /// target.
    AutoScheme(String),
}

impl Selection {
    /// What the final "nothing matched" error should call the selector.
    fn describe(&self) -> String {
        match self {
            Selection::Scheme { name, .. } | Selection::AutoScheme(name) => {
                format!("scheme {name}")
            }
            Selection::Target(t) => format!("target {t}"),
        }
    }

    /// Target-style queries stop at the first project that resolves them;
    /// scheme queries visit every member project (a workspace scheme can
    /// build targets across several).
    fn stops_at_first_match(&self) -> bool {
        !matches!(self, Selection::Scheme { .. })
    }
}

/// Resolve the `scheme` / `target` choice once, before the per-project loop.
/// A scheme file is looked up where `xcodebuild` finds it
/// ([`scheme::locate`]): the workspace bundle itself, then each member
/// project, then each local package, shared then the current user's
/// directories. A scheme with no
/// file falls back to [`Selection::AutoScheme`] only when Xcode's scheme
/// autocreation surfaces it: when the container lists it the way
/// `xcodebuild -list` does ([`project::Project::schemes`],
/// [`workspace::Workspace::project_for_scheme`]), which autocreates per
/// target rather than per container. Otherwise the unknown name is an
/// error, matching `xcodebuild -scheme Nope`.
fn resolve_selection(
    opts: &BuildSettingsOptions,
    projects: &[PathBuf],
) -> Result<Selection, String> {
    if let Some(name) = opts.scheme.as_deref() {
        let containers: Vec<&Path> = opts
            .workspace
            .as_deref()
            .into_iter()
            .chain(projects.iter().map(PathBuf::as_path))
            .collect();
        if let Some(path) = containers
            .first()
            .and_then(|primary| scheme::locate(primary, name))
        {
            let parsed = scheme::parse_file(&path)
                .map_err(|e| format!("failed to parse scheme {name} at {}: {e}", path.display()))?;
            return Ok(Selection::Scheme {
                name: name.to_string(),
                parsed: Box::new(parsed),
            });
        }
        let listed = match opts.workspace.as_deref() {
            Some(ws) => workspace::open(ws).is_ok_and(|ws| ws.project_for_scheme(name).is_some()),
            None => projects.first().is_some_and(|p| {
                project::open(p).is_ok_and(|project| project.schemes.iter().any(|s| s == name))
            }),
        };
        if !listed {
            return Err(format!(
                "the workspace/project does not contain a scheme named {name:?}"
            ));
        }
        return Ok(Selection::AutoScheme(name.to_string()));
    }
    if let Some(target) = opts.target.as_deref() {
        return Ok(Selection::Target(target.to_string()));
    }
    Err("either scheme or target is required".into())
}

/// Resolve build settings for the selected scheme/target across the supplied
/// project/workspace. Mirrors `xcodebuild -showBuildSettings`.
pub fn resolve_build_settings(opts: &BuildSettingsOptions) -> Result<Vec<TargetSettings>, String> {
    let catalog = load_catalog(
        opts.xcode.as_deref(),
        opts.xcspec_root.as_deref(),
        opts.sdksettings_root.as_deref(),
        opts.catalog_cache.as_deref(),
    )?;
    let projects = resolve_project_paths(opts.project.as_deref(), opts.workspace.as_deref())?;
    let selection = resolve_selection(opts, &projects)?;

    if projects.len() == 1 {
        // Single project: bubble up the underlying error directly so callers
        // get xcodebuild-equivalent messages ("no target named …").
        let ctx = build_one_context(
            &projects[0],
            catalog.as_ref(),
            opts.xcconfig.as_deref(),
            opts.workspace.as_deref(),
            opts.read_xcode_locations,
        )?;
        let queries = build_queries(&ctx, opts, &selection);
        let mut out = Vec::new();
        for query in queries {
            let resolved = ctx.resolve(&query).map_err(|e| e.to_string())?;
            out.push(TargetSettings {
                target: query.target.clone(),
                settings: resolved.settings,
            });
        }
        // Zero queries (a scheme whose entries are all testing-only, or whose
        // container references another project) must error like the workspace
        // branch does — an empty Ok would surface downstream as a misleading
        // "no app bundle"-style failure instead of naming the real cause.
        if out.is_empty() {
            return Err(format!("no target matched {}", selection.describe()));
        }
        project_keys(&mut out, opts.keys.as_deref());
        Ok(out)
    } else {
        // Workspace: try every project until queries resolve. Only a *lookup
        // miss* ("no target named …" / no usable configuration) means the
        // target lives in another member and is suppressed; a member that is
        // genuinely broken (unreadable or malformed xcconfig, parse failure)
        // surfaces immediately, tagged with the member project's path —
        // otherwise it would masquerade as a misleading "no target matched".
        let mut out = Vec::new();
        for project_path in &projects {
            let ctx = build_one_context(
                project_path,
                catalog.as_ref(),
                opts.xcconfig.as_deref(),
                opts.workspace.as_deref(),
                opts.read_xcode_locations,
            )?;
            let queries = build_queries(&ctx, opts, &selection);
            for query in queries {
                match ctx.resolve(&query) {
                    Ok(r) => out.push(TargetSettings {
                        target: query.target.clone(),
                        settings: r.settings,
                    }),
                    Err(e) if e.is_lookup_miss() => {}
                    Err(e) => return Err(format!("{}: {e}", project_path.display())),
                }
            }
            if !out.is_empty() && selection.stops_at_first_match() {
                break;
            }
        }
        if out.is_empty() {
            return Err(format!(
                "no target matched {} across the supplied workspace",
                selection.describe()
            ));
        }
        project_keys(&mut out, opts.keys.as_deref());
        Ok(out)
    }
}

/// Resolve the per-tool compiler/linker argv for a scheme or target — the
/// layer above [`resolve_build_settings`]. For each selected target it resolves
/// the build settings, reads the target's source files, and generates the
/// `swiftc` / `clang` / link argv (see [`compiler_args`]).
pub fn resolve_compiler_arguments(
    opts: &BuildSettingsOptions,
) -> Result<Vec<compiler_args::TargetCompilerArguments>, String> {
    let catalog = load_catalog(
        opts.xcode.as_deref(),
        opts.xcspec_root.as_deref(),
        opts.sdksettings_root.as_deref(),
        opts.catalog_cache.as_deref(),
    )?;
    let projects = resolve_project_paths(opts.project.as_deref(), opts.workspace.as_deref())?;
    let selection = resolve_selection(opts, &projects)?;

    let swift_opts = catalog
        .as_ref()
        .and_then(|c| {
            c.compiler_options
                .get("com.apple.xcode.tools.swift.compiler")
        })
        .map_or(&[][..], Vec::as_slice);
    let clang_opts = catalog
        .as_ref()
        .and_then(|c| c.compiler_options.get("com.apple.compilers.llvm.clang.1_0"))
        .map_or(&[][..], Vec::as_slice);
    let xcode_version = catalog
        .as_ref()
        .and_then(|c| c.xcode_version.as_deref())
        .unwrap_or("");

    let single = projects.len() == 1;
    let mut out = Vec::new();
    for project_path in &projects {
        let ctx = build_one_context(
            project_path,
            catalog.as_ref(),
            opts.xcconfig.as_deref(),
            opts.workspace.as_deref(),
            opts.read_xcode_locations,
        )?;
        for query in build_queries(&ctx, opts, &selection) {
            // Compiler argv is per concrete arch, so `[arch=…]` conditionals
            // fire — unlike the showBuildSettings emulation, which binds
            // xcodebuild's aggregated `undefined_arch` view.
            let query = query.with_per_arch_conditionals(true);
            match ctx.resolve(&query) {
                Ok(resolved) => {
                    let sources = project::target_source_files(&ctx.project.path, &query.target)
                        .unwrap_or_default();
                    let frameworks =
                        project::target_linked_frameworks(&ctx.project.path, &query.target)
                            .unwrap_or_default();
                    let has_package_products =
                        project::target_has_package_products(&ctx.project.path, &query.target)
                            .unwrap_or(false);
                    let macro_plugins = if has_package_products {
                        collect_macro_plugins(&resolved.settings)
                    } else {
                        Vec::new()
                    };
                    let libraries =
                        project::target_linked_libraries(&ctx.project.path, &query.target)
                            .unwrap_or_default();
                    out.push(compiler_args::target_arguments(
                        &query.target,
                        &resolved.settings,
                        &query.arch,
                        resolved.product_type.as_deref(),
                        &sources,
                        &frameworks,
                        &libraries,
                        swift_opts,
                        clang_opts,
                        xcode_version,
                        has_package_products,
                        &macro_plugins,
                    ));
                }
                // Single project: bubble the error (xcodebuild-equivalent wording).
                // Workspace: a lookup miss means the target just isn't in this
                // member, keep trying — but a broken member (unreadable or
                // malformed xcconfig, parse failure) is a real error and
                // surfaces with the member project's path.
                Err(e) if single => return Err(e.to_string()),
                Err(e) if e.is_lookup_miss() => {}
                Err(e) => return Err(format!("{}: {e}", project_path.display())),
            }
        }
        if !out.is_empty() && selection.stops_at_first_match() {
            break;
        }
    }
    if out.is_empty() {
        return Err(format!("no target matched {}", selection.describe()));
    }
    Ok(out)
}

/// Resolve the compiler arguments for a **single file** in `opts.target` — the
/// per-file invocation an editor / BSP server wants, where a clang file is gated
/// to its own language (a `.m` gets ObjC flags + `-x objective-c`; a `.mm` gets
/// C++/ObjC++), not the per-target union. A `.swift` file resolves to the whole
/// module's swiftc invocation (Swift type-checks a module at once).
pub fn resolve_file_arguments(
    opts: &BuildSettingsOptions,
    file: &Path,
) -> Result<compiler_args::ToolInvocation, String> {
    let target = opts
        .target
        .as_deref()
        .ok_or("resolve_file_arguments: a target is required")?;
    let catalog = load_catalog(
        opts.xcode.as_deref(),
        opts.xcspec_root.as_deref(),
        opts.sdksettings_root.as_deref(),
        opts.catalog_cache.as_deref(),
    )?;
    let swift_opts = catalog
        .as_ref()
        .and_then(|c| {
            c.compiler_options
                .get("com.apple.xcode.tools.swift.compiler")
        })
        .map_or(&[][..], Vec::as_slice);
    let clang_opts = catalog
        .as_ref()
        .and_then(|c| c.compiler_options.get("com.apple.compilers.llvm.clang.1_0"))
        .map_or(&[][..], Vec::as_slice);
    let xcode_version = catalog
        .as_ref()
        .and_then(|c| c.xcode_version.as_deref())
        .unwrap_or("");
    let projects = resolve_project_paths(opts.project.as_deref(), opts.workspace.as_deref())?;

    let selection = Selection::Target(target.to_string());
    let single = projects.len() == 1;
    for project_path in &projects {
        let ctx = build_one_context(
            project_path,
            catalog.as_ref(),
            opts.xcconfig.as_deref(),
            opts.workspace.as_deref(),
            opts.read_xcode_locations,
        )?;
        let Some(query) = build_queries(&ctx, opts, &selection).into_iter().next() else {
            continue;
        };
        // Per-file compile: the concrete arch binds `[arch=…]` conditionals
        // (the showBuildSettings emulation binds `undefined_arch` instead).
        let query = query.with_per_arch_conditionals(true);
        let resolved = match ctx.resolve(&query) {
            Ok(resolved) => resolved,
            // The target lives in another member project; keep trying.
            Err(e) if !single && e.is_lookup_miss() => continue,
            // A broken member project is a real error, not "target elsewhere".
            Err(e) if single => return Err(e.to_string()),
            Err(e) => return Err(format!("{}: {e}", project_path.display())),
        };
        let settings = &resolved.settings;
        let file_str = file.to_string_lossy().into_owned();
        if file.extension().is_some_and(|e| e == "swift") {
            let mut swift_inputs: Vec<String> =
                project::target_source_files(&ctx.project.path, target)
                    .unwrap_or_default()
                    .into_iter()
                    .filter(|p| p.extension().is_some_and(|e| e == "swift"))
                    .map(|p| p.to_string_lossy().into_owned())
                    .collect();
            // Build-time-generated Swift (Core Data subclasses, asset symbols,
            // intent classes, string-catalog symbols, build-rule output) is part
            // of the module but absent from the project graph — it lives under
            // DERIVED_SOURCES_DIR. Without it in the input set, the editor can't
            // resolve references to those generated symbols. A no-op until a
            // build has populated the dir.
            if let Some(derived) = settings.get("DERIVED_SOURCES_DIR") {
                collect_generated_swift(Path::new(derived), &mut swift_inputs);
            }
            let has_package_products =
                project::target_has_package_products(&ctx.project.path, target).unwrap_or(false);
            let macro_plugins = if has_package_products {
                collect_macro_plugins(settings)
            } else {
                Vec::new()
            };
            return Ok(compiler_args::ToolInvocation {
                tool: "swiftc".into(),
                arguments: compiler_args::swift_arguments(
                    settings,
                    &query.arch,
                    &swift_inputs,
                    swift_opts,
                    xcode_version,
                    has_package_products,
                    &macro_plugins,
                ),
                input_files: swift_inputs,
            });
        }
        // A clang file: gate options to this one file's language.
        let langs = compiler_args::clang_languages(std::slice::from_ref(&file_str));
        return Ok(compiler_args::ToolInvocation {
            tool: "clang".into(),
            arguments: compiler_args::clang_arguments(settings, &query.arch, clang_opts, &langs),
            input_files: vec![file_str],
        });
    }
    Err(format!("no project contained target {target}"))
}

/// Recursively append every `.swift` under `dir` to `out` — the build-time
/// generated sources xcodebuild deposits in `DERIVED_SOURCES_DIR` (and its
/// `CoreDataGenerated` / `IntentDefinitionGenerated` subdirs). A no-op when the
/// directory doesn't exist yet (no prior build).
fn collect_generated_swift(dir: &Path, out: &mut Vec<String>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_generated_swift(&path, out);
        } else if path.extension().is_some_and(|e| e == "swift") {
            out.push(path.to_string_lossy().into_owned());
        }
    }
}

/// Built Swift-macro plugin executables a package graph drops in the host
/// products dir (`$(BUILD_DIR)/$(CONFIGURATION)` — the macOS variant, since macro
/// plugins are host tools whatever the app's platform). Xcode hands each to the
/// frontend as `-load-plugin-executable <exe>#<module>`; the editor resolves a
/// `#externalMacro(module:)` reference only when we pass the same. The plugin is
/// an extension-less host Mach-O executable whose basename is the macro module
/// name. Empty until a build has populated the dir — the prepared-build
/// assumption the package-framework search path already relies on.
fn collect_macro_plugins(settings: &BTreeMap<String, String>) -> Vec<PathBuf> {
    let (Some(build_dir), Some(config)) =
        (settings.get("BUILD_DIR"), settings.get("CONFIGURATION"))
    else {
        return Vec::new();
    };
    let Ok(entries) = std::fs::read_dir(Path::new(build_dir).join(config)) else {
        return Vec::new();
    };
    let mut plugins: Vec<PathBuf> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| is_macro_plugin_executable(p))
        .collect();
    plugins.sort();
    plugins
}

/// Whether `path` is a host compiler-plugin executable: an extension-less,
/// user-executable Mach-O file. The host products dir also holds the plugin's
/// `.o` / `.swiftmodule` build products and a `PackageFrameworks` directory —
/// all carry an extension or aren't regular files, so they're skipped. A
/// non-macro host tool that slips through is harmless: `-load-plugin-executable`
/// is consulted lazily, only when a macro from that module is actually expanded.
fn is_macro_plugin_executable(path: &Path) -> bool {
    use std::io::Read;
    use std::os::unix::fs::PermissionsExt;
    if path.extension().is_some() {
        return false;
    }
    let Ok(meta) = path.metadata() else {
        return false;
    };
    if !meta.is_file() || meta.permissions().mode() & 0o111 == 0 {
        return false;
    }
    let Ok(mut file) = std::fs::File::open(path) else {
        return false;
    };
    let mut magic = [0u8; 4];
    if file.read_exact(&mut magic).is_err() {
        return false;
    }
    // Mach-O magic, either byte order (thin 32/64-bit or fat).
    matches!(
        u32::from_be_bytes(magic),
        0xFEED_FACE | 0xFEED_FACF | 0xCEFA_EDFE | 0xCFFA_EDFE | 0xCAFE_BABE | 0xBEBA_FECA
    )
}

/// Trim each target's settings to `keys` when a projection is requested. Keys
/// that didn't resolve are simply absent (never inserted empty); `None` leaves
/// the full map untouched.
fn project_keys(out: &mut [TargetSettings], keys: Option<&[String]>) {
    let Some(keys) = keys else { return };
    let wanted: HashSet<&str> = keys.iter().map(String::as_str).collect();
    for target in out.iter_mut() {
        target.settings.retain(|k, _| wanted.contains(k.as_str()));
    }
}

/// The defaults catalog for a `build-settings` run. Precedence:
///
/// 1. `xcode`: discover the spec + SDK roots inside that specific Xcode.
/// 2. `xcspec_root`: parse that tree directly (cached via a stat fingerprint).
/// 3. Neither: resolve against the **active** Xcode (`DEVELOPER_DIR` /
///    `xcode-select -p`), matching what `xcodebuild` does — falling back to the
///    catalog baked into the binary only if no Xcode can be located/parsed.
fn load_catalog(
    xcode: Option<&Path>,
    xcspec_root: Option<&Path>,
    sdksettings_root: Option<&Path>,
    catalog_cache: Option<&Path>,
) -> Result<Option<Catalog>, String> {
    let catalog = if let Some(xcode_path) = xcode {
        catalog_from_xcode(xcode_path, catalog_cache)?
    } else if let Some(xcspec_dir) = xcspec_root {
        catalog_cache::load_cached_or_build(xcspec_dir, sdksettings_root, catalog_cache)
            .map_err(|e| e.to_string())?
    } else {
        // No explicit source: resolve against the active Xcode, falling back to
        // the embedded snapshot if it can't be located/parsed (e.g. no Xcode).
        match catalog_from_xcode(&xcode::detect_developer_dir(), catalog_cache) {
            Ok(catalog) => catalog,
            Err(_) => catalog_cache::embedded().map_err(|e| e.to_string())?,
        }
    };
    Ok(Some(catalog))
}

/// Build the defaults catalog from a specific Xcode install: discover its spec +
/// SDK roots, parse them (cached, keyed by build version), and stamp the catalog
/// with that install's `DEVELOPER_DIR` + version so it resolves
/// self-consistently regardless of which Xcode is `xcode-select`ed.
fn catalog_from_xcode(xcode_path: &Path, catalog_cache: Option<&Path>) -> Result<Catalog, String> {
    let layout = xcode::locate(xcode_path)?;
    let mut catalog = catalog_cache::load_cached_or_build_keyed(
        &layout.xcspec_root,
        Some(&layout.sdksettings_root),
        catalog_cache,
        &layout.cache_key(),
    )
    .map_err(|e| e.to_string())?;
    catalog.developer_dir = Some(layout.developer_dir.to_string_lossy().into_owned());
    if !layout.short_version.is_empty() {
        catalog.xcode_version = Some(layout.short_version);
    }
    Ok(catalog)
}

/// Resolve the `project` / `workspace` selector to the concrete list of
/// `.xcodeproj` paths to try in order.
fn resolve_project_paths(
    project: Option<&Path>,
    workspace_path: Option<&Path>,
) -> Result<Vec<PathBuf>, String> {
    if let Some(ws_path) = workspace_path {
        let ws = workspace::open(ws_path).map_err(|e| e.to_string())?;
        Ok(ws.project_refs)
    } else if let Some(p) = project {
        Ok(vec![p.to_path_buf()])
    } else {
        Err("either project or workspace is required".into())
    }
}

fn build_one_context(
    project_path: &Path,
    catalog: Option<&Catalog>,
    extra_xcconfig: Option<&Path>,
    // The `.xcworkspace` the caller drove this resolution through, if any.
    workspace: Option<&Path>,
    read_xcode_locations: bool,
) -> Result<BuildContext, String> {
    let mut ctx = BuildContext::open(project_path).map_err(|e| e.to_string())?;
    if read_xcode_locations {
        ctx = ctx.with_xcode_locations();
    }
    if let Some(c) = catalog {
        ctx = ctx.with_xcspec(c.clone());
    }
    if let Some(p) = extra_xcconfig {
        ctx = ctx.with_extra_xcconfig(p).map_err(|e| e.to_string())?;
    }
    // Xcode keys DerivedData by whichever container it opened. When the caller
    // selected a `-workspace`, every member project's DerivedData paths
    // (`BUILD_DIR`, `OBJROOT`, `BUILT_PRODUCTS_DIR`, …) must hash the WORKSPACE
    // path, not the project's own location, or the built app can't be found
    // (issue #265). With no workspace the context hashes the project, as a
    // `-project` build does.
    if let Some(ws) = workspace {
        ctx = ctx.with_derived_data_container(ws);
    }
    Ok(ctx)
}

/// Turn the resolved [`Selection`] into the [`ResolveQuery`]s for one project
/// context. A scheme plans one query per BuildAction buildable owned by this
/// project (cross-container buildables resolve when the caller visits their
/// own project); a target — explicit or an autocreated scheme's — is a single
/// direct query.
fn build_queries(
    ctx: &BuildContext,
    opts: &BuildSettingsOptions,
    selection: &Selection,
) -> Vec<ResolveQuery> {
    let destination = opts.destination.as_ref();
    // A bound destination supplies the SDK + active arch (mirroring xcodebuild,
    // where `-destination` implies them); otherwise fall back to `sdk`/`arch`.
    let (sdk, arch) = match destination {
        Some(d) => (d.platform.as_str(), d.arch.as_str()),
        None => (opts.sdk.as_str(), opts.arch.as_str()),
    };
    let mut queries = Vec::new();
    match selection {
        Selection::Scheme { parsed, .. } => {
            // `-showBuildSettings` (and plain `build`) is the Run action's
            // build set; testing-only entries are excluded.
            let plan = ctx.plan_build(
                parsed,
                scheme::BuildFor::Running,
                &opts.configuration,
                sdk,
                arch,
                destination,
            );
            queries.extend(plan.entries);
        }
        Selection::Target(target_name) | Selection::AutoScheme(target_name) => {
            let mut q = ResolveQuery::new(target_name, &opts.configuration, sdk, arch);
            if let Some(d) = destination {
                q = q.with_destination(d.clone());
            }
            queries.push(q);
        }
    }
    let mut queries: Vec<ResolveQuery> = queries
        .into_iter()
        .map(|mut q| {
            if let Some(p) = &opts.derived_data_path {
                q = q.with_derived_data_path(p.clone());
            }
            for (key, value) in &opts.overrides {
                q = q.with_override(key, value);
            }
            q
        })
        .collect();
    if destination.is_some() {
        bind_destination_sdks(ctx, &mut queries);
    }
    queries
}

/// Bind each query's SDK the way xcodebuild specializes a build's targets
/// for its one run destination, which [`build_queries`] first binds every
/// query to. On a macOS destination an iOS target builds for Mac Catalyst or
/// "Designed for iPad" on `iphoneos` ([`BuildContext::mac_destination_variant`]).
/// xcodebuild picks that destination for the whole build, from its apps, so a
/// framework the app embeds builds for `iphoneos` too, although on its own it
/// would take Catalyst; with no app among the queries, each target's own
/// variant decides. That destination is an `iphoneos` one to the scheme's
/// other targets, so a macOS app beside the iOS app builds as it would for an
/// iOS device (full `ARCHS`, `ONLY_ACTIVE_ARCH = NO`). Any other target that
/// can't run on the destination builds for its own platform
/// ([`BuildContext::own_platform_sdk`]).
fn bind_destination_sdks(ctx: &BuildContext, queries: &mut [ResolveQuery]) {
    let variants: Vec<Option<MacVariant>> = queries
        .iter()
        .map(|q| {
            if q.destination.as_ref().is_some_and(RunDestination::is_macos) {
                ctx.mac_destination_variant(q).ok().flatten()
            } else {
                None
            }
        })
        .collect();
    let is_app = |target: &str| {
        ctx.project.targets.iter().any(|t| {
            t.name == target
                && t.product_type.as_deref() == Some("com.apple.product-type.application")
        })
    };
    let apps: Vec<MacVariant> = queries
        .iter()
        .zip(&variants)
        .filter(|(q, _)| is_app(&q.target))
        .filter_map(|(_, v)| *v)
        .collect();
    let designed_app = !apps.is_empty() && apps.iter().all(|v| *v == MacVariant::DesignedForIpad);
    for (q, variant) in queries.iter_mut().zip(variants) {
        if let Some(variant) = variant {
            let designed = if apps.is_empty() {
                variant == MacVariant::DesignedForIpad
            } else {
                designed_app
            };
            if designed {
                q.sdk = "iphoneos".into();
            }
            continue;
        }
        if designed_app && let Some(destination) = &mut q.destination {
            destination.platform = "iphoneos".into();
        }
        if let Ok(Some(sdk)) = ctx.own_platform_sdk(q) {
            q.sdk = sdk;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::os::unix::fs::PermissionsExt;

    use crate::scratch::ScratchDir;

    fn write_file(path: &Path, bytes: &[u8], exec: bool) {
        std::fs::File::create(path)
            .unwrap()
            .write_all(bytes)
            .unwrap();
        if exec {
            let mut perm = std::fs::metadata(path).unwrap().permissions();
            perm.set_mode(0o755);
            std::fs::set_permissions(path, perm).unwrap();
        }
    }

    /// MH_MAGIC_64 as it sits on disk (little-endian) plus padding.
    const MACHO: &[u8] = &[0xCF, 0xFA, 0xED, 0xFE, 0, 0, 0, 0];

    #[test]
    fn macro_plugin_filter_picks_only_host_executables() {
        let dir = ScratchDir::new("sweetpad-plugin-filter").unwrap();
        let plugin = dir.join("MyMacros"); // the macro plugin: ext-less, +x, Mach-O
        write_file(&plugin, MACHO, true);
        write_file(&dir.join("MyMacros.o"), MACHO, true); // build product (extension)
        write_file(&dir.join("MyMacros.swiftmodule"), MACHO, true); // (extension)
        write_file(&dir.join("NotExec"), MACHO, false); // Mach-O but not executable
        write_file(&dir.join("script"), b"#!/bin/sh\n", true); // +x but not Mach-O
        std::fs::create_dir_all(dir.join("PackageFrameworks")).unwrap(); // a directory

        assert!(is_macro_plugin_executable(&plugin));
        for skip in [
            "MyMacros.o",
            "MyMacros.swiftmodule",
            "NotExec",
            "script",
            "PackageFrameworks",
        ] {
            assert!(
                !is_macro_plugin_executable(&dir.join(skip)),
                "should skip {skip}"
            );
        }
    }

    #[test]
    fn collect_macro_plugins_scans_the_host_config_dir() {
        let root = ScratchDir::new("sweetpad-plugin-collect").unwrap();
        let host = root.join("Debug");
        std::fs::create_dir_all(&host).unwrap();
        write_file(&host.join("BetaMacros"), MACHO, true);
        write_file(&host.join("AlphaMacros"), MACHO, true);
        write_file(&host.join("Gamma.o"), MACHO, true); // skipped (extension)

        let settings = BTreeMap::from([
            ("BUILD_DIR".to_string(), root.to_string_lossy().into_owned()),
            ("CONFIGURATION".to_string(), "Debug".to_string()),
        ]);
        let names: Vec<_> = collect_macro_plugins(&settings)
            .iter()
            .map(|p| p.file_name().unwrap().to_str().unwrap().to_string())
            .collect();
        assert_eq!(names, ["AlphaMacros", "BetaMacros"]); // path-sorted, .o excluded

        // Missing BUILD_DIR/CONFIGURATION → never scans.
        assert!(collect_macro_plugins(&BTreeMap::new()).is_empty());
    }
}
