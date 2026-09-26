//! `sweetpad build …` — compile the project (via `xcodebuild`, or `swift build`
//! for a Swift package). `build` stays purely "compile"; the run/install/launch
//! lifecycle lives under [`crate::cli::commands::app`]. `test build` is this
//! same build over the scheme's test targets ([`for_testing`]).

use clap::Subcommand;

use crate::cli::buildlog::{self, DiagKind};
use crate::cli::output::Output;
use crate::cli::xcodebuild::BuildAction;
use crate::cli::{
    CommandResult, Context, ErrorKind, Render, Rendered, resolve, swiftpm, xcodebuild,
};

/// The build flags, declared `global` at the `build` resource so they parse on
/// either side of the (optional) `start` token: `sweetpad build --clean` and
/// `sweetpad build start --clean` are the same invocation.
#[derive(Debug, clap::Args)]
pub struct StartArgs {
    #[command(flatten)]
    pub target: crate::cli::BuildTargetArgs,

    /// Clean before building.
    #[arg(long, global = true)]
    pub clean: bool,

    /// Rebuild on every Swift save (Ctrl-C stops). Failures keep watching.
    #[arg(long, global = true, conflicts_with = "show_command")]
    pub watch: bool,

    /// Print the exact xcodebuild invocation that would run, then exit.
    #[arg(long, global = true)]
    pub show_command: bool,

    /// Extra arguments passed to xcodebuild verbatim (after '--'), e.g.
    /// 'sweetpad build -- -allowProvisioningUpdates KEY=VALUE'.
    #[arg(last = true, value_name = "XCODEBUILD_ARGS", global = true)]
    pub passthrough: Vec<String>,
}

#[derive(Debug, Subcommand)]
pub enum Action {
    /// Compile the resolved scheme (the default action: 'sweetpad build').
    Start,
    /// Show the errors/warnings from the last build, without rebuilding.
    Diagnostics(DiagnosticsArgs),
}

/// The start-only flags, redeclared hidden on `build diagnostics` under the
/// same ids. A subcommand's own arg keeps the resource's global one from
/// propagating into it, so its help leaves out flags that don't apply; a stray
/// one still parses, and its value reaches [`StartArgs`] for [`run`] to refuse.
#[derive(Debug, clap::Args)]
pub struct DiagnosticsArgs {
    #[arg(long, hide = true)]
    pub clean: bool,
    #[arg(long, hide = true)]
    pub watch: bool,
    #[arg(long, hide = true)]
    pub show_command: bool,
    #[arg(last = true, hide = true)]
    pub passthrough: Vec<String>,
}

pub fn run(ctx: &mut Context, args: &StartArgs, action: Option<&Action>) -> CommandResult {
    ctx.targeting = args.target.clone().into();
    match action {
        Some(Action::Diagnostics(_)) => {
            // The resource-global build flags parse here too; accepting and
            // ignoring them would silently not do what was asked.
            if args.clean || args.watch || args.show_command || !args.passthrough.is_empty() {
                return Err(crate::cli::CliError::new(
                    "build diagnostics re-reads the last build's record; \
                     --clean/--watch/--show-command and '--' passthrough don't apply \
                     (run 'sweetpad build' to build)",
                ));
            }
            diagnostics(ctx)
        }
        Some(Action::Start) | None if args.watch => {
            watch(ctx, BuildAction::Build, args.clean, &args.passthrough)
        }
        Some(Action::Start) | None => start(
            ctx,
            BuildAction::Build,
            args.clean,
            args.show_command,
            &args.passthrough,
        ),
    }
}

/// `test build`: this module's build, run as `build-for-testing` over the
/// targets the scheme tests. It shares everything else with `build` — the
/// transcript, `-q`, the machine result, and the record `build diagnostics`
/// reads back — so a test that stops compiling reads like any broken build.
pub(crate) fn for_testing(
    ctx: &mut Context,
    watch_saves: bool,
    show_command: bool,
    passthrough: &[String],
) -> CommandResult {
    if watch_saves {
        return watch(ctx, BuildAction::BuildForTesting, false, passthrough);
    }
    start(
        ctx,
        BuildAction::BuildForTesting,
        false,
        show_command,
        passthrough,
    )
}

/// `build --watch` (and `test build --watch`): build now, then rebuild on
/// every Swift save. A failed build reports and keeps watching; Ctrl-C ends
/// the loop.
fn watch(
    ctx: &mut Context,
    action: BuildAction,
    clean: bool,
    passthrough: &[String],
) -> CommandResult {
    let resolved = resolve::resolve(ctx)?;
    let root = resolved
        .container
        .path()
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .map_or_else(
            || std::path::PathBuf::from("."),
            std::path::Path::to_path_buf,
        );

    // Only the first iteration honors --clean.
    let mut clean = clean;
    super::watch_swift(ctx, &root, move |ctx| {
        let result = start(ctx, action, clean, false, passthrough);
        clean = false;
        result
    })
}

/// The last build's recorded diagnostics: errors/warnings from the
/// artifact the previous build wrote — agents stop re-running builds just to
/// re-read the errors.
struct DiagnosticsReport {
    record: serde_json::Value,
}

impl Render for DiagnosticsReport {
    fn human(&self, out: &Output) {
        let ok = self.record["ok"].as_bool().unwrap_or(false);
        let errors = self.record["errors"].as_u64().unwrap_or(0);
        let warnings = self.record["warnings"].as_u64().unwrap_or(0);
        let color = out.use_color();
        let outcome = if ok { "succeeded" } else { "FAILED" };
        out.line(&format!(
            "last build: {} ({errors} error(s), {warnings} warning(s))",
            buildlog::outcome_word(ok, outcome, color)
        ));
        if let Some(diags) = self.record["diagnostics"].as_array() {
            for d in diags {
                let kind = DiagKind::from_severity(d["severity"].as_str().unwrap_or("note"));
                let line = buildlog::diagnostic_line(
                    &kind,
                    d["location"].as_str(),
                    d["message"].as_str().unwrap_or_default(),
                    color,
                );
                out.line(&format!("  {line}"));
            }
        }
    }

    fn json(&self) -> serde_json::Value {
        self.record.clone()
    }
}

fn diagnostics(ctx: &mut Context) -> CommandResult {
    let container = resolve::container(ctx)?;
    let record = xcodebuild::last_build_diagnostics(&container).ok_or_else(|| {
        crate::cli::CliError::new(
            "no build has been recorded for this project yet — run 'sweetpad build' first",
        )
    })?;
    Ok(Rendered::data(DiagnosticsReport { record }))
}

/// The build result. Human mode already streamed the beautified log, so this
/// renders nothing extra there; `--json`/`-o ndjson` emit it as the terminal
/// envelope/result event, with the build's error/warning counts and duration
/// whenever its diagnostics were parsed.
struct BuildReport {
    scheme: Option<String>,
    configuration: String,
    destination: Option<String>,
    stats: Option<buildlog::BuildStats>,
    /// The `.app` this build produced, so a caller doesn't hand-assemble a
    /// DerivedData path. Only the machine-readable modes resolve it (see
    /// [`product_path`]); `null` when the scheme builds no launchable product
    /// (a Swift package, a library-only scheme), for a test build, or when the
    /// lookup failed.
    product_path: Option<std::path::PathBuf>,
}

impl Render for BuildReport {
    fn human(&self, _out: &Output) {}

    fn json(&self) -> serde_json::Value {
        let mut data = serde_json::json!({
            "built": true,
            "scheme": self.scheme,
            "configuration": self.configuration,
            "destination": self.destination,
            "productPath": self.product_path.as_ref().map(|p| p.display().to_string()),
        });
        if let (Some(stats), Some(map)) = (&self.stats, data.as_object_mut()) {
            map.insert("errors".into(), stats.errors.into());
            map.insert("warnings".into(), stats.warnings.into());
            map.insert("durationMs".into(), stats.duration_ms.into());
        }
        data
    }
}

/// The SPM `--clean --show-command` payload: the clean step plus the build,
/// matching what the real run executes.
struct SpmBuildPreview {
    commands: Vec<xcodebuild::CommandPreview>,
}

impl Render for SpmBuildPreview {
    fn human(&self, out: &Output) {
        for c in &self.commands {
            c.human(out);
        }
    }

    fn json(&self) -> serde_json::Value {
        let commands: Vec<serde_json::Value> = self.commands.iter().map(Render::json).collect();
        serde_json::json!({ "commands": commands })
    }
}

fn start(
    ctx: &mut Context,
    action: BuildAction,
    clean: bool,
    show_command: bool,
    passthrough: &[String],
) -> CommandResult {
    // Both entry points (`build` and each `--watch` iteration) land here, so
    // the project file's `[xcodebuild] args` join the tail once.
    let passthrough = &ctx.xcodebuild_args(passthrough)?;
    // A test build compiles what `test run` would run, so it settles on the
    // same scheme, configuration, and destination: the testing context.
    let testing = action == BuildAction::BuildForTesting;
    let mut resolved = if testing {
        resolve::resolve_testing(ctx)?
    } else {
        resolve::resolve(ctx)?
    };

    // Swift packages have no simulator destination; build them with the `swift`
    // toolchain rather than routing through xcodebuild (which would force a
    // destination on us). The dry run and `--` passthrough apply here too.
    if matches!(resolved.container, resolve::Container::SwiftPackage(_)) {
        let configuration = resolved
            .configuration
            .clone()
            .unwrap_or_else(|| "Debug".to_string());
        if show_command {
            let build_preview = xcodebuild::CommandPreview {
                program: "swift",
                args: swiftpm::build_args(&configuration, testing, passthrough),
                cwd: swiftpm::package_dir(&resolved.container),
            };
            // `--clean` runs `swift package clean` first — the preview shows
            // both steps, like archive's dual preview.
            if clean {
                return Ok(Rendered::data(SpmBuildPreview {
                    commands: vec![
                        xcodebuild::CommandPreview {
                            program: "swift",
                            args: vec!["package".into(), "clean".into()],
                            cwd: swiftpm::package_dir(&resolved.container),
                        },
                        build_preview,
                    ],
                }));
            }
            return Ok(Rendered::data(build_preview));
        }
        ctx.out.note(&if testing {
            format!("building Swift package tests ({configuration}) with swift build --build-tests")
        } else {
            format!("building Swift package ({configuration}) with swift build")
        });
        swiftpm::build(
            &resolved.container,
            &configuration,
            testing,
            clean,
            ctx.out.is_json() || ctx.out.is_ndjson(),
            passthrough,
        )
        .map_err(|e| e.or_kind(ErrorKind::BuildFailure))?;
        return Ok(Rendered::data(BuildReport {
            scheme: None,
            configuration,
            destination: None,
            stats: None,
            // A Swift package builds an executable or a library, never a
            // `.app` bundle.
            product_path: None,
        }));
    }

    let target = resolve::build_target(ctx, &mut resolved, !show_command)?;

    let plan = xcodebuild::BuildPlan {
        action,
        container: &resolved.container,
        scheme: &target.scheme,
        configuration: &target.configuration,
        destination: Some(&target.destination),
        sdk: resolved.sdk.as_deref(),
        clean,
        hot: false,
        hot_entitlements: None,
        result_bundle: Some(xcodebuild::build_result_bundle(&resolved.container)),
        passthrough,
    };

    // A dry run prints and exits before any state is persisted.
    if show_command {
        let (args, cwd) = plan.command();
        return Ok(Rendered::data(xcodebuild::CommandPreview {
            program: "xcodebuild",
            args,
            cwd,
        }));
    }
    // Remember the picks — but never a `--on`-sourced destination (a one-off
    // reference must not retarget the next plain build).
    let remember_destination = ctx.targeting.on.is_none();
    if testing {
        resolve::remember_testing(ctx, &resolved, &target, remember_destination);
    } else {
        resolve::remember(ctx, &resolved, &target, remember_destination);
    }

    ctx.out.note(&format!(
        "building {}{} ({}) for {}",
        target.scheme,
        if testing { "'s tests" } else { "" },
        target.configuration,
        target.destination
    ));

    let stats = plan
        .run(&ctx.out)
        .map_err(|e| e.or_kind(ErrorKind::BuildFailure))?;
    let product = product_path(&ctx.out, &plan);

    Ok(Rendered::data(BuildReport {
        scheme: Some(target.scheme),
        configuration: target.configuration,
        destination: Some(target.destination),
        stats,
        product_path: product,
    }))
}

/// The `.app` a completed build wrote, for [`BuildReport`]'s `productPath`.
///
/// Two guards keep this off the build's critical path. Locating the product
/// costs a full build-settings resolution (seconds), and `BuildReport::human`
/// renders nothing — so only the machine-readable modes pay for it. And a
/// scheme can legitimately produce nothing launchable, so every failure maps to
/// `None`: a build that succeeded must not fail over the path lookup.
///
/// A test build names none. What it wrote is the test bundles, and the app the
/// locator would name is built only when a test target depends on it.
fn product_path(out: &Output, plan: &xcodebuild::BuildPlan<'_>) -> Option<std::path::PathBuf> {
    if !(out.is_json() || out.is_ndjson()) || plan.action == BuildAction::BuildForTesting {
        return None;
    }
    xcodebuild::resolved_settings(plan)
        .and_then(|settings| xcodebuild::app_bundle(&settings, plan.destination))
        .ok()
        .map(|app| app.path)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn report(product_path: Option<&str>) -> BuildReport {
        BuildReport {
            scheme: Some("App".to_string()),
            configuration: "Debug".to_string(),
            destination: Some("platform=iOS Simulator,id=UDID".to_string()),
            stats: None,
            product_path: product_path.map(std::path::PathBuf::from),
        }
    }

    #[test]
    fn the_json_report_carries_the_located_product_as_a_string() {
        let json = report(Some("/dd/Build/Products/Debug-iphonesimulator/App.app")).json();
        assert_eq!(
            json["productPath"],
            serde_json::json!("/dd/Build/Products/Debug-iphonesimulator/App.app")
        );
        assert_eq!(json["built"], serde_json::json!(true));
    }

    #[test]
    fn the_json_report_emits_a_null_product_path_when_nothing_was_located() {
        let json = report(None).json();
        // Present but null — a consumer reads the key unconditionally rather
        // than distinguishing "no product" from "old sweetpad".
        assert!(json.get("productPath").is_some());
        assert!(json["productPath"].is_null());
        assert_eq!(json["built"], serde_json::json!(true));
    }

    #[test]
    fn the_json_report_folds_in_the_build_stats() {
        let diagnostics = buildlog::diagnostics_from_transcript(
            "/a/A.swift:1:1: error: boom\n\
             /a/A.swift:2:1: warning: unused\n\
             /a/A.swift:3:1: warning: unused\n\
             /a/A.swift:3:1: note: declared here\n",
        );
        let mut r = report(Some("/dd/App.app"));
        r.stats = Some(buildlog::BuildStats::tally(
            &diagnostics,
            std::time::Duration::from_millis(1234),
        ));
        let json = r.json();
        assert_eq!(json["errors"], serde_json::json!(1));
        assert_eq!(json["warnings"], serde_json::json!(2));
        assert_eq!(json["durationMs"], serde_json::json!(1234));
        assert_eq!(json["productPath"], serde_json::json!("/dd/App.app"));
    }
}
