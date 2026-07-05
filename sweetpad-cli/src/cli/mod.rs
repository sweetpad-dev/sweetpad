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
    version,
    about = "Build, run, and explore Xcode projects from the terminal",
    long_about = "sweetpad — xcodebuild for humans.\n\nA standalone, headless \
        CLI for Xcode projects. Use `sweetpad vscode` to control the VS Code \
        extension instead.",
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
    /// Run as if started in DIR (chdir before anything else), like `git -C`.
    #[arg(short = 'C', value_name = "DIR", global = true)]
    pub chdir: Option<std::path::PathBuf>,

    /// Xcode to use: sets DEVELOPER_DIR for every spawned tool (e.g.
    /// /Applications/Xcode-16.4.app/Contents/Developer). A project can pin one
    /// via `developer_dir` in sweetpad.toml.
    #[arg(long, value_name = "DIR", global = true, env = "DEVELOPER_DIR")]
    pub developer_dir: Option<std::path::PathBuf>,

    /// Output format. `json` is the one-shot envelope; `ndjson` streams one
    /// JSON event per line from the long-running verbs (build, test, logs) and
    /// ends with a `{"event":"result", …}` line. Wins over `--json`.
    #[arg(short = 'o', long = "output", global = true, value_enum)]
    pub output: Option<OutputMode>,

    /// Emit machine-readable JSON instead of human output (alias for
    /// `-o json`).
    #[arg(long, global = true)]
    pub json: bool,

    /// Assume no interactive terminal: never prompt or animate a spinner, turn a
    /// missing scheme/destination into an error instead of a picker, and run
    /// `app run` as a plain follow rather than the rebuild session. Also honored
    /// via the `SWEETPAD_NONINTERACTIVE` env var.
    #[arg(long, global = true)]
    pub non_interactive: bool,

    /// Disable colored output (also honored via the `NO_COLOR` env var and when
    /// stdout is not a TTY). `CLICOLOR_FORCE`/`FORCE_COLOR` force color back on
    /// when piped; an explicit `--no-color`/`NO_COLOR` still wins.
    #[arg(long, global = true)]
    pub no_color: bool,

    /// Print verbose diagnostics (raw tool output, extra detail).
    #[arg(short, long, global = true)]
    pub verbose: bool,

    /// Suppress progress chatter (notes, spinners, step labels). Errors and
    /// primary data/JSON are still emitted; wins over `--verbose`.
    #[arg(short, long, global = true)]
    pub quiet: bool,

    /// Emit GitHub Actions annotations (`::error file=…::…`) for build/test
    /// diagnostics, so failures surface inline on the PR.
    #[arg(long, global = true)]
    pub gh_annotations: bool,
}

/// The output axis (`-o`): how results reach stdout.
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum OutputMode {
    /// Colored human output (the default).
    Human,
    /// The `{schema, ok, data}` envelope, one JSON document per command.
    Json,
    /// Streaming events, one compact JSON object per line; the final line is
    /// `{"event":"result","ok":…,"data":…}`. Non-streaming commands emit just
    /// that final line.
    Ndjson,
    /// Human output with progress chatter muted (same as `--quiet`).
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
    /// Path to the `.xcworkspace` to operate on (overrides auto-discovery).
    #[arg(long, env = "SWEETPAD_WORKSPACE", global = true)]
    pub workspace: Option<std::path::PathBuf>,

    /// Path to the `.xcodeproj` to operate on (overrides auto-discovery).
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
    /// ("iPhone 16 Pro"), `booted`, `mac`, `device`, a platform word (`ios`,
    /// `watchos`, …), or a UDID. Resolved against the live device list;
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
    /// The env-var layer alone. The bare `sweetpad` status view parses no
    /// resource, so clap never folds `SWEETPAD_*` into flags there — without
    /// this the bare view would show a context the very next `build` ignores.
    fn from_env() -> Self {
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
/// exactly one was typed, the env-sourced one yields. Both-typed (and
/// both-env) keep both, and the resolver errors when it has to choose.
fn disambiguate_on_destination(
    on: Option<String>,
    destination: Option<String>,
    on_typed: bool,
    destination_typed: bool,
) -> (Option<String>, Option<String>) {
    match (on.is_some(), destination.is_some()) {
        (true, true) if on_typed && !destination_typed => (on, None),
        (true, true) if destination_typed && !on_typed => (None, destination),
        _ => (on, destination),
    }
}

/// Top-level resources. Each is a noun; actions are its subcommands.
#[derive(Debug, Subcommand)]
pub enum Resource {
    /// Inspect schemes.
    Scheme {
        #[command(subcommand)]
        action: commands::scheme::Action,
    },
    /// Inspect build destinations (hidden alias — see `devices`).
    #[command(hide = true)]
    Destination {
        #[command(subcommand)]
        action: commands::destination::Action,
    },
    /// Everything runnable — macOS, simulators, connected devices — each with
    /// its ready `-destination` specifier, most-used first, the remembered
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
        #[command(flatten)]
        target: BuildTargetArgs,
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
    /// `app run`).
    Run(commands::app::RunArgs),
    /// Compile the project (`build` alone runs `build start`).
    Build {
        #[command(flatten)]
        args: commands::build::StartArgs,
        #[command(subcommand)]
        action: Option<commands::build::Action>,
    },
    /// Run the project's tests (`test` alone runs `test run`).
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
    /// Run, install, and manage the built app's lifecycle (`app` alone runs
    /// `app run`).
    App {
        #[command(subcommand)]
        action: Option<commands::app::Action>,
    },
    /// Inspect connected physical devices (hidden alias — see `devices`).
    #[command(hide = true)]
    Device {
        #[command(subcommand)]
        action: commands::device::Action,
    },
    /// Format or lint Swift sources (`format` alone runs `format run`).
    #[command(visible_alias = "fmt")]
    Format {
        #[command(flatten)]
        args: commands::format::FormatArgs,
        #[command(subcommand)]
        action: Option<commands::format::Action>,
    },
    /// Work with `project.pbxproj` files (hidden alias — see `merge run`).
    #[command(hide = true)]
    Pbxproj {
        #[command(subcommand)]
        action: commands::pbxproj::Action,
    },
    /// Work with SwiftPM `Package.resolved` files (hidden alias — see
    /// `merge run`).
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
    /// Diagnose the local Xcode/Swift toolchain.
    Doctor,
    /// Show the effective build context — what would build, and where each
    /// value comes from (flag/env/config/remembered/default).
    Status {
        #[command(flatten)]
        target: BuildTargetArgs,
    },
    /// Update sweetpad (Homebrew installs run `brew upgrade sweetpad`).
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
}

impl Context {
    /// The committed `sweetpad.toml` next to `container` — the team-shared
    /// defaults layer between the user config and remembered state. Loaded
    /// once per process; lint warnings surface on that first load, and a
    /// pinned `developer_dir` takes effect (unless a flag/env already set it).
    pub fn project_file(&self, container: &resolve::Container) -> &config::ProjectFile {
        self.project_toml.get_or_init(|| {
            let dir = container
                .path()
                .parent()
                .filter(|p| !p.as_os_str().is_empty())
                .map_or_else(|| std::path::PathBuf::from("."), std::path::Path::to_path_buf);
            let (pf, warnings) = config::ProjectFile::load_for(&dir);
            for w in &warnings {
                self.out.warn(w);
            }
            if let Some(dev_dir) = &pf.developer_dir
                && std::env::var_os("DEVELOPER_DIR").is_none()
            {
                // Safety: first project-file access happens on the main thread
                // before tool children spawn.
                unsafe { std::env::set_var("DEVELOPER_DIR", dev_dir) };
            }
            pf
        })
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

    // `--version` under a machine output mode: clap's own --version is plain
    // text; agents get the envelope (or, under ndjson, the terminal result
    // event). Only tokens before `--` count — passthrough belongs to the
    // spawned tool — and `-o` wins over `--json` per that flag's contract.
    let head: Vec<&str> = argv
        .iter()
        .take_while(|a| *a != "--")
        .map(String::as_str)
        .collect();
    if head.iter().any(|a| *a == "--version" || *a == "-V")
        && let Some(mode) = machine_output_mode(&head)
    {
        #[allow(clippy::print_stdout)] // pre-clap fast path; Output isn't built yet
        {
            let version = env!("CARGO_PKG_VERSION");
            match mode {
                OutputMode::Ndjson => println!(
                    r#"{{"event":"result","ok":true,"data":{{"version":"{version}"}}}}"#
                ),
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
            // clap renders help/usage/errors and picks the right stream.
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
        let _ = Cli::command().print_help();
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
        Resource::Settings { target, action } => {
            ctx.targeting = target.into();
            commands::settings::run(&mut ctx, &action)
        }
        Resource::Simulator { action } => commands::simulator::run(&mut ctx, &action),
        // `build`/`test` carry their flags as resource-level globals; the bare
        // `start`/`run` tokens are optional markers, so both spellings land here.
        Resource::Build { args, action } => commands::build::run(&mut ctx, &args, action.as_ref()),
        Resource::Test { args, action: _ } => commands::test::run(&mut ctx, &args),
        Resource::Archive(archive_args) => commands::archive::run(&mut ctx, &archive_args),
        Resource::Clean(clean_args) => {
            let mut targeting: Targeting = clean_args.scheme.clone().into();
            targeting.configuration.clone_from(&clean_args.configuration);
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
        "error": {
            "code": err.error_kind().code_str(),
            "message": err.to_string(),
        },
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
        Err(e) => {
            // Under ndjson the stream must still end with exactly one result
            // line on stdout; the compact envelope on stderr stays the
            // machine-parsed error surface.
            ctx.out.ndjson_event(&serde_json::json!({
                "event": "result",
                "ok": false,
                "error": {
                    "code": e.error_kind().code_str(),
                    "message": e.to_string(),
                },
            }));
            ctx.out.error(&e);
            ExitCode::from(e.error_kind().exit_code())
        }
    }
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
        }
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
        assert_eq!(disambiguate_container(None, None, false, false), (None, None));
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
    fn both_typed_and_both_env_are_kept_for_the_resolver_to_reject() {
        assert_eq!(
            disambiguate_on_destination(s("mac"), s("platform=iOS"), true, true),
            (s("mac"), s("platform=iOS"))
        );
        assert_eq!(
            disambiguate_on_destination(s("mac"), s("platform=iOS"), false, false),
            (s("mac"), s("platform=iOS"))
        );
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
