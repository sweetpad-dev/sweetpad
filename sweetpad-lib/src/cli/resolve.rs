//! Target resolution: figuring out *what* a command acts on.
//!
//! Layered precedence, highest first:
//!
//! ```text
//! explicit flag  >  env var  >  config file  >  remembered state  >  auto-discovery
//! ```
//!
//! (Env vars are surfaced as flags by clap's `env = …`, so by the time we're
//! here they're folded into the flag layer.) When a value is still missing and
//! stdout is a TTY, commands may drop to an interactive picker; in non-TTY/CI
//! contexts that's a hard error instead. The picker itself lands with the
//! first command that needs it — this module wires the precedence.

use std::path::{Path, PathBuf};

use crate::cli::config::Defaults;
use crate::cli::{CliError, Context};

/// A discovered project container in the working directory.
#[derive(Debug, Clone)]
pub enum Container {
    Workspace(PathBuf),
    Project(PathBuf),
    SwiftPackage(PathBuf),
}

impl Container {
    /// Absolute path to the container, used as the config/state key.
    #[must_use]
    pub fn path(&self) -> &Path {
        match self {
            Container::Workspace(p) | Container::Project(p) | Container::SwiftPackage(p) => p,
        }
    }

    /// Stable identity used to key per-project config overrides and remembered
    /// state — the canonicalized absolute path.
    #[must_use]
    pub fn key(&self) -> String {
        std::fs::canonicalize(self.path())
            .unwrap_or_else(|_| self.path().to_path_buf())
            .to_string_lossy()
            .into_owned()
    }
}

/// Resolve the project container from explicit flags, else by auto-discovery in
/// the current directory.
pub fn container(ctx: &Context) -> Result<Container, CliError> {
    if let Some(ws) = &ctx.global.workspace {
        return Ok(Container::Workspace(ws.clone()));
    }
    if let Some(proj) = &ctx.global.project {
        return Ok(Container::Project(proj.clone()));
    }
    let cwd =
        std::env::current_dir().map_err(|e| CliError::new(format!("cannot read cwd: {e}")))?;
    discover(&cwd).ok_or_else(|| {
        CliError::new(
            "no .xcworkspace, .xcodeproj, or Package.swift found in the current \
             directory (pass --workspace/--project)",
        )
    })
}

/// Auto-discovery: prefer a workspace, then a project, then a Swift package.
/// Mirrors how Xcode/xcodebuild pick a container.
#[must_use]
pub fn discover(dir: &Path) -> Option<Container> {
    let mut workspace = None;
    let mut project = None;
    let mut package = None;
    let entries = std::fs::read_dir(dir).ok()?;
    for entry in entries.flatten() {
        let path = entry.path();
        match path.extension().and_then(|e| e.to_str()) {
            Some("xcworkspace") => workspace = workspace.or(Some(path)),
            Some("xcodeproj") => project = project.or(Some(path)),
            _ => {
                if path.file_name().and_then(|f| f.to_str()) == Some("Package.swift") {
                    package = package.or(Some(path));
                }
            }
        }
    }
    workspace
        .map(Container::Workspace)
        .or(project.map(Container::Project))
        .or(package.map(Container::SwiftPackage))
}

/// The fully resolved targeting for a command, after layering.
#[derive(Debug, Clone)]
pub struct Resolved {
    pub container: Container,
    pub scheme: Option<String>,
    pub configuration: Option<String>,
    pub destination: Option<String>,
}

/// Apply the layered precedence (flag > config > state) for the soft targeting
/// values. Auto-discovery / interactive selection of a *specific* scheme or
/// destination is left to the individual commands (they know the candidate
/// lists); this folds the persisted layers into a starting point.
pub fn resolve(ctx: &Context) -> Result<Resolved, CliError> {
    let container = container(ctx)?;
    let key = container.key();
    let cfg: Defaults = ctx.config.for_project(&key);
    let st = ctx.state.projects.get(&key);

    let pick = |flag: &Option<String>, cfg: &Option<String>, state: Option<&String>| {
        flag.clone()
            .or_else(|| cfg.clone())
            .or_else(|| state.cloned())
    };

    Ok(Resolved {
        scheme: pick(
            &ctx.global.scheme,
            &cfg.scheme,
            st.and_then(|s| s.scheme.as_ref()),
        ),
        configuration: pick(
            &ctx.global.configuration,
            &cfg.configuration,
            st.and_then(|s| s.configuration.as_ref()),
        ),
        destination: pick(
            &ctx.global.destination,
            &cfg.destination,
            st.and_then(|s| s.destination.as_ref()),
        ),
        container,
    })
}

/// Helper for commands: a value is required but unresolved, and we cannot
/// prompt (non-interactive). Produces the standard strict error.
#[must_use]
pub fn missing(what: &str) -> CliError {
    CliError::new(format!(
        "no {what} specified and stdout is not a TTY; pass --{what} or set it in config"
    ))
}

/// The `xcodebuild -list` scheme set for a container. Workspaces merge member
/// projects' schemes; bare projects use their own (file + autocreated) set.
/// Swift packages aren't supported — xcodebuild synthesizes those schemes from
/// the manifest, which the resolver doesn't model. Shared by every command that
/// needs to enumerate or pick a scheme.
pub fn schemes(container: &Container) -> Result<Vec<String>, CliError> {
    match container {
        Container::Workspace(p) => crate::workspace::open(p)
            .map(|w| w.merged_schemes())
            .map_err(|e| CliError::new(format!("failed to read workspace {}: {e}", p.display()))),
        Container::Project(p) => crate::project::open(p)
            .map(|proj| proj.schemes)
            .map_err(|e| CliError::new(format!("failed to read project {}: {e}", p.display()))),
        // Swift packages have no pbxproj; ask xcodebuild for the synthesized
        // schemes (this drives SPM build/test/run).
        Container::SwiftPackage(_) => crate::cli::xcodebuild::list_schemes(container),
    }
}

/// Resolve a required choice: return `current` if set, auto-pick when there's
/// exactly one candidate, prompt interactively when stdout is a TTY, else error
/// strictly. Used to settle the scheme/destination when a command needs one.
pub fn choose(
    ctx: &Context,
    what: &str,
    current: Option<String>,
    candidates: &[String],
) -> Result<String, CliError> {
    if let Some(c) = current {
        return Ok(c);
    }
    match candidates.len() {
        0 => Err(CliError::new(format!("no {what} available"))),
        1 => Ok(candidates[0].clone()),
        _ if ctx.out.is_interactive() => prompt_choice(what, candidates),
        _ => Err(missing(what)),
    }
}

/// A fully-settled build target: the three things `xcodebuild` always needs.
#[derive(Debug, Clone)]
pub struct BuildTarget {
    pub scheme: String,
    pub configuration: String,
    /// Raw `-destination` specifier (e.g. `platform=iOS Simulator,id=<udid>`).
    pub destination: String,
}

/// Settle a complete build target from the layered resolution, falling back to
/// interactive pickers (scheme from the project, destination from `simctl`)
/// when a TTY is available. Configuration defaults to `Debug`.
pub fn build_target(ctx: &Context, resolved: &Resolved) -> Result<BuildTarget, CliError> {
    let candidates = schemes(&resolved.container)?;
    let scheme = choose(ctx, "scheme", resolved.scheme.clone(), &candidates)?;
    let configuration = resolved
        .configuration
        .clone()
        .unwrap_or_else(|| "Debug".to_string());

    let destination = if let Some(d) = resolved.destination.clone() {
        d
    } else {
        let sims = crate::cli::simctl::list()?;
        let labels: Vec<String> = sims
            .iter()
            .map(crate::cli::simctl::Simulator::label)
            .collect();
        let chosen = choose(ctx, "destination", None, &labels)?;
        sims.iter()
            .find(|s| s.label() == chosen)
            .map(crate::cli::simctl::Simulator::destination)
            .ok_or_else(|| CliError::new("destination not found"))?
    };

    Ok(BuildTarget {
        scheme,
        configuration,
        destination,
    })
}

/// Persist the settled selections to the machine state file so later commands
/// don't re-prompt. Best-effort: failures to write state never fail a command.
pub fn remember(ctx: &mut Context, resolved: &Resolved, target: &BuildTarget) {
    let key = resolved.container.key();
    let st = ctx.state.project_mut(&key);
    st.scheme = Some(target.scheme.clone());
    st.configuration = Some(target.configuration.clone());
    st.destination = Some(target.destination.clone());
    let _ = ctx.state.save();
}

/// Interactive fuzzy picker (the design's TTY fallback): type to filter, arrows
/// to move, Enter to select. Only reached when stdout is a TTY.
fn prompt_choice(what: &str, candidates: &[String]) -> Result<String, CliError> {
    let idx = dialoguer::FuzzySelect::new()
        .with_prompt(format!("Select a {what}"))
        .items(candidates)
        .default(0)
        .interact()
        .map_err(|e| CliError::new(format!("selection cancelled: {e}")))?;
    Ok(candidates[idx].clone())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::{Context, GlobalArgs, config::Config, output::Output, state::State};
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_dir(tag: &str) -> std::path::PathBuf {
        let n = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("sweetpad-test-{tag}-{n}"));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

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
        };
        let out = Output::new(&global);
        Context {
            global,
            config: Config::default(),
            state: State::default(),
            out,
        }
    }

    #[test]
    fn discover_prefers_workspace_then_project_then_package() {
        let dir = temp_dir("disc");
        std::fs::create_dir(dir.join("App.xcodeproj")).unwrap();
        std::fs::write(dir.join("Package.swift"), "// pkg").unwrap();
        // Project beats a bare package.
        assert!(matches!(discover(&dir), Some(Container::Project(_))));

        std::fs::create_dir(dir.join("App.xcworkspace")).unwrap();
        // Workspace beats project.
        assert!(matches!(discover(&dir), Some(Container::Workspace(_))));

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn discover_finds_swift_package_alone() {
        let dir = temp_dir("pkg");
        std::fs::write(dir.join("Package.swift"), "// pkg").unwrap();
        assert!(matches!(discover(&dir), Some(Container::SwiftPackage(_))));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn discover_none_in_empty_dir() {
        let dir = temp_dir("empty");
        assert!(discover(&dir).is_none());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn container_key_is_canonical_path() {
        let dir = temp_dir("key");
        let proj = dir.join("App.xcodeproj");
        std::fs::create_dir(&proj).unwrap();
        let key = Container::Project(proj.clone()).key();
        assert_eq!(key, std::fs::canonicalize(&proj).unwrap().to_string_lossy());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn choose_returns_current_when_set() {
        let c = ctx();
        assert_eq!(choose(&c, "scheme", Some("X".into()), &[]).unwrap(), "X");
    }

    #[test]
    fn choose_auto_picks_single_candidate() {
        let c = ctx();
        assert_eq!(
            choose(&c, "scheme", None, &["only".into()]).unwrap(),
            "only"
        );
    }

    #[test]
    fn choose_errors_when_empty() {
        let c = ctx();
        assert!(choose(&c, "scheme", None, &[]).is_err());
    }

    #[test]
    fn choose_is_strict_when_ambiguous_and_non_interactive() {
        // Tests don't run under a TTY, so multiple candidates must error.
        let c = ctx();
        assert!(choose(&c, "scheme", None, &["a".into(), "b".into()]).is_err());
    }
}
