//! Apple's "default settings" catalog.
//!
//! Walks every `.xcspec` file under an Xcode tree (text OpenStep plists,
//! handled by [`crate::pbxproj`]) and every `SDKSettings.plist` (binary plists,
//! handled by [`crate::bplist`]), collecting their build-setting defaults into
//! a single [`Catalog`] keyed by ProductType and SDK canonical name.
//!
//! At resolve time, [`Catalog::layer_for`] selects the right product-type
//! chain (via `BasedOn`) and SDK and returns one flat `Vec<Assignment>` that
//! can be passed as the lowest-precedence layer to
//! [`crate::resolver::resolve`].

use std::collections::{BTreeMap, HashSet};
use std::ffi::OsStr;
use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use crate::bplist;
use crate::pbxproj::{self, Value};
use crate::xcconfig::{Assignment, Condition};

#[derive(Debug)]
pub enum Error {
    Io(io::Error),
    Pbxproj(pbxproj::ParseError, PathBuf),
    Bplist(bplist::Error, PathBuf),
}

impl From<io::Error> for Error {
    fn from(e: io::Error) -> Self {
        Error::Io(e)
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Io(e) => write!(f, "I/O error: {e}"),
            Error::Pbxproj(e, p) => write!(f, "xcspec parse in {}: {e}", p.display()),
            Error::Bplist(e, p) => write!(f, "SDKSettings parse in {}: {e}", p.display()),
        }
    }
}

impl std::error::Error for Error {}

#[derive(Debug, Default, Clone)]
pub struct Catalog {
    /// Universal defaults from every `BuildSystem.Options.DefaultValue` /
    /// `Properties.DefaultValue` across all xcspecs that have NO `_Domain`
    /// attribute — i.e. the generic, cross-platform defaults.
    pub universal: Vec<Assignment>,
    /// BuildSystem defaults from xcspecs with an explicit `_Domain`
    /// (e.g. `macosx`, `embedded-shared`, `embedded-simulator`). Applied at
    /// resolve time only when the build context matches: `macosx` for macOS
    /// targets, `embedded-shared` for any embedded Apple platform,
    /// `embedded-simulator` for the simulator variants on top of that.
    ///
    /// This is the mechanism that produces the right `BUNDLE_FORMAT` (deep on
    /// macOS, shallow on iOS/tvOS/watchOS/visionOS) and unlocks the cascade
    /// of bundle-layout settings derived from it.
    pub domain_specific: BTreeMap<String, Vec<Assignment>>,
    /// `ProductType` and `PackageType` defaults, keyed by `Identifier`.
    pub product_types: BTreeMap<String, ProductTypeDefaults>,
    /// `SDKSettings.plist::DefaultProperties` keyed by SDK canonical name.
    pub sdks: BTreeMap<String, Vec<Assignment>>,
    /// Absolute filesystem path of each indexed SDK (the `.sdk` directory),
    /// keyed by canonical name. Populated from where each `SDKSettings.plist`
    /// lives during ingestion. Used to resolve `SDKROOT` from a canonical name
    /// (e.g. `macosx`) to an absolute path (e.g.
    /// `…/Platforms/MacOSX.platform/Developer/SDKs/MacOSX26.0.sdk`).
    pub sdk_paths: BTreeMap<String, PathBuf>,
    /// The Xcode version this catalog was captured from (e.g. `16.4.0`), read
    /// from the sibling `meta.json`. Feeds the `XCODE_VERSION_{MAJOR,MINOR,
    /// ACTUAL}` built-in settings so version-conditional project settings —
    /// e.g. `$(SWIFT_STRICT_CONCURRENCY_XCODE_$(XCODE_VERSION_MAJOR))` —
    /// resolve against the right Xcode rather than the host's active one.
    /// `None` when no `meta.json` sits beside the xcspecs (the resolver then
    /// falls back to the host install's version).
    pub xcode_version: Option<String>,
    /// The `DEVELOPER_DIR` this catalog was captured from (e.g.
    /// `/Applications/Xcode-26.0.1.app/Contents/Developer`), read from the
    /// sibling `meta.json`. Feeds `DEVELOPER_DIR` and everything derived from it
    /// (`DEVELOPER_*_DIR`, `TOOLCHAIN_DIR`, `DT_TOOLCHAIN_DIR`, and the
    /// `-L$(DT_TOOLCHAIN_DIR)/usr/lib/swift/...` flags in `OTHER_LDFLAGS`) so a
    /// capture resolves against the Xcode it was taken with, not whichever Xcode
    /// is `xcode-select`ed on the host. `None` falls back to the host install.
    pub developer_dir: Option<String>,
    /// Command-line option encodings parsed from each `Type = Compiler` /
    /// `Type = Linker` xcspec, keyed by the tool `Identifier` (e.g.
    /// `com.apple.xcode.tools.swift.compiler`). The authoritative
    /// "build setting → argv" mapping the compiler-argument generator routes
    /// through (see [`crate::compiler_args`]).
    pub compiler_options: BTreeMap<String, Vec<CompilerOption>>,
}

/// One compiler/linker option's command-line encoding, parsed from an xcspec
/// `Options` entry. At most one of `args` / `flag` / `prefix_flag` is set; all
/// `None` means the option contributes no argv (it only feeds other settings).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompilerOption {
    /// The build-setting name (e.g. `SWIFT_OPTIMIZATION_LEVEL`).
    pub name: String,
    /// `Type = StringList` / `PathList`: the value is whitespace-split and the
    /// encoding applied per element.
    pub is_list: bool,
    /// `CommandLineFlag` — a single flag (Boolean: emitted when `YES`; scalar:
    /// followed by the value).
    pub flag: Option<String>,
    /// `CommandLinePrefixFlag` — glued to each list element (`-I` → `-I/p`).
    pub prefix_flag: Option<String>,
    /// `CommandLineArgs` — a flat arg list (with `$(value)` substitution) or a
    /// per-value map (Boolean `YES`/`NO`, Enumeration values, `<<otherwise>>`).
    pub args: Option<CliArgs>,
    /// `FileTypes` — the source languages this option applies to (e.g.
    /// `sourcecode.cpp.cpp`, `sourcecode.c.objc`). Empty means it applies to
    /// every C-family input; a non-empty set gates the option to matching
    /// languages so a C++ flag never reaches an ObjC compile.
    pub file_types: Vec<String>,
    /// `Architectures` — the arches this option applies to (e.g. `i386`,
    /// `x86_64`). Empty means every arch; a non-empty set gates the option so an
    /// Intel-only flag (`GCC_CW_ASM_SYNTAX` → `-fasm-blocks`) never lands on an
    /// arm64 compile.
    pub architectures: Vec<String>,
}

/// The two shapes Apple's `CommandLineArgs` takes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CliArgs {
    /// Array form: emitted verbatim (with `$(value)` substituted), once for a
    /// scalar or once per element for a list.
    List(Vec<String>),
    /// Dict form keyed by the resolved value, with an optional `<<otherwise>>`
    /// fallback. Covers Boolean (`YES`/`NO`) and Enumeration options.
    ByValue {
        map: BTreeMap<String, Vec<String>>,
        otherwise: Option<Vec<String>>,
    },
}

#[derive(Debug, Clone)]
pub struct ProductTypeDefaults {
    pub based_on: Option<String>,
    /// First entry in the xcspec's `PackageTypes` array (e.g.
    /// `com.apple.package-type.mach-o-executable` for `tool`). The resolver
    /// uses this to fill in the `PACKAGE_TYPE` build setting, which is the
    /// route through which package-type defaults get layered into a target.
    pub package_type: Option<String>,
    pub defaults: Vec<Assignment>,
}

impl Catalog {
    /// Flatten the catalog for a specific `(product_type, sdk)` pair into
    /// one ordered list of assignments suitable for the resolver.
    ///
    /// Apple resolves defaults from least-specific to most-specific:
    ///
    /// ```text
    /// universal  →  domain_specific (BuildSystem _Domain)
    ///            →  SDK (SDKSettings.plist DefaultProperties)
    ///            →  PackageType chain (base → derived)
    ///            →  ProductType chain (base → derived)
    /// ```
    ///
    /// The SDK *precedes* the ProductType so a ProductType's
    /// `DefaultBuildProperties` can override a platform-wide default (e.g.
    /// `com.apple.product-type.bundle` sets `ENTITLEMENTS_REQUIRED = NO`,
    /// overriding the macOS SDK's `YES`).
    ///
    /// Both arguments are optional; passing `None` skips that segment.
    #[must_use]
    pub fn layer_for(&self, product_type: Option<&str>, sdk: Option<&str>) -> Vec<Assignment> {
        let mut out = self.universal.clone();
        // BuildSystem defaults for each `_Domain` that the SDK belongs to,
        // base → derived (embedded-shared first, then embedded-simulator).
        if let Some(s) = sdk {
            for domain in applicable_domains(s) {
                if let Some(layer) = self.domain_specific.get(*domain) {
                    out.extend(layer.iter().cloned());
                }
            }
        }
        // SDK DefaultProperties before product-type chains so the product type
        // can override platform defaults.
        if let Some(want) = sdk {
            if let Some(d) = self.sdks.get(want) {
                out.extend(d.iter().cloned());
            } else if let Some((_, d)) = self.sdks.iter().find(|(k, _)| k.starts_with(want)) {
                out.extend(d.iter().cloned());
            }
        }
        if let Some(pt) = product_type {
            // The leaf ProductType pins PACKAGE_TYPE; its PackageType chain
            // contributes generic bundle-layout settings (e.g.
            // `CONTENTS_FOLDER_PATH` templates) — apply BEFORE the
            // ProductType chain so the ProductType can override.
            //
            // Apple's xcspecs frequently declare leaf ProductTypes (e.g.
            // `app-extension.messages`) without their own `PackageTypes`,
            // expecting the value to inherit through `BasedOn`. We walk
            // the ProductType chain to find the first ancestor that
            // declares one.
            if let Some(package_type) = self.based_on_chain(pt).iter().find_map(|id| {
                self.product_types
                    .get(id)
                    .and_then(|d| d.package_type.clone())
            }) {
                out.push(Assignment {
                    key: "PACKAGE_TYPE".into(),
                    conditions: Vec::new(),
                    value: package_type.clone(),
                    condition: None,
                });
                for id in self.based_on_chain(&package_type).iter().rev() {
                    if let Some(pkg) = self.product_types.get(id) {
                        out.extend(pkg.defaults.iter().cloned());
                    }
                }
            }
            // `based_on_chain` returns leaf → root; apply base first so derived
            // overrides, matching xcodebuild's semantics.
            for id in self.based_on_chain(pt).iter().rev() {
                if let Some(d) = self.product_types.get(id) {
                    out.extend(d.defaults.iter().cloned());
                }
            }
        }
        out
    }

    fn based_on_chain(&self, leaf: &str) -> Vec<String> {
        let mut chain = Vec::new();
        let mut seen: HashSet<String> = HashSet::new();
        let mut current = leaf.to_string();
        while seen.insert(current.clone()) {
            chain.push(current.clone());
            let Some(d) = self.product_types.get(&current) else {
                break;
            };
            let Some(parent) = d.based_on.as_ref() else {
                break;
            };
            current = parent.clone();
        }
        chain
    }

    /// Total number of (productType + universal + sdk) assignments — handy for
    /// sanity-checking ingest coverage.
    #[must_use]
    pub fn assignment_count(&self) -> usize {
        let pt: usize = self.product_types.values().map(|d| d.defaults.len()).sum();
        let sdks: usize = self.sdks.values().map(Vec::len).sum();
        self.universal.len() + pt + sdks
    }
}

/// Load every xcspec under `xcspec_root` and every `SDKSettings.plist` under
/// The set of `_Domain` keys that apply to a given SDK canonical name.
/// Ordering matters: base domains first, derived (simulator) last, so the
/// resolver's "last write wins per layer" semantics produce the derived
/// override.
fn applicable_domains(sdk: &str) -> &'static [&'static str] {
    let base: &str = sdk.trim_end_matches(|c: char| c.is_ascii_digit() || c == '.');
    match base {
        "macosx" => &["macosx"],
        "iphoneos" | "appletvos" | "watchos" | "xros" => &["embedded-shared"],
        "iphonesimulator" | "appletvsimulator" | "watchsimulator" | "xrsimulator" => {
            &["embedded-shared", "embedded-simulator"]
        }
        _ => &[],
    }
}

/// `sdksettings_root` (skipped when `None`) into a single [`Catalog`].
pub fn load_catalog(xcspec_root: &Path, sdksettings_root: Option<&Path>) -> Result<Catalog, Error> {
    let mut catalog = Catalog::default();
    walk_xcspec(xcspec_root, &mut catalog)?;
    if let Some(root) = sdksettings_root {
        walk_sdksettings(root, &mut catalog)?;
    }
    let meta = fs::read_to_string(xcspec_root.join("meta.json")).ok();
    catalog.xcode_version = meta
        .as_deref()
        .and_then(|t| scrape_json_string(t, "xcode_version"));
    catalog.developer_dir = meta
        .as_deref()
        .and_then(|t| scrape_json_string(t, "developer_dir"));
    Ok(catalog)
}

/// Extract the string value of a top-level `"key": "value"` pair from flat JSON.
fn scrape_json_string(json: &str, key: &str) -> Option<String> {
    let needle = format!("\"{key}\"");
    let after = &json[json.find(&needle)? + needle.len()..];
    let rest = &after[after.find(':')? + 1..];
    let q1 = rest.find('"')?;
    let tail = &rest[q1 + 1..];
    let q2 = tail.find('"')?;
    Some(tail[..q2].to_string())
}

fn walk_xcspec(dir: &Path, catalog: &mut Catalog) -> Result<(), Error> {
    let Ok(entries) = fs::read_dir(dir) else {
        return Ok(());
    };
    // Sort by file name so the catalog is deterministic across runs. The
    // alphabetic order also happens to give us platform-prefixed xcspecs
    // (macOSCoreBuildSystem.xcspec, iOSCoreBuildSystem.xcspec) AFTER the
    // generic CoreBuildSystem.xcspec, so platform overrides win for the
    // last-write semantics we rely on.
    let mut paths: Vec<PathBuf> = entries.flatten().map(|e| e.path()).collect();
    paths.sort();
    for p in paths {
        if p.is_dir() {
            walk_xcspec(&p, catalog)?;
        } else if p.extension() == Some(OsStr::new("xcspec")) {
            extract_xcspec(&p, catalog)?;
        }
    }
    Ok(())
}

fn extract_xcspec(path: &Path, catalog: &mut Catalog) -> Result<(), Error> {
    let value = pbxproj::parse_file(path).map_err(|e| match e {
        pbxproj::Error::Io(io_err) => Error::Io(io_err),
        pbxproj::Error::Parse(parse) => Error::Pbxproj(parse, path.to_path_buf()),
    })?;
    let entries: Vec<&Value> = match &value {
        Value::Array(a) => a.iter().collect(),
        Value::Dict(_) => vec![&value],
        Value::String(_) => return Ok(()),
    };
    let file_domain = spec_file_domain(path);
    for entry in entries {
        ingest_xcspec_entry(entry, file_domain, catalog);
    }
    Ok(())
}

/// Infer the `_Domain` for an xcspec that omits one, from its file name. Apple's
/// pre-Xcode-16 platform build-system specs encode the platform in the file name
/// ("MacOSX Core Build System.xcspec", "MacOSX Product Types.xcspec") instead of
/// a `_Domain` field; Xcode 16+ adds the explicit `_Domain = macosx`. Without
/// this, an undomained macOS spec's macOS-only defaults (e.g.
/// `BUNDLE_FORMAT = deep`) would land in `universal` and wrongly apply to every
/// platform — on 15.x that gave iOS/tvOS/watchOS apps a deep (`Contents/…`)
/// bundle layout. Only the unambiguous macOS prefix is mapped; every other spec
/// stays undomained. A no-op on Xcode 16+, where the `_Domain` is already set.
fn spec_file_domain(path: &Path) -> Option<&'static str> {
    let stem = path.file_stem().and_then(OsStr::to_str)?;
    (stem == "MacOSX" || stem.starts_with("MacOSX ")).then_some("macosx")
}

fn ingest_xcspec_entry(entry: &Value, file_domain: Option<&str>, catalog: &mut Catalog) {
    let Some(dict) = entry.as_dict() else {
        return;
    };
    let domain = dict.get("_Domain").and_then(Value::as_str).or(file_domain);
    // Apple uses two parallel arrays for BuildSystem-level defaults:
    //   `Options`    — settings that originated as user-facing build options
    //   `Properties` — internal settings the xcspec contributes
    // Both contribute `DefaultValue`s that feed Apple's resolver, so ingest both.
    // Route by `_Domain`: undomained entries land in `universal` (apply
    // everywhere), domained entries go to `domain_specific` and only apply
    // when the SDK matches.
    for key in ["Options", "Properties"] {
        if let Some(arr) = dict.get(key).and_then(Value::as_array) {
            let dest: &mut Vec<Assignment> = match domain {
                None => &mut catalog.universal,
                Some(d) => catalog.domain_specific.entry(d.to_string()).or_default(),
            };
            for opt in arr {
                let Some(opt_dict) = opt.as_dict() else {
                    continue;
                };
                let Some(name) = opt_dict.get("Name").and_then(Value::as_str) else {
                    continue;
                };
                if name.is_empty() {
                    continue;
                }
                let default = opt_dict
                    .get("DefaultValue")
                    .map_or_else(String::new, value_to_string);
                // Preserve the entry's `Condition` attribute so the resolver
                // can evaluate it against the in-progress settings dict in
                // its second pass. See [`crate::condition`] for the language.
                let condition = opt_dict
                    .get("Condition")
                    .and_then(Value::as_str)
                    .map(str::to_string);
                dest.push(Assignment {
                    key: name.to_string(),
                    conditions: Vec::new(),
                    value: default,
                    condition,
                });
            }
        }
    }
    let ty = dict.get("Type").and_then(Value::as_str);
    // Compiler/linker specs carry the authoritative "setting → argv" encoding
    // for each option. Parse it, keyed by the tool `Identifier`, so the
    // compiler-argument generator can route generation through it.
    if matches!(ty, Some("Compiler" | "Linker"))
        && let Some(id) = dict.get("Identifier").and_then(Value::as_str)
        && let Some(arr) = dict.get("Options").and_then(Value::as_array)
    {
        let opts: Vec<CompilerOption> = arr.iter().filter_map(parse_compiler_option).collect();
        if !opts.is_empty() {
            catalog.compiler_options.insert(id.to_string(), opts);
        }
    }
    if matches!(ty, Some("ProductType" | "PackageType"))
        && let Some(id) = dict.get("Identifier").and_then(Value::as_str)
    {
        // Apple's xcspecs duplicate the same `Identifier` across `_Domain`
        // namespaces (darwin / embedded / embedded-simulator / driverkit).
        // For most real targets the `darwin` (or undomained) variant is the
        // canonical one — every `embedded:...` BasedOn reference is one we
        // can't resolve at this layer. We additionally allow `macosx` and
        // the platform-shared domains (`watchos-shared`, `appletvos-shared`,
        // `xros-shared`) because those declare disjoint, platform-only
        // identifiers (`application.watchapp2`, `watchkit2-extension`,
        // `tv-app-extension`, …) that don't exist in `darwin` at all.
        if matches!(
            domain,
            None | Some(
                "darwin" | "macosx" | "watchos-shared" | "appletvos-shared" | "xros-shared"
            )
        ) {
            // Apple is inconsistent: ProductType entries use
            // `DefaultBuildProperties`, PackageType entries use
            // `DefaultBuildSettings`. Pull from both.
            let mut defaults = dict
                .get("DefaultBuildProperties")
                .and_then(Value::as_dict)
                .map(dict_to_assignments)
                .unwrap_or_default();
            if let Some(more) = dict.get("DefaultBuildSettings").and_then(Value::as_dict) {
                defaults.extend(dict_to_assignments(more));
            }
            let based_on = dict
                .get("BasedOn")
                .and_then(Value::as_str)
                .map(String::from);
            let package_type = dict
                .get("PackageTypes")
                .and_then(Value::as_array)
                .and_then(|arr| arr.first())
                .and_then(Value::as_str)
                .map(String::from);
            let new_entry = ProductTypeDefaults {
                based_on,
                package_type,
                defaults,
            };
            // Apple declares the same ProductType identifier in several xcspec
            // files. Through Xcode 15 these duplicates are all *undomained*, so
            // the `_Domain` allowlist above can't single out the canonical one,
            // and a platform shim that re-declares the identifier WITHOUT its
            // `PackageTypes` (e.g. `application` in "watchOS Device.xcspec") would
            // clobber the real "Darwin Product Types.xcspec" definition under a
            // plain last-wins `insert` — leaving `PACKAGE_TYPE` empty and
            // collapsing the entire bundle-layout chain (`WRAPPER_NAME`,
            // `CONTENTS_FOLDER_PATH`, `BUNDLE_FORMAT`, …). Treat the definition
            // that carries `PackageTypes` as authoritative: never overwrite a
            // `Some` package_type with a `None` one. (Xcode 16+ gives these
            // distinct domains — `watchos`/`watchsimulator` vs `darwin` — so the
            // shims are filtered out before this point and there is no conflict
            // to resolve; this only changes behaviour on the undomained 15.x
            // specs.)
            match catalog.product_types.get(id) {
                Some(existing)
                    if existing.package_type.is_some() && new_entry.package_type.is_none() => {}
                _ => {
                    catalog.product_types.insert(id.to_string(), new_entry);
                }
            }
        }
    }
}

fn walk_sdksettings(dir: &Path, catalog: &mut Catalog) -> Result<(), Error> {
    let Ok(entries) = fs::read_dir(dir) else {
        return Ok(());
    };
    for entry in entries.flatten() {
        let p = entry.path();
        if p.is_dir() {
            walk_sdksettings(&p, catalog)?;
        } else if p.file_name() == Some(OsStr::new("SDKSettings.plist")) {
            extract_sdksettings(&p, catalog)?;
        }
    }
    Ok(())
}

fn extract_sdksettings(path: &Path, catalog: &mut Catalog) -> Result<(), Error> {
    let value = bplist::parse_file(path).map_err(|e| Error::Bplist(e, path.to_path_buf()))?;
    let Some(dict) = value.as_dict() else {
        return Ok(());
    };
    let canonical = dict
        .get("CanonicalName")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    if canonical.is_empty() {
        return Ok(());
    }
    let defaults = dict
        .get("DefaultProperties")
        .and_then(Value::as_dict)
        .map(dict_to_assignments)
        .unwrap_or_default();
    // The SDKSettings.plist lives at `<…>.sdk/SDKSettings.plist`, so the
    // parent directory IS the SDK root. Map both the version-qualified
    // canonical name and its base prefix so `sdk_paths.get("macosx")` works
    // alongside `sdk_paths.get("macosx26.0")`.
    if let Some(sdk_dir) = path.parent() {
        catalog
            .sdk_paths
            .insert(canonical.clone(), sdk_dir.to_path_buf());
        let base: String = canonical
            .chars()
            .take_while(|c| !c.is_ascii_digit())
            .collect();
        if !base.is_empty() && base != canonical {
            catalog
                .sdk_paths
                .entry(base)
                .or_insert_with(|| sdk_dir.to_path_buf());
        }
    }
    catalog.sdks.insert(canonical, defaults);
    Ok(())
}

fn dict_to_assignments(dict: &BTreeMap<String, Value>) -> Vec<Assignment> {
    let mut out = Vec::with_capacity(dict.len());
    for (key_raw, val) in dict {
        let (key, conditions) = split_conditional_key(key_raw);
        out.push(Assignment {
            key,
            conditions,
            value: value_to_string(val),
            condition: None,
        });
    }
    out
}

fn split_conditional_key(s: &str) -> (String, Vec<Condition>) {
    let Some(idx) = s.find('[') else {
        return (s.to_string(), Vec::new());
    };
    let key = s[..idx].to_string();
    let mut rest = &s[idx..];
    let mut conditions = Vec::new();
    while let Some(stripped) = rest.strip_prefix('[') {
        let Some(end) = stripped.find(']') else { break };
        for part in stripped[..end].split(',') {
            let part = part.trim();
            if part.is_empty() {
                continue;
            }
            if let Some(eq) = part.find('=') {
                conditions.push(Condition {
                    key: part[..eq].trim().to_string(),
                    value: part[eq + 1..].trim().to_string(),
                });
            }
        }
        rest = stripped[end + 1..].trim_start();
    }
    (key, conditions)
}

fn value_to_string(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Array(arr) => arr
            .iter()
            .filter_map(Value::as_str)
            .collect::<Vec<_>>()
            .join(" "),
        Value::Dict(_) => String::new(),
    }
}

/// A command-line arg slot is one string, or an array of strings; flatten either.
fn value_to_strings(v: &Value) -> Vec<String> {
    match v {
        Value::String(s) => vec![s.clone()],
        Value::Array(arr) => arr.iter().filter_map(Value::as_str).map(String::from).collect(),
        Value::Dict(_) => Vec::new(),
    }
}

/// Parse one compiler/linker `Options` entry's command-line encoding. Returns
/// `None` for an unnamed option.
fn parse_compiler_option(opt: &Value) -> Option<CompilerOption> {
    let dict = opt.as_dict()?;
    let name = dict.get("Name").and_then(Value::as_str)?;
    if name.is_empty() {
        return None;
    }
    let ty = dict.get("Type").and_then(Value::as_str).unwrap_or("");
    let is_list = matches!(ty, "StringList" | "PathList" | "stringlist");

    let args = dict.get("CommandLineArgs").map(|v| match v {
        Value::Dict(d) => {
            let mut map = BTreeMap::new();
            let mut otherwise = None;
            for (k, val) in d {
                let strs = value_to_strings(val);
                if k == "<<otherwise>>" {
                    otherwise = Some(strs);
                } else {
                    map.insert(k.clone(), strs);
                }
            }
            CliArgs::ByValue { map, otherwise }
        }
        _ => CliArgs::List(value_to_strings(v)),
    });

    Some(CompilerOption {
        name: name.to_string(),
        is_list,
        flag: dict
            .get("CommandLineFlag")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .map(String::from),
        prefix_flag: dict
            .get("CommandLinePrefixFlag")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .map(String::from),
        args,
        file_types: dict.get("FileTypes").map(value_to_strings).unwrap_or_default(),
        architectures: dict.get("Architectures").map(value_to_strings).unwrap_or_default(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn xcspec_root() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("xcspec-cache/xcode-26.5.0")
    }

    fn sdksettings_root() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("xcspec-cache/xcode-26.5.0/sdksettings")
    }

    #[test]
    fn loads_substantial_catalog() {
        let cat = load_catalog(&xcspec_root(), Some(&sdksettings_root())).unwrap();
        assert!(
            cat.universal.len() > 200,
            "expected hundreds of universal defaults, got {}",
            cat.universal.len()
        );
        assert!(
            cat.product_types.len() > 5,
            "expected several ProductTypes, got {}",
            cat.product_types.len()
        );
        assert!(
            cat.sdks.len() >= 5,
            "expected several SDKs, got {}",
            cat.sdks.len()
        );
        assert!(
            cat.assignment_count() > 500,
            "expected hundreds of total assignments, got {}",
            cat.assignment_count()
        );
    }

    #[test]
    fn parses_swift_compiler_command_line_options() {
        let cat = load_catalog(&xcspec_root(), Some(&sdksettings_root())).unwrap();
        let opts = cat
            .compiler_options
            .get("com.apple.xcode.tools.swift.compiler")
            .expect("swift compiler options should be parsed");
        assert!(opts.len() > 20, "expected many options, got {}", opts.len());
        let by = |n: &str| opts.iter().find(|o| o.name == n);

        // Enumeration with `<<otherwise>> = $(value)`.
        let opt = by("SWIFT_OPTIMIZATION_LEVEL").expect("SWIFT_OPTIMIZATION_LEVEL");
        match opt.args.as_ref().expect("args") {
            CliArgs::ByValue { otherwise, .. } => {
                assert_eq!(otherwise.as_deref(), Some(&["$(value)".to_string()][..]));
            }
            CliArgs::List(_) => panic!("expected ByValue"),
        }

        // StringList emitting `-D$(value)` per element.
        let cc = by("SWIFT_ACTIVE_COMPILATION_CONDITIONS").expect("conditions");
        assert!(cc.is_list);
        match cc.args.as_ref().expect("args") {
            CliArgs::List(v) => assert_eq!(v.as_slice(), &["-D$(value)".to_string()]),
            CliArgs::ByValue { .. } => panic!("expected List"),
        }

        // The upcoming-feature family carries the authoritative feature name.
        let ea = by("SWIFT_UPCOMING_FEATURE_EXISTENTIAL_ANY").expect("existential any");
        match ea.args.as_ref().expect("args") {
            CliArgs::ByValue { map, .. } => assert_eq!(
                map.get("YES").map(Vec::as_slice),
                Some(&["-enable-upcoming-feature".to_string(), "ExistentialAny".to_string()][..])
            ),
            CliArgs::List(_) => panic!("expected ByValue"),
        }
    }

    #[test]
    fn application_product_type_is_indexed() {
        let cat = load_catalog(&xcspec_root(), None).unwrap();
        let app = cat
            .product_types
            .get("com.apple.product-type.application")
            .expect("application product type should be present");
        // The `BasedOn` chain is canonical for an Application — bundle.
        assert_eq!(
            app.based_on.as_deref(),
            Some("com.apple.product-type.bundle")
        );
        // And the defaults include MACH_O_TYPE = "mh_execute".
        let mach_o = app
            .defaults
            .iter()
            .find(|a| a.key == "MACH_O_TYPE")
            .expect("application should declare MACH_O_TYPE");
        assert_eq!(mach_o.value, "mh_execute");
    }

    #[test]
    fn based_on_chain_walks_inheritance() {
        let cat = load_catalog(&xcspec_root(), None).unwrap();
        let chain = cat.based_on_chain("com.apple.product-type.application");
        // First entry is the leaf itself.
        assert_eq!(chain[0], "com.apple.product-type.application");
        // Chain should include a couple of the standard ancestors.
        assert!(
            chain.iter().any(|s| s == "com.apple.product-type.bundle"),
            "expected `bundle` in chain {chain:?}"
        );
    }

    #[test]
    fn layer_for_application_includes_universal_and_product_type() {
        let cat = load_catalog(&xcspec_root(), Some(&sdksettings_root())).unwrap();
        let layer = cat.layer_for(
            Some("com.apple.product-type.application"),
            Some("macosx26.0"),
        );
        let keys: std::collections::BTreeSet<&str> = layer.iter().map(|a| a.key.as_str()).collect();
        // Universal: PROJECT_NAME defined in CoreBuildSystem.xcspec.
        assert!(
            keys.contains("PROJECT_NAME"),
            "missing universal PROJECT_NAME"
        );
        // Product-type-specific: MACH_O_TYPE from application's DefaultBuildProperties.
        assert!(
            keys.contains("MACH_O_TYPE"),
            "missing application MACH_O_TYPE"
        );
        // SDK-specific: PLATFORM_NAME from MacOSX.sdk.
        assert!(keys.contains("PLATFORM_NAME"), "missing SDK PLATFORM_NAME");
    }

    #[test]
    fn sdk_lookup_falls_back_to_prefix_match() {
        let cat = load_catalog(&xcspec_root(), Some(&sdksettings_root())).unwrap();
        // Pass the bare canonical "macosx" rather than "macosx26.0" — should
        // still pick up the macOS DefaultProperties via the prefix fallback.
        let layer = cat.layer_for(None, Some("macosx"));
        let keys: std::collections::BTreeSet<&str> = layer.iter().map(|a| a.key.as_str()).collect();
        assert!(keys.contains("PLATFORM_NAME"));
    }

    #[test]
    fn split_conditional_key_works_for_sdk_settings() {
        let (key, conds) = split_conditional_key("KASAN_DEFAULT_CFLAGS[arch=arm64]");
        assert_eq!(key, "KASAN_DEFAULT_CFLAGS");
        assert_eq!(conds.len(), 1);
        assert_eq!(conds[0].key, "arch");
        assert_eq!(conds[0].value, "arm64");
    }
}
