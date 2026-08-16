//! The standalone, headless `sweetpad` CLI — "xcodebuild for humans".
//!
//! A pure-native front-end to the resolver in this crate for building,
//! running, and exploring Xcode projects without an editor. It lives in the
//! same `sweetpad` binary as the [`crate::vscode_cli`] namespace (which
//! controls the VS Code extension); `vscode` is dispatched separately in
//! `src/bin/sweetpad.rs`, everything else routes through [`run`] here.
//!
//! Design goals and the full command surface live in `CLI_DESIGN.md`.
//!
//! Grammar is **resource-first**: `sweetpad <resource> <action> [flags]`, with
//! resources at the top level, over shared plumbing ([`config`], [`state`],
//! [`resolve`], [`output`]).

// Every byte of stdout/stderr in the CLI must route through `Output` so the
// `--json`/color/quiet contract holds; a raw `println!`/`eprintln!` here is a
// bug, denied under `cargo clippy`. The sanctioned sinks (`output` itself, the
// `app run` live-log threads) opt out locally with `#[allow]`. Scoped to this
// module so it never touches the BSP server or `vscode` client, which own their
// own output.
#![deny(clippy::print_stdout, clippy::print_stderr)]

use std::process::ExitCode;

use clap::{CommandFactory, Parser, Subcommand};

pub mod buildlog;
pub mod config;
pub mod devicectl;
pub mod inject;
pub mod merge;
pub mod oslog;
pub mod output;
pub mod pbxedit;
pub mod process;
pub mod progress;
pub mod pymobiledevice3;
pub mod rawmode;
pub mod render;
pub mod resolve;
pub mod scaffold;
pub mod signals;
pub mod simctl;
pub mod state;
pub mod swiftpm;
pub mod xcodebuild;

pub mod commands;

pub use render::{Render, Rendered};

/// Top-level CLI definition. Note this parses the *non-`vscode`* argument
/// vector: the binary peels off the `vscode` subcommand before we get here, so
/// clap owns the rest of the resource-first tree.
#[derive(Debug, Parser)]
#[command(
    name = "sweetpad",
    version = env!("SWEETPAD_VERSION"),
    about = "Build, run, and explore Xcode projects from the terminal",
    long_about = "sweetpad — xcodebuild for humans.\n\nA standalone, headless \
        CLI for Xcode projects.",
    disable_help_subcommand = true
)]
pub struct Cli {
    #[command(flatten)]
    pub global: GlobalArgs,

    /// Bare `sweetpad` (no subcommand) prints the status view inside a
    /// project, and the help outside one.
    #[command(subcommand)]
    pub resource: Option<Resource>,
}

/// The truly universal flags — accepted on every command and propagated to
/// nested actions. Targeting (which workspace/scheme/… a command acts on) is
/// *not* here: those flags live on the commands that consume them, as the
/// [`ContainerArgs`]/[`SchemeArgs`]/[`BuildTargetArgs`] tiers below.
#[derive(Debug, clap::Args)]
#[command(next_help_heading = "Global")]
#[allow(clippy::struct_excessive_bools)] // independent CLI toggles, not a state machine
pub struct GlobalArgs {
    /// Run as if started in DIR (chdir before anything else), like 'git -C'.
    #[arg(short = 'C', value_name = "DIR", global = true)]
    pub chdir: Option<std::path::PathBuf>,

    /// Xcode to use: sets DEVELOPER_DIR for every spawned tool (e.g.
    /// /Applications/Xcode-16.4.app/Contents/Developer). A project can pin one
    /// via 'developer_dir' in sweetpad.toml.
    #[arg(long, value_name = "DIR", global = true, env = "DEVELOPER_DIR")]
    pub developer_dir: Option<std::path::PathBuf>,

    /// Output format. 'json' is the one-shot envelope; 'ndjson' streams one
    /// JSON event per line from the long-running verbs (build, test, logs) and
    /// ends with a '{"event":"result", …}' line. Wins over '--json'.
    #[arg(short = 'o', long = "output", global = true, value_enum)]
    pub output: Option<OutputMode>,

    /// Emit machine-readable JSON instead of human output (alias for
    /// '-o json').
    #[arg(long, global = true)]
    pub json: bool,

    /// Assume no interactive terminal: never prompt or animate a spinner, turn a
    /// missing scheme/destination into an error instead of a picker, and run
    /// 'app run' as a plain follow rather than the rebuild session. Also honored
    /// via the 'SWEETPAD_NONINTERACTIVE' env var.
    #[arg(long, global = true)]
    pub non_interactive: bool,

    /// Disable colored output (also honored via the 'NO_COLOR' env var and when
    /// stdout is not a TTY). 'CLICOLOR_FORCE'/'FORCE_COLOR' force color back on
    /// when piped; an explicit '--no-color'/'NO_COLOR' still wins.
    #[arg(long, global = true)]
    pub no_color: bool,

    /// Print verbose diagnostics (raw tool output, extra detail).
    #[arg(short, long, global = true)]
    pub verbose: bool,

    /// Suppress progress chatter (notes, spinners, step labels). Errors and
    /// primary data/JSON are still emitted; wins over '--verbose'.
    #[arg(short, long, global = true)]
    pub quiet: bool,

    /// Emit GitHub Actions annotations ('::error file=…::…') for build/test
    /// diagnostics, so failures surface inline on the PR.
    #[arg(long, global = true)]
    pub gh_annotations: bool,
}

/// The output axis (`-o`): how results reach stdout.
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum OutputMode {
    /// Colored human output (the default).
    Human,
    /// The '{schema, ok, data}' envelope, one JSON document per command.
    Json,
    /// Streaming events, one compact JSON object per line; the final line is
    /// '{"event":"result","ok":…,"data":…}'. Non-streaming commands emit just
    /// that final line.
    Ndjson,
    /// Human output with progress chatter muted (same as '--quiet').
    Quiet,
}

/// Tier 1 — which project container to act on. Flattened into every command
/// that locates a workspace/project — either at the resource level (when every
/// action consumes it, the flags being `global` within that resource so they
/// parse on either side of the action token) or directly on the consuming
/// action (so a sibling like `project new` never advertises flags it ignores).
#[derive(Debug, Clone, Default, clap::Args)]
#[command(next_help_heading = "Target selection")]
pub struct ContainerArgs {
    /// Path to the '.xcworkspace' to operate on (overrides auto-discovery).
    #[arg(long, env = "SWEETPAD_WORKSPACE", global = true)]
    pub workspace: Option<std::path::PathBuf>,

    /// Path to the '.xcodeproj' to operate on (overrides auto-discovery).
    #[arg(long, env = "SWEETPAD_PROJECT", global = true)]
    pub project: Option<std::path::PathBuf>,
}

/// Tier 2 — container plus a scheme. For commands that need to know *which*
/// scheme but not a full build target.
#[derive(Debug, Clone, Default, clap::Args)]
#[command(next_help_heading = "Target selection")]
pub struct SchemeArgs {
    #[command(flatten)]
    pub container: ContainerArgs,

    /// Scheme to use (overrides config and remembered selection).
    #[arg(long, env = "SWEETPAD_SCHEME", global = true)]
    pub scheme: Option<String>,
}

/// Tier 3 — everything `xcodebuild` needs: container, scheme, configuration,
/// and destination. For the build-ish commands (`build`, `test`, `settings`,
/// `app`).
#[derive(Debug, Clone, Default, clap::Args)]
#[command(next_help_heading = "Target selection")]
pub struct BuildTargetArgs {
    #[command(flatten)]
    pub scheme: SchemeArgs,

    /// Build configuration to use (e.g. Debug, Release).
    #[arg(long, env = "SWEETPAD_CONFIGURATION", global = true)]
    pub configuration: Option<String>,

    /// Destination specifier (e.g. "platform=iOS Simulator,name=iPhone 15").
    #[arg(long, env = "SWEETPAD_DESTINATION", global = true)]
    pub destination: Option<String>,

    /// Where to build/run, as a human reference: a fuzzy simulator/device name
    /// ("iPhone 16 Pro"), 'booted', 'mac', 'device', a platform word ('ios',
    /// 'watchos', …), or a UDID. Resolved against the live device list;
    /// --destination stays the raw escape hatch.
    #[arg(long, env = "SWEETPAD_ON", global = true)]
    pub on: Option<String>,

    /// SDK to build against (e.g. iphonesimulator, macosx). Rarely needed —
    /// the destination usually implies it.
    #[arg(long, env = "SWEETPAD_SDK", global = true)]
    pub sdk: Option<String>,
}

/// The resolved-from-flags targeting handed to commands via [`Context`]. Each
/// command populates the subset of fields its tier exposes; the rest stay
/// `None`. Resolution precedence (flag > env > config > state > auto-discovery)
/// is applied over this in [`resolve`].
#[derive(Debug, Default)]
pub struct Targeting {
    pub workspace: Option<std::path::PathBuf>,
    pub project: Option<std::path::PathBuf>,
    pub scheme: Option<String>,
    pub configuration: Option<String>,
    pub destination: Option<String>,
    /// The human `--on` destination reference, resolved lazily against the
    /// device list where a destination is settled.
    pub on: Option<String>,
    pub sdk: Option<String>,
}

/// Normalize clap's `env = …` fallback: an exported-but-empty `SWEETPAD_*`
/// var arrives as `Some("")`, which would skip the resolution layers below it
/// and hand xcodebuild an empty `-scheme`/`-destination`. Empty means unset —
/// matching `Targeting::from_env`, which the bare status view uses.
fn non_empty(v: Option<String>) -> Option<String> {
    v.filter(|s| !s.is_empty())
}

/// [`non_empty`] for path-valued flags.
fn non_empty_path(v: Option<std::path::PathBuf>) -> Option<std::path::PathBuf> {
    v.filter(|p| !p.as_os_str().is_empty())
}

impl From<ContainerArgs> for Targeting {
    fn from(a: ContainerArgs) -> Self {
        // clap's `env = …` folds `SWEETPAD_WORKSPACE`/`SWEETPAD_PROJECT` into
        // the flag layer, which would let an exported workspace silently beat
        // an explicit `--project` (the container check prefers workspace).
        // Consult the real argv to restore the documented flag > env order.
        let (workspace, project) = disambiguate_container(
            non_empty_path(a.workspace),
            non_empty_path(a.project),
            flag_typed("--workspace"),
            flag_typed("--project"),
        );
        Self {
            workspace,
            project,
            scheme: None,
            configuration: None,
            destination: None,
            on: None,
            sdk: None,
        }
    }
}

/// Drop present-but-empty targeting env vars before clap reads them: a
/// placeholder `export SWEETPAD_SCHEME=` in CI must mean "unset", not an
/// always-wrong `Some("")` (or, for the `PathBuf` flags, a clap "a value is
/// required" usage error) — and the bare status view's [`Targeting::from_env`]
/// (which filters empties) must agree with what commands parse.
fn scrub_empty_env() {
    for key in [
        "SWEETPAD_WORKSPACE",
        "SWEETPAD_PROJECT",
        "SWEETPAD_SCHEME",
        "SWEETPAD_CONFIGURATION",
        "SWEETPAD_DESTINATION",
        "SWEETPAD_ON",
        "SWEETPAD_SDK",
        "DEVELOPER_DIR",
    ] {
        if std::env::var_os(key).is_some_and(|v| v.is_empty()) {
            // Safety: single-threaded startup, before clap or any tool spawn
            // reads the environment.
            unsafe { std::env::remove_var(key) };
        }
    }
}

/// Whether the literal flag token was typed on the command line, as opposed to
/// the value arriving through the flag's `env = …` fallback (clap reports both
/// identically). A value-carrying flag can't itself be consumed as a value, so
/// a matching token is the flag. The scan stops at the first bare `--`:
/// passthrough tokens belong to the child tool and must not flip
/// disambiguation.
fn flag_typed(flag: &str) -> bool {
    std::env::args()
        .take_while(|a| a != "--")
        .any(|a| a == flag || (a.starts_with(flag) && a[flag.len()..].starts_with('=')))
}

/// Apply flag > env between the two container flags: when both are set and
/// exactly one was typed on the command line, the env-sourced one is dropped.
/// Both-typed stays meaningful (a workspace container plus `--project` as the
/// member to mutate, used by `dependency`), and both-from-env keeps the
/// documented workspace-first preference.
fn disambiguate_container(
    workspace: Option<std::path::PathBuf>,
    project: Option<std::path::PathBuf>,
    workspace_typed: bool,
    project_typed: bool,
) -> (Option<std::path::PathBuf>, Option<std::path::PathBuf>) {
    match (workspace.is_some(), project.is_some()) {
        (true, true) if project_typed && !workspace_typed => (None, project),
        (true, true) if workspace_typed && !project_typed => (workspace, None),
        _ => (workspace, project),
    }
}

/// Whether `-V`/`--version` is the first token besides output-mode flags —
/// the only shape the pre-clap fast path may answer. Anywhere later the token
/// belongs to a subcommand parse (`sweetpad build --json -V` is a usage error
/// in human mode and must stay one under `--json`).
fn version_flag_leads(head: &[&str]) -> bool {
    let mut i = 0;
    while i < head.len() {
        let a = head[i];
        if a == "--json"
            || a.starts_with("--output=")
            || (a.starts_with("-o") && !a.starts_with("--") && a.len() > 2)
        {
            i += 1;
        } else if a == "-o" || a == "--output" {
            i += 2;
        } else {
            return a == "--version" || a == "-V";
        }
    }
    false
}

/// The machine output mode the pre-clap fast path should honor: the last
/// `-o`/`--output` value wins outright (including `-o human` *disabling* a
/// `--json`); with no `-o`, `--json` selects the envelope. `None` = human.
fn machine_output_mode(head: &[&str]) -> Option<OutputMode> {
    let mut from_o: Option<Option<OutputMode>> = None;
    let mut json_flag = false;
    let parse = |v: &str| match v {
        "json" => Some(OutputMode::Json),
        "ndjson" => Some(OutputMode::Ndjson),
        _ => None, // human/quiet — machine output off
    };
    let mut i = 0;
    while i < head.len() {
        let a = head[i];
        if a == "--json" {
            json_flag = true;
        } else if a == "-o" || a == "--output" {
            if let Some(v) = head.get(i + 1) {
                from_o = Some(parse(v));
                i += 1;
            }
        } else if let Some(v) = a.strip_prefix("--output=") {
            from_o = Some(parse(v));
        } else if let Some(v) = a.strip_prefix("-o=").or_else(|| {
            (a.len() > 2 && a.starts_with("-o") && !a.starts_with("--")).then(|| &a[2..])
        }) {
            from_o = Some(parse(v));
        }
        i += 1;
    }
    match from_o {
        Some(mode) => mode,
        None => json_flag.then_some(OutputMode::Json),
    }
}

impl Targeting {
    /// The env-var layer alone. The bare `sweetpad` status view and the bare
    /// `sweetpad app` default action parse no flag-carrying subcommand, so
    /// clap never folds `SWEETPAD_*` into flags there — without this they
    /// would ignore an env context the very next explicit command honors.
    pub(crate) fn from_env() -> Self {
        let var = |k: &str| std::env::var(k).ok().filter(|v| !v.is_empty());
        let (on, destination) = disambiguate_on_destination(
            var("SWEETPAD_ON"),
            var("SWEETPAD_DESTINATION"),
            false,
            false,
        );
        Self {
            workspace: var("SWEETPAD_WORKSPACE").map(Into::into),
            project: var("SWEETPAD_PROJECT").map(Into::into),
            scheme: var("SWEETPAD_SCHEME"),
            configuration: var("SWEETPAD_CONFIGURATION"),
            destination,
            on,
            sdk: var("SWEETPAD_SDK"),
        }
    }
}

impl From<SchemeArgs> for Targeting {
    fn from(a: SchemeArgs) -> Self {
        Self {
            scheme: non_empty(a.scheme),
            ..a.container.into()
        }
    }
}

impl From<BuildTargetArgs> for Targeting {
    fn from(a: BuildTargetArgs) -> Self {
        // `--on` and `--destination` are exclusive ways to name the same
        // thing. A clap-level conflict would fire on *env-sourced* values
        // (an exported SWEETPAD_DESTINATION breaking a typed `--on`), so the
        // pair resolves post-parse like the container flags: the typed one
        // wins; both-typed is rejected where the destination settles.
        let (on, destination) = disambiguate_on_destination(
            non_empty(a.on),
            non_empty(a.destination),
            flag_typed("--on"),
            flag_typed("--destination"),
        );
        Self {
            configuration: non_empty(a.configuration),
            destination,
            on,
            sdk: non_empty(a.sdk),
            ..a.scheme.into()
        }
    }
}

/// Apply flag > env between `--on` and `--destination`: when both are set and
/// exactly one was typed, the env-sourced one yields. Both-from-env resolves
/// per the documented contract — `SWEETPAD_ON` overrides
/// `SWEETPAD_DESTINATION` (`help environment`) — so an `.envrc` exporting both
/// works instead of failing every build. Only both-*typed* keeps both, and
/// the resolver rejects that.
fn disambiguate_on_destination(
    on: Option<String>,
    destination: Option<String>,
    on_typed: bool,
    destination_typed: bool,
) -> (Option<String>, Option<String>) {
    match (on.is_some(), destination.is_some()) {
        (true, true) if on_typed && !destination_typed => (on, None),
        (true, true) if destination_typed && !on_typed => (None, destination),
        (true, true) if !on_typed && !destination_typed => (on, None),
        _ => (on, destination),
    }
}

/// Apply flag > env between `--on` and the mode flags (`--mac`, `--device`,
/// `--device-id`): an env-sourced `SWEETPAD_ON` yields to a typed mode flag
/// instead of turning it into an error about a flag the user never typed;
/// `--on` *typed* alongside a mode flag is a real conflict.
pub(crate) fn settle_on_vs_mode(
    targeting: &mut Targeting,
    mode_typed: bool,
) -> Result<(), CliError> {
    if targeting.on.is_none() || !mode_typed {
        return Ok(());
    }
    if flag_typed("--on") {
        return Err(CliError::new(
            "--on and --mac/--device/--device-id are mutually exclusive; pass one",
        )
        .kind(ErrorKind::TargetResolution));
    }
    targeting.on = None;
    Ok(())
}

/// Top-level resources. Each is a noun; actions are its subcommands.
#[derive(Debug, Subcommand)]
pub enum Resource {
    /// Inspect schemes.
    Scheme {
        #[command(subcommand)]
        action: commands::scheme::Action,
    },
    /// Inspect build destinations (hidden alias — see 'devices').
    #[command(hide = true)]
    Destination {
        #[command(subcommand)]
        action: commands::destination::Action,
    },
    /// Everything runnable — macOS, simulators, connected devices — each with
    /// its ready '-destination' specifier, most-used first, the remembered
    /// one marked.
    Devices {
        #[command(flatten)]
        target: ContainerArgs,
    },
    /// Show, select, or clear the project's remembered build context.
    Context {
        #[command(flatten)]
        target: ContainerArgs,
        #[command(subcommand)]
        action: commands::context::Action,
    },
    /// Inspect the project: targets, configurations, schemes.
    Project {
        #[command(subcommand)]
        action: commands::project::Action,
    },
    /// View and manage the project's Swift Package Manager dependencies.
    #[command(visible_alias = "dep")]
    Dependency {
        #[command(flatten)]
        target: ContainerArgs,
        #[command(subcommand)]
        action: commands::dependency::Action,
    },
    /// Show resolved build settings.
    Settings {
        #[command(subcommand)]
        action: commands::settings::Action,
    },
    /// Manage iOS simulators.
    #[command(visible_alias = "sim")]
    Simulator {
        #[command(subcommand)]
        action: commands::simulator::Action,
    },
    /// Build, install, launch, and follow logs (the flagship loop; same as
    /// 'app run').
    Run(commands::app::RunArgs),
    /// Compile the project ('build' alone runs 'build start').
    Build {
        #[command(flatten)]
        args: commands::build::StartArgs,
        #[command(subcommand)]
        action: Option<commands::build::Action>,
    },
    /// Run the project's tests ('test' alone runs 'test run').
    Test {
        #[command(flatten)]
        args: commands::test::TestArgs,
        #[command(subcommand)]
        action: Option<commands::test::Action>,
    },
    /// Archive the app and export an .ipa (xcodebuild archive + -exportArchive).
    Archive(commands::archive::ArchiveArgs),
    /// Clean build artifacts (xcodebuild clean; --purge adds DerivedData).
    Clean(commands::clean::CleanArgs),
    /// Run, install, and manage the built app's lifecycle ('app' alone runs
    /// 'app run').
    App {
        #[command(subcommand)]
        action: Option<commands::app::Action>,
    },
    /// Inspect connected physical devices (hidden alias — see 'devices').
    #[command(hide = true)]
    Device {
        #[command(subcommand)]
        action: commands::device::Action,
    },
    /// Format or lint Swift sources ('format' alone runs 'format run').
    #[command(visible_alias = "fmt")]
    Format {
        #[command(flatten)]
        args: commands::format::FormatArgs,
        #[command(subcommand)]
        action: Option<commands::format::Action>,
    },
    /// Low-level 'project.pbxproj' editing: stored settings, synchronized
    /// folders, per-file membership, merge resolution (plumbing; §9g).
    Pbxproj {
        #[command(subcommand)]
        action: commands::pbxproj::Action,
    },
    /// Work with SwiftPM 'Package.resolved' files (hidden alias — see
    /// 'merge run').
    #[command(hide = true)]
    Spm {
        #[command(subcommand)]
        action: commands::spm::Action,
    },
    /// Git integration: install/run sweetpad's semantic merge drivers.
    Merge {
        #[command(subcommand)]
        action: commands::merge::Action,
    },
    /// Build Server Protocol integration (sourcekit-lsp autocomplete).
    Bsp {
        #[command(flatten)]
        target: ContainerArgs,
        #[command(subcommand)]
        action: commands::bsp::Action,
    },
    /// Inspect and purge Xcode's DerivedData.
    #[command(visible_alias = "dd")]
    DerivedData {
        #[command(flatten)]
        target: ContainerArgs,
        #[command(subcommand)]
        action: commands::derived_data::Action,
    },
    /// Open the project in Xcode, Simulator.app, the DerivedData folder, or
    /// the config file.
    Open {
        #[command(flatten)]
        target: ContainerArgs,
        /// What to open.
        #[arg(value_enum)]
        what: commands::open::What,
    },
    /// Inspect or clear the hot-reload listener a dead '--hot' session left
    /// bound.
    Hot {
        #[command(subcommand)]
        action: commands::hot::Action,
    },
    /// Diagnose the local Xcode/Swift toolchain.
    Doctor,
    /// Show the effective build context — what would build, and where each
    /// value comes from (flag/env/config/remembered/default).
    Status {
        #[command(flatten)]
        target: BuildTargetArgs,
    },
    /// Update sweetpad (Homebrew installs run 'brew upgrade sweetpad').
    SelfUpdate,
    /// Explain a topic: config, environment, exit-codes, destinations, hot-reload.
    Help {
        /// The topic to explain (omit to list the topics).
        topic: Option<String>,
    },
    /// Generate shell completion scripts.
    Completions {
        /// Shell to generate completions for.
        shell: clap_complete::Shell,
    },
}

/// Shared context handed to every command: parsed global flags plus lazily
/// loaded config and state. Resolution helpers in [`resolve`] read from here.
pub struct Context {
    pub global: GlobalArgs,
    /// Targeting flags from the command that's running, folded into a uniform
    /// shape. Empty for commands that don't target a project.
    pub targeting: Targeting,
    pub config: config::Config,
    pub state: state::State,
    pub out: output::Output,
    /// The committed `sweetpad.toml`, loaded once on first use (see
    /// [`Context::project_file`]).
    project_toml: std::cell::OnceCell<config::ProjectFile>,
    /// The nearest `sweetpad.toml` at or above the cwd, resolved *before* the
    /// container so its `workspace`/`project` key can name one (see
    /// [`Context::root_file`]).
    root_toml: std::cell::OnceCell<Option<config::RootFile>>,
    /// Latches once the generated-project staleness check has run, so a
    /// command that resolves more than once (`build --watch` re-resolves per
    /// rebuild) warns a single time.
    stale_checked: std::cell::OnceCell<()>,
}

impl Context {
    /// Warn when this project is generated from a spec that has been edited
    /// since — see [`pbxedit::stale_generated`] for why that is worth saying
    /// out loud. Fires at most once per process; a `.xcworkspace` or Swift
    /// package has no single generated `.xcodeproj` to compare against.
    pub fn warn_if_project_stale(&self, container: &resolve::Container) {
        let resolve::Container::Project(xcodeproj) = container else {
            return;
        };
        if self.stale_checked.set(()).is_err() {
            return;
        }
        if let Some(warning) = pbxedit::stale_generated(self.project_file(container), xcodeproj) {
            self.out.warn(&warning);
        }
    }

    /// The nearest committed `sweetpad.toml` at or above the working
    /// directory, or `None` when this checkout has none. Consulted during
    /// container resolution, so it loads before a container exists — which is
    /// the point: its `workspace`/`project` key is what names one when the
    /// container sits in a subdirectory the upward walk never reaches.
    pub fn root_file(&self) -> Option<&config::RootFile> {
        self.root_toml
            .get_or_init(|| {
                let cwd = std::env::current_dir().ok()?;
                let (root, warnings) = config::RootFile::find_upward(&cwd)?;
                for w in &warnings {
                    self.out.warn(w);
                }
                pin_developer_dir(&root.file);
                Some(root)
            })
            .as_ref()
    }

    /// The committed `sweetpad.toml` for `container` — the team-shared
    /// defaults layer between the user config and remembered state. Loaded
    /// once per process; lint warnings surface on that first load, and a
    /// pinned `developer_dir` takes effect (unless a flag/env already set it).
    ///
    /// The root file serves as the project file whenever it covers this
    /// container, so the file that named a nested container also supplies its
    /// defaults, and the beside-the-container case isn't read and linted twice.
    /// A file that names some *other* container supplies nothing here — its
    /// defaults belong to the project it named, even for a container sitting
    /// beside it.
    pub fn project_file(&self, container: &resolve::Container) -> &config::ProjectFile {
        if let Some(root) = self.root_file()
            && root.covers(container)
        {
            return &root.file;
        }
        self.project_toml.get_or_init(|| {
            let dir = container
                .path()
                .parent()
                .filter(|p| !p.as_os_str().is_empty())
                .map_or_else(
                    || std::path::PathBuf::from("."),
                    std::path::Path::to_path_buf,
                );
            let (pf, warnings) = config::ProjectFile::load_for(&dir);
            let beside = config::RootFile { dir, file: pf };
            if !beside.covers(container) {
                return config::ProjectFile::default();
            }
            for w in &warnings {
                self.out.warn(w);
            }
            pin_developer_dir(&beside.file);
            beside.file
        })
    }

    /// The `xcodebuild` arguments for a command that builds: the project
    /// file's `[xcodebuild] args`, then the `--` tail typed on this
    /// invocation. Every verb that spawns `xcodebuild` resolves its
    /// passthrough through here, so a committed argument reaches the builds
    /// inside `app run`/`install`/`debug`/`diagnose` as well as
    /// `build`/`test`/`archive`.
    pub fn xcodebuild_args(&self, tail: &[String]) -> Result<Vec<String>, CliError> {
        // Silent resolution: this runs *before* the command resolves for real,
        // and `container` narrates its discovery ("using X (found below …)") —
        // saying it twice per build would be the whole visible effect of a peek
        // at a config table. No container found is not an error here either;
        // resolution is about to fail on its own terms, and the typed tail is
        // still the caller's.
        let Some(container) = resolve::container_silently(self) else {
            return Ok(tail.to_vec());
        };
        // `swift build`/`swift run` take the tail directly and know none of
        // xcodebuild's flags, so a package's file contributes nothing here.
        if matches!(container, resolve::Container::SwiftPackage(_)) {
            return Ok(tail.to_vec());
        }
        let configured = self.project_file(&container).xcodebuild.args.clone();
        config::effective_xcodebuild_args(&configured, tail).map_err(CliError::new)
    }
}

/// Apply a project file's `developer_dir`, unless a flag or the ambient
/// environment already chose an Xcode.
fn pin_developer_dir(pf: &config::ProjectFile) {
    if let Some(dev_dir) = &pf.developer_dir
        && std::env::var_os("DEVELOPER_DIR").is_none()
    {
        // Safety: first project-file access happens on the main thread before
        // tool children spawn.
        unsafe { std::env::set_var("DEVELOPER_DIR", dev_dir) };
    }
}

/// Entry point for the CLI half of the binary. `argv` is the full process
/// argument vector minus `argv[0]` (clap re-prepends the program name).
#[must_use]
#[allow(clippy::too_many_lines)] // the one-arm-per-resource dispatch table
pub fn run(argv: &[String]) -> ExitCode {
    // SIGINT/SIGTERM cleanup (terminal restore, build-group forwarding, child
    // reaping) — installed before anything spawns or flips terminal modes.
    signals::install();
    scrub_empty_env();

    // `--version` under a machine output mode: clap's own --version is plain
    // text; agents get the envelope (or, under ndjson, the terminal result
    // event). Only tokens before `--` count — passthrough belongs to the
    // spawned tool — and `-o` wins over `--json` per that flag's contract.
    let head: Vec<&str> = argv
        .iter()
        .take_while(|a| *a != "--")
        .map(String::as_str)
        .collect();
    if version_flag_leads(&head)
        && let Some(mode) = machine_output_mode(&head)
    {
        #[allow(clippy::print_stdout)] // pre-clap fast path; Output isn't built yet
        {
            let version = env!("SWEETPAD_VERSION");
            match mode {
                OutputMode::Ndjson => {
                    println!(r#"{{"event":"result","ok":true,"data":{{"version":"{version}"}}}}"#);
                }
                _ => println!(r#"{{"schema":1,"ok":true,"data":{{"version":"{version}"}}}}"#),
            }
        }
        return ExitCode::SUCCESS;
    }

    let cli = match Cli::try_parse_from(
        std::iter::once("sweetpad".to_string()).chain(argv.iter().cloned()),
    ) {
        Ok(cli) => cli,
        Err(err) => {
            // Top-level `--help`/`-h` gets our three-group command listing;
            // subcommand help and usage/errors stay clap's (it renders them and
            // picks the right stream).
            if err.kind() == clap::error::ErrorKind::DisplayHelp
                && let Some(long) = top_level_help(argv)
            {
                render_root_help(stdout_wants_color(argv), long);
                return ExitCode::SUCCESS;
            }
            let err = hint_output_file(err, argv);
            let _ = err.print();
            return ExitCode::from(if err.use_stderr() { 2 } else { 0 });
        }
    };

    let out = output::Output::new(&cli.global);
    // `-C DIR` chdirs before any discovery/config touches the filesystem.
    if let Some(dir) = &cli.global.chdir
        && let Err(e) = std::env::set_current_dir(dir)
    {
        let err = CliError::new(format!("cannot change directory to {}: {e}", dir.display()));
        render_early_error(&out, &err);
        return ExitCode::from(err.error_kind().exit_code());
    }
    // `--developer-dir` pins the Xcode every spawned tool uses (xcrun,
    // xcodebuild, simctl all honor DEVELOPER_DIR).
    if let Some(dir) = &cli.global.developer_dir {
        // Safety: single-threaded startup; no other thread reads the env yet.
        unsafe { std::env::set_var("DEVELOPER_DIR", dir) };
    }
    if cli.global.gh_annotations && (out.is_json() || out.is_ndjson()) {
        let err = CliError::new(
            "--gh-annotations writes ::error workflow commands to stdout, which -o json/ndjson \
             reserve for the envelope/event stream; use --gh-annotations with human output",
        );
        render_early_error(&out, &err);
        return ExitCode::from(err.error_kind().exit_code());
    }
    let config = match config::Config::load() {
        Ok(c) => c,
        Err(e) => {
            out.warn(&format!(
                "failed to load config: {e} — continuing with defaults \
                 (`sweetpad open config` to fix it)"
            ));
            config::Config::default()
        }
    };
    for w in &config.warnings {
        out.warn(w);
    }
    // A corrupt state file is quarantined (renamed) and reported, never
    // silently wiped by the next save.
    let (state, state_warning) = state::State::load_or_quarantine();
    if let Some(w) = state_warning {
        out.warn(&w);
    }

    // Completions need nothing from config/state — emit and return.
    if let Some(Resource::Completions { shell }) = &cli.resource {
        clap_complete::generate(
            *shell,
            &mut Cli::command(),
            "sweetpad",
            &mut std::io::stdout(),
        );
        return ExitCode::SUCCESS;
    }

    let mut ctx = Context {
        global: cli.global,
        targeting: Targeting::default(),
        config,
        state,
        out,
        project_toml: std::cell::OnceCell::new(),
        root_toml: std::cell::OnceCell::new(),
        stale_checked: std::cell::OnceCell::new(),
    };

    // Bare `sweetpad`: the status view inside a project (the daily "where am
    // I" glance), the help wall only outside one — and under a machine mode,
    // an error envelope instead of a help wall on stdout. The probe is
    // silent; status itself resolves (and warns about ambiguity) once.
    let Some(resource) = cli.resource else {
        ctx.targeting = Targeting::from_env();
        if resolve::container_silently(&ctx).is_some() {
            let result = commands::status::run(&mut ctx);
            return render_result(&ctx, result);
        }
        if ctx.out.is_json() || ctx.out.is_ndjson() {
            let err = CliError::new(
                "no .xcworkspace, .xcodeproj, or Package.swift found in the current \
                 directory or its ancestors (run inside a project, or pass -C/--project)",
            )
            .kind(ErrorKind::TargetResolution);
            return render_result(&ctx, Err(err));
        }
        render_root_help(ctx.out.use_color(), false);
        return ExitCode::SUCCESS;
    };

    let result = match resource {
        // Scheme/project/app carry their targeting per action (a sibling like
        // `project new` or `app open-url` consumes none), so their `run`s set
        // `ctx.targeting` themselves.
        Resource::Scheme { action } => commands::scheme::run(&mut ctx, &action),
        Resource::Run(run_args) => {
            commands::app::run(&mut ctx, &commands::app::Action::Run(run_args))
        }
        Resource::Destination { action } => commands::destination::run(&mut ctx, &action),
        Resource::Devices { target } => {
            ctx.targeting = target.into();
            commands::destination::devices(&mut ctx)
        }
        Resource::Context { target, action } => {
            ctx.targeting = target.into();
            commands::context::run(&mut ctx, &action)
        }
        Resource::Project { action } => commands::project::run(&mut ctx, &action),
        Resource::Dependency { target, action } => {
            ctx.targeting = target.into();
            commands::dependency::run(&mut ctx, &action)
        }
        Resource::Settings { action } => commands::settings::run(&mut ctx, &action),
        Resource::Simulator { action } => commands::simulator::run(&mut ctx, &action),
        // `build`/`test` carry their flags as resource-level globals; the bare
        // `start`/`run` tokens are optional markers, so both spellings land here.
        Resource::Build { args, action } => commands::build::run(&mut ctx, &args, action.as_ref()),
        Resource::Test { args, action } => commands::test::run(&mut ctx, &args, action.as_ref()),
        Resource::Archive(archive_args) => commands::archive::run(&mut ctx, &archive_args),
        Resource::Clean(clean_args) => {
            let mut targeting: Targeting = clean_args.scheme.clone().into();
            // Same `env = SWEETPAD_CONFIGURATION` normalization as
            // `BuildTargetArgs`: exported-but-empty means unset.
            targeting.configuration = non_empty(clean_args.configuration.clone());
            ctx.targeting = targeting;
            commands::clean::run(&mut ctx, clean_args.purge)
        }
        Resource::App { action } => {
            let action = action.unwrap_or_else(commands::app::Action::default_run);
            commands::app::run(&mut ctx, &action)
        }
        Resource::Device { action } => commands::device::run(&mut ctx, &action),
        Resource::Format { args, action } => {
            commands::format::run(&mut ctx, &args, action.as_ref())
        }
        Resource::Pbxproj { action } => commands::pbxproj::run(&mut ctx, &action),
        Resource::Spm { action } => commands::spm::run(&mut ctx, &action),
        Resource::Merge { action } => commands::merge::run(&mut ctx, &action),
        Resource::Bsp { target, action } => {
            ctx.targeting = target.into();
            commands::bsp::run(&mut ctx, &action)
        }
        Resource::DerivedData { target, action } => {
            ctx.targeting = target.into();
            commands::derived_data::run(&mut ctx, &action)
        }
        Resource::Open { target, what } => {
            ctx.targeting = target.into();
            commands::open::run(&mut ctx, what)
        }
        Resource::Hot { action } => commands::hot::run(&mut ctx, &action),
        Resource::Doctor => commands::doctor::run(&mut ctx),
        Resource::Status { target } => {
            ctx.targeting = target.into();
            commands::status::run(&mut ctx)
        }
        Resource::SelfUpdate => commands::self_update::run(&mut ctx),
        Resource::Help { topic } => commands::help_topics::run(&mut ctx, topic.as_deref()),
        Resource::Completions { .. } => unreachable!("handled above"),
    };

    let code = render_result(&ctx, result);
    first_run_hint(&ctx.out);
    code
}

/// The top-level command listing, split into three tiers so the surface reads
/// at a glance: the daily-loop shortcuts, the full namespaced commands, and the
/// low-level plumbing for scripts and agents. Each entry names a top-level
/// command; unlisted (or hidden) ones fall through to [`GROUP_MORE`].
struct HelpGroup {
    /// The section heading, rendered where clap would print `Commands:`.
    heading: &'static str,
    /// Mark every entry with the everyday glyph and lead the listing.
    everyday: bool,
    /// Command names in the order they should appear under the heading.
    names: &'static [&'static str],
}

/// The daily loop — mostly action shortcuts for the longer namespaced verbs
/// (`run` = `app run`, `build` = `build start`, …), plus the aggregated views.
const GROUP_EVERYDAY: HelpGroup = HelpGroup {
    heading: "Everyday commands",
    everyday: true,
    names: &["run", "build", "test", "devices", "clean", "format"],
};

/// The plumbing tier: low-level, scriptable, aimed at scripts and AI agents.
/// `vscode` is [synthetic](SYNTHETIC) — peeled off before clap, so it has no
/// subcommand entry to reuse.
const GROUP_PLUMBING: HelpGroup = HelpGroup {
    heading: "Plumbing (scripting & agents)",
    everyday: false,
    names: &["pbxproj", "spm", "merge", "bsp", "vscode"],
};

/// Commands `main` peels off before the clap resource tree parses, so they have
/// no subcommand to introspect — listed here with the description to render.
const SYNTHETIC: &[(&str, &str)] = &[(
    "vscode",
    "Control the running VS Code extension over JSON-RPC",
)];

/// Everything else — the full, feature-complete namespaced commands. The
/// catch-all: any visible command not claimed above lands here, in clap's
/// declaration order.
const GROUP_MORE: HelpGroup = HelpGroup {
    heading: "Commands",
    everyday: false,
    names: &[],
};

/// The everyday-command marker: a bright glyph in place of one indent space, so
/// the daily-loop commands catch the eye without disturbing clap's column.
fn everyday_marker(color: bool) -> String {
    if color {
        "\x1b[36m▸\x1b[0m ".to_string()
    } else {
        "▸ ".to_string()
    }
}

/// A command listing line for a [`SYNTHETIC`] command, padded to line its
/// description up with clap's `desc_col` and bolding the name the way clap does.
fn synthetic_entry(name: &str, desc: &str, desc_col: usize, color: bool) -> String {
    let pad = " ".repeat(desc_col.saturating_sub(2 + name.len()).max(1));
    let name = if color {
        format!("\x1b[1m{name}\x1b[0m")
    } else {
        name.to_string()
    };
    format!("  {name}{pad}{desc}")
}

/// Render `heading:` the way clap styles its section headers (bold + underline).
fn help_heading(heading: &str, color: bool) -> String {
    if color {
        format!("\x1b[1m\x1b[4m{heading}:\x1b[0m")
    } else {
        format!("{heading}:")
    }
}

/// Strip ANSI SGR escapes (`\x1b[…m`) so a styled command line can be read for
/// its leading command name.
fn strip_ansi(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c == '\x1b' {
            // Drop through the terminating `m` of the escape sequence.
            for e in chars.by_ref() {
                if e == 'm' {
                    break;
                }
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// Print the top-level help with the command listing split into
/// [`GROUP_EVERYDAY`]/[`GROUP_MORE`]/[`GROUP_PLUMBING`]. The `about`, `usage`,
/// and `Options` sections are clap's own output, verbatim; only the command
/// listing is regrouped, and the trailing `Global:` flag section is replaced
/// with a pointer (those flags apply to every command and are shown on each
/// command's own `--help`).
#[allow(clippy::print_stdout)] // help text is the command's primary output
fn render_root_help(color: bool, long: bool) {
    let mut cmd = Cli::command();
    // Match clap's own `--help` (long) vs `-h` (short) verbosity for the
    // options/flags; only the command listing is regrouped.
    let help = if long {
        cmd.render_long_help()
    } else {
        cmd.render_help()
    };
    let text = if color {
        help.ansi().to_string()
    } else {
        help.to_string()
    };
    let lines: Vec<&str> = text.lines().collect();

    // Locate the `Commands:` block: its heading line, and the blank line that
    // closes it (clap separates sections with a blank line).
    let Some(start) = lines
        .iter()
        .position(|l| strip_ansi(l).trim_end() == "Commands:")
    else {
        // No command block to regroup (shouldn't happen) — emit clap's help.
        print!("{text}");
        return;
    };
    let end = lines[start + 1..]
        .iter()
        .position(|l| strip_ansi(l).trim().is_empty())
        .map_or(lines.len(), |off| start + 1 + off);

    // Split each command entry into its (name, [lines]); an entry starts at the
    // two-space indent, wrapped-description continuations are indented deeper.
    let mut entries: Vec<(String, Vec<&str>)> = Vec::new();
    for &line in &lines[start + 1..end] {
        let plain = strip_ansi(line);
        let is_new = plain.starts_with("  ") && plain.as_bytes().get(2).is_some_and(|b| *b != b' ');
        if is_new {
            let name = plain.split_whitespace().next().unwrap_or("");
            entries.push((name.to_string(), vec![line]));
        } else if let Some(last) = entries.last_mut() {
            last.1.push(line);
        }
    }

    let claimed: std::collections::HashSet<&str> = GROUP_EVERYDAY
        .names
        .iter()
        .chain(GROUP_PLUMBING.names)
        .copied()
        .collect();

    // The column clap aligns descriptions to (2-space indent + longest name +
    // gap), read off a real entry so synthetic lines line up with it.
    let desc_col = entries.first().map_or(16, |(name, entry_lines)| {
        let plain = strip_ansi(entry_lines[0]);
        let after = 2 + name.len();
        after + plain[after..].chars().take_while(|c| *c == ' ').count()
    });

    let mut block: Vec<String> = Vec::new();
    for group in [&GROUP_EVERYDAY, &GROUP_MORE, &GROUP_PLUMBING] {
        // The names to list: explicit order for the tiered groups, or — for the
        // catch-all — every clap entry no other group claimed, in clap's order.
        let names: Vec<&str> = if group.names.is_empty() {
            entries
                .iter()
                .map(|(n, _)| n.as_str())
                .filter(|n| !claimed.contains(n))
                .collect()
        } else {
            group.names.to_vec()
        };

        let mut group_lines: Vec<String> = Vec::new();
        for name in names {
            if let Some((_, entry_lines)) = entries.iter().find(|(n, _)| n == name) {
                for (li, &line) in entry_lines.iter().enumerate() {
                    if group.everyday && li == 0 && line.starts_with("  ") {
                        // Swap the first two-space indent for the everyday glyph.
                        group_lines.push(format!("{}{}", everyday_marker(color), &line[2..]));
                    } else {
                        group_lines.push(line.to_string());
                    }
                }
            } else if let Some((_, desc)) = SYNTHETIC.iter().find(|(n, _)| *n == name) {
                group_lines.push(synthetic_entry(name, desc, desc_col, color));
            }
            // Otherwise the command is hidden (e.g. `spm`) — nothing to list.
        }

        if group_lines.is_empty() {
            continue;
        }
        if !block.is_empty() {
            block.push(String::new()); // one blank line between groups
        }
        block.push(help_heading(group.heading, color));
        block.extend(group_lines);
    }

    // Splice the regrouped block back in place of clap's flat `Commands:` list,
    // then drop the trailing `Global:` flag section for a pointer — those flags
    // work on every command and belong on each command's own `--help`, not
    // cluttering the top-level discovery view. The blank lines bracketing the
    // command block come from clap's own section separators (already in
    // `lines[..start]` and the tail).
    let mut result: Vec<String> = lines[..start].iter().map(|s| (*s).to_string()).collect();
    result.extend(block);

    let tail = &lines[end..];
    if let Some(g) = tail
        .iter()
        .position(|l| strip_ansi(l).trim_end() == "Global:")
    {
        // Keep the `Options` section up to the blank line that precedes
        // `Global:`, then swap `Global:` for a one-line pointer.
        result.extend(tail[..g.saturating_sub(1)].iter().map(|s| (*s).to_string()));
        result.push(String::new());
        result.push(help_heading("Global options", color));
        result.push(
            "  Apply to every command — see any command's --help \
             (e.g. 'sweetpad build --help')."
                .to_string(),
        );
    } else {
        result.extend(tail.iter().map(|s| (*s).to_string()));
    }
    println!("{}", result.join("\n"));
}

/// Whether top-level help was requested (vs. a subcommand's), and if so whether
/// it was the long `--help` (`Some(true)`) or short `-h` (`Some(false)`).
/// `None` once a bare subcommand token precedes the help flag — that help is
/// clap's to render. Value-carrying global options (`-C`, `-o`,
/// `--developer-dir`) consume their following token so it is not mistaken for a
/// subcommand.
fn top_level_help(argv: &[String]) -> Option<bool> {
    let mut i = 0;
    while i < argv.len() {
        let a = argv[i].as_str();
        if a == "--" {
            return None;
        }
        if a == "--help" {
            return Some(true);
        }
        if a == "-h" {
            return Some(false);
        }
        // Options that take a separate value: skip the value too.
        if matches!(a, "-C" | "-o" | "--output" | "--developer-dir") {
            i += 2;
            continue;
        }
        if a.starts_with('-') {
            i += 1;
            continue;
        }
        // A bare word — a subcommand name. Its help is clap's to render.
        return None;
    }
    None
}

/// Whether a value clap rejected for `-o/--output` reads as a filesystem path
/// rather than a mistyped format name: it names a directory (`/`), starts at
/// the home directory (`~`), or carries a file extension (`shot.png`,
/// `build.log`).
fn looks_like_path(value: &str) -> bool {
    value.contains('/')
        || value.starts_with('~')
        || std::path::Path::new(value)
            .extension()
            .is_some_and(|e| !e.is_empty())
}

/// Whether the subcommand this command line names declares an `--output-file`
/// argument. The scan descends the clap tree through the bare words in `argv`;
/// a word that names no subcommand at the current level (an option's value, a
/// positional) is skipped, and `--` ends the scan since passthrough tokens
/// belong to the spawned tool.
fn takes_output_file(argv: &[String]) -> bool {
    let mut root = Cli::command();
    root.build();
    let mut cmd = &root;
    for token in argv.iter().take_while(|a| *a != "--") {
        if token.starts_with('-') {
            continue;
        }
        if let Some(sub) = cmd.find_subcommand(token) {
            cmd = sub;
        }
    }
    cmd.get_arguments()
        .any(|a| a.get_long() == Some("output-file"))
}

/// Point a path at `--output-file`, from either way of guessing at it.
///
/// `-o shot.png`: the global `-o/--output` selects the output *format*, so a
/// path lands as an invalid enum value and clap's nearest-value tip reads "a
/// similar value exists: 'json'" — which points away from the flag that
/// actually takes a path.
///
/// `sweetpad app screenshot shot.png`: a command whose whole job is to write
/// one file reads as taking a destination positionally, and clap rejects the
/// bare path with an unadorned "unexpected argument" that names no flag at all.
/// A positional is *not* the fix here — `simulator screenshot` already spends
/// its positional on the target device, so accepting one on the sibling would
/// make the same word mean a file in one command and a simulator in the other.
///
/// Either way the rejected value is a path and the invoked subcommand has an
/// `--output-file`; every other usage error renders as clap wrote it.
fn hint_output_file(mut err: clap::Error, argv: &[String]) -> clap::Error {
    use clap::error::{ContextKind, ContextValue};

    let kind = err.kind();
    let value = match kind {
        clap::error::ErrorKind::InvalidValue => {
            // clap renders the arg as `--output <OUTPUT>`; the leading token
            // is the flag.
            let is_output = matches!(
                err.get(ContextKind::InvalidArg),
                Some(ContextValue::String(arg)) if arg.split_whitespace().next() == Some("--output")
            );
            let Some(ContextValue::String(value)) = err.get(ContextKind::InvalidValue) else {
                return err;
            };
            if !is_output {
                return err;
            }
            value.clone()
        }
        clap::error::ErrorKind::UnknownArgument => {
            let Some(ContextValue::String(value)) = err.get(ContextKind::InvalidArg) else {
                return err;
            };
            value.clone()
        }
        _ => return err,
    };
    if !looks_like_path(&value) || !takes_output_file(argv) {
        return err;
    }
    err.remove(ContextKind::SuggestedValue);
    // Single quotes, matching clap's own messages — a terminal prints
    // backticks literally.
    let fix = if kind == clap::error::ErrorKind::InvalidValue {
        format!(
            "'-o/--output' picks the output format; to write the file, pass \
             '--output-file {value}'"
        )
    } else {
        format!("to write the file, pass '--output-file {value}'")
    };
    err.insert(
        ContextKind::Suggested,
        ContextValue::StyledStrs(vec![fix.into()]),
    );
    err
}

/// Whether stdout should carry color on the pre-`Context` help path, mirroring
/// [`output::Output`]'s decision: `--no-color`/`NO_COLOR` win, then
/// `CLICOLOR_FORCE`/`FORCE_COLOR`, else stdout being a TTY.
fn stdout_wants_color(argv: &[String]) -> bool {
    use std::io::IsTerminal;
    let flagged_off = argv
        .iter()
        .take_while(|a| *a != "--")
        .any(|a| a == "--no-color");
    let env_off = std::env::var_os("NO_COLOR").is_some_and(|v| !v.is_empty());
    if flagged_off || env_off {
        return false;
    }
    let forced = ["CLICOLOR_FORCE", "FORCE_COLOR"]
        .iter()
        .any(|k| std::env::var(k).is_ok_and(|v| output::Output::truthy(&v)));
    forced || std::io::stdout().is_terminal()
}

/// The single success/error render site: human view, the JSON envelope
/// (`json_value` wraps the payload's data in `{schema, ok, data}`), or — under
/// `-o ndjson` — the terminal `{"event":"result","ok":true,"data":…}` line
/// closing the event stream. Also maps errors to exit codes.
/// Mirror [`render_result`]'s error path for failures raised before a
/// `Context` exists (`-C` chdir, flag conflicts): the ndjson stream's
/// documented contract — it ends with a `{"event":"result",…}` line — must
/// hold for these errors too.
fn render_early_error(out: &output::Output, err: &CliError) {
    out.ndjson_event(&serde_json::json!({
        "event": "result",
        "ok": false,
        "error": err.json(),
    }));
    out.error(err);
}

fn render_result(ctx: &Context, result: CommandResult) -> ExitCode {
    match result {
        Ok(Rendered::Data { payload, exit }) => {
            if ctx.out.is_ndjson() {
                ctx.out.ndjson_event(&serde_json::json!({
                    "event": "result",
                    "ok": true,
                    "data": payload.json(),
                }));
            } else if ctx.out.is_json() {
                ctx.out.json_value(&payload.json());
            } else {
                payload.human(&ctx.out);
            }
            ExitCode::from(exit)
        }
        // The command streamed its own output (or self-emitted); nothing to render.
        Ok(Rendered::Streamed) => ExitCode::SUCCESS,
        Err(e) => early_error(&ctx.out, &e),
    }
}

/// Render an error with the full output contract — usable before a `Context`
/// exists (the pre-dispatch gates). Under ndjson the stream must still end
/// with exactly one terminal result line on stdout; the compact envelope on
/// stderr stays the machine-parsed error surface.
fn early_error(out: &output::Output, e: &CliError) -> ExitCode {
    out.ndjson_event(&serde_json::json!({
        "event": "result",
        "ok": false,
        "error": e.json(),
    }));
    out.error(e);
    ExitCode::from(e.error_kind().exit_code())
}

/// A one-time tip after the very first invocation, pointing at the setup
/// commands. A marker file in the state dir suppresses every later showing;
/// interactive-only, so scripts, CI, and `--json` consumers never see it.
fn first_run_hint(out: &output::Output) {
    if !out.is_interactive() || out.is_quiet() {
        return;
    }
    let Some(dir) = sweetpad_core::paths::state_dir().map(|d| d.join("sweetpad")) else {
        return;
    };
    let marker = dir.join("first-run");
    if marker.exists() {
        return;
    }
    let _ = std::fs::create_dir_all(&dir);
    if std::fs::write(&marker, b"shown\n").is_ok() {
        out.note(
            "tip: `sweetpad doctor` checks your toolchain, `sweetpad completions <shell>` \
             sets up tab-completion, and `sweetpad help config` explains configuration \
             (this tip shows once)",
        );
    }
}

/// The class of a failure. Drives both the process exit code and the `--json`
/// error envelope's `code`, from one taxonomy. Exit code 2 is owned by clap
/// (usage errors) and 0 is success, so neither is an `ErrorKind`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorKind {
    Generic,
    BuildFailure,
    TargetResolution,
    ToolMissing,
    UserCancel,
}

impl ErrorKind {
    /// The process exit code for this class (never 0 or 2).
    #[must_use]
    pub fn exit_code(self) -> u8 {
        match self {
            ErrorKind::Generic => 1,
            ErrorKind::BuildFailure => 3,
            ErrorKind::TargetResolution => 4,
            ErrorKind::ToolMissing => 5,
            ErrorKind::UserCancel => 6,
        }
    }

    /// The `error.code` string in the JSON envelope — the same taxonomy as
    /// [`exit_code`](ErrorKind::exit_code).
    #[must_use]
    pub fn code_str(self) -> &'static str {
        match self {
            ErrorKind::Generic => "generic",
            ErrorKind::BuildFailure => "build_failure",
            ErrorKind::TargetResolution => "target_resolution",
            ErrorKind::ToolMissing => "tool_missing",
            ErrorKind::UserCancel => "user_cancel",
        }
    }
}

/// The error type every command returns. Carries an optional operation
/// [`context`](CliError::context) (the bold headline when rendered) separately
/// from the underlying `message` (the dimmed detail) so [`output`] can style
/// them on two lines; [`Display`](std::fmt::Display) flattens them to
/// `context: message` for `--json` and plain logging. A [`kind`](ErrorKind)
/// classifies the failure for the exit code and JSON `code`.
#[derive(Debug)]
pub struct CliError {
    context: Option<String>,
    message: String,
    kind: ErrorKind,
    /// Compiler diagnostics behind this failure, parsed at the chokepoint that
    /// captured the transcript. They ride into the machine-readable error
    /// object so a caller reads `error.diagnostics` instead of scraping a log
    /// out of `error.message`.
    diagnostics: Vec<serde_json::Value>,
}

impl std::fmt::Display for CliError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.context {
            Some(c) => write!(f, "{c}: {}", self.message),
            None => f.write_str(&self.message),
        }
    }
}

impl std::error::Error for CliError {}

impl CliError {
    pub fn new(msg: impl Into<String>) -> Self {
        Self {
            context: None,
            message: msg.into(),
            kind: ErrorKind::Generic,
            diagnostics: Vec::new(),
        }
    }

    /// Tag this error's failure class. Defaults to [`ErrorKind::Generic`]; set it
    /// at the chokepoint where the cause is known (target resolution, a missing
    /// tool, a build failure, a user cancel).
    #[must_use]
    pub fn kind(mut self, kind: ErrorKind) -> Self {
        self.kind = kind;
        self
    }

    /// Set the kind only if it is still the default [`ErrorKind::Generic`], so a
    /// more specific classification from a deeper layer (e.g. a `ToolMissing`
    /// from the process spawn) survives an outer `.or_kind` at a build site.
    #[must_use]
    pub fn or_kind(mut self, kind: ErrorKind) -> Self {
        if self.kind == ErrorKind::Generic {
            self.kind = kind;
        }
        self
    }

    /// This error's failure class — drives the exit code and JSON `code`.
    #[must_use]
    pub fn error_kind(&self) -> ErrorKind {
        self.kind
    }

    /// Prepend operational context so a low-level tool failure says what we were
    /// trying to do. `CliError::new("xcrun simctl install … exited")
    /// .context("installing the app on the simulator")` renders as the headline
    /// `installing the app on the simulator` over the dimmed detail
    /// `xcrun simctl install … exited` — the operation plus the tool that
    /// failed. Re-wrapping folds the previous layers into the detail and
    /// preserves the [`kind`](ErrorKind), so a classified error keeps its exit
    /// code through every `?`-with-context layer.
    #[must_use]
    pub fn context(self, context: impl std::fmt::Display) -> Self {
        Self {
            message: self.to_string(),
            context: Some(context.to_string()),
            kind: self.kind,
            diagnostics: self.diagnostics,
        }
    }

    /// Attach the parsed diagnostics behind this failure. Set at the layer that
    /// captured the transcript, so the error object can carry the compile
    /// errors as data and the message can stay one line.
    #[must_use]
    pub fn diagnostics(mut self, diagnostics: Vec<serde_json::Value>) -> Self {
        self.diagnostics = diagnostics;
        self
    }

    /// The machine-readable error object: the taxonomy code, the flattened
    /// message, and — when the failure carried any — the parsed diagnostics.
    /// The single shape every `--json`/`-o ndjson` error surface renders.
    #[must_use]
    pub fn json(&self) -> serde_json::Value {
        let mut value = serde_json::json!({
            "code": self.kind.code_str(),
            "message": self.to_string(),
        });
        if !self.diagnostics.is_empty()
            && let Some(map) = value.as_object_mut()
        {
            map.insert("diagnostics".into(), self.diagnostics.clone().into());
        }
        value
    }

    /// The operation context — rendered as the bold headline. `None` for a bare
    /// error, where [`detail`](CliError::detail) is the whole message.
    #[must_use]
    pub fn headline(&self) -> Option<&str> {
        self.context.as_deref()
    }

    /// The underlying message — rendered dimmed and indented beneath the
    /// headline (or on its own when there is no context).
    #[must_use]
    pub fn detail(&self) -> &str {
        &self.message
    }
}

/// Attach operational context to a fallible step. Lets call sites read
/// `simctl::install(…).context("installing the app on the simulator")?`, so the
/// surfaced error names both the operation and (via the wrapped message) the
/// tool — instead of a bare `xcrun exited with a non-zero status`.
pub trait ErrorContext<T> {
    /// Wrap any error with a fixed context string.
    ///
    /// # Errors
    /// Returns the wrapped error unchanged on the `Err` path.
    fn context(self, context: impl std::fmt::Display) -> Result<T, CliError>;

    /// Wrap with a context computed lazily — only paid on the error path.
    ///
    /// # Errors
    /// Returns the wrapped error unchanged on the `Err` path.
    fn with_context<C: std::fmt::Display>(self, f: impl FnOnce() -> C) -> Result<T, CliError>;
}

impl<T> ErrorContext<T> for Result<T, CliError> {
    fn context(self, context: impl std::fmt::Display) -> Result<T, CliError> {
        self.map_err(|e| e.context(context))
    }

    fn with_context<C: std::fmt::Display>(self, f: impl FnOnce() -> C) -> Result<T, CliError> {
        self.map_err(|e| e.context(f()))
    }
}

/// Convenience alias for the unit results that helpers and side-effecting steps
/// return — they emit through [`output`] and carry no payload.
pub type CliResult = Result<(), CliError>;

/// What a command's top-level `run` returns: a [`Rendered`] payload the
/// dispatcher renders once (human vs the JSON envelope), or
/// [`Rendered::Streamed`] when the command emitted its own output live.
pub type CommandResult = Result<Rendered, CliError>;

#[cfg(test)]
mod cli_definition_tests {
    /// clap's debug assertions catch arg-id collisions (a per-command arg
    /// shadowing a global one panics at *access* time in production — this
    /// catches it at test time instead).
    #[test]
    fn clap_definition_is_internally_consistent() {
        use clap::CommandFactory;
        super::Cli::command().debug_assert();
    }

    /// Every name the grouped `--help` lists under Everyday/Plumbing must be a
    /// real top-level subcommand (or a declared [`SYNTHETIC`](super::SYNTHETIC)
    /// one) — otherwise a rename would silently drop the command from its group
    /// into the catch-all instead of failing loudly.
    #[test]
    fn help_group_names_are_real_subcommands() {
        use clap::CommandFactory;
        let cmd = super::Cli::command();
        let mut known: std::collections::HashSet<&str> =
            cmd.get_subcommands().map(clap::Command::get_name).collect();
        known.extend(super::SYNTHETIC.iter().map(|(n, _)| *n));
        for &name in super::GROUP_EVERYDAY
            .names
            .iter()
            .chain(super::GROUP_PLUMBING.names)
        {
            assert!(
                known.contains(name),
                "help group lists `{name}`, which is not a top-level subcommand"
            );
        }
    }

    /// The `-- XCODEBUILD_ARGS` tail follows the build: every `app` verb that
    /// spawns xcodebuild takes it, and the verbs that only act on an installed
    /// app refuse it rather than accept args that reach nothing.
    #[test]
    fn the_app_verbs_that_build_take_the_xcodebuild_passthrough() {
        use crate::cli::commands::app;
        use clap::Parser;

        let parse = |verb: &str| {
            super::Cli::try_parse_from(["sweetpad", "app", verb, "--", "-allowProvisioningUpdates"])
        };
        let tail = |verb: &str| match parse(verb).expect("`--` tail rejected").resource {
            Some(super::Resource::App { action }) => match action.expect("no action parsed") {
                app::Action::Install { xcodebuild, .. }
                | app::Action::Debug { xcodebuild, .. }
                | app::Action::Diagnose { xcodebuild, .. } => xcodebuild.passthrough,
                app::Action::Run(args) => args.xcodebuild.passthrough,
                other => panic!("`app {verb}` parsed as {other:?}"),
            },
            other => panic!("`app {verb}` parsed as {other:?}"),
        };

        for verb in ["run", "install", "debug", "diagnose"] {
            assert_eq!(tail(verb), ["-allowProvisioningUpdates"], "app {verb}");
        }
        for verb in ["launch", "uninstall", "stop", "logs"] {
            assert!(parse(verb).is_err(), "app {verb} accepted a `--` tail");
        }
    }
}

#[cfg(test)]
mod targeting_tests {
    use super::disambiguate_container;
    use std::path::PathBuf;

    #[allow(clippy::unnecessary_wraps)] // the Option is the type under test
    fn p(s: &str) -> Option<PathBuf> {
        Some(PathBuf::from(s))
    }

    #[test]
    fn typed_flag_beats_env_sourced_value() {
        // SWEETPAD_WORKSPACE exported, `--project` typed → the env workspace yields.
        assert_eq!(
            disambiguate_container(p("/ws.xcworkspace"), p("/p.xcodeproj"), false, true),
            (None, p("/p.xcodeproj"))
        );
        // And the mirror image.
        assert_eq!(
            disambiguate_container(p("/ws.xcworkspace"), p("/p.xcodeproj"), true, false),
            (p("/ws.xcworkspace"), None)
        );
    }

    #[test]
    fn both_typed_and_both_env_are_kept() {
        // `--workspace … --project …` is meaningful (member selection).
        assert_eq!(
            disambiguate_container(p("/ws.xcworkspace"), p("/p.xcodeproj"), true, true),
            (p("/ws.xcworkspace"), p("/p.xcodeproj"))
        );
        // Two exported env vars keep the documented workspace-first order.
        assert_eq!(
            disambiguate_container(p("/ws.xcworkspace"), p("/p.xcodeproj"), false, false),
            (p("/ws.xcworkspace"), p("/p.xcodeproj"))
        );
    }

    #[test]
    fn single_values_pass_through() {
        assert_eq!(
            disambiguate_container(None, p("/p.xcodeproj"), false, false),
            (None, p("/p.xcodeproj"))
        );
        assert_eq!(
            disambiguate_container(None, None, false, false),
            (None, None)
        );
    }
}

#[cfg(test)]
mod fast_path_tests {
    use super::{OutputMode, machine_output_mode};

    #[test]
    fn json_flag_selects_the_envelope() {
        assert_eq!(
            machine_output_mode(&["--version", "--json"]),
            Some(OutputMode::Json)
        );
        assert_eq!(machine_output_mode(&["--version"]), None);
    }

    #[test]
    fn output_flag_wins_over_json_in_both_directions() {
        // `-o` selects a machine mode --json alone wouldn't…
        assert_eq!(
            machine_output_mode(&["-o", "json", "--version"]),
            Some(OutputMode::Json)
        );
        assert_eq!(
            machine_output_mode(&["--output=ndjson"]),
            Some(OutputMode::Ndjson)
        );
        assert_eq!(machine_output_mode(&["-ojson"]), Some(OutputMode::Json));
        // …and `-o human` disables a --json (the flag's documented priority).
        assert_eq!(machine_output_mode(&["--json", "-o", "human"]), None);
        assert_eq!(machine_output_mode(&["--json", "-o", "quiet"]), None);
    }
}

#[cfg(test)]
mod on_destination_tests {
    use super::disambiguate_on_destination;

    #[allow(clippy::unnecessary_wraps)] // the Option is the type under test
    fn s(v: &str) -> Option<String> {
        Some(v.to_string())
    }

    #[test]
    fn typed_flag_beats_env_sourced_value() {
        // SWEETPAD_DESTINATION exported, `--on` typed → the env value yields
        // (previously a hard clap conflict).
        assert_eq!(
            disambiguate_on_destination(s("mac"), s("platform=iOS"), true, false),
            (s("mac"), None)
        );
        assert_eq!(
            disambiguate_on_destination(s("mac"), s("platform=iOS"), false, true),
            (None, s("platform=iOS"))
        );
    }

    #[test]
    fn both_typed_is_kept_for_the_resolver_to_reject() {
        assert_eq!(
            disambiguate_on_destination(s("mac"), s("platform=iOS"), true, true),
            (s("mac"), s("platform=iOS"))
        );
    }

    #[test]
    fn both_env_resolves_on_wins_per_the_documented_contract() {
        // `help environment`: SWEETPAD_ON overrides SWEETPAD_DESTINATION.
        assert_eq!(
            disambiguate_on_destination(s("mac"), s("platform=iOS"), false, false),
            (s("mac"), None)
        );
    }
}

#[cfg(test)]
mod output_file_hint_tests {
    use super::{Cli, hint_output_file, looks_like_path, takes_output_file};
    use clap::Parser;

    fn argv(args: &[&str]) -> Vec<String> {
        args.iter().map(|s| (*s).to_string()).collect()
    }

    /// The rendered error for a command line clap rejects, after the hint pass.
    fn rendered(args: &[&str]) -> String {
        let tokens = argv(args);
        let err = Cli::try_parse_from(
            std::iter::once("sweetpad".to_string()).chain(tokens.iter().cloned()),
        )
        .expect_err("expected a usage error");
        hint_output_file(err, &tokens).render().to_string()
    }

    #[test]
    fn a_value_with_a_separator_extension_or_home_reference_reads_as_a_path() {
        assert!(looks_like_path("shot.png"));
        assert!(looks_like_path("out/shot.png"));
        assert!(looks_like_path("/tmp/build.log"));
        assert!(looks_like_path("~/shot.png"));
        assert!(looks_like_path("./out"));
    }

    #[test]
    fn a_mistyped_format_name_does_not_read_as_a_path() {
        assert!(!looks_like_path("json"));
        assert!(!looks_like_path("ndjson"));
        assert!(!looks_like_path("jsonn"));
        assert!(!looks_like_path("quiet"));
        // A trailing dot leaves an empty extension, which is not a filename.
        assert!(!looks_like_path("json."));
    }

    #[test]
    fn only_the_commands_that_write_a_file_advertise_output_file() {
        assert!(takes_output_file(&argv(&["app", "screenshot"])));
        assert!(takes_output_file(&argv(&["simulator", "screenshot"])));
        assert!(takes_output_file(&argv(&["bsp", "init"])));
        assert!(takes_output_file(&argv(&["archive"])));
        assert!(!takes_output_file(&argv(&["build"])));
        assert!(!takes_output_file(&argv(&["devices"])));
        assert!(!takes_output_file(&argv(&[])));
    }

    #[test]
    fn the_scan_skips_option_values_and_stops_at_the_passthrough() {
        // `shot.png` is `-o`'s value, not a subcommand name.
        assert!(takes_output_file(&argv(&[
            "app",
            "screenshot",
            "-o",
            "shot.png"
        ])));
        // A passthrough token that happens to name a command must not retarget
        // the scan away from the real subcommand.
        assert!(takes_output_file(&argv(&[
            "app",
            "screenshot",
            "--",
            "build"
        ])));
    }

    #[test]
    fn a_path_given_to_output_is_pointed_at_output_file() {
        let text = rendered(&["app", "screenshot", "-o", "shot.png"]);
        assert!(
            text.contains("--output-file shot.png"),
            "expected the --output-file tip, got:\n{text}"
        );
        // clap's nearest-enum-value tip points away from the fix, so it goes.
        assert!(
            !text.contains("similar value exists"),
            "the misleading value tip survived:\n{text}"
        );
    }

    #[test]
    fn a_mistyped_format_keeps_claps_own_value_suggestion() {
        let text = rendered(&["app", "screenshot", "-o", "jsonn"]);
        assert!(
            text.contains("similar value exists"),
            "expected clap's value tip, got:\n{text}"
        );
        assert!(!text.contains("--output-file"));
    }

    #[test]
    fn a_path_given_to_a_command_without_output_file_keeps_claps_error() {
        // `build` writes no file, so there is nothing better to point at.
        let text = rendered(&["build", "-o", "shot.png"]);
        assert!(!text.contains("--output-file"), "unexpected tip:\n{text}");
    }

    #[test]
    fn unrelated_usage_errors_are_left_alone() {
        let text = rendered(&["nosuchcommand"]);
        assert!(!text.contains("--output-file"), "unexpected tip:\n{text}");
    }

    #[test]
    fn a_bare_path_is_pointed_at_output_file() {
        // A command whose whole job is writing one file reads as taking the
        // destination positionally; clap's bare "unexpected argument" names
        // no flag, so the guess costs a --help round-trip.
        for spelling in ["/tmp/launch.png", "shot.png", "~/out/shot.png"] {
            let text = rendered(&["app", "screenshot", spelling]);
            assert!(
                text.contains(&format!("--output-file {spelling}")),
                "expected the tip for {spelling}, got:\n{text}"
            );
        }
    }

    #[test]
    fn an_unexpected_argument_that_is_not_a_path_is_left_alone() {
        // A misspelled flag and a stray word are ordinary usage errors; only a
        // value that reads as a path has a better answer.
        for spelling in ["--bogus", "notapath"] {
            let text = rendered(&["app", "screenshot", spelling]);
            assert!(
                !text.contains("--output-file"),
                "unexpected tip for {spelling}:\n{text}"
            );
        }
        // And a command that writes no file has nothing to point at.
        let text = rendered(&["build", "shot.png"]);
        assert!(!text.contains("--output-file"), "unexpected tip:\n{text}");
    }

    #[test]
    fn the_tips_quote_the_way_a_terminal_reads() {
        // Backticks render literally in a terminal; clap's own messages use
        // single quotes, and these sit right beside them.
        for args in [
            vec!["app", "screenshot", "/tmp/launch.png"],
            vec!["app", "screenshot", "-o", "/tmp/launch.png"],
        ] {
            let text = rendered(&args);
            let tip = text
                .lines()
                .find(|l| l.contains("--output-file"))
                .unwrap_or_default();
            assert!(!tip.contains('`'), "backtick in a rendered tip: {tip}");
        }
    }
}

#[cfg(test)]
mod error_tests {
    use super::{CliError, ErrorContext, ErrorKind};

    #[test]
    fn default_kind_is_generic() {
        assert_eq!(CliError::new("boom").error_kind(), ErrorKind::Generic);
        assert_eq!(ErrorKind::Generic.exit_code(), 1);
    }

    #[test]
    fn context_preserves_the_error_kind() {
        // Nearly every surfaced error is `.context`-wrapped through `?`; the
        // classification (and thus the exit code) must survive every layer.
        let e = CliError::new("`xcrun` not found on PATH")
            .kind(ErrorKind::ToolMissing)
            .context("installing the app on the simulator")
            .context("running the app");
        assert_eq!(e.error_kind(), ErrorKind::ToolMissing);
        assert_eq!(e.error_kind().exit_code(), 5);
        assert_eq!(e.error_kind().code_str(), "tool_missing");
    }

    #[test]
    fn diagnostics_ride_into_the_error_object_and_survive_context() {
        let diag = serde_json::json!({
            "event": "diagnostic",
            "severity": "error",
            "location": "App/Model.swift:12:5",
            "message": "cannot find 'foo' in scope",
        });
        let e = CliError::new("xcodebuild test failed before any test ran")
            .kind(ErrorKind::BuildFailure)
            .diagnostics(vec![diag.clone()])
            // The wrap every command applies on the way out must not drop them.
            .context("running the tests");
        let json = e.json();
        assert_eq!(json["code"], "build_failure");
        assert_eq!(json["diagnostics"], serde_json::json!([diag]));
    }

    #[test]
    fn an_error_without_diagnostics_has_no_diagnostics_key() {
        // The key is absent rather than an empty array, so a consumer testing
        // for it isn't told "we parsed the log and found nothing".
        let json = CliError::new("boom").json();
        assert!(json.get("diagnostics").is_none());
        assert_eq!(json["message"], "boom");
    }

    #[test]
    fn context_splits_into_headline_and_detail() {
        let e = CliError::new("xcrun simctl install A B exited with a non-zero status")
            .context("installing the app on the simulator");
        assert_eq!(e.headline(), Some("installing the app on the simulator"));
        assert_eq!(
            e.detail(),
            "xcrun simctl install A B exited with a non-zero status"
        );
        // Flattened form (used for `--json` and logging) keeps `context: detail`.
        assert_eq!(
            e.to_string(),
            "installing the app on the simulator: xcrun simctl install A B exited with a non-zero status"
        );
    }

    #[test]
    fn bare_error_has_no_headline() {
        let e = CliError::new("no .xcodeproj found");
        assert_eq!(e.headline(), None);
        assert_eq!(e.detail(), "no .xcodeproj found");
        assert_eq!(e.to_string(), "no .xcodeproj found");
    }

    #[test]
    fn re_wrapping_folds_prior_layers_into_the_detail() {
        let e = CliError::new("xcrun … exited")
            .context("installing the app on the simulator")
            .context("running the app");
        assert_eq!(e.headline(), Some("running the app"));
        assert_eq!(
            e.detail(),
            "installing the app on the simulator: xcrun … exited"
        );
    }

    #[test]
    fn result_context_extension_wraps_the_error() {
        let r: Result<(), CliError> = Err(CliError::new("boom"));
        let wrapped = r.context("doing the thing").unwrap_err();
        assert_eq!(wrapped.headline(), Some("doing the thing"));
        assert_eq!(wrapped.detail(), "boom");
    }
}
