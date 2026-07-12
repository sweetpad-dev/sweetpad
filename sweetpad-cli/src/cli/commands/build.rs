//! `sweetpad build …` — compile the project (via `xcodebuild`, or `swift build`
//! for a Swift package). `build` stays purely "compile"; the run/install/launch
//! lifecycle lives under [`crate::cli::commands::app`].

use clap::Subcommand;

use crate::cli::output::Output;
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
    Diagnostics,
}

pub fn run(ctx: &mut Context, args: &StartArgs, action: Option<&Action>) -> CommandResult {
    ctx.targeting = args.target.clone().into();
    match action {
        Some(Action::Diagnostics) => {
            // The resource-global build flags parse here too; accepting and
            // ignoring them would silently not do what was asked.
            if args.clean || args.watch || args.show_command || !args.passthrough.is_empty() {
                return Err(crate::cli::CliError::new(
                    "build diagnostics re-reads the last build's record; \
                     --clean/--watch/--show-command and `--` passthrough don't apply \
                     (run `sweetpad build` to build)",
                ));
            }
            diagnostics(ctx)
        }
        Some(Action::Start) | None if args.watch => watch(ctx, args),
        Some(Action::Start) | None => start(ctx, args.clean, args.show_command, &args.passthrough),
    }
}

/// `build --watch`: build now, then rebuild on every Swift save. A
/// failed build reports and keeps watching; Ctrl-C ends the loop.
fn watch(ctx: &mut Context, args: &StartArgs) -> CommandResult {
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
    let mut clean = args.clean;
    super::watch_swift(ctx, &root, move |ctx| {
        let result = start(ctx, clean, false, &args.passthrough);
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
        out.line(&format!(
            "last build: {} ({errors} error(s), {warnings} warning(s))",
            if ok { "succeeded" } else { "FAILED" }
        ));
        if let Some(diags) = self.record["diagnostics"].as_array() {
            for d in diags {
                let location = d["location"]
                    .as_str()
                    .map(|l| format!("{l}: "))
                    .unwrap_or_default();
                out.line(&format!(
                    "  {}: {location}{}",
                    d["severity"].as_str().unwrap_or("note"),
                    d["message"].as_str().unwrap_or_default()
                ));
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
            "no build has been recorded for this project yet — run `sweetpad build` first",
        )
    })?;
    Ok(Rendered::data(DiagnosticsReport { record }))
}

/// The build result. Human mode already streamed the beautified log, so this
/// renders nothing extra there; `--json`/`-o ndjson` emit it as the terminal
/// envelope/result event, with the stream's error/warning counts and duration
/// when the ndjson runner collected them.
struct BuildReport {
    scheme: Option<String>,
    configuration: String,
    destination: Option<String>,
    stats: Option<crate::cli::buildlog::StreamStats>,
}

impl Render for BuildReport {
    fn human(&self, _out: &Output) {}

    fn json(&self) -> serde_json::Value {
        let mut data = serde_json::json!({
            "built": true,
            "scheme": self.scheme,
            "configuration": self.configuration,
            "destination": self.destination,
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
    clean: bool,
    show_command: bool,
    passthrough: &[String],
) -> CommandResult {
    let mut resolved = resolve::resolve(ctx)?;

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
                args: swiftpm::build_args(&configuration, passthrough),
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
        ctx.out.note(&format!(
            "building Swift package ({configuration}) with swift build"
        ));
        swiftpm::build(
            &resolved.container,
            &configuration,
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
        }));
    }

    let target = resolve::build_target(ctx, &mut resolved, !show_command)?;

    let plan = xcodebuild::BuildPlan {
        container: &resolved.container,
        scheme: &target.scheme,
        configuration: &target.configuration,
        destination: Some(&target.destination),
        sdk: resolved.sdk.as_deref(),
        clean,
        hot: false,
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
    resolve::remember(ctx, &resolved, &target, ctx.targeting.on.is_none());

    ctx.out.note(&format!(
        "building {} ({}) for {}",
        target.scheme, target.configuration, target.destination
    ));

    let stats = plan
        .run(&ctx.out)
        .map_err(|e| e.or_kind(ErrorKind::BuildFailure))?;

    Ok(Rendered::data(BuildReport {
        scheme: Some(target.scheme),
        configuration: target.configuration,
        destination: Some(target.destination),
        stats,
    }))
}
