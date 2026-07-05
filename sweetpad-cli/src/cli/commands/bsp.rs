//! `sweetpad bsp …` — Build Server Protocol integration. `bsp init` writes a
//! `buildServer.json` pointing sourcekit-lsp at this binary's bundled BSP
//! server (see [`sweetpad_core::bsp`]), enabling cross-file autocomplete.

use std::path::{Path, PathBuf};

use clap::Subcommand;

use crate::cli::output::Output;
use crate::cli::resolve::{self, Container};
use crate::cli::{CliError, CommandResult, Context, Render, Rendered};

#[derive(Debug, Subcommand)]
pub enum Action {
    /// Generate buildServer.json for sourcekit-lsp autocomplete.
    Init {
        /// Where to write buildServer.json (defaults next to the container —
        /// its parent directory; for a Swift package, the package directory).
        // `--output-file`, not `--output`: the global `-o/--output` selects the
        // output *mode* (json/ndjson) on every command, so it owns that flag.
        #[arg(long = "output-file")]
        output_file: Option<PathBuf>,
    },
    /// Check the buildServer.json wiring: does it exist, is it complete, and
    /// does it point at a binary that's still there?
    Doctor,
}

pub fn run(ctx: &mut Context, action: &Action) -> CommandResult {
    match action {
        Action::Init { output_file } => init(ctx, output_file.as_deref()),
        Action::Doctor => doctor(ctx),
    }
}

/// One `bsp doctor` check line.
struct BspCheck {
    ok: bool,
    what: String,
}

/// The `bsp doctor` report: ✓/✗ per check; exits 1 when anything failed.
struct BspDoctor {
    path: String,
    checks: Vec<BspCheck>,
}

impl Render for BspDoctor {
    fn human(&self, out: &Output) {
        out.line(&format!("buildServer.json: {}", self.path));
        for c in &self.checks {
            out.line(&format!("  {} {}", if c.ok { "✓" } else { "✗" }, c.what));
        }
    }

    fn json(&self) -> serde_json::Value {
        let checks: Vec<serde_json::Value> = self
            .checks
            .iter()
            .map(|c| serde_json::json!({ "ok": c.ok, "check": c.what }))
            .collect();
        serde_json::json!({ "buildServerJson": self.path, "checks": checks })
    }
}

/// sourcekit-lsp silently skips a buildServer.json missing any of these.
const REQUIRED_FIELDS: [&str; 5] = ["name", "version", "bspVersion", "languages", "argv"];

fn doctor(ctx: &mut Context) -> CommandResult {
    let container = resolve::container(ctx)?;
    if matches!(container, Container::SwiftPackage(_)) {
        return Ok(Rendered::data(init_swift_package(container.path(), None)));
    }
    let path = buildserver_path(container.path(), None);
    let mut checks = Vec::new();

    let Ok(text) = std::fs::read_to_string(&path) else {
        checks.push(BspCheck {
            ok: false,
            what: "file exists (run `sweetpad bsp init` to create it)".to_string(),
        });
        return Ok(Rendered::data_with_exit(
            BspDoctor {
                path: path.display().to_string(),
                checks,
            },
            1,
        ));
    };
    checks.push(BspCheck {
        ok: true,
        what: "file exists".to_string(),
    });

    match serde_json::from_str::<serde_json::Value>(&text) {
        Err(e) => checks.push(BspCheck {
            ok: false,
            what: format!("valid JSON ({e})"),
        }),
        Ok(json) => {
            for field in REQUIRED_FIELDS {
                let present = !json.get(field).is_none_or(serde_json::Value::is_null);
                checks.push(BspCheck {
                    ok: present,
                    what: format!(
                        "`{field}` present{}",
                        if present {
                            ""
                        } else {
                            " (sourcekit-lsp silently skips the file without it — rerun `sweetpad bsp init`)"
                        }
                    ),
                });
            }
            let server = json
                .get("argv")
                .and_then(|a| a.get(0))
                .and_then(serde_json::Value::as_str)
                .map(str::to_string);
            if let Some(server) = server {
                let exists = std::path::Path::new(&server).exists();
                checks.push(BspCheck {
                    ok: exists,
                    what: format!(
                        "server binary {server}{}",
                        if exists {
                            " exists"
                        } else {
                            " is missing — rerun `sweetpad bsp init`"
                        }
                    ),
                });
            }
        }
    }

    let failed = checks.iter().any(|c| !c.ok);
    let report = BspDoctor {
        path: path.display().to_string(),
        checks,
    };
    if failed {
        Ok(Rendered::data_with_exit(report, 1))
    } else {
        Ok(Rendered::data(report))
    }
}

fn init(ctx: &mut Context, output: Option<&std::path::Path>) -> CommandResult {
    let container = resolve::container(ctx)?;

    let mut args: Vec<String> = match &container {
        Container::Workspace(p) => vec!["--workspace".into(), p.display().to_string()],
        Container::Project(p) => vec!["--project".into(), p.display().to_string()],
        Container::SwiftPackage(p) => {
            return Ok(Rendered::data(init_swift_package(p, output)));
        }
    };
    if let Some(out) = output {
        args.push("--output".into());
        args.push(out.display().to_string());
    }

    sweetpad_core::bsp::write_config(&args).map_err(CliError::new)?;

    let path = buildserver_path(container.path(), output);
    Ok(Rendered::data(BspInit {
        build_server_json: path.display().to_string(),
    }))
}

/// `bsp init` for an Xcode project/workspace: the config is written for its side
/// effect, and JSON reports the `buildServer.json` path. Human mode prints
/// nothing (the file on disk is the result).
struct BspInit {
    build_server_json: String,
}

impl Render for BspInit {
    fn human(&self, _out: &Output) {}

    fn json(&self) -> serde_json::Value {
        serde_json::json!({
            "buildServerJson": self.build_server_json,
        })
    }
}

/// `bsp init` for a Swift package: no `buildServer.json` is written (sourcekit-lsp
/// handles SwiftPM natively). JSON reports `{buildServerJson: null, native: true,
/// staleBuildServerJson}`; human mode prints the native-support note plus a stale
/// warning when a leftover config exists.
struct BspSwiftPackage {
    /// A pre-existing `buildServer.json` to warn about, if any.
    stale: Option<String>,
}

impl Render for BspSwiftPackage {
    fn human(&self, out: &Output) {
        out.note(
            "Swift package detected; sourcekit-lsp supports SwiftPM natively, so no \
             buildServer.json is written",
        );
        if let Some(stale) = &self.stale {
            out.note(&format!(
                "a buildServer.json already exists at {stale} — remove it so sourcekit-lsp \
                 uses its native SwiftPM support instead of the BSP server (which only \
                 handles Xcode projects)"
            ));
        }
    }

    fn json(&self) -> serde_json::Value {
        serde_json::json!({
            "buildServerJson": null,
            "native": true,
            "staleBuildServerJson": self.stale,
        })
    }
}

/// `bsp init` for a Swift package. sourcekit-lsp supports SwiftPM natively — it
/// reads `Package.swift` and builds the index itself — so we deliberately do
/// *not* write a `buildServer.json`; one would override that native path with
/// our BSP server, which only understands Xcode projects. We don't trigger a
/// build either; sourcekit-lsp populates the index on demand.
///
/// The one real hazard is a pre-existing `buildServer.json`: its mere presence
/// makes sourcekit-lsp use BSP and breaks SPM semantics, so flag it loudly. We
/// warn rather than delete — removing a file the user may have authored is their
/// call (or the extension's).
fn init_swift_package(manifest_path: &Path, output: Option<&Path>) -> BspSwiftPackage {
    let config_path = buildserver_path(manifest_path, output);
    let stale = config_path
        .exists()
        .then(|| config_path.display().to_string());
    BspSwiftPackage { stale }
}

/// Where sourcekit-lsp looks for a `buildServer.json`: the explicit `--output`,
/// else next to the container (its parent directory).
fn buildserver_path(container: &Path, output: Option<&Path>) -> PathBuf {
    output.map_or_else(
        || {
            container
                .parent()
                .unwrap_or_else(|| Path::new("."))
                .join("buildServer.json")
        },
        Path::to_path_buf,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn buildserver_path_defaults_next_to_container() {
        assert_eq!(
            buildserver_path(Path::new("/pkg/Package.swift"), None),
            PathBuf::from("/pkg/buildServer.json")
        );
    }

    #[test]
    fn buildserver_path_honors_explicit_output() {
        assert_eq!(
            buildserver_path(
                Path::new("/pkg/Package.swift"),
                Some(Path::new("/tmp/bs.json"))
            ),
            PathBuf::from("/tmp/bs.json")
        );
    }
}
