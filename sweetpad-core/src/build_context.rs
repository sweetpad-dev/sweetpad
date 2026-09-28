//! One-shot input collection for the build-settings resolver.
//!
//! [`BuildContext::open`] parses the project document, optionally accepts an
//! xcspec catalog and an extra `.xcconfig`, and exposes [`BuildContext::resolve`]
//! as the cheap repeated query. Same context, different `(target, config, sdk,
//! arch, destination, overrides)` — re-resolution walks the cached parse
//! instead of re-reading anything from disk.
//!
//! The layer stack `resolve` assembles, listed bottom-up (later wins):
//!
//! 1. xcspec + SDKSettings defaults (when [`with_xcspec`] is set).
//! 2. Computed built-in settings (`PROJECT_DIR`, `ARCHS`, `BUILD_DIR`, …).
//! 3. The four user-authored layers from the project document (project
//!    xcconfig, project settings, target xcconfig, target settings).
//! 4. The extra `.xcconfig` overlay (when [`with_extra_xcconfig`] is set),
//!    with the command-line overrides of the keys it also sets just below
//!    it (see [`BuildContext::split_overrides`]).
//! 5. Forced xcodebuild overrides (e.g. config-derived `ENABLE_PREVIEWS`).
//! 6. SDKROOT in its absolute-path form when the catalog supplied one.
//! 7. Command-line `KEY=VALUE` overrides from [`ResolveQuery::overrides`],
//!    the rest of them.
//! 8. The build locations `xcodebuild` folds (`SYMROOT`, `OBJROOT`, …),
//!    pinned to their folded values when resolving changed their spelling
//!    (see `resolve_folding_locations`).

use std::collections::BTreeMap;
use std::fmt;
use std::path::{Path, PathBuf};

use sweetpad_lib::destination::RunDestination;
use sweetpad_lib::project::{self, Document, Project};
use sweetpad_lib::resolver::{self, ResolveContext};
use sweetpad_lib::scheme::{self, BuildableRef, Scheme};
use sweetpad_lib::xcconfig::Assignment;
use sweetpad_lib::xcspec::Catalog;

/// Pre-parsed inputs the resolver needs. Build once, query many times.
#[derive(Debug, Clone)]
pub struct BuildContext {
    /// High-level project metadata (targets, configurations, schemes, path).
    pub project: Project,
    /// The parsed project document, in whichever format the bundle holds — a
    /// shared, mtime-validated cache entry (see [`Document::parse`]) reused by
    /// each [`Self::resolve`] and shared across every `BuildContext` opened on
    /// the same project.
    document: Document,
    /// xcspec + `SDKSettings.plist` defaults catalog. `None` skips the
    /// defaults layer — you'll get only the user-authored settings + built-ins.
    pub xcspec: Option<Catalog>,
    /// Extra `.xcconfig` overlay, flattened with `#include`s. Layered above
    /// the user-authored project / target settings, below forced overrides.
    /// Equivalent to `xcodebuild -xcconfig FILE`.
    pub extra_xcconfig: Vec<Assignment>,
    /// The container the build was opened with, when it isn't this project
    /// itself. Xcode keys DerivedData by whatever was opened: a
    /// `xcodebuild -workspace W.xcworkspace` build hashes `W.xcworkspace`
    /// for EVERY member project — including members nested several
    /// directories deep, which the next-to-or-one-above workspace heuristic
    /// in [`project::built_in_settings`] cannot see (the tuist fixtures'
    /// `Modules/A/A.xcodeproj` under a root `App.xcworkspace`). `None`
    /// keeps the heuristic (project opened directly).
    pub derived_data_container: Option<PathBuf>,
    /// Place build output the way this machine's Xcode is configured to,
    /// honouring the app-wide Derived Data preference and the container's
    /// per-user workspace settings (see [`sweetpad_lib::derived_data`]).
    ///
    /// Off by default: it makes resolution depend on host state, which the
    /// capture-backed suites must not do. Interfaces driving a real build —
    /// the CLI, the VS Code addon — turn it on so the product they install is
    /// the one `xcodebuild` just wrote.
    pub read_xcode_locations: bool,
}

/// The SDK a target resolves against when a query names neither an SDK nor a
/// destination: the platform its own `SDKROOT` names, which is what a plain
/// `xcodebuild -showBuildSettings` picks. `auto`, an unset `SDKROOT` and
/// anything that isn't a platform fall back to `macosx`, the SDK a
/// multiplatform target's no-platform view resolves under.
fn default_sdk(layers: &[Vec<Assignment>]) -> String {
    project::natural_sdkroot(layers)
        .map(|sdkroot| {
            // A path to an SDK names it by its directory, `iPhoneOS17.0.sdk`.
            let name = sdkroot.rsplit('/').next().unwrap_or(&sdkroot);
            let name = name.strip_suffix(".sdk").unwrap_or(name);
            project::canonicalize_sdk_base(&name.to_ascii_lowercase())
        })
        .filter(|sdk| sweetpad_lib::destination::Platform::from_sdk(sdk).is_some())
        .unwrap_or_else(|| "macosx".to_string())
}

/// How an iOS target builds for a macOS run destination
/// ([`BuildContext::mac_destination_variant`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MacVariant {
    /// Mac Catalyst: the `macosx` SDK with the iOS support libraries.
    Catalyst,
    /// "Designed for iPad": the `iphoneos` SDK, the iOS app run on the Mac.
    DesignedForIpad,
    /// Neither: xcodebuild has no macOS destination for it.
    None,
}

/// One resolution query against a [`BuildContext`].
#[derive(Debug, Clone)]
pub struct ResolveQuery {
    /// Target name (must exist in `ctx.project.targets`).
    pub target: String,
    /// Configuration name (e.g. `Debug`, `Release`).
    pub configuration: String,
    /// Canonical SDK base (e.g. `macosx`, `iphonesimulator`). Drives
    /// `[sdk=...]` conditionals and platform-specific defaults. Empty, with
    /// no destination either, means the SDK the target's own `SDKROOT` names.
    pub sdk: String,
    /// Active architecture (e.g. `arm64`). Drives `[arch=...]` conditionals.
    pub arch: String,
    /// Optional run destination. When present, destination-aware defaults
    /// fire (`ARCHS` collapses, `ONLY_ACTIVE_ARCH` flips for cross-platform
    /// builds, asset-catalog filters synthesise, …).
    pub destination: Option<RunDestination>,
    /// Top-priority `KEY=VALUE` overrides, applied above every other layer.
    /// Equivalent to passing `KEY=VALUE` on the `xcodebuild` command line.
    pub overrides: Vec<Assignment>,
    /// `xcodebuild -derivedDataPath PATH` — when set, replaces the
    /// computed `~/Library/Developer/Xcode/DerivedData/<Container-Hash>`
    /// root for this resolution. `BUILD_DIR`, `OBJROOT`, `BUILT_PRODUCTS_DIR`,
    /// `DERIVED_DATA_DIR` all rebase under this path.
    pub derived_data_path: Option<PathBuf>,
    /// Match `[arch=…]` conditionals against [`Self::arch`] instead of the
    /// `undefined_arch` placeholder. xcodebuild's aggregated
    /// `-showBuildSettings` view resolves with `arch=undefined_arch` — user
    /// per-arch conditionals deliberately don't fire there (the
    /// conditional-arch synthetic capture reports the base value with a
    /// destination bound and `NATIVE_ARCH = arm64`) — so the emulation leaves
    /// this `false`. The per-file/per-target compiler-args path resolves a
    /// concrete compile and opts in.
    pub per_arch_conditionals: bool,
    /// Whether the driving scheme's `TestAction` has
    /// `codeCoverageEnabled="YES"`. When set, xcodebuild forces
    /// `CLANG_COVERAGE_MAPPING=YES` on every target it resolves for the
    /// scheme — a scheme-level fact the per-target project document can't
    /// carry.
    pub code_coverage_enabled: bool,
    /// The driving scheme's `LaunchAction` sanitizer toggles
    /// (`enableAddressSanitizer` / `enableThreadSanitizer` /
    /// `enableUBSanitizer`). When set, xcodebuild forces the matching
    /// `ENABLE_*_SANITIZER = YES` on every target it resolves for the scheme
    /// and suffixes the per-variant object dirs (Swift Build's
    /// `Settings.swift` appends `-asan` / `-tsan` / `-ubsan` to
    /// `OBJECT_FILE_DIR_<variant>`). Another scheme-level fact the project
    /// document can't carry; defaults to all-off.
    pub scheme_sanitizers: scheme::SanitizerEnables,
}

impl ResolveQuery {
    /// New query with the minimum required bindings. `destination` is `None`
    /// and `overrides` is empty.
    pub fn new(
        target: impl Into<String>,
        configuration: impl Into<String>,
        sdk: impl Into<String>,
        arch: impl Into<String>,
    ) -> Self {
        Self {
            target: target.into(),
            configuration: configuration.into(),
            sdk: sdk.into(),
            arch: arch.into(),
            destination: None,
            overrides: Vec::new(),
            derived_data_path: None,
            per_arch_conditionals: false,
            code_coverage_enabled: false,
            scheme_sanitizers: scheme::SanitizerEnables::default(),
        }
    }

    /// Bind a run destination.
    #[must_use]
    pub fn with_destination(mut self, destination: RunDestination) -> Self {
        self.destination = Some(destination);
        self
    }

    /// Inject a top-priority `KEY=VALUE` override. Repeat to inject more.
    #[must_use]
    pub fn with_override(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.overrides.push(Assignment {
            key: key.into(),
            conditions: Vec::new(),
            value: value.into(),
            condition: None,
        });
        self
    }

    /// Override the DerivedData root for this resolution — same effect as
    /// `xcodebuild -derivedDataPath PATH`. Useful when a downstream tool
    /// (a build server, an IDE) directs builds at a custom location.
    #[must_use]
    pub fn with_derived_data_path(mut self, path: impl Into<PathBuf>) -> Self {
        self.derived_data_path = Some(path.into());
        self
    }

    /// Mark that the driving scheme gathers code coverage, forcing
    /// `CLANG_COVERAGE_MAPPING=YES`.
    #[must_use]
    pub fn with_code_coverage_enabled(mut self, enabled: bool) -> Self {
        self.code_coverage_enabled = enabled;
        self
    }

    /// Bind the driving scheme's `LaunchAction` sanitizer toggles (see
    /// [`Self::scheme_sanitizers`]).
    #[must_use]
    pub fn with_scheme_sanitizers(mut self, sanitizers: scheme::SanitizerEnables) -> Self {
        self.scheme_sanitizers = sanitizers;
        self
    }

    /// Fire `[arch=…]` conditionals against the query's concrete arch (see
    /// [`Self::per_arch_conditionals`]).
    #[must_use]
    pub fn with_per_arch_conditionals(mut self, enabled: bool) -> Self {
        self.per_arch_conditionals = enabled;
        self
    }
}

/// What [`BuildContext::plan_build`] produces from a scheme.
#[derive(Debug, Clone, Default)]
pub struct BuildPlan {
    /// One [`ResolveQuery`] per scheme `BuildActionEntry` whose target
    /// lives in this project. Order matches the scheme's declared order.
    pub entries: Vec<ResolveQuery>,
    /// Buildables that don't belong to this context's project — typically
    /// cross-container references in a workspace scheme. They resolve when
    /// the caller plans the same scheme against their own project (see
    /// `build_settings::resolve_build_settings`'s workspace loop).
    pub skipped: Vec<BuildableRef>,
}

/// Resolution output.
#[derive(Debug, Clone)]
pub struct Resolved {
    /// Fully expanded `KEY → value` map.
    pub settings: BTreeMap<String, String>,
    /// The matched target's `productType` (e.g.
    /// `com.apple.product-type.application`).
    pub product_type: Option<String>,
}

/// The user-authored facts the built-in + override layers peek at to make
/// their own decisions, produced once per query by
/// [`BuildContext::authored_probe`] so every gate shares one view (instead of
/// each re-implementing setting precedence with its own quirks).
struct AuthoredProbe {
    /// Effective authored `KEY → value` map: the user layers + `-xcconfig`
    /// overlay + CLI overrides pre-resolved under [`Self::ctx`] (see
    /// [`project::effective_authored_settings`]).
    settings: BTreeMap<String, String>,
    /// Like [`Self::settings`] but WITHOUT the command-line `KEY=VALUE`
    /// overrides — the configuration-level view xcodebuild's optimization
    /// gate evaluates. The gcc-optimization-s synthetic capture pins the
    /// split: `xcodebuild GCC_OPTIMIZATION_LEVEL=s` on Debug reports the
    /// forced level yet keeps every debug-shaped flip (`dwarf`,
    /// `GCC_SYMBOLS_PRIVATE_EXTERN=NO`, `STRIP_INSTALLED_PRODUCT=NO`, the
    /// ONLY_ACTIVE_ARCH collapse) — so that gate must not see the override
    /// layer, while the ARCHS / MERGEABLE_LIBRARY probes must (the
    /// archs-arm64e and mergeable-library captures fire on CLI-forced
    /// values). The extra `-xcconfig` overlay is in BOTH views — it merges
    /// into the settings tables like any other xcconfig.
    sans_overrides: BTreeMap<String, String>,
    /// The authored layer stack [`Self::settings`] was resolved from (user
    /// layers, then the extra xcconfig, then the CLI overrides) — for the few
    /// probes that need the raw recipe via [`project::last_matching_setting`].
    layers: Vec<Vec<Assignment>>,
    /// The condition bindings of both the probe AND the main resolve.
    ctx: ResolveContext,
    /// The no-platform "auto" verdict (see [`BuildContext::authored_probe`]).
    auto_no_destination: bool,
}

#[derive(Debug)]
pub enum Error {
    Project(project::Error),
    Resolver(resolver::Error),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Project(e) => write!(f, "{e}"),
            Error::Resolver(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for Error {}

impl Error {
    /// Whether this is a target/configuration lookup miss (see
    /// [`project::Error::is_lookup_miss`]) — the project is fine, it just
    /// doesn't declare what was asked for. Workspace member loops swallow
    /// exactly these and propagate everything else.
    #[must_use]
    pub fn is_lookup_miss(&self) -> bool {
        matches!(self, Error::Project(e) if e.is_lookup_miss())
    }
}

impl From<project::Error> for Error {
    fn from(e: project::Error) -> Self {
        Error::Project(e)
    }
}

impl From<resolver::Error> for Error {
    fn from(e: resolver::Error) -> Self {
        Error::Resolver(e)
    }
}

impl BuildContext {
    /// Parse the `.xcodeproj` once and cache it.
    pub fn open(project_path: &Path) -> Result<Self, Error> {
        let document = Document::parse(project_path)?;
        let project = document.open(project_path)?;
        Ok(Self {
            project,
            document,
            xcspec: None,
            extra_xcconfig: Vec::new(),
            derived_data_container: None,
            read_xcode_locations: false,
        })
    }

    /// Place build output where this machine's Xcode is configured to put it,
    /// rather than assuming the stock DerivedData layout. Set this in an
    /// interface that goes on to launch, install, or index the built product.
    #[must_use]
    pub fn with_xcode_locations(mut self) -> Self {
        self.read_xcode_locations = true;
        self
    }

    /// Declare the container this build was opened with (the
    /// `.xcworkspace` of a `-workspace` invocation). DerivedData paths
    /// (`BUILD_DIR`, `OBJROOT`, `SYMROOT`, …) hash this path instead of the
    /// project's own. Xcode keys DerivedData by whatever was opened, for
    /// every member project, and `xcodebuild -project` opens the project even
    /// when a workspace beside it lists it.
    #[must_use]
    pub fn with_derived_data_container(mut self, container: impl Into<PathBuf>) -> Self {
        self.derived_data_container = Some(container.into());
        self
    }

    /// Attach an xcspec + SDKSettings defaults catalog. Loading the catalog
    /// via [`sweetpad_lib::xcspec::load_catalog`] is expensive and the same catalog
    /// can power many `BuildContext`s; we hold a clone here.
    #[must_use]
    pub fn with_xcspec(mut self, catalog: Catalog) -> Self {
        self.xcspec = Some(catalog);
        self
    }

    /// Layer in an additional `.xcconfig`, equivalent to xcodebuild's
    /// `-xcconfig` flag.
    pub fn with_extra_xcconfig(mut self, path: &Path) -> Result<Self, Error> {
        self.extra_xcconfig = resolver::flatten_xcconfig(path)?;
        Ok(self)
    }

    /// Split the command-line overrides around the `-xcconfig` overlay:
    /// `(below, above)`. xcodebuild applies the overlay above command-line
    /// `KEY=VALUE` settings (its man page: the file's settings "override all
    /// other settings, including settings passed individually on the command
    /// line"). With `-xcconfig X.xcconfig FOO=cli SWIFT_VERSION=5.9` and
    /// `X.xcconfig` holding `FOO = $(inherited) x` and `SWIFT_VERSION = 6.0`,
    /// Xcode 27 resolves `FOO = cli x` and `SWIFT_VERSION = 6.0`. So an
    /// override of a key the overlay assigns goes just below the overlay,
    /// where the overlay's `$(inherited)` reads it; every other override stays
    /// on top of all the layers.
    fn split_overrides(&self, overrides: &[Assignment]) -> (Vec<Assignment>, Vec<Assignment>) {
        overrides
            .iter()
            .cloned()
            .partition(|o| self.extra_xcconfig.iter().any(|a| a.key == o.key))
    }

    /// Resolve build settings for one `(target, config, sdk, arch, …)` tuple.
    /// Cheap to call repeatedly against the same context.
    pub fn resolve(&self, query: &ResolveQuery) -> Result<Resolved, Error> {
        let bundle = self.document.build_settings(
            &self.project.path,
            &query.target,
            &query.configuration,
        )?;
        let defaulted;
        let query = if query.sdk.is_empty() && query.destination.is_none() {
            defaulted = ResolveQuery {
                sdk: default_sdk(&bundle.layers),
                ..query.clone()
            };
            &defaulted
        } else {
            query
        };
        let probe = self.authored_probe(&bundle, query);
        let layers = self.build_layers(&bundle, query, &probe);
        Ok(Resolved {
            // The probe pre-resolved the user layers under the very same
            // bindings (see [`Self::authored_probe`]), so the gates and the
            // final resolve agree on every conditional.
            settings: resolve_folding_locations(layers, &probe.ctx),
            product_type: bundle.product_type,
        })
    }

    /// The SDK `query`'s target builds with when its run destination's
    /// platform isn't one the target supports, or `None` when it is. A
    /// build for one destination builds each target that can't run there for
    /// its own platform: on Xcode 27, a scheme with an iOS app and a macOS app
    /// builds the macOS app for `macosx` under an iOS Simulator destination. A
    /// simulator destination takes the simulator of the target's platform
    /// where the target supports it (a watchOS app under an iPhone simulator).
    /// The supported platforms are the target's authored
    /// `SUPPORTED_PLATFORMS` under its own SDK, else that SDK's default: a
    /// device SDK and its simulator (`iphoneos iphonesimulator`), or `macosx`.
    pub fn own_platform_sdk(&self, query: &ResolveQuery) -> Result<Option<String>, Error> {
        let Some(destination) = &query.destination else {
            return Ok(None);
        };
        let bundle = self.document.build_settings(
            &self.project.path,
            &query.target,
            &query.configuration,
        )?;
        let own = default_sdk(&bundle.layers);
        let probe = ResolveQuery {
            sdk: own.clone(),
            destination: None,
            ..query.clone()
        };
        let simulator = own
            .strip_suffix("os")
            .map(|family| format!("{family}simulator"));
        let supported: Vec<String> = self
            .authored_probe(&bundle, &probe)
            .settings
            .get("SUPPORTED_PLATFORMS")
            .map_or_else(
                || {
                    std::iter::once(own.clone())
                        .chain(simulator.clone())
                        .collect()
                },
                |authored| {
                    authored
                        .split_whitespace()
                        .map(str::to_ascii_lowercase)
                        .collect()
                },
            );
        let supports = |sdk: &str| supported.iter().any(|p| p == sdk);
        if supports(&destination.platform) {
            return Ok(None);
        }
        let simulator = simulator.filter(|sim| destination.is_simulator() && supports(sim));
        Ok(Some(simulator.unwrap_or(own)))
    }

    /// How `query`'s target builds for a macOS run destination (`-destination
    /// platform=macOS`) when its own SDK is `iphoneos`, or `None` for any
    /// other target, and for one whose `SUPPORTED_PLATFORMS` lists `macosx`,
    /// which builds natively. xcodebuild picks the Mac Catalyst destination for a
    /// target that supports Catalyst, and otherwise the "Designed for iPad"
    /// one, which builds for `iphoneos` and runs the iOS app on the Mac. On
    /// Xcode 27 an iOS app with neither `SUPPORTS_MACCATALYST` nor
    /// `SUPPORTS_MAC_DESIGNED_FOR_IPHONE_IPAD` authored reports `PLATFORM_NAME
    /// = iphoneos` and builds into `Debug-iphoneos` there. An application or
    /// app extension doesn't support Catalyst unless it says so (their product
    /// types default `SUPPORTS_MACCATALYST` to `NO`); a framework or library
    /// does (the iOS platform's default is `YES`). The query's `-xcconfig` and
    /// `KEY=VALUE` settings count, as they do for the build.
    pub fn mac_destination_variant(
        &self,
        query: &ResolveQuery,
    ) -> Result<Option<MacVariant>, Error> {
        let bundle = self.document.build_settings(
            &self.project.path,
            &query.target,
            &query.configuration,
        )?;
        if default_sdk(&bundle.layers) != "iphoneos" {
            return Ok(None);
        }
        let ios = ResolveQuery {
            sdk: "iphoneos".into(),
            destination: None,
            ..query.clone()
        };
        let authored = self.authored_probe(&bundle, &ios).settings;
        // A target that lists `macosx` among its supported platforms builds
        // natively for a macOS destination, as xcodebuild builds it.
        if authored
            .get("SUPPORTED_PLATFORMS")
            .is_some_and(|platforms| {
                platforms
                    .split_whitespace()
                    .any(|p| p.eq_ignore_ascii_case("macosx"))
            })
        {
            return Ok(None);
        }
        let yes = |key: &str| authored.get(key).map(|v| v.eq_ignore_ascii_case("YES"));
        let catalyst = yes("SUPPORTS_MACCATALYST").unwrap_or_else(|| {
            let default = self.xcspec.as_ref().and_then(|catalog| {
                let layer = catalog.layer_for(bundle.product_type.as_deref(), Some("iphoneos"));
                project::last_unconditional_setting(&[layer], "SUPPORTS_MACCATALYST")
            });
            default.map_or_else(
                || {
                    !matches!(
                        bundle.product_type.as_deref(),
                        Some(
                            "com.apple.product-type.application"
                                | "com.apple.product-type.app-extension"
                        )
                    )
                },
                |v| v.eq_ignore_ascii_case("YES"),
            )
        });
        Ok(Some(if catalyst {
            MacVariant::Catalyst
        } else if yes("SUPPORTS_MAC_DESIGNED_FOR_IPHONE_IPAD").unwrap_or(true) {
            MacVariant::DesignedForIpad
        } else {
            MacVariant::None
        }))
    }

    /// Turn a scheme's `BuildAction` into a list of [`ResolveQuery`]s
    /// against this context. One query per entry that participates in
    /// `build_for` (xcodebuild only builds the entries whose matching
    /// `buildFor*` flag is set — a testing-only entry is skipped for a
    /// plain build) and whose `BlueprintName` resolves to a target in
    /// `self.project`; cross-container entries land in
    /// [`BuildPlan::skipped`].
    ///
    /// The scheme itself doesn't pick a configuration for the build
    /// action — xcodebuild inherits one from whichever action it's
    /// performing (launch / test / archive). The caller passes the
    /// chosen `configuration` here.
    ///
    /// When the scheme's `TestAction` gathers code coverage, every planned
    /// query carries [`ResolveQuery::code_coverage_enabled`] — xcodebuild
    /// forces `CLANG_COVERAGE_MAPPING=YES` on every buildable it resolves
    /// for such a scheme, whatever the action.
    #[must_use]
    pub fn plan_build(
        &self,
        scheme: &Scheme,
        build_for: sweetpad_lib::scheme::BuildFor,
        configuration: &str,
        sdk: &str,
        arch: &str,
        destination: Option<&RunDestination>,
    ) -> BuildPlan {
        let code_coverage = scheme
            .test_action
            .as_ref()
            .is_some_and(|t| t.code_coverage_enabled);
        let mut entries = Vec::new();
        let mut skipped = Vec::new();
        for entry in &scheme.build_entries {
            if !entry.builds_for(build_for) {
                continue;
            }
            let name = &entry.buildable.blueprint_name;
            let owned = self.project.targets.iter().any(|t| t.name == *name)
                && container_matches(&entry.buildable.container, &self.project.path);
            if owned {
                let mut q = ResolveQuery::new(name, configuration, sdk, arch)
                    .with_code_coverage_enabled(code_coverage)
                    .with_scheme_sanitizers(scheme.launch_sanitizers);
                if let Some(d) = destination {
                    q = q.with_destination(d.clone());
                }
                entries.push(q);
            } else {
                skipped.push(entry.buildable.clone());
            }
        }
        BuildPlan { entries, skipped }
    }

    /// The arch bound for `[arch=…]` condition matching. xcodebuild's
    /// aggregated `-showBuildSettings` view resolves with
    /// `arch=undefined_arch`, so per-arch conditionals don't fire there —
    /// the conditional-arch synthetic capture reports the base value even
    /// with a macOS destination bound and `NATIVE_ARCH = arm64`. A per-arch
    /// resolution (the compiler-args path) opts into the concrete arch via
    /// [`ResolveQuery::per_arch_conditionals`].
    fn condition_arch(query: &ResolveQuery) -> String {
        if query.per_arch_conditionals {
            query.arch.clone()
        } else {
            "undefined_arch".into()
        }
    }

    /// Pre-resolve the authored layers (pbxproj + xcconfigs + the extra
    /// `-xcconfig` overlay + command-line overrides) into the
    /// [`AuthoredProbe`] every built-in/override gate reads, under the SAME
    /// condition bindings the main resolve will use — so a conditional
    /// assignment (`SUPPORTS_MACCATALYST[sdk=macosx*] = YES`), an
    /// `$(inherited)` chain, or `$(VAR)` indirection
    /// (`GCC_OPTIMIZATION_LEVEL = $(MY_LEVEL)`) reaches the gates exactly as
    /// the resolver sees it.
    ///
    /// The no-platform "auto" verdict falls out of the first pass: the
    /// authored `SDKROOT` resolves to the multiplatform `auto` sentinel, no
    /// run destination is bound, and the requested sdk isn't one the target
    /// declares support for. xcodebuild leaves such a resolution genuinely
    /// platform-less (no SDK defaults, no `[sdk=...]` conditional matches);
    /// our pipeline falls back to a macosx catalog to keep going, with the
    /// platform-derived divergences pinned back in
    /// [`project::built_in_settings`]. In that mode the main resolve binds NO
    /// sdk at all, so the probe re-resolves the same way to keep the gates
    /// and the final pass in lockstep.
    fn authored_probe(
        &self,
        bundle: &project::BuildSettingsContext,
        query: &ResolveQuery,
    ) -> AuthoredProbe {
        // The extra `-xcconfig` layer and the command-line `KEY=VALUE`
        // overrides participate in the authored-value checks (the
        // synthetic-override captures author ARCHS / MERGEABLE_LIBRARY via
        // `xcodebuild KEY=VALUE`; `-xcconfig overrides.xcconfig` merges into
        // the settings tables like any other xcconfig) — with the
        // configuration-level carve-out [`AuthoredProbe::sans_overrides`]
        // documents.
        let mut sans_overrides_layers = bundle.layers.clone();
        if !self.extra_xcconfig.is_empty() {
            sans_overrides_layers.push(self.extra_xcconfig.clone());
        }
        let (below_overlay, above) = self.split_overrides(&query.overrides);
        let mut layers = bundle.layers.clone();
        if !below_overlay.is_empty() {
            layers.push(below_overlay);
        }
        if !self.extra_xcconfig.is_empty() {
            layers.push(self.extra_xcconfig.clone());
        }
        if !above.is_empty() {
            layers.push(above);
        }
        // xcodebuild binds `[sdk=...]` conditionals against the resolved
        // SDK's canonical (versioned) name, e.g. `macosx26.0` — that's why
        // xcconfig authors write `[sdk=iphoneos*]`. Bind the canonical name
        // when the catalog knows it.
        let mut ctx = ResolveContext {
            sdk: self.canonical_sdk(&query.sdk),
            arch: Self::condition_arch(query),
            configuration: query.configuration.clone(),
            // xcodebuild's default build variant — `[variant=normal]`
            // conditionals (Apple xcspecs carry them) must match.
            variant: "normal".into(),
        };
        let mut settings = project::effective_authored_settings(&layers, &ctx);
        let requested_supported = settings.get("SUPPORTED_PLATFORMS").is_some_and(|sp| {
            sp.split_whitespace()
                .any(|p| p.eq_ignore_ascii_case(&query.sdk))
        });
        let auto_no_destination = query.destination.is_none()
            && !requested_supported
            && settings
                .get("SDKROOT")
                .is_some_and(|s| s.eq_ignore_ascii_case("auto"));
        if auto_no_destination {
            // No-platform mode: there is no resolved SDK at all, so NO
            // `[sdk=...]` conditional matches — xcodebuild reports the
            // unconditional base values (IceCubesApp's project-only captures:
            // the authored ad-hoc `CODE_SIGN_IDENTITY = "-"` wins over its
            // `[sdk=macosx*]` variants).
            ctx = ResolveContext {
                sdk: String::new(),
                ..ctx
            };
            settings = project::effective_authored_settings(&layers, &ctx);
        }
        let sans_overrides = if query.overrides.is_empty() {
            settings.clone()
        } else {
            project::effective_authored_settings(&sans_overrides_layers, &ctx)
        };
        AuthoredProbe {
            settings,
            sans_overrides,
            layers,
            ctx,
            auto_no_destination,
        }
    }

    /// Assemble the layer stack for one query, bottom-up (later wins).
    #[allow(clippy::too_many_lines)]
    fn build_layers(
        &self,
        bundle: &project::BuildSettingsContext,
        query: &ResolveQuery,
        probe: &AuthoredProbe,
    ) -> Vec<Vec<Assignment>> {
        // The handful of user-authored values the built-in + override layers
        // peek at, all read from the one shared pre-resolve (conditionals,
        // `$(inherited)`, and `$(VAR)` indirection already folded — see
        // [`Self::authored_probe`]).
        let authored = &probe.settings;
        let mut layers: Vec<Vec<Assignment>> = Vec::new();

        // Resolve the absolute SDKROOT once if we have a catalog. It feeds
        // both the lowest-priority layer (defaults) AND the top SDKROOT
        // overlay, so we compute it up front.
        let mut resolved_sdkroot: Option<String> = None;
        // `CODE_SIGNING_REQUIRED` originates in the xcspec ProductType defaults
        // (NO for frameworks / dynamic libraries), not the user layers, so we
        // read it from the catalog layer below before the user layers can
        // override it. Default to YES when no catalog is attached — that's the
        // CoreBuildSystem.xcspec default for signable products.
        let mut catalog_code_signing_required: Option<String> = None;
        if let Some(catalog) = &self.xcspec {
            let catalog_layer = catalog.layer_for(bundle.product_type.as_deref(), Some(&query.sdk));
            catalog_code_signing_required = project::last_unconditional_setting(
                std::slice::from_ref(&catalog_layer),
                "CODE_SIGNING_REQUIRED",
            );
            layers.push(catalog_layer);
            resolved_sdkroot = catalog
                .sdk_paths
                .get(&query.sdk)
                .or_else(|| {
                    catalog
                        .sdk_paths
                        .iter()
                        .find(|(k, _)| k.starts_with(&query.sdk))
                        .map(|(_, p)| p)
                })
                .map(|p| p.display().to_string());
        }

        // The "natural" SDK — what the project itself is authored against —
        // deliberately reads ONLY the pbxproj/xcconfig layers, skipping the
        // `-xcconfig` overlay and CLI overrides: an overlay forcing a
        // different SDKROOT is exactly the situation `macos_destination_
        // unbound` below detects (NetNewsWire's iOS xcconfigs captured
        // against the macOS scheme — the project stays macOS-natural).
        let natural_sdk = project::natural_sdkroot(&bundle.layers);
        let supports_maccatalyst = authored.get("SUPPORTS_MACCATALYST");
        // A multiplatform `SDKROOT = auto` target with no `-destination` never
        // resolves a concrete platform, so it never becomes Mac Catalyst even
        // when it authors `SUPPORTS_MACCATALYST = YES`: xcodebuild's
        // `-showBuildSettings` reports IS_MACCATALYST unset, the -macabi triple
        // suffix / iOSSupport search paths unset, the standard (non-Catalyst)
        // app rpath and deployment target, and the macOS-native arch list.
        // The probe detected that no-platform mode so `detect_catalyst`
        // doesn't pull the macosx fallback (`query.sdk = "macosx"`) into the
        // Catalyst path. A bound destination (or a concrete SDKROOT) keeps
        // the normal logic. A multiplatform `SDKROOT = auto` target binds to
        // a concrete SDK the moment a *supported* sdk is requested:
        // xcodebuild resolves `auto` to that SDK's path even with no
        // `-destination` (`-sdk iphonesimulator` gives the iOS-simulator
        // SDK). So only the genuinely unbound view — no destination AND an
        // unsupported / `auto` sdk, e.g. a plain `-showBuildSettings` — stays
        // in the no-platform `auto` mode. The editor (BSP) path always
        // requests a supported sdk, so it binds correctly instead of emitting
        // `-sdk auto` with an `-unknown` platform.
        let user_supported_platforms = authored.get("SUPPORTED_PLATFORMS").map(String::as_str);
        let auto_no_destination = probe.auto_no_destination;
        let is_catalyst = !auto_no_destination
            && project::detect_catalyst(
                &query.sdk,
                natural_sdk.as_deref(),
                supports_maccatalyst.map(String::as_str),
            );
        // A macOS run destination only binds a non-macOS resolved SDK when
        // the target is iOS-natural (Catalyst or designed-for-iPad). When a
        // macOS-NATURAL target ends up on a device SDK anyway (an iOS
        // `-xcconfig` layered over the macOS scheme forces SDKROOT), the
        // destination can't run the product at all and xcodebuild falls back
        // to the destination-less device view (full ARCHS, ONLY_ACTIVE_ARCH
        // and BUILD_ACTIVE_RESOURCES_ONLY both NO).
        let macos_destination_unbound = query
            .destination
            .as_ref()
            .is_some_and(sweetpad_lib::destination::RunDestination::is_macos)
            && !is_catalyst
            && project::canonicalize_sdk_base(&query.sdk) != "macosx"
            && natural_sdk
                .as_deref()
                .is_some_and(|s| project::canonicalize_sdk_base(s) == "macosx");
        let user_ios_deployment = authored
            .get("IPHONEOS_DEPLOYMENT_TARGET")
            .map(String::as_str);
        let user_only_active_arch = authored.get("ONLY_ACTIVE_ARCH").map(String::as_str);
        // Catalyst bundle-id prefixing: the user-authored opt-in flag and the
        // probe's view of the base id — the override layer only gates on the
        // id existing (and not already carrying the prefix); the prefix
        // itself is pushed onto `$(inherited)` so the full stack resolves it.
        let user_product_bundle_identifier = authored
            .get("PRODUCT_BUNDLE_IDENTIFIER")
            .map(String::as_str);
        // Whether the target authors a signing team / identity — gates the
        // macOS ad-hoc CODE_SIGN_IDENTITY collapse in `built_in_overrides`.
        let user_development_team = authored.get("DEVELOPMENT_TEAM").map(String::as_str);
        let user_code_sign_identity = authored.get("CODE_SIGN_IDENTITY").map(String::as_str);
        // ARCHS is read RAW (the last condition-matching assignment, no
        // expansion): the override layer token-checks a *literal* authored
        // list and must leave recipe values (`$(ARCHS_STANDARD)`) alone,
        // which the probe's user-layer expansion would erase.
        let user_archs = project::last_matching_setting(&probe.layers, "ARCHS", &probe.ctx);
        let user_ld_runpath_search_paths =
            authored.get("LD_RUNPATH_SEARCH_PATHS").map(String::as_str);
        let mergeable_library = authored
            .get("MERGEABLE_LIBRARY")
            .is_some_and(|v| v.eq_ignore_ascii_case("YES"));
        let derive_maccatalyst_bundle_id = authored
            .get("DERIVE_MACCATALYST_PRODUCT_BUNDLE_IDENTIFIER")
            .is_some_and(|v| v.eq_ignore_ascii_case("YES"));
        let supports_maccatalyst_yes =
            supports_maccatalyst.is_some_and(|v| v.eq_ignore_ascii_case("YES"));
        // Effective `CODE_SIGNING_REQUIRED`: the catalog ProductType default,
        // overridable by a user-authored value (or the extra xcconfig / CLI
        // overrides). Treat anything other than an explicit "NO" as required.
        let code_signing_required = authored
            .get("CODE_SIGNING_REQUIRED")
            .cloned()
            .or(catalog_code_signing_required)
            .is_none_or(|v| !v.eq_ignore_ascii_case("NO"));

        let mut built_in = project::built_in_settings(
            &self.project.path,
            &query.target,
            &query.configuration,
            bundle.product_type.as_deref(),
            &query.sdk,
            query.destination.as_ref(),
            is_catalyst,
            auto_no_destination,
            user_ios_deployment,
            user_only_active_arch,
            // The configuration-level view: the optimization gate inside must
            // not see CLI overrides (see [`AuthoredProbe::sans_overrides`]).
            &probe.sans_overrides,
            query.derived_data_path.as_deref(),
            self.derived_data_container.as_deref(),
            self.read_xcode_locations,
            self.xcspec
                .as_ref()
                .and_then(|c| c.xcode_version.as_deref()),
            self.xcspec
                .as_ref()
                .and_then(|c| c.product_build_version.as_deref()),
            self.xcspec
                .as_ref()
                .and_then(|c| c.developer_dir.as_deref()),
            self.xcspec.as_ref().and_then(|c| c.host_macos.as_deref()),
            macos_destination_unbound,
            query.scheme_sanitizers,
        );
        // `built_in_settings` never sees the project document, so it reports
        // `en`; xcodebuild reports the project's own development region.
        if let Some(region) = &bundle.development_region
            && let Some(language) = built_in
                .iter_mut()
                .find(|a| a.key == "DEVELOPMENT_LANGUAGE")
        {
            language.value.clone_from(region);
        }
        layers.push(built_in);

        // Target-graph derived settings (parent-app / test-host edges).
        // Sits between built-ins and user layers so an explicit user value
        // for these keys still wins.
        let mut graph_layer = target_graph_layer(bundle, authored);
        // When the test bundle doesn't author `TEST_TARGET_NAME`, xcodebuild
        // still synthesizes `TARGET_BUILD_SUBPATH` from the test-host *target
        // dependency* (`bundle.test_host_target`). The corpus oracle binds a
        // destination and recovers this through scheme aggregation, but the
        // no-destination per-target view can't — so we synthesize it here,
        // gated on `destination.is_none()` to leave the scheme path untouched.
        // The host wrapper name is the host target's resolved `PRODUCT_NAME`
        // (e.g. `NetNewsWire-iOS` ships `NetNewsWire.app`), so we sub-resolve
        // the host once. macOS hosts use a deep bundle (`/Contents/PlugIns`).
        if graph_layer.is_empty()
            && query.destination.is_none()
            && let Some(host_target) = &bundle.test_host_target
            && let Some(subpath) = self.test_bundle_subpath(host_target, query)
        {
            graph_layer.push(Assignment {
                key: "TARGET_BUILD_SUBPATH".into(),
                conditions: Vec::new(),
                value: subpath,
                condition: None,
            });
        }
        if !graph_layer.is_empty() {
            layers.push(graph_layer);
        }

        layers.extend(bundle.layers.iter().cloned());

        let (below_overlay, above_overrides) = self.split_overrides(&query.overrides);
        if !below_overlay.is_empty() {
            layers.push(below_overlay);
        }
        if !self.extra_xcconfig.is_empty() {
            layers.push(self.extra_xcconfig.clone());
        }

        layers.push(project::built_in_overrides(
            project::effective_xcode_major(
                self.xcspec
                    .as_ref()
                    .and_then(|c| c.xcode_version.as_deref()),
            ),
            // The "debug build" gate evaluates the configuration-level view —
            // CLI overrides excluded (see [`AuthoredProbe::sans_overrides`]).
            project::is_unoptimized_build(&probe.sans_overrides),
            is_catalyst,
            supports_maccatalyst_yes,
            user_supported_platforms,
            user_ios_deployment,
            bundle.product_type.as_deref(),
            &query.sdk,
            query.destination.as_ref(),
            bundle.has_package_product_dependencies,
            query.code_coverage_enabled,
            code_signing_required,
            derive_maccatalyst_bundle_id,
            user_product_bundle_identifier,
            user_development_team,
            user_code_sign_identity,
            authored.contains_key("ENABLE_PREVIEWS"),
            authored.contains_key("DEBUG_INFORMATION_FORMAT"),
            user_archs.as_deref(),
            user_ld_runpath_search_paths,
            mergeable_library,
            macos_destination_unbound,
            query.scheme_sanitizers,
        ));

        // Pin the absolute SDKROOT only when a platform actually resolved. A
        // multiplatform `SDKROOT = auto` target with no `-destination` keeps the
        // literal `auto` in xcodebuild's `-showBuildSettings` (no concrete SDK
        // is selected), so leave the user's value untouched in that mode.
        if let Some(p) = resolved_sdkroot
            && !auto_no_destination
        {
            let mut sdk_layer = vec![Assignment {
                key: "SDKROOT".into(),
                conditions: Vec::new(),
                value: p.clone(),
                condition: None,
            }];
            // Xcode 27 reports SYSROOT alongside SDKROOT and always equal to
            // it (154/154 per-target captures; the two exceptions are the
            // `SDKROOT = auto` targets, where 27 omits SYSROOT too, which is
            // why this sits under the same guard). 26.5 and older never
            // emitted the key. Its `Swift.xcspec` entry is a `Path` option
            // with no `DefaultValue`, so the spec alone resolves it empty.
            if project::effective_xcode_major(
                self.xcspec
                    .as_ref()
                    .and_then(|c| c.xcode_version.as_deref()),
            ) >= 27
            {
                sdk_layer.push(Assignment {
                    key: "SYSROOT".into(),
                    conditions: Vec::new(),
                    value: p,
                    condition: None,
                });
            }
            layers.push(sdk_layer);
        }

        if !above_overrides.is_empty() {
            layers.push(above_overrides);
        }

        layers
    }

    /// The canonical (versioned) name of the SDK a query binds, e.g.
    /// `macosx26.0` for a `macosx` request — the name xcodebuild matches
    /// `[sdk=...]` conditionals against. The catalog keys `sdk_paths` by
    /// both the canonical name and its unversioned base; pick the versioned
    /// sibling of the requested base. Falls back to the request verbatim
    /// when there's no catalog or no versioned entry (then bare patterns
    /// keep matching, the lenient pre-catalog behavior).
    fn canonical_sdk(&self, sdk: &str) -> String {
        let Some(catalog) = &self.xcspec else {
            return sdk.to_string();
        };
        catalog
            .sdk_paths
            .keys()
            .filter(|k| {
                k.len() > sdk.len()
                    && k.starts_with(sdk)
                    && k.as_bytes()[sdk.len()].is_ascii_digit()
            })
            .min()
            .cloned()
            .unwrap_or_else(|| sdk.to_string())
    }

    /// Synthesize a test bundle's `TARGET_BUILD_SUBPATH` from its host app
    /// target. The host's product wrapper is `<resolved PRODUCT_NAME>.app`, and
    /// the test bundle nests into the host's `PlugIns` directory — under
    /// `Contents/` for a deep (macOS) bundle. Resolves the host target once
    /// against the same config/sdk/arch to read its `PRODUCT_NAME`; returns
    /// `None` if the host can't be resolved.
    fn test_bundle_subpath(&self, host_target: &str, query: &ResolveQuery) -> Option<String> {
        let host_query =
            ResolveQuery::new(host_target, &query.configuration, &query.sdk, &query.arch);
        let host = self.resolve(&host_query).ok()?;
        let wrapper = host.settings.get("PRODUCT_NAME")?;
        // macOS apps are deep bundles (`App.app/Contents/PlugIns`); every other
        // platform is shallow (`App.app/PlugIns`).
        let contents = if query.sdk.starts_with("macos") {
            "/Contents"
        } else {
            ""
        };
        Some(format!("/{wrapper}.app{contents}/PlugIns"))
    }
}

/// The build-location settings `xcodebuild` folds once they resolve, in the
/// order it settles them: `.` and empty components dropped and each `..`
/// folded into the component before it, without reading the filesystem, so
/// `SYMROOT=/tmp/../tmp/x` reports `/tmp/x`. Those marked `true` read a
/// relative value against the project's directory first, so `OBJROOT=obj`
/// is `<project dir>/obj`. The settings built from one see the folded value,
/// `BUILD_DIR` from `SYMROOT` among them, while a `BUILD_DIR` set itself
/// keeps its spelling like any setting not listed here. Pinned against
/// `xcodebuild -showBuildSettings` on Xcode 27 with the value typed on the
/// command line, in an `-xcconfig` and in the project (the
/// `build_location_fold_oracle` suite).
const FOLDED_LOCATIONS: [(&str, bool); 12] = [
    ("SYMROOT", true),
    ("OBJROOT", true),
    ("DSTROOT", true),
    ("CONFIGURATION_BUILD_DIR", true),
    ("BUILT_PRODUCTS_DIR", true),
    ("CONFIGURATION_TEMP_DIR", true),
    ("TARGET_TEMP_DIR", true),
    ("TEMP_DIR", true),
    ("SHARED_PRECOMPS_DIR", true),
    ("INSTALL_DIR", false),
    ("LOCROOT", true),
    ("LOCSYMROOT", true),
];

/// Resolve `layers`, folding [`FOLDED_LOCATIONS`] the way `xcodebuild` does.
/// Each one that changes is pinned to its folded value in a layer on top and
/// the stack resolved again, so everything expanded from it afterwards sees
/// the folded spelling. The default layout folds to itself, so a project that
/// moves nothing resolves once.
///
/// `TARGET_BUILD_DIR` is the exception Xcode 27 makes: a value set for it is
/// reported folded, never anchored, while `CODESIGNING_FOLDER_PATH` and
/// `METAL_LIBRARY_OUTPUT_DIR` keep the spelling it was given, so only the
/// reported value is folded.
fn resolve_folding_locations(
    mut layers: Vec<Vec<Assignment>>,
    ctx: &ResolveContext,
) -> BTreeMap<String, String> {
    let resolve = |layers: &[Vec<Assignment>]| {
        let refs: Vec<&[Assignment]> = layers.iter().map(Vec::as_slice).collect();
        resolver::resolve(&refs, ctx)
    };
    let mut settings = resolve(&layers);
    let pinned = layers.len();
    for (key, anchored) in FOLDED_LOCATIONS {
        let project_dir = settings.get("PROJECT_DIR").filter(|_| anchored);
        let Some(folded) = settings
            .get(key)
            .and_then(|value| folded_location(value, project_dir.map(String::as_str)))
        else {
            continue;
        };
        if layers.len() == pinned {
            layers.push(Vec::new());
        }
        layers[pinned].push(Assignment {
            key: key.to_string(),
            conditions: Vec::new(),
            // A literal: a `$` in the path is not a reference.
            value: folded.replace('$', "$$"),
            condition: None,
        });
        settings = resolve(&layers);
    }
    if let Some(folded) = settings
        .get("TARGET_BUILD_DIR")
        .and_then(|value| folded_location(value, None))
    {
        settings.insert("TARGET_BUILD_DIR".to_string(), folded);
    }
    settings
}

/// `value` folded as a build location, read against `project_dir` when it is
/// relative and one is given, or `None` when that changes nothing. An empty
/// value stays empty: Xcode 15 reports an empty `LOCROOT`.
fn folded_location(value: &str, project_dir: Option<&str>) -> Option<String> {
    if value.is_empty() {
        return None;
    }
    let folded = match project_dir {
        Some(dir) if !value.starts_with('/') => {
            resolver::standardize_path(&format!("{dir}/{value}"))
        }
        _ => resolver::standardize_path(value),
    };
    (folded != value).then_some(folded)
}

/// Whether a scheme entry's `ReferencedContainer` (e.g.
/// `container:Sub/Foo.xcodeproj`) plausibly refers to this context's project.
/// The container path is relative to the scheme's own anchor directory, which
/// the planner doesn't know, so compare by `.xcodeproj` basename — enough to
/// keep a workspace scheme's buildable out of a *different* project that
/// happens to own a same-named target. An entry with no parseable container
/// matches permissively.
fn container_matches(container: &str, project_path: &Path) -> bool {
    let Some(rest) = container.strip_prefix("container:") else {
        return true;
    };
    match Path::new(rest).file_name() {
        Some(basename) => project_path.file_name() == Some(basename),
        None => true,
    }
}

/// Settings derived from the project's target graph — the relationships
/// between targets that aren't visible from a single target's own settings.
/// Today: where a test bundle nests.
///
/// A **unit-test** bundle nests into its host app's `PlugIns`: xcodebuild's
/// XCTest product-embedding machinery reads `TEST_TARGET_NAME` from the test
/// bundle's user-authored settings and synthesizes
/// `TARGET_BUILD_SUBPATH = /<host wrapper>/PlugIns`, which combined with the
/// xcspec recipe `TARGET_BUILD_DIR = $(CONFIGURATION_BUILD_DIR)$(TARGET_BUILD_SUBPATH)`
/// places the test bundle alongside the host's app bundle. We approximate
/// the host's wrapper as `<TEST_TARGET_NAME>.app` — correct whenever the
/// host target's `PRODUCT_NAME` matches its `TARGET_NAME`, which is the case
/// for every test-host pair in the corpus.
///
/// A **UI-test** bundle runs inside its own XCTRunner app instead — even
/// when it authors `TEST_TARGET_NAME` (that names the app it *drives*, not
/// where it embeds). xcodebuild reports `TARGET_BUILD_SUBPATH =
/// /<PRODUCT_NAME>-Runner.app/PlugIns` (the watchapp2 tuist capture:
/// `/WatchAppUITests-Runner.app/PlugIns`, with `USES_XCTRUNNER = YES`). The
/// value is emitted as a `$(PRODUCT_NAME)` recipe so a renamed product
/// resolves correctly.
fn target_graph_layer(
    bundle: &project::BuildSettingsContext,
    authored: &BTreeMap<String, String>,
) -> Vec<Assignment> {
    let mut out = Vec::new();
    let is_ui_test =
        bundle.product_type.as_deref() == Some("com.apple.product-type.bundle.ui-testing");
    if is_ui_test {
        out.push(Assignment {
            key: "TARGET_BUILD_SUBPATH".into(),
            conditions: Vec::new(),
            value: "/$(PRODUCT_NAME)-Runner.app/PlugIns".into(),
            condition: None,
        });
    } else if project::is_unit_test_bundle_product_type(bundle.product_type.as_deref())
        && let Some(host) = authored.get("TEST_TARGET_NAME")
        && !host.is_empty()
    {
        out.push(Assignment {
            key: "TARGET_BUILD_SUBPATH".into(),
            conditions: Vec::new(),
            value: format!("/{host}.app/PlugIns"),
            condition: None,
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scratch::ScratchDir;
    use std::path::PathBuf;

    fn scratch_path() -> PathBuf {
        PathBuf::from(env!("SWEETPAD_LIB_DIR"))
            .join("fixtures/_synthetic-xcconfigs/xcode-26.5.0/project/Scratch.xcodeproj")
    }

    #[test]
    fn open_caches_pbxproj_and_resolves_user_settings() {
        let ctx = BuildContext::open(&scratch_path()).unwrap();
        assert_eq!(ctx.project.name, "Scratch");

        let resolved = ctx
            .resolve(&ResolveQuery::new("Scratch", "Debug", "macosx", "arm64"))
            .unwrap();

        // User-authored values from the pbxproj survive (no defaults catalog
        // attached, so they're just merged with built-ins).
        assert_eq!(
            resolved.settings.get("PRODUCT_NAME").map(String::as_str),
            Some("Scratch"),
        );
        assert_eq!(
            resolved.settings.get("SDKROOT").map(String::as_str),
            Some("macosx"),
        );
        assert_eq!(
            resolved.product_type.as_deref(),
            Some("com.apple.product-type.tool"),
        );
    }

    #[test]
    fn overrides_win_against_user_settings() {
        let ctx = BuildContext::open(&scratch_path()).unwrap();
        let query = ResolveQuery::new("Scratch", "Debug", "macosx", "arm64")
            .with_override("PRODUCT_NAME", "Overridden");
        let resolved = ctx.resolve(&query).unwrap();
        assert_eq!(
            resolved.settings.get("PRODUCT_NAME").map(String::as_str),
            Some("Overridden"),
        );
    }

    #[test]
    fn derived_data_path_override_rewrites_build_dir() {
        let ctx = BuildContext::open(&scratch_path()).unwrap();
        let q = ResolveQuery::new("Scratch", "Debug", "macosx", "arm64")
            .with_derived_data_path("/tmp/custom-dd");
        let resolved = ctx.resolve(&q).unwrap();
        assert_eq!(
            resolved.settings.get("BUILD_DIR").map(String::as_str),
            Some("/tmp/custom-dd/Build/Products"),
        );
        assert_eq!(
            resolved.settings.get("OBJROOT").map(String::as_str),
            Some("/tmp/custom-dd/Build/Intermediates.noindex"),
        );
        assert_eq!(
            resolved
                .settings
                .get("DERIVED_DATA_DIR")
                .map(String::as_str),
            Some("/tmp/custom-dd"),
        );
    }

    #[test]
    fn unknown_target_errors() {
        let ctx = BuildContext::open(&scratch_path()).unwrap();
        let err = ctx
            .resolve(&ResolveQuery::new(
                "Nonexistent",
                "Debug",
                "macosx",
                "arm64",
            ))
            .unwrap_err();
        assert!(format!("{err}").contains("no target named"));
        assert!(err.is_lookup_miss(), "a missing target is a lookup miss");
    }

    /// A unique scratch dir holding `content` as an extra `.xcconfig`, which
    /// goes when the returned guard drops.
    fn scratch_xcconfig(tag: &str, content: &str) -> (ScratchDir, PathBuf) {
        let dir = ScratchDir::new(&format!("sweetpad-bc-{tag}")).unwrap();
        let path = dir.join("overlay.xcconfig");
        std::fs::write(&path, content).unwrap();
        (dir, path)
    }

    fn get(resolved: &Resolved, key: &str) -> String {
        resolved.settings.get(key).cloned().unwrap_or_default()
    }

    /// `[arch=…]` conditionals stay unfired in the showBuildSettings
    /// emulation (xcodebuild binds `arch=undefined_arch` there — the
    /// conditional-arch capture reports the base value with `NATIVE_ARCH =
    /// arm64`), and fire only for a per-arch resolve (compiler args).
    #[test]
    fn arch_conditionals_fire_only_for_per_arch_resolves() {
        let xcconfig = PathBuf::from(env!("SWEETPAD_LIB_DIR"))
            .join("fixtures/_synthetic-xcconfigs/xcode-26.5.0/xcconfigs/conditional-arch.xcconfig");
        let ctx = BuildContext::open(&scratch_path())
            .unwrap()
            .with_extra_xcconfig(&xcconfig)
            .unwrap();
        let query = ResolveQuery::new("Scratch", "Debug", "macosx", "arm64");
        let aggregated = ctx.resolve(&query).unwrap();
        assert_eq!(get(&aggregated, "BAR"), "base");
        let per_arch = ctx
            .resolve(&query.clone().with_per_arch_conditionals(true))
            .unwrap();
        assert_eq!(get(&per_arch, "BAR"), "arm64_val");
    }

    /// ASan and UBSan are *orthogonal* (only ASan/TSan are mutually
    /// exclusive), so a target can enable both at once. Swift Build appends the
    /// per-sanitizer suffixes to `OBJECT_FILE_DIR_<variant>` in a fixed order —
    /// address, then undefined-behaviour — giving `Objects-normal-asan-ubsan`.
    /// The corpus only pins the single-sanitizer case (`-tsan`); this pins the
    /// *combination* + ordering so a refactor can't silently reorder or drop a
    /// suffix. (A real `xcodebuild` oracle for the combined dir would need a
    /// macOS capture; this guards our concatenation against the Swift Build
    /// source-derived order.)
    #[test]
    fn orthogonal_sanitizers_concatenate_object_dir_suffix_in_order() {
        let (_dir, xcconfig) = scratch_xcconfig(
            "sanitizers",
            "ENABLE_ADDRESS_SANITIZER = YES\nENABLE_UNDEFINED_BEHAVIOR_SANITIZER = YES\n",
        );
        let ctx = BuildContext::open(&scratch_path())
            .unwrap()
            .with_extra_xcconfig(&xcconfig)
            .unwrap();
        let resolved = ctx
            .resolve(&ResolveQuery::new("Scratch", "Debug", "macosx", "arm64"))
            .unwrap();
        let dir = get(&resolved, "OBJECT_FILE_DIR_normal");
        assert!(
            dir.ends_with("-normal-asan-ubsan"),
            "address+undefined suffix must concatenate in order: {dir}"
        );
        assert!(!dir.contains("-tsan"), "thread sanitizer is off: {dir}");
    }

    /// An `-xcconfig` overlay that resolves `GCC_OPTIMIZATION_LEVEL = 0` —
    /// through a `[config=…]` conditional AND `$(VAR)` indirection — flips
    /// the unoptimized-build gates, exactly like it changes xcodebuild's
    /// output. Scratch authors no optimization level, so its plain Debug is
    /// an optimized build.
    #[test]
    fn extra_xcconfig_flips_the_optimization_gates() {
        let (_dir, xcconfig) = scratch_xcconfig(
            "opt-gate",
            "MY_LEVEL = 0\nGCC_OPTIMIZATION_LEVEL[config=Debug] = $(MY_LEVEL)\n",
        );
        let plain = BuildContext::open(&scratch_path()).unwrap();
        let overlaid = BuildContext::open(&scratch_path())
            .unwrap()
            .with_extra_xcconfig(&xcconfig)
            .unwrap();
        let query = ResolveQuery::new("Scratch", "Debug", "macosx", "arm64");
        let before = plain.resolve(&query).unwrap();
        assert_eq!(get(&before, "GCC_SYMBOLS_PRIVATE_EXTERN"), "YES");
        let after = overlaid.resolve(&query).unwrap();
        assert_eq!(get(&after, "GCC_OPTIMIZATION_LEVEL"), "0");
        assert_eq!(get(&after, "GCC_SYMBOLS_PRIVATE_EXTERN"), "NO");
        assert_eq!(get(&after, "ENABLE_PREVIEWS"), "YES");
        // The conditional doesn't match Release, so its gates keep the
        // optimized values.
        let release = overlaid
            .resolve(&ResolveQuery::new("Scratch", "Release", "macosx", "arm64"))
            .unwrap();
        assert_eq!(get(&release, "GCC_SYMBOLS_PRIVATE_EXTERN"), "YES");
    }

    /// A command-line `KEY=VALUE` override changes the reported value but NOT
    /// the optimization gate — pinned by the gcc-optimization-s synthetic
    /// capture, where a CLI-forced `s` on Debug keeps every debug-shaped flip.
    #[test]
    fn cli_override_does_not_flip_the_optimization_gate() {
        let ctx = BuildContext::open(&scratch_path()).unwrap();
        let query = ResolveQuery::new("Scratch", "Debug", "macosx", "arm64")
            .with_override("GCC_OPTIMIZATION_LEVEL", "0");
        let resolved = ctx.resolve(&query).unwrap();
        assert_eq!(get(&resolved, "GCC_OPTIMIZATION_LEVEL"), "0");
        assert_eq!(get(&resolved, "GCC_SYMBOLS_PRIVATE_EXTERN"), "YES");
    }

    /// A conditional `SUPPORTS_MACCATALYST[sdk=macosx*] = YES` (from an
    /// `-xcconfig` overlay) reaches the Catalyst gate when the condition
    /// matches the query's SDK binding.
    #[test]
    fn conditional_supports_maccatalyst_reaches_the_catalyst_gate() {
        let (_dir, xcconfig) =
            scratch_xcconfig("catalyst-gate", "SUPPORTS_MACCATALYST[sdk=macosx*] = YES\n");
        let plain = BuildContext::open(&scratch_path()).unwrap();
        let overlaid = BuildContext::open(&scratch_path())
            .unwrap()
            .with_extra_xcconfig(&xcconfig)
            .unwrap();
        let query = ResolveQuery::new("Scratch", "Debug", "macosx", "arm64");
        assert_eq!(get(&plain.resolve(&query).unwrap(), "IS_MACCATALYST"), "NO");
        assert_eq!(
            get(&overlaid.resolve(&query).unwrap(), "IS_MACCATALYST"),
            "YES"
        );
    }

    /// `PROJECT_DIR`, `SRCROOT`, `PROJECT_FILE_PATH` and every location read
    /// against them take the one spelling `xcodebuild -showBuildSettings`
    /// prints however the project is named: symlinks resolved, and a leading
    /// `/private` dropped.
    ///
    /// Captured on Xcode 27 for a project at `/private/tmp/…/app`, opened as
    /// `/tmp/…/app`, as `/private/tmp/…/app` and through a symlinked
    /// `/private/tmp/…/link`, each as an absolute `-project` and as a relative
    /// one from a directory reached that way. All six reported `PROJECT_DIR =
    /// /tmp/…/app`, and with `SYMROOT=build` on the command line, `SYMROOT =
    /// BUILD_DIR = /tmp/…/app/build`. A symlinked checkout under `/Users`
    /// reported its real directory. A `-derivedDataPath` loses the `/private`,
    /// and keeps the symlink until the directory exists: `/private/tmp/…/link/dd`
    /// reported `BUILD_DIR = /tmp/…/link/dd/Build/Products`, and `/tmp/…/app/dd/
    /// Build/Products` once a run had created it. The scratch project here sits
    /// under `$TMPDIR`, which `/var` reaches the way `/tmp` reaches
    /// `/private/tmp`.
    #[test]
    fn project_paths_take_the_spelling_xcodebuild_prints() {
        let root = ScratchDir::new("sweetpad-bc-spelling").unwrap();
        let private = std::fs::canonicalize(&*root).unwrap();
        let short = sweetpad_lib::project::standardize(&private);
        if short == private {
            eprintln!(
                "skipped: {} is not under a /private root",
                private.display()
            );
            return;
        }
        let real = private.join("real");
        std::fs::create_dir_all(real.join("Scratch.xcodeproj")).unwrap();
        std::fs::copy(
            scratch_path().join("project.pbxproj"),
            real.join("Scratch.xcodeproj/project.pbxproj"),
        )
        .unwrap();
        std::os::unix::fs::symlink(&real, private.join("link")).unwrap();

        let dir = short.join("real").display().to_string();
        let query = ResolveQuery::new("Scratch", "Debug", "macosx", "arm64");
        for opened in [
            short.join("real/Scratch.xcodeproj"),
            private.join("real/Scratch.xcodeproj"),
            private.join("link/Scratch.xcodeproj"),
            short.join("link/Scratch.xcodeproj"),
        ] {
            let ctx = BuildContext::open(&opened).unwrap();
            let relocated = ctx
                .resolve(&query.clone().with_override("SYMROOT", "build"))
                .unwrap();
            for (key, value) in [
                ("PROJECT_DIR", dir.clone()),
                ("SRCROOT", dir.clone()),
                ("PROJECT_FILE_PATH", format!("{dir}/Scratch.xcodeproj")),
                ("SYMROOT", format!("{dir}/build")),
                ("BUILD_DIR", format!("{dir}/build")),
                ("TARGET_BUILD_DIR", format!("{dir}/build/Debug")),
            ] {
                assert_eq!(
                    get(&relocated, key),
                    value,
                    "{key} for {}",
                    opened.display()
                );
            }

            let moved = ctx
                .resolve(
                    &query
                        .clone()
                        .with_derived_data_path(private.join("link/dd")),
                )
                .unwrap();
            assert_eq!(
                get(&moved, "BUILD_DIR"),
                short.join("link/dd/Build/Products").display().to_string(),
                "BUILD_DIR for {}",
                opened.display()
            );
        }

        std::fs::create_dir(real.join("dd")).unwrap();
        let ctx = BuildContext::open(&private.join("link/Scratch.xcodeproj")).unwrap();
        let moved = ctx
            .resolve(&query.with_derived_data_path(private.join("link/dd")))
            .unwrap();
        assert_eq!(get(&moved, "BUILD_DIR"), format!("{dir}/dd/Build/Products"));
    }

    /// The DerivedData container hash uses the *standardized* project path —
    /// symlinks resolved, a leading `/private` dropped for the symlinked roots
    /// — because that is the spelling xcodebuild hashes.
    ///
    /// Grounded in `xcodebuild -showBuildSettings`, not inference: one project
    /// reached through a symlinked root and through its real path reports a
    /// single shared `BUILD_DIR`, and the `/tmp` and `/private/tmp` spellings
    /// of another report the `/tmp` one. Hashing the path as opened sends
    /// every consumer of `BUILD_DIR` (product lookup, `app install`,
    /// `build --json`'s `productPath`) to a folder no build ever wrote.
    #[test]
    fn derived_data_hash_uses_the_standardized_path() {
        let root = ScratchDir::new("sweetpad-bc-link").unwrap();
        let real = root.join("real");
        let link = root.join("link");
        std::fs::create_dir_all(real.join("Scratch.xcodeproj")).unwrap();
        std::fs::copy(
            scratch_path().join("project.pbxproj"),
            real.join("Scratch.xcodeproj/project.pbxproj"),
        )
        .unwrap();
        std::os::unix::fs::symlink(&real, &link).unwrap();

        let through_link = link.join("Scratch.xcodeproj");
        let ctx = BuildContext::open(&through_link).unwrap();
        let resolved = ctx
            .resolve(&ResolveQuery::new("Scratch", "Debug", "macosx", "arm64"))
            .unwrap();
        let build_dir = get(&resolved, "BUILD_DIR");
        let link_hash =
            sweetpad_lib::xcode_hash::derived_data_hash(&through_link.display().to_string());
        let standard_hash = sweetpad_lib::xcode_hash::derived_data_hash(
            &sweetpad_lib::project::standardize(&through_link)
                .display()
                .to_string(),
        );
        assert_ne!(
            link_hash, standard_hash,
            "the symlink and standardized spellings must differ, or this proves nothing"
        );
        assert!(
            build_dir.contains(&format!("Scratch-{standard_hash}")),
            "BUILD_DIR must hash the standardized path: {build_dir}"
        );
        assert!(
            !build_dir.contains(&format!("Scratch-{link_hash}")),
            "BUILD_DIR must not hash the symlink spelling: {build_dir}"
        );
    }

    /// Regression for #285: when the declared DerivedData container is the
    /// `.xcodeproj/project.xcworkspace` stub Xcode auto-generates inside every
    /// project bundle (a user can point `xcodeWorkspacePath` straight at it),
    /// the folder must still resolve as `<Project>-<hash-of-.xcodeproj>` — NOT
    /// the literal `project-<hash-of-stub>`, which sends the launcher looking
    /// for the built app in a directory Xcode never wrote to.
    #[test]
    fn xcodeproj_stub_workspace_container_resolves_to_the_outer_project() {
        let root = ScratchDir::new("sweetpad-bc-stub").unwrap();
        let xcodeproj = root.join("Scratch.xcodeproj");
        std::fs::create_dir_all(&xcodeproj).unwrap();
        std::fs::copy(
            scratch_path().join("project.pbxproj"),
            xcodeproj.join("project.pbxproj"),
        )
        .unwrap();
        // The auto-generated stub workspace nested inside the bundle.
        let stub = xcodeproj.join("project.xcworkspace");
        std::fs::create_dir_all(&stub).unwrap();

        let ctx = BuildContext::open(&xcodeproj)
            .unwrap()
            .with_derived_data_container(&stub);
        let resolved = ctx
            .resolve(&ResolveQuery::new("Scratch", "Debug", "macosx", "arm64"))
            .unwrap();
        let build_dir = get(&resolved, "BUILD_DIR");

        let project_hash = sweetpad_lib::derived_data::container_hash(&xcodeproj);
        let stub_hash = sweetpad_lib::derived_data::container_hash(&stub);
        assert!(
            build_dir.contains(&format!("Scratch-{project_hash}")),
            "BUILD_DIR must use the outer .xcodeproj name + hash: {build_dir}"
        );
        assert!(
            !build_dir.contains(&format!("project-{stub_hash}")),
            "BUILD_DIR must not use the project.xcworkspace stub: {build_dir}"
        );
    }

    /// `xcodebuild -project Scratch.xcodeproj` on Xcode 27 builds into
    /// `Scratch-<hash of the project>`, whose `info.plist` names the
    /// `.xcodeproj` as its `WorkspacePath`, even with a workspace beside it
    /// that lists the project. Only a declared workspace keys DerivedData.
    #[test]
    fn a_project_beside_a_workspace_that_lists_it_keys_derived_data_by_itself() {
        let root = crate::scratch::ScratchDir::new("sweetpad-bc-member").unwrap();
        let xcodeproj = root.join("Scratch.xcodeproj");
        std::fs::create_dir_all(&xcodeproj).unwrap();
        std::fs::copy(
            scratch_path().join("project.pbxproj"),
            xcodeproj.join("project.pbxproj"),
        )
        .unwrap();
        let workspace = root.join("App.xcworkspace");
        std::fs::create_dir_all(&workspace).unwrap();
        std::fs::write(
            workspace.join("contents.xcworkspacedata"),
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<Workspace version = \"1.0\">\n   \
             <FileRef location = \"group:Scratch.xcodeproj\"></FileRef>\n</Workspace>\n",
        )
        .unwrap();
        let query = ResolveQuery::new("Scratch", "Debug", "macosx", "arm64");
        let project_hash = sweetpad_lib::derived_data::container_hash(&xcodeproj);
        let workspace_hash = sweetpad_lib::derived_data::container_hash(&workspace);

        let alone = BuildContext::open(&xcodeproj).unwrap();
        let build_dir = get(&alone.resolve(&query).unwrap(), "BUILD_DIR");
        assert!(
            build_dir.contains(&format!("/Scratch-{project_hash}/")),
            "{build_dir}"
        );

        let through = BuildContext::open(&xcodeproj)
            .unwrap()
            .with_derived_data_container(&workspace);
        let build_dir = get(&through.resolve(&query).unwrap(), "BUILD_DIR");
        assert!(
            build_dir.contains(&format!("/App-{workspace_hash}/")),
            "{build_dir}"
        );
    }
}
