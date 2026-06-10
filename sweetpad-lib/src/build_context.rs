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
use crate::scheme::{BuildableRef, Scheme};
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
    /// Whether the driving scheme's `TestAction` has
    /// `codeCoverageEnabled="YES"`. When set, xcodebuild forces
    /// `CLANG_COVERAGE_MAPPING=YES` on every target it resolves for the
    /// scheme — a scheme-level fact the per-target pbxproj can't carry.
    pub code_coverage_enabled: bool,
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
            code_coverage_enabled: false,
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
        })
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
        let layers = self.build_layers(&bundle, query);
        let layer_refs: Vec<&[Assignment]> = layers.iter().map(Vec::as_slice).collect();
        let ctx = ResolveContext {
            // xcodebuild binds `[sdk=...]` conditionals against the resolved
            // SDK's canonical (versioned) name, e.g. `macosx26.0` — that's
            // why xcconfig authors write `[sdk=iphoneos*]`. Bind the
            // canonical name when the catalog knows it.
            sdk: self.canonical_sdk(&query.sdk),
            arch: query.arch.clone(),
            configuration: query.configuration.clone(),
            // xcodebuild's default build variant — `[variant=normal]`
            // conditionals (Apple xcspecs carry them) must match.
            variant: "normal".into(),
        };
        Ok(Resolved {
            settings: resolver::resolve(&layer_refs, &ctx),
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
                    .with_code_coverage_enabled(code_coverage);
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

    /// Assemble the layer stack for one query, bottom-up (later wins).
    #[allow(clippy::too_many_lines)]
    fn build_layers(
        &self,
        bundle: &project::BuildSettingsContext,
        query: &ResolveQuery,
    ) -> Vec<Vec<Assignment>> {
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

        // The handful of user-authored values that the built-in + override
        // layers need to peek at to make their own decisions.
        let natural_sdk = project::natural_sdkroot(&bundle.layers);
        let supports_maccatalyst =
            project::last_unconditional_setting(&bundle.layers, "SUPPORTS_MACCATALYST");
        // A multiplatform `SDKROOT = auto` target with no `-destination` never
        // resolves a concrete platform, so it never becomes Mac Catalyst even
        // when it authors `SUPPORTS_MACCATALYST = YES`: xcodebuild's
        // `-showBuildSettings` reports IS_MACCATALYST unset, the -macabi triple
        // suffix / iOSSupport search paths unset, the standard (non-Catalyst)
        // app rpath and deployment target, and the macOS-native arch list.
        // Detect that no-platform mode here so `detect_catalyst` doesn't pull
        // the macosx fallback (`query.sdk = "macosx"`) into the Catalyst path.
        // A bound destination (or a concrete SDKROOT) keeps the normal logic.
        let user_supported_platforms =
            project::last_unconditional_setting(&bundle.layers, "SUPPORTED_PLATFORMS");
        // A multiplatform `SDKROOT = auto` target binds to a concrete SDK the
        // moment a *supported* sdk is requested: xcodebuild resolves `auto` to
        // that SDK's path even with no `-destination` (`-sdk iphonesimulator`
        // gives the iOS-simulator SDK). So only the genuinely unbound view — no
        // destination AND an unsupported / `auto` sdk, e.g. a plain
        // `-showBuildSettings` — stays in the no-platform `auto` mode. The editor
        // (BSP) path always requests a supported sdk, so it now binds correctly
        // instead of emitting `-sdk auto` with an `-unknown` platform.
        let requested_supported = user_supported_platforms.as_deref().is_some_and(|sp| {
            sp.split_whitespace()
                .any(|p| p.eq_ignore_ascii_case(&query.sdk))
        });
        let auto_no_destination = query.destination.is_none()
            && !requested_supported
            && natural_sdk
                .as_deref()
                .is_some_and(|s| s.eq_ignore_ascii_case("auto"));
        let is_catalyst = !auto_no_destination
            && project::detect_catalyst(
                &query.sdk,
                natural_sdk.as_deref(),
                supports_maccatalyst.as_deref(),
            );
        let user_ios_deployment =
            project::last_unconditional_setting(&bundle.layers, "IPHONEOS_DEPLOYMENT_TARGET");
        let user_only_active_arch =
            project::last_unconditional_setting(&bundle.layers, "ONLY_ACTIVE_ARCH");
        // Catalyst bundle-id prefixing: the user-authored opt-in flag and the
        // base id the override layer prepends `maccatalyst.` onto. The id is
        // passed verbatim (it may be a `$(...)` recipe) for the resolver to
        // expand in the override layer.
        let user_product_bundle_identifier =
            project::last_unconditional_setting(&bundle.layers, "PRODUCT_BUNDLE_IDENTIFIER");
        // Whether the target authors a signing team / identity — gates the
        // macOS ad-hoc CODE_SIGN_IDENTITY collapse in `built_in_overrides`.
        let user_development_team =
            project::last_unconditional_setting(&bundle.layers, "DEVELOPMENT_TEAM");
        let user_code_sign_identity =
            project::last_unconditional_setting(&bundle.layers, "CODE_SIGN_IDENTITY");
        // Whether the target authors `ENABLE_PREVIEWS` — gates the previews
        // override family (ENABLE_DEBUG_DYLIB / ENABLE_HARDENED_RUNTIME) in
        // `built_in_overrides`.
        let user_enable_previews =
            project::last_unconditional_setting(&bundle.layers, "ENABLE_PREVIEWS");
        let derive_maccatalyst_bundle_id = project::last_unconditional_setting(
            &bundle.layers,
            "DERIVE_MACCATALYST_PRODUCT_BUNDLE_IDENTIFIER",
        )
        .is_some_and(|v| v.eq_ignore_ascii_case("YES"));
        let supports_maccatalyst_yes = supports_maccatalyst
            .as_deref()
            .is_some_and(|v| v.eq_ignore_ascii_case("YES"));
        // Effective `CODE_SIGNING_REQUIRED`: the catalog ProductType default,
        // overridable by a user-authored value (or the extra xcconfig). Treat
        // anything other than an explicit "NO" as required.
        let code_signing_required = project::last_unconditional_setting(
            &[bundle.layers.concat(), self.extra_xcconfig.clone()],
            "CODE_SIGNING_REQUIRED",
        )
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
            user_ios_deployment.as_deref(),
            user_only_active_arch.as_deref(),
            &bundle.layers,
            query.derived_data_path.as_deref(),
            self.xcspec
                .as_ref()
                .and_then(|c| c.xcode_version.as_deref()),
            self.xcspec
                .as_ref()
                .and_then(|c| c.developer_dir.as_deref()),
        ));

        // Target-graph derived settings (parent-app / test-host edges).
        // Sits between built-ins and user layers so an explicit user value
        // for these keys still wins.
        let mut graph_layer = target_graph_layer(bundle);
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
            &query.configuration,
            is_catalyst,
            supports_maccatalyst_yes,
            user_supported_platforms.as_deref(),
            user_ios_deployment.as_deref(),
            bundle.product_type.as_deref(),
            &query.sdk,
            query.destination.as_ref(),
            bundle.has_package_product_dependencies,
            query.code_coverage_enabled,
            code_signing_required,
            derive_maccatalyst_bundle_id,
            user_product_bundle_identifier.as_deref(),
            user_development_team.as_deref(),
            user_code_sign_identity.as_deref(),
            user_enable_previews.as_deref(),
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
fn target_graph_layer(bundle: &project::BuildSettingsContext) -> Vec<Assignment> {
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
        && let Some(host) = project::last_unconditional_setting(&bundle.layers, "TEST_TARGET_NAME")
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
    }
}
