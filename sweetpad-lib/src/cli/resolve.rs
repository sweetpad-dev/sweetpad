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
use crate::cli::state::{SelectedDestination, State};
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
    if let Some(ws) = &ctx.targeting.workspace {
        return Ok(Container::Workspace(ws.clone()));
    }
    if let Some(proj) = &ctx.targeting.project {
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
            &ctx.targeting.scheme,
            &cfg.scheme,
            st.and_then(|s| s.scheme.as_ref()),
        ),
        configuration: pick(
            &ctx.targeting.configuration,
            &cfg.configuration,
            st.and_then(|s| s.configuration.as_ref()),
        ),
        destination: pick(
            &ctx.targeting.destination,
            &cfg.destination,
            st.and_then(|s| s.destination.as_ref()),
        ),
        container,
    })
}

/// Like [`resolve`], but for the test action. Layers, highest first:
///
/// ```text
/// flag > testing config > testing state > build config > build state
/// ```
///
/// So a testing override (config `[….testing]` or a `context select --testing`
/// pick) wins, and `test` otherwise follows the build selection — it diverges
/// only once you pin something test-specific.
pub fn resolve_testing(ctx: &Context) -> Result<Resolved, CliError> {
    let container = container(ctx)?;
    let key = container.key();
    let cfg: Defaults = ctx.config.for_project(&key);
    let st = ctx.state.projects.get(&key);

    let pick = |flag: &Option<String>,
                test_cfg: &Option<String>,
                test_state: Option<&String>,
                build_cfg: &Option<String>,
                build_state: Option<&String>| {
        flag.clone()
            .or_else(|| test_cfg.clone())
            .or_else(|| test_state.cloned())
            .or_else(|| build_cfg.clone())
            .or_else(|| build_state.cloned())
    };

    Ok(Resolved {
        scheme: pick(
            &ctx.targeting.scheme,
            &cfg.testing.scheme,
            st.and_then(|s| s.testing.scheme.as_ref()),
            &cfg.scheme,
            st.and_then(|s| s.scheme.as_ref()),
        ),
        configuration: pick(
            &ctx.targeting.configuration,
            &cfg.testing.configuration,
            st.and_then(|s| s.testing.configuration.as_ref()),
            &cfg.configuration,
            st.and_then(|s| s.configuration.as_ref()),
        ),
        destination: pick(
            &ctx.targeting.destination,
            &cfg.testing.destination,
            st.and_then(|s| s.testing.destination.as_ref()),
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

/// The scheme set for a container, read without xcodebuild. Workspaces merge
/// member projects' schemes; bare projects use their own (file + autocreated)
/// set, both via the in-process pbxproj reader. Swift packages have no pbxproj,
/// so their schemes — the product names xcodebuild would synthesize — are read
/// straight from the manifest (`swift package dump-package`). Shared by every
/// command that needs to enumerate or pick a scheme.
pub fn schemes(container: &Container) -> Result<Vec<String>, CliError> {
    match container {
        Container::Workspace(p) => crate::workspace::open(p)
            .map(|w| w.merged_schemes())
            .map_err(|e| CliError::new(format!("failed to read workspace {}: {e}", p.display()))),
        Container::Project(p) => crate::project::open(p)
            .map(|proj| proj.schemes)
            .map_err(|e| CliError::new(format!("failed to read project {}: {e}", p.display()))),
        // Swift packages have no pbxproj; derive schemes from the manifest
        // (this drives SPM build/test/run) — no xcodebuild needed.
        Container::SwiftPackage(_) => crate::cli::swiftpm::schemes(container),
    }
}

/// The build configuration names for a container — the candidates for picking a
/// configuration. Workspaces merge member projects' configurations (like
/// schemes); bare projects use their own. Swift packages have no Xcode
/// configurations, so expose the Debug/Release pair SwiftPM maps to its
/// `--configuration` (see [`crate::cli::swiftpm::configuration_arg`]).
pub fn configurations(container: &Container) -> Result<Vec<String>, CliError> {
    match container {
        Container::Workspace(p) => crate::workspace::open(p)
            .map(|w| w.merged_configurations())
            .map_err(|e| CliError::new(format!("failed to read workspace {}: {e}", p.display()))),
        Container::Project(p) => crate::project::open(p)
            .map(|proj| proj.configurations)
            .map_err(|e| CliError::new(format!("failed to read project {}: {e}", p.display()))),
        Container::SwiftPackage(_) => Ok(vec!["Debug".to_string(), "Release".to_string()]),
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

/// Pick a single simulator for the side-effecting `simulator`/`app` actions.
/// An explicit `target` (UDID or name) wins; otherwise prefer the booted one
/// when exactly one is booted, prompt among the booted set when several are,
/// and fall back to the full list (auto-pick/​prompt/​strict-error via
/// [`choose`]) when none are booted. Shared so simulator subcommands and
/// `app open-url` resolve a sim the same way.
pub fn select_simulator<'a>(
    ctx: &Context,
    sims: &'a [crate::cli::simctl::Simulator],
    target: Option<&str>,
) -> Result<&'a crate::cli::simctl::Simulator, CliError> {
    use crate::cli::simctl::Simulator;

    if let Some(t) = target {
        return crate::cli::simctl::find(sims, t)
            .ok_or_else(|| CliError::new(format!("no simulator matching {t:?}")));
    }

    let booted: Vec<&Simulator> = sims.iter().filter(|s| s.is_booted()).collect();
    if booted.len() == 1 {
        return Ok(booted[0]);
    }

    // Prompt among the booted set when several are running, else the full list.
    let pool: Vec<&Simulator> = if booted.len() > 1 {
        booted
    } else {
        sims.iter().collect()
    };
    let labels: Vec<String> = pool.iter().map(|s| s.label()).collect();
    let chosen = choose(ctx, "simulator", None, &labels)?;
    pool.into_iter()
        .find(|s| s.label() == chosen)
        .ok_or_else(|| CliError::new("simulator not found"))
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
pub fn build_target(ctx: &mut Context, resolved: &Resolved) -> Result<BuildTarget, CliError> {
    let candidates = schemes(&resolved.container)?;
    let scheme = choose(ctx, "scheme", resolved.scheme.clone(), &candidates)?;
    let configuration = resolved
        .configuration
        .clone()
        .unwrap_or_else(|| "Debug".to_string());

    let destination = if let Some(d) = resolved.destination.clone() {
        d
    } else {
        let key = resolved.container.key();
        pick_destination(ctx, &key, &crate::cli::simctl::list()?)?
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

/// Persist a test run's selections. Writes the *build* context for the fields
/// the test took from it (so `test` keeps the build selection fresh, like
/// `build` does) but never overwrites a pinned testing override — those are
/// owned solely by `context … --testing`. Best-effort.
pub fn remember_testing(ctx: &mut Context, resolved: &Resolved, target: &BuildTarget) {
    let key = resolved.container.key();
    let st = ctx.state.project_mut(&key);
    if st.testing.scheme.is_none() {
        st.scheme = Some(target.scheme.clone());
    }
    if st.testing.configuration.is_none() {
        st.configuration = Some(target.configuration.clone());
    }
    if st.testing.destination.is_none() {
        st.destination = Some(target.destination.clone());
    }
    let _ = ctx.state.save();
}

/// Prompt for a destination from the simulator list and return the chosen
/// simulator's `-destination` specifier. The list is ordered most-used first
/// (from the project's usage stats), then booted, then the resolver's order, so
/// your habitual target sits at the top; the chosen simulator is recorded into
/// the project's recents and usage stats. Shared by `build_target` and
/// `context select`.
pub fn pick_destination(
    ctx: &mut Context,
    key: &str,
    sims: &[crate::cli::simctl::Simulator],
) -> Result<String, CliError> {
    let usage = ctx.state.projects.get(key).map(|s| &s.destination_usage);
    let (ordered, labels) = destination_choices(sims, usage);
    let chosen = choose(ctx, "destination", None, &labels)?;
    // Match by label position, not `Simulator::label`, since the displayed
    // labels carry the booted marker.
    let idx = labels
        .iter()
        .position(|l| *l == chosen)
        .ok_or_else(|| CliError::new("destination not found"))?;
    let sim = ordered[idx];
    track_destination(&mut ctx.state, key, sim);
    let _ = ctx.state.save();
    Ok(sim.destination())
}

/// Record a freshly-picked simulator into the project's recents (unique by id,
/// first-seen order) and bump its usage count — the inputs to most-used-first
/// ordering. Mirrors the extension's `trackSelectedDestination`.
fn track_destination(state: &mut State, key: &str, sim: &crate::cli::simctl::Simulator) {
    let st = state.project_mut(key);
    *st.destination_usage.entry(sim.udid.clone()).or_insert(0) += 1;
    if !st.destination_recents.iter().any(|d| d.id == sim.udid) {
        st.destination_recents.push(SelectedDestination {
            id: sim.udid.clone(),
            kind: sim.kind(),
            name: sim.name.clone(),
        });
    }
}

/// Adaptive ordering and display labels for the destination picker: most-used
/// first (by the project's usage stats), then booted, then the resolver's order
/// preserved by the stable sort. Booted simulators get a dot, with an aligning
/// gutter on the others — added only when some simulator is booted, so the
/// common all-shutdown list stays plain. Pure, so it's unit-testable.
fn destination_choices<'a>(
    sims: &'a [crate::cli::simctl::Simulator],
    usage: Option<&std::collections::BTreeMap<String, u32>>,
) -> (Vec<&'a crate::cli::simctl::Simulator>, Vec<String>) {
    let count = |s: &crate::cli::simctl::Simulator| {
        usage.and_then(|u| u.get(&s.udid)).copied().unwrap_or(0)
    };
    let mut ordered: Vec<&crate::cli::simctl::Simulator> = sims.iter().collect();
    // Stable: the input is already in the resolver's order, so equal keys keep
    // it. Float most-used, then booted, above that baseline.
    ordered.sort_by(|a, b| {
        count(b)
            .cmp(&count(a))
            .then_with(|| b.is_booted().cmp(&a.is_booted()))
    });
    let any_booted = sims.iter().any(crate::cli::simctl::Simulator::is_booted);
    let labels = ordered
        .iter()
        .map(|s| match (any_booted, s.is_booted()) {
            (false, _) => s.label(),
            (true, true) => format!("● {}", s.label()),
            (true, false) => format!("  {}", s.label()),
        })
        .collect();
    (ordered, labels)
}

/// Interactive fuzzy picker (the design's TTY fallback): type to filter, arrows
/// to move, Enter to select. Only reached when stdout is a TTY.
fn prompt_choice(what: &str, candidates: &[String]) -> Result<String, CliError> {
    let idx = dialoguer::FuzzySelect::with_theme(&dialoguer::theme::ColorfulTheme::default())
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
    use crate::cli::{
        Context, GlobalArgs, Targeting, config::Config, output::Output, state::State,
    };
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
        };
        let out = Output::new(&global);
        Context {
            global,
            targeting: Targeting::default(),
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

    fn sim(udid: &str, name: &str, booted: bool) -> crate::cli::simctl::Simulator {
        crate::cli::simctl::Simulator {
            udid: udid.into(),
            name: name.into(),
            state: if booted { "Booted" } else { "Shutdown" }.into(),
            available: true,
            os: "iOS".into(),
            os_version: "17.0".into(),
        }
    }

    #[test]
    fn select_simulator_honors_explicit_target() {
        let c = ctx();
        let sims = vec![
            sim("AAAA", "iPhone 15", false),
            sim("BBBB", "iPhone 14", true),
        ];
        assert_eq!(
            select_simulator(&c, &sims, Some("AAAA")).unwrap().udid,
            "AAAA"
        );
        assert_eq!(
            select_simulator(&c, &sims, Some("iPhone 14")).unwrap().udid,
            "BBBB"
        );
        assert!(select_simulator(&c, &sims, Some("nope")).is_err());
    }

    #[test]
    fn select_simulator_prefers_the_single_booted_one() {
        let c = ctx();
        let sims = vec![
            sim("AAAA", "iPhone 15", false),
            sim("BBBB", "iPhone 14", true),
        ];
        // No target, exactly one booted → that one, no prompt needed.
        assert_eq!(select_simulator(&c, &sims, None).unwrap().udid, "BBBB");
    }

    #[test]
    fn select_simulator_auto_picks_lone_shutdown_sim() {
        let c = ctx();
        let sims = vec![sim("AAAA", "iPhone 15", false)];
        // None booted, only one candidate → auto-picked (no TTY needed).
        assert_eq!(select_simulator(&c, &sims, None).unwrap().udid, "AAAA");
    }

    #[test]
    fn select_simulator_is_strict_when_ambiguous_and_non_interactive() {
        let c = ctx();
        // None booted, several candidates, no TTY → strict error.
        let sims = vec![
            sim("AAAA", "iPhone 15", false),
            sim("BBBB", "iPhone 14", false),
        ];
        assert!(select_simulator(&c, &sims, None).is_err());
        // Several booted, no TTY → also strict.
        let booted = vec![
            sim("AAAA", "iPhone 15", true),
            sim("BBBB", "iPhone 14", true),
        ];
        assert!(select_simulator(&c, &booted, None).is_err());
    }

    #[test]
    fn destination_choices_float_and_mark_booted() {
        let sims = vec![
            sim("A", "iPhone 15", false),
            sim("B", "iPhone 14", true),
            sim("C", "iPad Air", false),
        ];
        let (ordered, labels) = destination_choices(&sims, None);
        // Booted "B" floats to the top and is marked; the rest keep order,
        // aligned under the marker's gutter.
        assert_eq!(
            ordered.iter().map(|s| s.udid.as_str()).collect::<Vec<_>>(),
            vec!["B", "A", "C"]
        );
        assert_eq!(labels[0], "● iPhone 14 (17.0)");
        assert_eq!(labels[1], "  iPhone 15 (17.0)");
    }

    #[test]
    fn destination_choices_unmarked_when_none_booted() {
        let sims = vec![sim("A", "iPhone 15", false), sim("B", "iPhone 14", false)];
        let (_, labels) = destination_choices(&sims, None);
        // No booted simulator → no marker, no gutter.
        assert_eq!(labels, vec!["iPhone 15 (17.0)", "iPhone 14 (17.0)"]);
    }

    #[test]
    fn destination_choices_float_most_used_first() {
        let sims = vec![
            sim("A", "iPhone 15", false),
            sim("B", "iPhone 14", false),
            sim("C", "iPad Air", false),
        ];
        // C used most, B once, A never → C, B, A regardless of the base order.
        let usage = std::collections::BTreeMap::from([("C".to_string(), 5), ("B".to_string(), 1)]);
        let (ordered, _) = destination_choices(&sims, Some(&usage));
        assert_eq!(
            ordered.iter().map(|s| s.udid.as_str()).collect::<Vec<_>>(),
            vec!["C", "B", "A"]
        );
    }
}
