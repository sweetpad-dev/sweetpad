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
    /// does its argv actually start a BSP server?
    Doctor,
    /// Run the BSP server loop over stdio — what buildServer.json's argv
    /// execs. sourcekit-lsp is the caller; humans never need this.
    #[command(hide = true)]
    Serve {
        /// Raw server flags (--project/--workspace, --xcode,
        /// --derived-data-path), parsed by the server itself.
        #[arg(allow_hyphen_values = true, trailing_var_arg = true)]
        args: Vec<String>,
    },
}

pub fn run(ctx: &mut Context, action: &Action) -> CommandResult {
    match action {
        Action::Init { output_file } => init(ctx, output_file.as_deref()),
        Action::Doctor => doctor(ctx),
        Action::Serve { args } => {
            // The bsp resource's ContainerArgs are global, so clap parses
            // --project/--workspace out of the argv before the trailing args
            // see them — reassemble the server's flag list from the parsed
            // targeting, then append whatever else the config carried
            // (--xcode, --derived-data-path, --config).
            let mut server_args: Vec<String> = Vec::new();
            if let Some(ws) = &ctx.targeting.workspace {
                server_args.push("--workspace".into());
                server_args.push(ws.display().to_string());
            }
            if let Some(p) = &ctx.targeting.project {
                server_args.push("--project".into());
                server_args.push(p.display().to_string());
            }
            server_args.extend(args.iter().cloned());
            // The server owns stdio (JSON-RPC frames on stdout); no envelope.
            sweetpad_core::bsp::run(&server_args).map_err(CliError::new)?;
            Ok(Rendered::Streamed)
        }
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
        return doctor_swift_package(container.path());
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
            let argv: Vec<String> = json
                .get("argv")
                .and_then(serde_json::Value::as_array)
                .map(|a| {
                    a.iter()
                        .filter_map(serde_json::Value::as_str)
                        .map(str::to_string)
                        .collect()
                })
                .unwrap_or_default();
            if let Some(server) = argv.first() {
                let exists = std::path::Path::new(server).exists();
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
                if exists {
                    checks.push(probe_launch(&argv));
                }
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

/// The definitive doctor check: exec the config's argv with stdin closed. A
/// real BSP server reads EOF and exits cleanly; a wrong argv (a usage error,
/// a binary that isn't a server) exits non-zero immediately.
fn probe_launch(argv: &[String]) -> BspCheck {
    use std::process::{Command, Stdio};
    let result = Command::new(&argv[0])
        .args(&argv[1..])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
    match result {
        Ok(status) if status.success() => BspCheck {
            ok: true,
            what: "argv starts a BSP server".to_string(),
        },
        Ok(status) => BspCheck {
            ok: false,
            what: format!(
                "argv starts a BSP server (exec exited with {status} — rerun `sweetpad bsp init`)"
            ),
        },
        Err(e) => BspCheck {
            ok: false,
            what: format!("argv starts a BSP server (exec failed: {e})"),
        },
    }
}

/// `bsp doctor` for a Swift package: healthy means *no* `buildServer.json`
/// (sourcekit-lsp handles SwiftPM natively; a leftover config overrides that
/// and breaks SPM semantics).
#[allow(clippy::unnecessary_wraps)] // uniform CommandResult across the doctor arms
fn doctor_swift_package(manifest_path: &Path) -> CommandResult {
    let config_path = buildserver_path(manifest_path, None);
    let stale = config_path.exists();
    let checks = vec![BspCheck {
        ok: !stale,
        what: format!(
            "no buildServer.json overriding sourcekit-lsp's native SwiftPM support{}",
            if stale {
                " — remove it (the BSP server only handles Xcode projects)"
            } else {
                ""
            }
        ),
    }];
    let report = BspDoctor {
        path: config_path.display().to_string(),
        checks,
    };
    if stale {
        Ok(Rendered::data_with_exit(report, 1))
    } else {
        Ok(Rendered::data(report))
    }
}

fn init(ctx: &mut Context, out: Option<&std::path::Path>) -> CommandResult {
    let container = resolve::container(ctx)?;

    let mut args: Vec<String> = match &container {
        Container::Workspace(p) => vec!["--workspace".into(), p.display().to_string()],
        Container::Project(p) => vec!["--project".into(), p.display().to_string()],
        Container::SwiftPackage(p) => {
            return Ok(Rendered::data(init_swift_package(p, out)));
        }
    };
    if let Some(out) = out {
        args.push("--output".into());
        args.push(out.display().to_string());
    }

    // The written argv must exec *this* binary's server loop: `sweetpad bsp
    // serve …` (the standalone bsp-server binary spells it `bsp …`).
    sweetpad_core::bsp::write_config(&args, &["bsp", "serve"]).map_err(CliError::new)?;

    let path = buildserver_path(container.path(), out);
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
