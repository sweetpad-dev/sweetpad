//! One-shot input collection for the build-settings resolver.
//!
//! [`BuildContext::open`] parses the `project.pbxproj`, optionally accepts an
//! xcspec catalog and an extra `.xcconfig`, and exposes [`BuildContext::resolve`]
//! as the cheap repeated query. Same context, different `(target, config, sdk,
//! arch, destination, overrides)` — re-resolution walks the cached parse
//! instead of re-reading anything from disk.
//!
//! The layer stack `resolve` assembles, listed bottom-up (later wins):
//!
//! 1. xcspec + SDKSettings defaults (when [`with_xcspec`] is set).
//! 2. Computed built-in settings (`PROJECT_DIR`, `ARCHS`, `BUILD_DIR`, …).
//! 3. The four user-authored layers from pbxproj (project xcconfig, project
//!    inline buildSettings, target xcconfig, target inline buildSettings).
//! 4. The extra `.xcconfig` overlay (when [`with_extra_xcconfig`] is set).
//! 5. Forced xcodebuild overrides (e.g. config-derived `ENABLE_PREVIEWS`).
//! 6. SDKROOT in its absolute-path form when the catalog supplied one.
//! 7. Command-line `KEY=VALUE` overrides from [`ResolveQuery::overrides`].

use std::collections::BTreeMap;
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::destination::RunDestination;
use crate::pbxproj::Value;
use crate::project::{self, Project};
use crate::resolver::{self, ResolveContext};
use crate::scheme::{self, BuildableRef, Scheme};
use crate::xcconfig::Assignment;
use crate::xcspec::Catalog;

/// Pre-parsed inputs the resolver needs. Build once, query many times.
#[derive(Debug, Clone)]
pub struct BuildContext {
    /// High-level project metadata (targets, configurations, schemes, path).
    pub project: Project,
    /// Parsed pbxproj root — a shared, mtime-validated cache entry (see
    /// [`project::parse_pbxproj`]) reused by each [`Self::resolve`] and shared
    /// across every `BuildContext` opened on the same project.
    pbxproj: Arc<Value>,
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
}

/// One resolution query against a [`BuildContext`].
#[derive(Debug, Clone)]
pub struct ResolveQuery {
    /// Target name (must exist in `ctx.project.targets`).
    pub target: String,
    /// Configuration name (e.g. `Debug`, `Release`).
    pub configuration: String,
    /// Canonical SDK base (e.g. `macosx`, `iphonesimulator`). Drives
    /// `[sdk=...]` conditionals and platform-specific defaults.
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
    /// scheme — a scheme-level fact the per-target pbxproj can't carry.
    pub code_coverage_enabled: bool,
    /// The driving scheme's `LaunchAction` sanitizer toggles
    /// (`enableAddressSanitizer` / `enableThreadSanitizer` /
    /// `enableUBSanitizer`). When set, xcodebuild forces the matching
    /// `ENABLE_*_SANITIZER = YES` on every target it resolves for the scheme
    /// and suffixes the per-variant object dirs (Swift Build's
    /// `Settings.swift` appends `-asan` / `-tsan` / `-ubsan` to
    /// `OBJECT_FILE_DIR_<variant>`). Another scheme-level fact the pbxproj
    /// can't carry; defaults to all-off.
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
        let pbxproj = project::parse_pbxproj(project_path)?;
        let project = project::open_from_value(&pbxproj, project_path)?;
        Ok(Self {
            project,
            pbxproj,
            xcspec: None,
            extra_xcconfig: Vec::new(),
            derived_data_container: None,
        })
    }

    /// Declare the container this build was opened with (the
    /// `.xcworkspace` of a `-workspace` invocation). DerivedData paths
    /// (`BUILD_DIR`, `OBJROOT`, `SYMROOT`, …) hash this path instead of
    /// inferring a container from the project's own location — Xcode keys
    /// DerivedData by whatever was opened, for every member project.
    #[must_use]
    pub fn with_derived_data_container(mut self, container: impl Into<PathBuf>) -> Self {
        self.derived_data_container = Some(container.into());
        self
    }

    /// Attach an xcspec + SDKSettings defaults catalog. Loading the catalog
    /// via [`crate::xcspec::load_catalog`] is expensive and the same catalog
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

    /// Resolve build settings for one `(target, config, sdk, arch, …)` tuple.
    /// Cheap to call repeatedly against the same context.
    pub fn resolve(&self, query: &ResolveQuery) -> Result<Resolved, Error> {
        let bundle = project::build_settings_from_value(
            &self.pbxproj,
            &self.project.path,
            &query.target,
            &query.configuration,
        )?;
        let probe = self.authored_probe(&bundle, query);
        let layers = self.build_layers(&bundle, query, &probe);
        let layer_refs: Vec<&[Assignment]> = layers.iter().map(Vec::as_slice).collect();
        Ok(Resolved {
            // The probe pre-resolved the user layers under the very same
            // bindings (see [`Self::authored_probe`]), so the gates and the
            // final resolve agree on every conditional.
            settings: resolver::resolve(&layer_refs, &probe.ctx),
            product_type: bundle.product_type,
        })
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
        build_for: crate::scheme::BuildFor,
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
        let mut layers = sans_overrides_layers.clone();
        if !query.overrides.is_empty() {
            layers.push(query.overrides.clone());
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
            .is_some_and(super::destination::RunDestination::is_macos)
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

        layers.push(project::built_in_settings(
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
        ));

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
            layers.push(vec![Assignment {
                key: "SDKROOT".into(),
                conditions: Vec::new(),
                value: p,
                condition: None,
            }]);
        }

        if !query.overrides.is_empty() {
            layers.push(query.overrides.clone());
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
    use std::path::PathBuf;

    fn scratch_path() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
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

    /// A unique scratch dir holding `content` as an extra `.xcconfig`.
    fn scratch_xcconfig(tag: &str, content: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("sweetpad-bc-{tag}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("overlay.xcconfig");
        std::fs::write(&path, content).unwrap();
        path
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
        let xcconfig = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
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
        let xcconfig = scratch_xcconfig(
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
        let xcconfig = scratch_xcconfig(
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
        let xcconfig =
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

    /// The DerivedData container hash uses the project path *as opened*:
    /// resolving through a symlinked root must hash the symlink spelling
    /// (what Xcode itself would hash), not the canonicalized target.
    #[test]
    fn derived_data_hash_uses_the_path_as_opened() {
        let root = std::env::temp_dir().join(format!("sweetpad-bc-link-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
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
        let link_hash = crate::xcode_hash::derived_data_hash(&through_link.display().to_string());
        let real_hash = crate::xcode_hash::derived_data_hash(
            &real.join("Scratch.xcodeproj").display().to_string(),
        );
        assert!(
            build_dir.contains(&format!("Scratch-{link_hash}")),
            "BUILD_DIR must hash the symlink spelling: {build_dir}"
        );
        assert!(
            !build_dir.contains(&format!("Scratch-{real_hash}")),
            "BUILD_DIR must not hash the canonicalized path: {build_dir}"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    /// Regression for #285: when the declared DerivedData container is the
    /// `.xcodeproj/project.xcworkspace` stub Xcode auto-generates inside every
    /// project bundle (a user can point `xcodeWorkspacePath` straight at it),
    /// the folder must still resolve as `<Project>-<hash-of-.xcodeproj>` — NOT
    /// the literal `project-<hash-of-stub>`, which sends the launcher looking
    /// for the built app in a directory Xcode never wrote to.
    #[test]
    fn xcodeproj_stub_workspace_container_resolves_to_the_outer_project() {
        let root =
            std::env::temp_dir().join(format!("sweetpad-bc-stub-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
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

        let project_hash =
            crate::xcode_hash::derived_data_hash(&xcodeproj.display().to_string());
        let stub_hash = crate::xcode_hash::derived_data_hash(&stub.display().to_string());
        assert!(
            build_dir.contains(&format!("Scratch-{project_hash}")),
            "BUILD_DIR must use the outer .xcodeproj name + hash: {build_dir}"
        );
        assert!(
            !build_dir.contains(&format!("project-{stub_hash}")),
            "BUILD_DIR must not use the project.xcworkspace stub: {build_dir}"
        );
        let _ = std::fs::remove_dir_all(&root);
    }
}
