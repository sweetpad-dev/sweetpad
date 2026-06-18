//! Pluggable build backends.
//!
//! A *backend* is a build tool that can turn a resolved project into a compiled
//! app: `xcodebuild` (the default), `swift build` for Swift packages, and —
//! later — config-generating backends like xtool or Bazel (see
//! `docs/dev/build-backends.md`). The `build` command resolves the project once
//! and then hands it to the selected backend; it never branches on the tool
//! itself.
//!
//! Phase 1 only lifts the existing `xcodebuild` / `swift build` dispatch behind
//! the [`BuildBackend`] trait — no behavior change. The trait deliberately stays
//! minimal (`id` / `can_build` / `build`); capability reporting, `detect()` for
//! `doctor`, and on-the-fly config generation arrive with the non-native
//! backends in later phases.

use crate::cli::resolve::{self, Container, Resolved};
use crate::cli::{CliError, CliResult, Context, swiftpm, xcodebuild};

/// Knobs the `build` command passes through to whichever backend runs.
pub struct BuildOptions {
    /// Clean before building.
    pub clean: bool,
}

/// A build tool the `build` command can drive. Implementors wrap the actual
/// invocation (today: [`xcodebuild`] / [`swiftpm`]); the command layer only sees
/// this trait, so adding a backend needs no change to `build.rs`.
pub trait BuildBackend {
    /// Stable identifier, matched by `--backend <id>` and config.
    fn id(&self) -> &'static str;

    /// Whether this backend can build `container` during *auto*-selection. An
    /// explicit `--backend` bypasses this (so e.g. xcodebuild can still be asked
    /// to build a Swift package).
    fn can_build(&self, container: &Container) -> bool;

    /// Compile the resolved project. Backends settle whatever extra target
    /// detail they need (scheme/destination for xcodebuild, configuration for
    /// SwiftPM) from `ctx` + `resolved`.
    fn build(&self, ctx: &mut Context, resolved: &Resolved, opts: &BuildOptions) -> CliResult;
}

/// All registered backends, in auto-selection priority order. Adding a backend
/// is a single line here.
fn registry() -> &'static [&'static dyn BuildBackend] {
    &[&Xcodebuild, &SwiftPm]
}

/// Pick the backend to run. An explicit `requested` id wins (and is validated);
/// otherwise the first registered backend whose [`can_build`](BuildBackend::can_build)
/// accepts the container is used — which reproduces the historical routing
/// (Swift packages → `swift build`, everything else → `xcodebuild`).
pub fn select(
    requested: Option<&str>,
    container: &Container,
) -> Result<&'static dyn BuildBackend, CliError> {
    if let Some(id) = requested {
        return registry()
            .iter()
            .copied()
            .find(|b| b.id() == id)
            .ok_or_else(|| {
                CliError::new(format!(
                    "unknown build backend {id:?}; available: {}",
                    available_ids()
                ))
            });
    }
    registry()
        .iter()
        .copied()
        .find(|b| b.can_build(container))
        .ok_or_else(|| CliError::new("no build backend supports this project"))
}

/// Comma-separated list of registered backend ids, for error messages.
fn available_ids() -> String {
    registry()
        .iter()
        .map(|b| b.id())
        .collect::<Vec<_>>()
        .join(", ")
}

/// `xcodebuild` — the default backend for `.xcworkspace` / `.xcodeproj` (and,
/// when explicitly requested, Swift packages).
struct Xcodebuild;

impl BuildBackend for Xcodebuild {
    fn id(&self) -> &'static str {
        "xcodebuild"
    }

    fn can_build(&self, container: &Container) -> bool {
        // Auto-route Swift packages to `swift build` instead (no forced
        // destination); xcodebuild still builds workspaces and bare projects.
        !matches!(container, Container::SwiftPackage(_))
    }

    fn build(&self, ctx: &mut Context, resolved: &Resolved, opts: &BuildOptions) -> CliResult {
        let target = resolve::build_target(ctx, resolved)?;
        resolve::remember(ctx, resolved, &target);

        ctx.out.note(&format!(
            "building {} ({}) for {}",
            target.scheme, target.configuration, target.destination
        ));

        xcodebuild::BuildPlan {
            container: &resolved.container,
            scheme: &target.scheme,
            configuration: &target.configuration,
            destination: Some(&target.destination),
            clean: opts.clean,
            hot: false,
        }
        .run(&ctx.out)
    }
}

/// `swift build` — for Swift packages, which have no simulator destination and
/// are driven straight from the package directory.
struct SwiftPm;

impl BuildBackend for SwiftPm {
    fn id(&self) -> &'static str {
        "swiftpm"
    }

    fn can_build(&self, container: &Container) -> bool {
        matches!(container, Container::SwiftPackage(_))
    }

    fn build(&self, ctx: &mut Context, resolved: &Resolved, opts: &BuildOptions) -> CliResult {
        let configuration = resolved
            .configuration
            .clone()
            .unwrap_or_else(|| "Debug".to_string());
        ctx.out.note(&format!(
            "building Swift package ({configuration}) with swift build"
        ));
        swiftpm::build(&resolved.container, &configuration, opts.clean)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn project() -> Container {
        Container::Project(PathBuf::from("/work/App.xcodeproj"))
    }
    fn workspace() -> Container {
        Container::Workspace(PathBuf::from("/work/App.xcworkspace"))
    }
    fn package() -> Container {
        Container::SwiftPackage(PathBuf::from("/work/Package.swift"))
    }

    #[test]
    fn auto_routes_projects_and_workspaces_to_xcodebuild() {
        assert_eq!(select(None, &project()).unwrap().id(), "xcodebuild");
        assert_eq!(select(None, &workspace()).unwrap().id(), "xcodebuild");
    }

    #[test]
    fn auto_routes_swift_packages_to_swiftpm() {
        assert_eq!(select(None, &package()).unwrap().id(), "swiftpm");
    }

    #[test]
    fn explicit_backend_wins_over_auto_routing() {
        // xcodebuild can be forced onto a Swift package even though auto-routing
        // would pick swiftpm.
        assert_eq!(
            select(Some("xcodebuild"), &package()).unwrap().id(),
            "xcodebuild"
        );
        assert_eq!(select(Some("swiftpm"), &project()).unwrap().id(), "swiftpm");
    }

    #[test]
    fn unknown_backend_is_an_error_listing_available_ids() {
        // `&dyn BuildBackend` isn't `Debug`, so take the error via `.err()`
        // rather than `unwrap_err()` (which would require the Ok type to print).
        let err = select(Some("bazel"), &project())
            .err()
            .expect("should error");
        let msg = err.to_string();
        assert!(msg.contains("unknown build backend"));
        assert!(msg.contains("xcodebuild"));
        assert!(msg.contains("swiftpm"));
    }
}
