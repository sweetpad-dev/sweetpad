//! `sweetpad clean` — remove build artifacts: `xcodebuild clean` for the
//! resolved scheme (or `swift package clean` for a package), with `--purge`
//! also deleting the project's DerivedData folder(s). The standalone
//! counterpart of `build --clean`, so a clean no longer requires a rebuild.

use crate::cli::output::Output;
use crate::cli::resolve::{self, Container};
use crate::cli::{
    CliError, CommandResult, Context, ErrorContext, Render, Rendered, buildlog, process,
    xcodebuild,
};

/// `clean`'s flags: exactly the values `xcodebuild clean` consumes — the
/// container, a scheme, and a configuration. The full build tier would
/// advertise `--destination`/`--on`/`--sdk` only to ignore them.
#[derive(Debug, clap::Args)]
pub struct CleanArgs {
    #[command(flatten)]
    pub scheme: crate::cli::SchemeArgs,

    /// Build configuration whose products to clean (e.g. Debug, Release).
    #[arg(long, env = "SWEETPAD_CONFIGURATION")]
    pub configuration: Option<String>,

    /// Also delete this project's DerivedData folder(s). The flag itself is
    /// the consent — it only ever touches this project's folders, unlike the
    /// store-wide `derived-data purge` (which prompts).
    #[arg(long)]
    pub purge: bool,
}

/// The clean outcome: what was cleaned, and any DerivedData folders purged.
struct CleanReport {
    cleaned: &'static str,
    purged: Vec<String>,
}

impl Render for CleanReport {
    fn human(&self, out: &Output) {
        out.note(&format!("cleaned ({})", self.cleaned));
        for p in &self.purged {
            out.note(&format!("purged {p}"));
        }
    }

    fn json(&self) -> serde_json::Value {
        serde_json::json!({
            "cleaned": self.cleaned,
            "purged": self.purged,
        })
    }
}

pub fn run(ctx: &mut Context, purge: bool) -> CommandResult {
    let resolved = resolve::resolve(ctx)?;

    let cleaned = match &resolved.container {
        Container::SwiftPackage(_) => {
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
            if let Some(s) = &resolved.scheme {
                resolve::validate_choice("scheme", s, &schemes)?;
            }
            let scheme = resolve::choose(ctx, "scheme", resolved.scheme.clone(), &schemes)?;
            let configuration = resolve::settle_configuration(ctx, &resolved)?;

            let mut args: Vec<String> = vec![
                "clean".into(),
                "-scheme".into(),
                scheme,
                "-configuration".into(),
                configuration,
            ];
            args.extend(xcodebuild::container_args(&resolved.container));
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
                buildlog::run("xcodebuild", &arg_refs, cwd.as_deref(), &ctx.out, "Cleaning")?
            };
            if !ok {
                return Err(CliError::new("xcodebuild clean exited with a non-zero status")
                    .context("cleaning the project"));
            }
            "xcodebuild clean"
        }
    };

    // `--purge` is explicit consent on the command line — it deletes only this
    // project's DerivedData folder(s), never the whole store.
    let mut purged = Vec::new();
    if purge {
        for path in super::derived_data::project_paths(ctx)? {
            std::fs::remove_dir_all(&path)
                .map_err(|e| CliError::new(format!("failed to remove {}: {e}", path.display())))?;
            purged.push(path.display().to_string());
        }
    }

    Ok(Rendered::data(CleanReport { cleaned, purged }))
}
