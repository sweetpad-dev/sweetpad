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
use crate::cli::{CliError, Context, ErrorKind};

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
    /// state — the canonicalized absolute path. When canonicalization fails
    /// (e.g. the path just disappeared), fall back to a lexically absolutized
    /// form rather than the raw argument, so a relative `--project` can't mint
    /// a second, cwd-dependent state key for the same project.
    #[must_use]
    pub fn key(&self) -> String {
        std::fs::canonicalize(self.path())
            .unwrap_or_else(|_| absolutize(self.path()))
            .to_string_lossy()
            .into_owned()
    }
}

/// Join a path onto the cwd (when relative) and squash `.`/`..` lexically — the
/// canonicalize fallback that never touches the filesystem.
fn absolutize(path: &Path) -> PathBuf {
    use std::path::Component;
    let mut out = if path.is_absolute() {
        PathBuf::new()
    } else {
        std::env::current_dir().unwrap_or_default()
    };
    for comp in path.components() {
        match comp {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            c => out.push(c.as_os_str()),
        }
    }
    out
}

/// Resolve the project container from explicit flags, else by auto-discovery in
/// the current directory (warning when same-kind siblings make the pick
/// ambiguous).
pub fn container(ctx: &Context) -> Result<Container, CliError> {
    if let Some(ws) = &ctx.targeting.workspace {
        return Ok(Container::Workspace(ws.clone()));
    }
    if let Some(proj) = &ctx.targeting.project {
        return Ok(Container::Project(proj.clone()));
    }
    let cwd =
        std::env::current_dir().map_err(|e| CliError::new(format!("cannot read cwd: {e}")))?;
    let found = discover_walk_up(&cwd);
    if let Some(ambiguous) = found.ambiguity() {
        ctx.out.warn(&ambiguous);
    }
    found.best().ok_or_else(|| {
        CliError::new(
            "no .xcworkspace, .xcodeproj, or Package.swift found in the current \
             directory or its ancestors (pass --workspace/--project)",
        )
        .kind(ErrorKind::TargetResolution)
    })
}

/// The container the bare-`sweetpad` gate probes — the same resolution as
/// [`container`], minus the ambiguity warning: the status view resolves again
/// immediately after and warns exactly once.
#[must_use]
pub fn container_silently(ctx: &Context) -> Option<Container> {
    if let Some(ws) = &ctx.targeting.workspace {
        return Some(Container::Workspace(ws.clone()));
    }
    if let Some(proj) = &ctx.targeting.project {
        return Some(Container::Project(proj.clone()));
    }
    let cwd = std::env::current_dir().ok()?;
    discover_walk_up(&cwd).best()
}

/// Walk from `start` up toward the root, returning the first directory whose
/// discovery finds a container — so `sweetpad build` works from
/// `Sources/Feature/` like git/cargo/npm would. The walk stops at the git root
/// (a repository above this one must not donate its project) and at the
/// filesystem root.
fn discover_walk_up(start: &Path) -> Discovery {
    let mut dir = start.to_path_buf();
    loop {
        let found = discover_all(&dir);
        if found.best().is_some() || dir.join(".git").exists() {
            return found;
        }
        match dir.parent() {
            Some(parent) => dir = parent.to_path_buf(),
            None => return found,
        }
    }
}

/// Auto-discovery: prefer a workspace, then a project, then a Swift package.
/// Mirrors how Xcode/xcodebuild pick a container. Deterministic: same-kind
/// siblings resolve to the alphabetically first (see [`Discovery::ambiguity`]).
#[must_use]
pub fn discover(dir: &Path) -> Option<Container> {
    discover_all(dir).best()
}

/// Everything discoverable in a directory, each kind sorted by name so the
/// pick never depends on `read_dir` order.
#[derive(Debug, Default)]
pub struct Discovery {
    workspaces: Vec<PathBuf>,
    projects: Vec<PathBuf>,
    package: Option<PathBuf>,
}

impl Discovery {
    /// The winning container: workspace > project > package, alphabetically
    /// first within a kind.
    #[must_use]
    pub fn best(&self) -> Option<Container> {
        self.workspaces
            .first()
            .cloned()
            .map(Container::Workspace)
            .or_else(|| self.projects.first().cloned().map(Container::Project))
            .or_else(|| self.package.clone().map(Container::SwiftPackage))
    }

    /// A warning when the *winning kind* has several candidates — the pick is
    /// then a policy (alphabetical), not the user's intent.
    #[must_use]
    pub fn ambiguity(&self) -> Option<String> {
        let (kind, flag, candidates) = if self.workspaces.len() > 1 {
            ("workspaces", "--workspace", &self.workspaces)
        } else if self.workspaces.is_empty() && self.projects.len() > 1 {
            ("projects", "--project", &self.projects)
        } else {
            return None;
        };
        let names: Vec<String> = candidates
            .iter()
            .filter_map(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
            .collect();
        Some(format!(
            "multiple {kind} here ({}); using {} — pass {flag} to disambiguate",
            names.join(", "),
            names.first().map(String::as_str).unwrap_or_default(),
        ))
    }
}

/// Collect every container in `dir`, sorted by file name within each kind.
fn discover_all(dir: &Path) -> Discovery {
    let mut found = Discovery::default();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return found;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        match path.extension().and_then(|e| e.to_str()) {
            Some("xcworkspace") => found.workspaces.push(path),
            Some("xcodeproj") => found.projects.push(path),
            _ => {
                if path.file_name().and_then(|f| f.to_str()) == Some("Package.swift") {
                    found.package = Some(path);
                }
            }
        }
    }
    found.workspaces.sort();
    found.projects.sort();
    found
}

/// The fully resolved targeting for a command, after layering.
#[derive(Debug, Clone)]
pub struct Resolved {
    pub container: Container,
    pub scheme: Option<String>,
    pub configuration: Option<String>,
    pub destination: Option<String>,
    /// SDK override (`--sdk` / config / `context select sdk`). `None` lets the
    /// destination imply it, which is the common case.
    pub sdk: Option<String>,
}

/// Apply the layered precedence (flag > config > state) for the soft targeting
/// values. Auto-discovery / interactive selection of a *specific* scheme or
/// destination is left to the individual commands (they know the candidate
/// lists); this folds the persisted layers into a starting point.
pub fn resolve(ctx: &Context) -> Result<Resolved, CliError> {
    let container = container(ctx)?;
    let key = container.key();
    let cfg: Defaults = ctx.config.for_project(&key);
    let pf = ctx.project_file(&container);
    let st = ctx.state.projects.get(&key);

    let pick = |flag: &Option<String>,
                cfg: &Option<String>,
                project: &Option<String>,
                state: Option<&String>| {
        flag.clone()
            .or_else(|| cfg.clone())
            .or_else(|| project.clone())
            .or_else(|| state.cloned())
    };

    Ok(Resolved {
        scheme: pick(
            &ctx.targeting.scheme,
            &cfg.scheme,
            &pf.scheme,
            st.and_then(|s| s.scheme.as_ref()),
        ),
        configuration: pick(
            &ctx.targeting.configuration,
            &cfg.configuration,
            &pf.configuration,
            st.and_then(|s| s.configuration.as_ref()),
        ),
        destination: pick(
            &ctx.targeting.destination,
            &cfg.destination,
            &pf.destination,
            st.and_then(|s| s.destination.as_ref()),
        ),
        sdk: pick(
            &ctx.targeting.sdk,
            &cfg.sdk,
            &pf.sdk,
            st.and_then(|s| s.sdk.as_ref()),
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
    let pf = ctx.project_file(&container);
    let st = ctx.state.projects.get(&key);

    let pick = |flag: &Option<String>,
                test_cfg: &Option<String>,
                test_project: &Option<String>,
                test_state: Option<&String>,
                build_cfg: &Option<String>,
                build_project: &Option<String>,
                build_state: Option<&String>| {
        flag.clone()
            .or_else(|| test_cfg.clone())
            .or_else(|| test_project.clone())
            .or_else(|| test_state.cloned())
            .or_else(|| build_cfg.clone())
            .or_else(|| build_project.clone())
            .or_else(|| build_state.cloned())
    };

    Ok(Resolved {
        scheme: pick(
            &ctx.targeting.scheme,
            &cfg.testing.scheme,
            &pf.testing.scheme,
            st.and_then(|s| s.testing.scheme.as_ref()),
            &cfg.scheme,
            &pf.scheme,
            st.and_then(|s| s.scheme.as_ref()),
        ),
        configuration: pick(
            &ctx.targeting.configuration,
            &cfg.testing.configuration,
            &pf.testing.configuration,
            st.and_then(|s| s.testing.configuration.as_ref()),
            &cfg.configuration,
            &pf.configuration,
            st.and_then(|s| s.configuration.as_ref()),
        ),
        destination: pick(
            &ctx.targeting.destination,
            &cfg.testing.destination,
            &pf.testing.destination,
            st.and_then(|s| s.testing.destination.as_ref()),
            &cfg.destination,
            &pf.destination,
            st.and_then(|s| s.destination.as_ref()),
        ),
        // The testing context has no sdk of its own; follow the build layers.
        sdk: ctx
            .targeting
            .sdk
            .clone()
            .or_else(|| cfg.sdk.clone())
            .or_else(|| pf.sdk.clone())
            .or_else(|| st.and_then(|s| s.sdk.clone())),
        container,
    })
}

/// The pinned default test target (config `[….testing] target`, then the
/// committed `sweetpad.toml`, then `context select target --testing`) —
/// `test run` narrows to it when no explicit `--only-testing` selector is
/// given.
#[must_use]
pub fn testing_target(ctx: &Context, container: &Container) -> Option<String> {
    let key = container.key();
    ctx.config
        .for_project(&key)
        .testing
        .target
        .clone()
        .or_else(|| ctx.project_file(container).testing.target.clone())
        .or_else(|| {
            ctx.state
                .projects
                .get(&key)
                .and_then(|s| s.testing.target.clone())
        })
}

/// Helper for commands: a value is required but unresolved, and we cannot
/// prompt (non-interactive). Produces the standard strict error, with a hint
/// matching how *this* value can actually be supplied — not every `what` has a
/// `--{what}` flag or a config key.
#[must_use]
pub fn missing(what: &str) -> CliError {
    let hint = match what {
        "scheme" | "configuration" | "destination" => {
            format!("pass --{what} or set it in config")
        }
        "simulator" => "name one (the TARGET argument, or --simulator where offered)".to_string(),
        "device" => "pass --device-id".to_string(),
        "project" => "pass --project".to_string(),
        _ => format!("specify the {what} explicitly"),
    };
    CliError::new(format!(
        "no {what} specified and the terminal is not interactive; {hint}"
    ))
    .kind(ErrorKind::TargetResolution)
}

/// The scheme set for a container, read without xcodebuild. Workspaces merge
/// member projects' schemes; bare projects use their own (file + autocreated)
/// set, both via the in-process pbxproj reader. Swift packages have no pbxproj,
/// so their schemes — the product names xcodebuild would synthesize — are read
/// straight from the manifest (`swift package dump-package`). Shared by every
/// command that needs to enumerate or pick a scheme.
pub fn schemes(container: &Container) -> Result<Vec<String>, CliError> {
    match container {
        Container::Workspace(p) => sweetpad_lib::workspace::open(p)
            .map(|w| w.merged_schemes())
            .map_err(|e| CliError::new(format!("failed to read workspace {}: {e}", p.display()))),
        Container::Project(p) => sweetpad_lib::project::open(p)
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
        Container::Workspace(p) => sweetpad_lib::workspace::open(p)
            .map(|w| w.merged_configurations())
            .map_err(|e| CliError::new(format!("failed to read workspace {}: {e}", p.display()))),
        Container::Project(p) => sweetpad_lib::project::open(p)
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
        0 => Err(CliError::new(format!("no {what} available")).kind(ErrorKind::TargetResolution)),
        1 => Ok(candidates[0].clone()),
        _ if ctx.out.is_interactive() => prompt_choice(what, candidates, ctx.out.use_color_stderr()),
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
        return crate::cli::simctl::find(sims, t).ok_or_else(|| {
            CliError::new(format!("no simulator matching {t:?}")).kind(ErrorKind::TargetResolution)
        });
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
    // Disambiguate identical labels (same name + OS clones) and map the choice
    // back by position — matching on the label string alone would silently act
    // on the *first* twin no matter which row the user picked.
    let mut labels: Vec<String> = pool.iter().map(|s| s.label()).collect();
    disambiguate_labels(&mut labels, &pool);
    let chosen = choose(ctx, "simulator", None, &labels)?;
    labels
        .iter()
        .position(|l| *l == chosen)
        .map(|i| pool[i])
        .ok_or_else(|| CliError::new("simulator not found").kind(ErrorKind::TargetResolution))
}

/// A resolved `--on` reference: the concrete run target plus its ready
/// `-destination` specifier.
#[derive(Debug, Clone)]
pub enum OnTarget {
    Mac,
    Simulator { udid: String, specifier: String },
    Device { udid: String, specifier: String },
}

impl OnTarget {
    /// The raw `-destination` specifier for this target.
    #[must_use]
    pub fn specifier(&self) -> String {
        match self {
            OnTarget::Mac => "platform=macOS".to_string(),
            OnTarget::Simulator { specifier, .. } | OnTarget::Device { specifier, .. } => {
                specifier.clone()
            }
        }
    }
}

/// Resolve a human `--on` reference against the live device lists —
/// xcodebuild's raw specifier grammar stays the `--destination` escape hatch.
/// Accepted forms:
///
/// - `mac`/`macos` — the local Mac
/// - `booted` — the booted simulator (most-used first when several are up)
/// - `device` — the sole connected physical device
/// - `ios`/`watchos`/`tvos`/`visionos` — the newest matching simulator,
///   most-used-first tiebreak
/// - a UDID — that simulator or device
/// - anything else — a name match over simulators and connected devices: an
///   exact (case-insensitive) name wins outright, simulators before devices;
///   otherwise a substring match over simulators (booted, then newest OS,
///   then most-used), falling back to a substring-matched device.
///
/// `sims` is the caller's `simctl list` — passed in so one listing serves
/// resolution, usage tracking, and display.
#[allow(clippy::too_many_lines)] // one linear match ladder per reference form
pub fn resolve_on(
    ctx: &Context,
    key: &str,
    reference: &str,
    sims: &[crate::cli::simctl::Simulator],
) -> Result<OnTarget, CliError> {
    // A named alias (`context alias work-phone <UDID>`) substitutes its stored
    // reference up front — resolved once, so an alias can't point at another.
    let alias = ctx
        .state
        .projects
        .get(key)
        .and_then(|s| s.destination_aliases.get(reference))
        .cloned();
    let reference = alias.as_deref().unwrap_or(reference);
    let lower = reference.to_ascii_lowercase();
    if lower == "mac" || lower == "macos" {
        return Ok(OnTarget::Mac);
    }

    let usage = ctx
        .state
        .projects
        .get(key)
        .map(|s| s.destination_usage.clone())
        .unwrap_or_default();
    let used = |udid: &str| usage.get(udid).copied().unwrap_or(0);
    let sim_target = |s: &crate::cli::simctl::Simulator| OnTarget::Simulator {
        udid: s.udid.clone(),
        specifier: s.destination(),
    };

    if lower == "booted" {
        let mut booted: Vec<&crate::cli::simctl::Simulator> =
            sims.iter().filter(|s| s.is_booted()).collect();
        booted.sort_by_key(|s| std::cmp::Reverse(used(&s.udid)));
        return booted.first().map(|s| sim_target(s)).ok_or_else(|| {
            CliError::new("--on booted: no simulator is booted")
                .kind(ErrorKind::TargetResolution)
        });
    }

    if lower == "device" {
        let devices = crate::cli::devicectl::list()?;
        return match devices.as_slice() {
            [] => Err(CliError::new("--on device: no physical device is connected")
                .kind(ErrorKind::TargetResolution)),
            [dev] => Ok(device_target(dev)),
            many => Err(CliError::new(format!(
                "--on device is ambiguous — connected: {}; name one",
                many.iter()
                    .map(crate::cli::devicectl::Device::label)
                    .collect::<Vec<_>>()
                    .join(", ")
            ))
            .kind(ErrorKind::TargetResolution)),
        };
    }

    // Platform words: the newest matching simulator, most-used first among
    // equals.
    let platform = match lower.as_str() {
        "ios" | "iphone" | "ipad" => Some("iOS"),
        "watchos" => Some("watchOS"),
        "tvos" => Some("tvOS"),
        "visionos" | "xros" => Some("visionOS"),
        _ => None,
    };
    if let Some(platform) = platform {
        let mut candidates: Vec<&crate::cli::simctl::Simulator> = sims
            .iter()
            .filter(|s| {
                s.os.eq_ignore_ascii_case(platform)
                    && (lower != "ipad" || s.name.to_ascii_lowercase().contains("ipad"))
                    && (lower != "iphone" || s.name.to_ascii_lowercase().contains("iphone"))
            })
            .collect();
        candidates.sort_by(|a, b| {
            version_key(&b.os_version)
                .cmp(&version_key(&a.os_version))
                .then_with(|| used(&b.udid).cmp(&used(&a.udid)))
        });
        return candidates.first().map(|s| sim_target(s)).ok_or_else(|| {
            CliError::new(format!("--on {reference}: no matching simulator"))
                .kind(ErrorKind::TargetResolution)
        });
    }

    // A UDID names one simulator or device outright.
    if let Some(s) = sims.iter().find(|s| s.udid.eq_ignore_ascii_case(reference)) {
        return Ok(sim_target(s));
    }
    let devices = crate::cli::devicectl::list().unwrap_or_default();
    if let Some(d) = devices
        .iter()
        .find(|d| d.udid.eq_ignore_ascii_case(reference))
    {
        return Ok(device_target(d));
    }

    // Name match over simulators + devices. Exactness outranks the
    // sims-before-devices policy: a device the user named outright must not
    // lose to a simulator that merely contains the reference.
    let matches_name = |name: &str| name.to_ascii_lowercase().contains(&lower);
    let exact_sims: Vec<&crate::cli::simctl::Simulator> = sims
        .iter()
        .filter(|s| s.name.eq_ignore_ascii_case(reference))
        .collect();
    if exact_sims.is_empty()
        && let Some(d) = devices.iter().find(|d| d.name.eq_ignore_ascii_case(reference))
    {
        return Ok(device_target(d));
    }
    let mut candidates: Vec<&crate::cli::simctl::Simulator> = if exact_sims.is_empty() {
        sims.iter().filter(|s| matches_name(&s.name)).collect()
    } else {
        exact_sims
    };
    // A substring-matched physical device wins only when no simulator matches
    // at all — simulators are the common daily target.
    if candidates.is_empty()
        && let Some(d) = devices.iter().find(|d| matches_name(&d.name))
    {
        return Ok(device_target(d));
    }
    candidates.sort_by(|a, b| {
        b.is_booted()
            .cmp(&a.is_booted())
            .then_with(|| version_key(&b.os_version).cmp(&version_key(&a.os_version)))
            .then_with(|| used(&b.udid).cmp(&used(&a.udid)))
    });
    match candidates.as_slice() {
        [] => {
            let known: Vec<String> = sims.iter().map(|s| s.name.clone()).collect();
            let mut names: Vec<String> = known;
            names.extend(devices.iter().map(|d| d.name.clone()));
            names.sort();
            names.dedup();
            Err(CliError::new(format!(
                "--on {reference:?} matches nothing (try one of: {})",
                names.join(", ")
            ))
            .kind(ErrorKind::TargetResolution))
        }
        [one] => Ok(sim_target(one)),
        several => {
            // Distinct names → genuinely ambiguous; same name across OS
            // versions → the sort already put the newest/booted first.
            let mut names: Vec<&str> = several.iter().map(|s| s.name.as_str()).collect();
            names.dedup();
            if names.len() > 1 {
                Err(CliError::new(format!(
                    "--on {reference:?} is ambiguous ({}) — be more specific",
                    several
                        .iter()
                        .map(|s| s.label())
                        .collect::<Vec<_>>()
                        .join(", ")
                ))
                .kind(ErrorKind::TargetResolution))
            } else {
                Ok(sim_target(several[0]))
            }
        }
    }
}

fn device_target(dev: &crate::cli::devicectl::Device) -> OnTarget {
    let platform = if dev.platform.is_empty() {
        "iOS"
    } else {
        &dev.platform
    };
    OnTarget::Device {
        udid: dev.udid.clone(),
        specifier: format!("platform={platform},id={}", dev.udid),
    }
}

/// Numeric sort key for an OS version string (`"17.0"` → `[17, 0]`).
fn version_key(version: &str) -> Vec<u32> {
    version
        .split('.')
        .map(|part| part.parse().unwrap_or(0))
        .collect()
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
/// when a TTY is available. An explicitly-requested scheme or configuration is
/// validated against the project's candidates with a did-you-mean hint, so a
/// typo dies here instead of deep inside xcodebuild; a *remembered* value that
/// no longer exists is cleared and re-resolved instead of failing forever.
/// `track` gates the usage/recents bookkeeping — dry runs (`--show-command`)
/// pass `false` so a preview never mutates state.
pub fn build_target(
    ctx: &mut Context,
    resolved: &Resolved,
    track: bool,
) -> Result<BuildTarget, CliError> {
    let candidates = schemes(&resolved.container)?;
    let scheme = settle_scheme(ctx, resolved, &candidates)?;
    let configuration = settle_configuration(ctx, resolved)?;

    let key = resolved.container.key();
    // An explicit `--on` reference outranks every persisted destination layer
    // (it's a flag); the resolved simulator's usage is bumped so it floats in
    // future pickers, but — per the picker-sourced-only rule — it is never
    // written as the remembered destination itself (callers gate `remember`'s
    // destination on `--on` being absent).
    let destination = if let Some(reference) = ctx.targeting.on.clone() {
        if ctx.targeting.destination.is_some() {
            return Err(CliError::new(
                "--on and --destination are mutually exclusive; pass one",
            ));
        }
        let sims = crate::cli::simctl::list()?;
        let target = resolve_on(ctx, &key, &reference, &sims)?;
        if track
            && let OnTarget::Simulator { udid, .. } = &target
            && let Some(sim) = sims.iter().find(|s| &s.udid == udid)
        {
            track_destination(&mut ctx.state, &key, sim);
            let _ = ctx.state.save();
        }
        target.specifier()
    } else if let Some(d) = resolved.destination.clone() {
        d
    } else {
        pick_destination(ctx, &key, &crate::cli::simctl::list()?)?
    };

    Ok(BuildTarget {
        scheme,
        configuration,
        destination,
    })
}

/// Settle the scheme: validate an explicit/remembered value against the
/// project's schemes, recovering from a stale *remembered* one (see
/// [`recover_stale`]), then auto-pick/prompt via [`choose`].
fn settle_scheme(
    ctx: &mut Context,
    resolved: &Resolved,
    candidates: &[String],
) -> Result<String, CliError> {
    let mut current = resolved.scheme.clone();
    if let Some(s) = current.clone()
        && let Err(err) = validate_choice("scheme", &s, candidates)
    {
        current = recover_stale(ctx, &resolved.container, "scheme", &s, err)?;
        if let Some(higher) = &current {
            validate_choice("scheme", higher, candidates)?;
        }
    }
    choose(ctx, "scheme", current, candidates)
}

/// Settle the build configuration: an explicit/remembered value is validated
/// against the project's configurations (a stale remembered one recovers, see
/// [`recover_stale`]), and the `Debug` default applies only when the project
/// actually has a `Debug` — a project with only, say, `UAT-Debug`/`Prod`
/// drops to the configuration picker instead of invoking xcodebuild with a
/// nonexistent configuration. Swift packages skip validation (SwiftPM maps
/// any name onto debug/release itself).
pub fn settle_configuration(ctx: &mut Context, resolved: &Resolved) -> Result<String, CliError> {
    if matches!(resolved.container, Container::SwiftPackage(_)) {
        return Ok(resolved
            .configuration
            .clone()
            .unwrap_or_else(|| "Debug".to_string()));
    }
    // Candidate enumeration is advisory: if the project can't be read here the
    // build command itself will surface that, so fall back to the old behavior.
    let candidates = configurations(&resolved.container).unwrap_or_default();
    let mut current = resolved.configuration.clone();
    if let Some(c) = current.clone()
        && let Err(err) = validate_choice("configuration", &c, &candidates)
    {
        current = recover_stale(ctx, &resolved.container, "configuration", &c, err)?;
        if let Some(higher) = &current {
            validate_choice("configuration", higher, &candidates)?;
        }
    }
    if let Some(c) = current {
        return Ok(c);
    }
    if candidates.is_empty() || candidates.iter().any(|c| c == "Debug") {
        return Ok("Debug".to_string());
    }
    choose(ctx, "configuration", None, &candidates)
}

/// Recovery for a value that failed validation: when it came from *remembered
/// state* (the project changed under a stale pick), clear it, warn with the
/// provenance, and hand back the layer above it (flag/config/sweetpad.toml) —
/// `None` drops the caller to its auto-pick/prompt path. A failing value from
/// any other layer re-raises `err` untouched: flags and config are the user's
/// to fix.
fn recover_stale(
    ctx: &mut Context,
    container: &Container,
    what: &str,
    value: &str,
    err: CliError,
) -> Result<Option<String>, CliError> {
    let key = container.key();
    // The stale value may live in the build state, the testing state
    // (`context select … --testing`), or both — `resolve_testing` reads both
    // layers, so recovery must clear whichever one(s) hold it, or a stale
    // testing pick fails every `sweetpad test` forever.
    let st = ctx.state.projects.get(&key);
    let build_match = st
        .and_then(|p| match what {
            "scheme" => p.scheme.as_deref(),
            _ => p.configuration.as_deref(),
        })
        == Some(value);
    let testing_match = st
        .and_then(|p| match what {
            "scheme" => p.testing.scheme.as_deref(),
            _ => p.testing.configuration.as_deref(),
        })
        == Some(value);
    if !build_match && !testing_match {
        return Err(err);
    }
    let flag = match what {
        "scheme" => ctx.targeting.scheme.clone(),
        _ => ctx.targeting.configuration.clone(),
    };
    if flag.as_deref() == Some(value) {
        // The flag independently names the same bad value — blame the flag.
        return Err(err);
    }
    let entry = ctx.state.project_mut(&key);
    if what == "scheme" {
        if build_match {
            entry.scheme = None;
        }
        if testing_match {
            entry.testing.scheme = None;
        }
    } else {
        if build_match {
            entry.configuration = None;
        }
        if testing_match {
            entry.testing.configuration = None;
        }
    }
    let _ = ctx.state.save();
    let hint = if testing_match {
        format!("`sweetpad context remove {what} --testing` does this by hand")
    } else {
        format!("`sweetpad context remove {what}` does this by hand")
    };
    ctx.out.warn(&format!(
        "the remembered {what} {value:?} no longer exists in the project — cleared it ({hint})"
    ));
    let cfg = ctx.config.for_project(&key);
    let pf = ctx.project_file(container);
    let higher = match what {
        "scheme" => flag
            .or_else(|| testing_match.then(|| cfg.testing.scheme.clone()).flatten())
            .or_else(|| testing_match.then(|| pf.testing.scheme.clone()).flatten())
            .or_else(|| cfg.scheme.clone())
            .or_else(|| pf.scheme.clone()),
        _ => flag
            .or_else(|| {
                testing_match
                    .then(|| cfg.testing.configuration.clone())
                    .flatten()
            })
            .or_else(|| {
                testing_match
                    .then(|| pf.testing.configuration.clone())
                    .flatten()
            })
            .or_else(|| cfg.configuration.clone())
            .or_else(|| pf.configuration.clone()),
    };
    Ok(higher)
}

/// Reject an explicitly-requested value that isn't in the candidate list, with
/// a did-you-mean hint and the full list — instead of handing xcodebuild a
/// scheme/configuration it will die on with a cryptic error. An empty candidate
/// list (enumeration failed) validates nothing.
///
/// # Errors
/// Returns a `TargetResolution` error when `value` matches no candidate.
pub(crate) fn validate_choice(what: &str, value: &str, candidates: &[String]) -> Result<(), CliError> {
    if candidates.is_empty() || candidates.iter().any(|c| c == value) {
        return Ok(());
    }
    let suggestion = candidates
        .iter()
        .map(|c| {
            (
                crate::cli::config::edit_distance(&c.to_lowercase(), &value.to_lowercase()),
                c,
            )
        })
        .filter(|(d, _)| *d <= 2)
        .min_by_key(|(d, _)| *d)
        .map(|(_, c)| format!(" — did you mean {c:?}?"))
        .unwrap_or_default();
    Err(CliError::new(format!(
        "unknown {what} {value:?}{suggestion} ({what}s: {})",
        candidates.join(", ")
    ))
    .kind(ErrorKind::TargetResolution))
}

/// Persist the just-settled picks to the state file so later commands don't
/// re-prompt. Only values that were *unresolved* before the pickers ran are
/// written — a value that arrived via flag/env/config (or is already
/// remembered) never overwrites the stored context. State stays "the last
/// interactive picks": a one-off `--configuration Release` or `--destination …`
/// can't poison the next plain `build`. `destination: false` skips the
/// destination entirely — the `--mac`/`--device`/`--on` runs, whose
/// synthesized specifier must never become the remembered simulator context.
/// Best-effort: failures to write state never fail a command.
pub fn remember(ctx: &mut Context, resolved: &Resolved, target: &BuildTarget, destination: bool) {
    let key = resolved.container.key();
    let st = ctx.state.project_mut(&key);
    let mut dirty = false;
    if resolved.scheme.is_none() {
        st.scheme = Some(target.scheme.clone());
        dirty = true;
    }
    if resolved.configuration.is_none() {
        st.configuration = Some(target.configuration.clone());
        dirty = true;
    }
    if destination && resolved.destination.is_none() {
        st.destination = Some(target.destination.clone());
        dirty = true;
    }
    if dirty {
        let _ = ctx.state.save();
    }
}

/// Persist a test run's picks, mirroring [`remember`]'s picker-sourced-only
/// rule (including its `destination` gate — a `--on`-sourced specifier is a
/// one-off, never the remembered context). A field is unresolved here only
/// when neither a flag, config, testing pin, nor build context supplied it —
/// so writing the pick into the *build* context keeps `test` and `build`
/// sharing one selection, and a pinned testing override (owned by
/// `context … --testing`) is never touched. Best-effort.
pub fn remember_testing(
    ctx: &mut Context,
    resolved: &Resolved,
    target: &BuildTarget,
    destination: bool,
) {
    let key = resolved.container.key();
    let st = ctx.state.project_mut(&key);
    let mut dirty = false;
    if resolved.scheme.is_none() {
        st.scheme = Some(target.scheme.clone());
        dirty = true;
    }
    if resolved.configuration.is_none() {
        st.configuration = Some(target.configuration.clone());
        dirty = true;
    }
    if destination && resolved.destination.is_none() {
        st.destination = Some(target.destination.clone());
        dirty = true;
    }
    if dirty {
        let _ = ctx.state.save();
    }
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
    let (ordered, mut labels) = destination_choices(sims, usage);
    // The local Mac is always addressable (`platform=macOS`); listed last so
    // the simulator daily loop stays on top, but present so a macOS-app
    // project can settle its destination without knowing `--on mac`.
    let mac_label = "My Mac (macOS)".to_string();
    labels.push(mac_label.clone());
    let chosen = choose(ctx, "destination", None, &labels)?;
    if chosen == mac_label {
        return Ok("platform=macOS".to_string());
    }
    // Match by label position, not `Simulator::label`, since the displayed
    // labels carry the booted marker.
    let idx = labels
        .iter()
        .position(|l| *l == chosen)
        .ok_or_else(|| CliError::new("destination not found").kind(ErrorKind::TargetResolution))?;
    let sim = ordered[idx];
    track_destination(&mut ctx.state, key, sim);
    let _ = ctx.state.save();
    Ok(sim.destination())
}

/// Record a freshly-picked simulator into the project's recents (unique by
/// id, least-recently-picked first) and bump its usage count — the inputs to
/// most-used-first ordering. A re-pick moves the entry to the back, so the
/// recents cap ([`crate::cli::state`]'s prune) evicts genuinely stale
/// destinations rather than the daily driver that happened to be picked
/// first. Mirrors the extension's `trackSelectedDestination`.
fn track_destination(state: &mut State, key: &str, sim: &crate::cli::simctl::Simulator) {
    let st = state.project_mut(key);
    *st.destination_usage.entry(sim.udid.clone()).or_insert(0) += 1;
    if let Some(pos) = st.destination_recents.iter().position(|d| d.id == sim.udid) {
        let entry = st.destination_recents.remove(pos);
        st.destination_recents.push(entry);
    } else {
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
    let mut labels: Vec<String> = ordered
        .iter()
        .map(|s| match (any_booted, s.is_booted()) {
            (false, _) => s.label(),
            (true, true) => format!("● {}", s.label()),
            (true, false) => format!("  {}", s.label()),
        })
        .collect();
    disambiguate_labels(&mut labels, &ordered);
    (ordered, labels)
}

/// Append a short udid suffix to any labels that would otherwise be identical
/// (same name, OS version, and boot marker), so the picker shows distinct rows
/// and the chosen label maps back to exactly one simulator in [`pick_destination`].
pub(crate) fn disambiguate_labels(labels: &mut [String], ordered: &[&crate::cli::simctl::Simulator]) {
    use std::fmt::Write as _;
    let mut counts: std::collections::HashMap<&str, usize> = std::collections::HashMap::new();
    for label in labels.iter() {
        *counts.entry(label.as_str()).or_insert(0) += 1;
    }
    let collisions: std::collections::HashSet<String> = counts
        .into_iter()
        .filter(|&(_, n)| n > 1)
        .map(|(label, _)| label.to_string())
        .collect();
    for (label, sim) in labels.iter_mut().zip(ordered) {
        if collisions.contains(label.as_str()) {
            let short = sim.udid.get(..8).unwrap_or(sim.udid.as_str());
            let _ = write!(label, " [{short}]");
        }
    }
}

/// Interactive fuzzy picker (the design's TTY fallback): type to filter, arrows
/// to move, Enter to select. Only reached when stdout is a TTY.
fn prompt_choice(what: &str, candidates: &[String], color: bool) -> Result<String, CliError> {
    use dialoguer::theme::{ColorfulTheme, SimpleTheme, Theme};
    // Honor `--no-color`/`NO_COLOR` for the prompt itself, not just the output.
    let colorful = ColorfulTheme::default();
    let simple = SimpleTheme;
    let theme: &dyn Theme = if color { &colorful } else { &simple };
    let idx = dialoguer::FuzzySelect::with_theme(theme)
        .with_prompt(format!("Select a {what}"))
        .items(candidates)
        .default(0)
        .interact()
        .map_err(|e| {
            CliError::new(format!("selection cancelled: {e}")).kind(ErrorKind::UserCancel)
        })?;
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
            chdir: None,
            developer_dir: None,
            output: None,
            json: false,
            non_interactive: false,
            no_color: true,
            verbose: false,
            quiet: false,
            gh_annotations: false,
        };
        let out = Output::new(&global);
        Context {
            global,
            targeting: Targeting::default(),
            config: Config::default(),
            state: State::default(),
            out,
            project_toml: std::cell::OnceCell::new(),
        }
    }

    #[test]
    fn track_destination_moves_a_repick_to_the_back() {
        let sim = |udid: &str| crate::cli::simctl::Simulator {
            udid: udid.into(),
            name: format!("Sim {udid}"),
            state: "Shutdown".into(),
            available: true,
            os: "iOS".into(),
            os_version: "17.0".into(),
        };
        let mut state = State::default();
        track_destination(&mut state, "/p", &sim("A"));
        track_destination(&mut state, "/p", &sim("B"));
        track_destination(&mut state, "/p", &sim("A"));
        let entry = &state.projects["/p"];
        // Front = least recently picked, so the recents cap evicts B before
        // the re-picked A (true LRU, not first-seen).
        let order: Vec<&str> = entry.destination_recents.iter().map(|d| d.id.as_str()).collect();
        assert_eq!(order, vec!["B", "A"]);
        assert_eq!(entry.destination_usage["A"], 2);
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
    fn destination_choices_disambiguate_identical_labels() {
        // Two simulators with the same name + OS version would collide; each row
        // must stay distinct so a pick maps back to exactly one simulator.
        let sims = vec![
            sim("AAAA1111BBBB", "iPhone 15", false),
            sim("CCCC2222DDDD", "iPhone 15", false),
            sim("EEEE", "iPad Air", false),
        ];
        let (_, labels) = destination_choices(&sims, None);
        assert_eq!(labels[0], "iPhone 15 (17.0) [AAAA1111]");
        assert_eq!(labels[1], "iPhone 15 (17.0) [CCCC2222]");
        // A unique label is left untouched.
        assert_eq!(labels[2], "iPad Air (17.0)");
        let unique: std::collections::HashSet<&String> = labels.iter().collect();
        assert_eq!(unique.len(), labels.len());
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
