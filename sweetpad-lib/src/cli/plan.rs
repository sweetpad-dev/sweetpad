//! The normalized **Build Plan**: a backend-agnostic description of *what* to
//! build, extracted once from the resolved Xcode project.
//!
//! Native backends (xcodebuild / swiftpm) ignore most of this — they hand the
//! project straight to their tool. Config-generating backends (xtool, Bazel)
//! render their config from it: app identity comes from the in-process build
//! settings resolver, and the source/dependency graph from the pbxproj reader
//! (the "hard part" that a generated `Package.swift` / `BUILD.bazel` needs but
//! `-showBuildSettings` can't provide). See `docs/dev/build-backends.md`.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::build_settings::{BuildSettingsOptions, resolve_build_settings};
use crate::cli::resolve::{self, Container, Resolved};
use crate::cli::{CliError, Context};
use crate::project;

/// Apple's product type for an application target.
const APPLICATION_PRODUCT_TYPE: &str = "com.apple.product-type.application";

/// A normalized, tool-agnostic description of an app build, ready for a backend
/// to consume (route to xcodebuild, or render xtool/Bazel config).
#[derive(Debug, Serialize)]
pub struct BuildPlan {
    pub scheme: String,
    pub configuration: String,
    /// The application target the scheme builds.
    pub app_target: String,
    pub product_name: String,
    pub bundle_id: String,
    /// `IPHONEOS_DEPLOYMENT_TARGET`, when set.
    pub deployment_target: Option<String>,
    /// `SUPPORTED_PLATFORMS` (e.g. `"iphoneos iphonesimulator"`), when set.
    pub supported_platforms: Option<String>,
    /// Absolute source paths the app target compiles.
    pub sources: Vec<PathBuf>,
    /// Same-project target dependency names (build-graph edges).
    pub dependencies: Vec<String>,
}

/// Extract the [`BuildPlan`] from the resolved project: settle the scheme and
/// configuration, locate the owning `.xcodeproj` and its application target,
/// then gather identity (resolved build settings) plus the source/dependency
/// graph (pbxproj reader). Swift packages are already SwiftPM and carry no
/// pbxproj to normalize.
pub fn build_plan(ctx: &Context, resolved: &Resolved) -> Result<BuildPlan, CliError> {
    // Swift packages are already SwiftPM and carry no pbxproj to normalize —
    // reject before touching scheme resolution (which would shell out to swift).
    if let Container::SwiftPackage(p) = &resolved.container {
        return Err(CliError::new(format!(
            "build plan is not available for Swift packages ({}); they are already SwiftPM",
            p.display()
        )));
    }

    let scheme = settle_scheme(ctx, resolved)?;
    let configuration = resolved
        .configuration
        .clone()
        .unwrap_or_else(|| "Debug".to_string());

    let xcodeproj = owning_project(resolved, &scheme)?;
    let proj = project::open(&xcodeproj)
        .map_err(|e| CliError::new(format!("failed to read {}: {e}", xcodeproj.display())))?;
    let app_target = app_target(&proj, &scheme)?;

    let settings = resolve_identity(&xcodeproj, &app_target, &configuration)?;
    let get = |k: &str| settings.get(k).filter(|s| !s.is_empty()).cloned();

    let product_name = get("PRODUCT_NAME").unwrap_or_else(|| app_target.clone());
    let bundle_id = get("PRODUCT_BUNDLE_IDENTIFIER").ok_or_else(|| {
        CliError::new(format!(
            "target {app_target:?} has no PRODUCT_BUNDLE_IDENTIFIER"
        ))
    })?;

    let sources = project::target_source_files(&xcodeproj, &app_target)
        .map_err(|e| CliError::new(format!("reading sources for {app_target:?}: {e}")))?;
    let dependencies = project::target_dependencies(&xcodeproj, &app_target)
        .map_err(|e| CliError::new(format!("reading dependencies for {app_target:?}: {e}")))?;

    Ok(BuildPlan {
        scheme,
        configuration,
        app_target,
        product_name,
        bundle_id,
        deployment_target: get("IPHONEOS_DEPLOYMENT_TARGET"),
        supported_platforms: get("SUPPORTED_PLATFORMS"),
        sources,
        dependencies,
    })
}

/// Settle the scheme: the resolved value if present, else auto-pick/prompt from
/// the container's scheme list (strict-errors when non-interactive).
fn settle_scheme(ctx: &Context, resolved: &Resolved) -> Result<String, CliError> {
    let schemes = resolve::schemes(&resolved.container)?;
    resolve::choose(ctx, "scheme", resolved.scheme.clone(), &schemes)
}

/// The `.xcodeproj` that owns the build: the container itself for a bare
/// project, or the workspace member that owns the scheme.
fn owning_project(resolved: &Resolved, scheme: &str) -> Result<PathBuf, CliError> {
    match &resolved.container {
        Container::Project(p) => Ok(p.clone()),
        Container::Workspace(p) => {
            let ws = crate::workspace::open(p).map_err(|e| {
                CliError::new(format!("failed to read workspace {}: {e}", p.display()))
            })?;
            ws.project_for_scheme(scheme)
                .map(Path::to_path_buf)
                .ok_or_else(|| CliError::new(format!("no workspace member owns scheme {scheme:?}")))
        }
        Container::SwiftPackage(p) => Err(CliError::new(format!(
            "build plan is not available for Swift packages ({}); they are already SwiftPM",
            p.display()
        ))),
    }
}

/// Pick the application target: prefer one named like the scheme, else the sole
/// application target; error if there are none or several.
fn app_target(proj: &project::Project, scheme: &str) -> Result<String, CliError> {
    let apps: Vec<&str> = proj
        .targets
        .iter()
        .filter(|t| t.product_type.as_deref() == Some(APPLICATION_PRODUCT_TYPE))
        .map(|t| t.name.as_str())
        .collect();
    if apps.contains(&scheme) {
        return Ok(scheme.to_string());
    }
    match apps.as_slice() {
        [] => Err(CliError::new(
            "no application target found in the project (build plan targets apps)",
        )),
        [one] => Ok((*one).to_string()),
        many => Err(CliError::new(format!(
            "multiple application targets ({}); select one with --scheme",
            many.join(", ")
        ))),
    }
}

/// Resolve one target's build settings, in-process. Pinned to the owning
/// `.xcodeproj` and target (no scheme), so the identity is unambiguous.
fn resolve_identity(
    xcodeproj: &Path,
    target: &str,
    configuration: &str,
) -> Result<BTreeMap<String, String>, CliError> {
    let opts = BuildSettingsOptions {
        project: Some(xcodeproj.to_path_buf()),
        workspace: None,
        scheme: None,
        target: Some(target.to_string()),
        configuration: configuration.to_string(),
        sdk: String::new(),
        arch: String::new(),
        destination: None,
        xcconfig: None,
        xcode: None,
        xcspec_root: None,
        sdksettings_root: None,
        catalog_cache: None,
        derived_data_path: None,
        keys: None,
    };
    resolve_build_settings(&opts)
        .map_err(CliError::new)?
        .into_iter()
        .next()
        .map(|t| t.settings)
        .ok_or_else(|| CliError::new(format!("no build settings resolved for target {target:?}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::config::Config;
    use crate::cli::output::Output;
    use crate::cli::state::State;
    use crate::cli::{Context, GlobalArgs};

    fn fixture(rel: &str) -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("fixtures")
            .join(rel)
    }

    /// A non-interactive context (no TTY), enough to drive `build_plan` when the
    /// scheme is already resolved (so no prompting occurs).
    fn ctx() -> Context {
        let global = GlobalArgs {
            json: false,
            no_color: true,
            verbose: 0,
            workspace: None,
            project: None,
            scheme: None,
            configuration: None,
            destination: None,
            backend: None,
        };
        let out = Output::new(&global);
        Context {
            global,
            config: Config::default(),
            state: State::default(),
            out,
        }
    }

    fn strcat() -> Resolved {
        Resolved {
            container: Container::Project(fixture(
                "_synthetic-strcat/project/StringCatGen.xcodeproj",
            )),
            scheme: Some("StringCatGen".to_string()),
            configuration: None,
            destination: None,
        }
    }

    #[test]
    fn extracts_identity_and_sources_from_a_project() {
        let plan = build_plan(&ctx(), &strcat()).expect("plan");
        assert_eq!(plan.app_target, "StringCatGen");
        assert_eq!(plan.product_name, "StringCatGen");
        assert_eq!(plan.bundle_id, "dev.sweetpad.fixture.StringCatGen");
        assert_eq!(plan.configuration, "Debug");
        // The app's Swift sources are recovered from the PBXSourcesBuildPhase.
        assert!(plan.sources.iter().any(|p| p.ends_with("App.swift")));
        assert!(
            plan.sources
                .iter()
                .any(|p| p.ends_with("StringCatProbe.swift"))
        );
    }

    #[test]
    fn picks_the_sole_application_target_when_scheme_does_not_match() {
        let proj =
            project::open(&fixture("_synthetic-strcat/project/StringCatGen.xcodeproj")).unwrap();
        // A scheme name that isn't a target falls back to the lone app target.
        assert_eq!(app_target(&proj, "SomethingElse").unwrap(), "StringCatGen");
    }

    #[test]
    fn swift_packages_have_no_plan() {
        let resolved = Resolved {
            container: Container::SwiftPackage(PathBuf::from("/x/Package.swift")),
            scheme: Some("X".to_string()),
            configuration: None,
            destination: None,
        };
        let err = build_plan(&ctx(), &resolved).err().expect("should error");
        assert!(err.to_string().contains("Swift packages"));
    }
}
