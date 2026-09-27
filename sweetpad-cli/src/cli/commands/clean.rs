//! `sweetpad clean` — remove build artifacts: `xcodebuild clean` for the
//! resolved scheme with the project's `[xcodebuild] args` (or `swift package
//! clean` for a package), with `--purge` also deleting the project's
//! DerivedData folder(s). The standalone counterpart of `build --clean`, so a
//! clean needs no rebuild.

use crate::cli::output::Output;
use crate::cli::resolve::{self, Container};
use crate::cli::{
    CliError, CommandResult, Context, ErrorContext, Render, Rendered, buildlog, process, xcodebuild,
};

/// `clean`'s flags: exactly the values `xcodebuild clean` consumes — the
/// container, a scheme, and a configuration. The full build tier would
/// advertise `--destination`/`--on`/`--sdk` only to ignore them.
#[derive(Debug, clap::Args)]
pub struct CleanArgs {
    #[command(flatten)]
    pub scheme: crate::cli::SchemeArgs,

    /// Build configuration whose products to clean (e.g. Debug, Release).
    #[arg(
        long,
        env = "SWEETPAD_CONFIGURATION",
        help_heading = crate::cli::TARGET_SELECTION
    )]
    pub configuration: Option<String>,

    /// Also delete this project's DerivedData folder(s), without a prompt:
    /// the flag is the consent. 'derived-data purge' deletes the same folders,
    /// or every project's with '--all', and asks first unless given '--yes'.
    #[arg(long)]
    pub purge: bool,
}

/// The clean outcome: what was cleaned, any DerivedData folders purged, and
/// the note naming the same-named folders `--purge` kept.
struct CleanReport {
    cleaned: &'static str,
    purged: Vec<String>,
    others_note: Option<String>,
}

impl Render for CleanReport {
    fn human(&self, out: &Output) {
        out.note(&format!("cleaned ({})", self.cleaned));
        for p in &self.purged {
            out.note(&format!("purged {p}"));
        }
        if let Some(note) = &self.others_note {
            out.note(note);
        }
    }

    fn json(&self) -> serde_json::Value {
        serde_json::json!({
            "cleaned": self.cleaned,
            "purged": self.purged,
        })
    }
}

/// The `xcodebuild clean` argv for a scheme and configuration. The project's
/// `[xcodebuild] args` place the products (a `SYMROOT=`, an `-xcconfig`), so
/// the clean takes them to reach where the build wrote, without the flags
/// `xcodebuild clean` fails on ([`xcodebuild::for_action`]).
fn xcodebuild_clean_args(
    ctx: &Context,
    container: &Container,
    scheme: String,
    configuration: String,
) -> Result<Vec<String>, CliError> {
    let mut args: Vec<String> = vec![
        "clean".into(),
        "-scheme".into(),
        scheme,
        "-configuration".into(),
        configuration,
    ];
    args.extend(xcodebuild::container_args(container));
    args.extend(ctx.xcodebuild_args(xcodebuild::Action::Clean, &[])?);
    Ok(args)
}

pub fn run(ctx: &mut Context, purge: bool) -> CommandResult {
    let mut resolved = resolve::resolve(ctx)?;

    let cleaned = match &resolved.container {
        Container::SwiftPackage(_) => {
            // `swift package clean` takes neither, and accepting them here
            // would silently not do what was asked (the rule `build` states
            // for its own inapplicable flags).
            if ctx.targeting.scheme.is_some() {
                return Err(CliError::new(
                    "--scheme doesn't apply to a Swift package; 'swift package clean' cleans \
                     the whole package",
                ));
            }
            if ctx.targeting.configuration.is_some() {
                return Err(CliError::new(
                    "--configuration doesn't apply to a Swift package; 'swift package clean' \
                     removes every configuration's products",
                ));
            }
            let cwd = xcodebuild::working_dir(&resolved.container);
            // Under --json/-o ndjson, capture the toolchain's output so
            // nothing interleaves with the envelope on stdout — the same
            // discipline as the xcodebuild branch below.
            if ctx.out.is_json() || ctx.out.is_ndjson() {
                let run = process::run_captured("swift", &["package", "clean"], cwd.as_deref())?;
                if !run.success {
                    return Err(CliError::new(format!(
                        "swift package clean exited with a non-zero status:\n{}",
                        run.tail
                    ))
                    .context("cleaning the package"));
                }
            } else {
                process::stream("swift", &["package", "clean"], cwd.as_deref())
                    .context("cleaning the package")?;
            }
            "swift package clean"
        }
        Container::Project(_) | Container::Workspace(_) => {
            let schemes = resolve::schemes(&resolved.container)?;
            let scheme = resolve::settle_scheme(ctx, &mut resolved, &schemes, true)?;
            let configuration = resolve::settle_configuration(ctx, &mut resolved, true)?;
            let args = xcodebuild_clean_args(ctx, &resolved.container, scheme, configuration)?;
            let arg_refs: Vec<&str> = args.iter().map(String::as_str).collect();
            let cwd = xcodebuild::working_dir(&resolved.container);
            let ok = if ctx.out.is_json() || ctx.out.is_ndjson() {
                let run = process::run_captured("xcodebuild", &arg_refs, cwd.as_deref())?;
                if !run.success {
                    return Err(CliError::new(format!(
                        "xcodebuild clean exited with a non-zero status:\n{}",
                        run.tail
                    ))
                    .context("cleaning the project"));
                }
                true
            } else {
                buildlog::run(
                    "xcodebuild",
                    &arg_refs,
                    cwd.as_deref(),
                    &ctx.out,
                    "Cleaning",
                )?
            };
            if !ok {
                return Err(
                    CliError::new("xcodebuild clean exited with a non-zero status")
                        .context("cleaning the project"),
                );
            }
            "xcodebuild clean"
        }
    };

    // `--purge` is explicit consent on the command line — it deletes only this
    // project's DerivedData folder(s), never the whole store, and never the
    // folder another checkout of the same project wrote.
    let mut purged = Vec::new();
    let mut others_note = None;
    if purge {
        // A Swift package builds into `.build/` beside `Package.swift`, not
        // DerivedData — purging only the DerivedData store would report
        // success having deleted nothing that `swift build` wrote.
        if let Container::SwiftPackage(manifest) = &resolved.container
            && let Some(dir) = manifest.parent()
        {
            let build_dir = dir.join(".build");
            if build_dir.is_dir() {
                std::fs::remove_dir_all(&build_dir).map_err(|e| {
                    CliError::new(format!("failed to remove {}: {e}", build_dir.display()))
                })?;
                purged.push(build_dir.display().to_string());
            }
        }
        let scope = super::derived_data::project_scope(ctx)?;
        for path in &scope.own {
            std::fs::remove_dir_all(path)
                .map_err(|e| CliError::new(format!("failed to remove {}: {e}", path.display())))?;
            purged.push(path.display().to_string());
        }
        others_note = scope.others_note("kept");
    }

    Ok(Rendered::data(CleanReport {
        cleaned,
        purged,
        others_note,
    }))
}
