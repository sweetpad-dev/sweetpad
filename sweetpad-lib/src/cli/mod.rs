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

use std::process::ExitCode;

use clap::{CommandFactory, Parser, Subcommand};

pub mod buildlog;
pub mod config;
pub mod devicectl;
pub mod inject;
pub mod merge;
pub mod output;
pub mod process;
pub mod rawmode;
pub mod resolve;
pub mod scaffold;
pub mod simctl;
pub mod state;
pub mod swiftpm;
pub mod xcodebuild;

pub mod commands;

/// Top-level CLI definition. Note this parses the *non-`vscode`* argument
/// vector: the binary peels off the `vscode` subcommand before we get here, so
/// clap owns the rest of the resource-first tree.
#[derive(Debug, Parser)]
#[command(
    name = "sweetpad",
    about = "Build, run, and explore Xcode projects from the terminal",
    long_about = "sweetpad — xcodebuild for humans.\n\nA standalone, headless \
        CLI for Xcode projects. Use `sweetpad vscode` to control the VS Code \
        extension instead.",
    disable_help_subcommand = true
)]
pub struct Cli {
    #[command(flatten)]
    pub global: GlobalArgs,

    #[command(subcommand)]
    pub resource: Resource,
}

/// The truly universal flags — accepted on every command and propagated to
/// nested actions. Targeting (which workspace/scheme/… a command acts on) is
/// *not* here: those flags live on the commands that consume them, as the
/// [`ContainerArgs`]/[`SchemeArgs`]/[`BuildTargetArgs`] tiers below.
#[derive(Debug, clap::Args)]
pub struct GlobalArgs {
    /// Emit machine-readable JSON instead of human output.
    #[arg(long, global = true)]
    pub json: bool,

    /// Disable colored output (also honored via the `NO_COLOR` env var and
    /// when stdout is not a TTY).
    #[arg(long, global = true)]
    pub no_color: bool,

    /// Increase log verbosity (repeatable: -v, -vv).
    #[arg(short, long, global = true, action = clap::ArgAction::Count)]
    pub verbose: u8,
}

/// Tier 1 — which project container to act on. Flattened into every command
/// that locates a workspace/project. Not `global`, so it must precede the
/// action token (`sweetpad project --project App.xcodeproj info`).
#[derive(Debug, clap::Args)]
pub struct ContainerArgs {
    /// Path to the `.xcworkspace` to operate on (overrides auto-discovery).
    #[arg(long, env = "SWEETPAD_WORKSPACE")]
    pub workspace: Option<std::path::PathBuf>,

    /// Path to the `.xcodeproj` to operate on (overrides auto-discovery).
    #[arg(long, env = "SWEETPAD_PROJECT")]
    pub project: Option<std::path::PathBuf>,
}

/// Tier 2 — container plus a scheme. For commands that need to know *which*
/// scheme but not a full build target (e.g. `scheme list`).
#[derive(Debug, clap::Args)]
pub struct SchemeArgs {
    #[command(flatten)]
    pub container: ContainerArgs,

    /// Scheme to use (overrides config and remembered selection).
    #[arg(long, env = "SWEETPAD_SCHEME")]
    pub scheme: Option<String>,
}

/// Tier 3 — everything `xcodebuild` needs: container, scheme, configuration,
/// and destination. For the build-ish commands (`build`, `test`, `settings`,
/// `app`).
#[derive(Debug, clap::Args)]
pub struct BuildTargetArgs {
    #[command(flatten)]
    pub scheme: SchemeArgs,

    /// Build configuration to use (e.g. Debug, Release).
    #[arg(long, env = "SWEETPAD_CONFIGURATION")]
    pub configuration: Option<String>,

    /// Destination specifier (e.g. "platform=iOS Simulator,name=iPhone 15").
    #[arg(long, env = "SWEETPAD_DESTINATION")]
    pub destination: Option<String>,
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
}

impl From<ContainerArgs> for Targeting {
    fn from(a: ContainerArgs) -> Self {
        Self {
            workspace: a.workspace,
            project: a.project,
            scheme: None,
            configuration: None,
            destination: None,
        }
    }
}

impl From<SchemeArgs> for Targeting {
    fn from(a: SchemeArgs) -> Self {
        Self {
            scheme: a.scheme,
            ..a.container.into()
        }
    }
}

impl From<BuildTargetArgs> for Targeting {
    fn from(a: BuildTargetArgs) -> Self {
        Self {
            configuration: a.configuration,
            destination: a.destination,
            ..a.scheme.into()
        }
    }
}

/// Top-level resources. Each is a noun; actions are its subcommands.
#[derive(Debug, Subcommand)]
pub enum Resource {
    /// Inspect schemes.
    Scheme {
        #[command(flatten)]
        target: SchemeArgs,
        #[command(subcommand)]
        action: commands::scheme::Action,
    },
    /// Inspect build destinations.
    Destination {
        #[command(subcommand)]
        action: commands::destination::Action,
    },
    /// Inspect the project: targets, configurations, schemes.
    Project {
        #[command(flatten)]
        target: ContainerArgs,
        #[command(subcommand)]
        action: commands::project::Action,
    },
    /// Show resolved build settings.
    Settings {
        #[command(flatten)]
        target: BuildTargetArgs,
        #[command(subcommand)]
        action: commands::settings::Action,
    },
    /// Manage iOS simulators.
    Simulator {
        #[command(subcommand)]
        action: commands::simulator::Action,
    },
    /// Compile the project.
    Build {
        #[command(flatten)]
        target: BuildTargetArgs,
        #[command(subcommand)]
        action: commands::build::Action,
    },
    /// Run the project's tests.
    Test {
        #[command(flatten)]
        target: BuildTargetArgs,
        #[command(subcommand)]
        action: commands::test::Action,
    },
    /// Run, install, and manage the built app's lifecycle.
    App {
        #[command(flatten)]
        target: BuildTargetArgs,
        #[command(subcommand)]
        action: commands::app::Action,
    },
    /// Inspect connected physical devices.
    Device {
        #[command(subcommand)]
        action: commands::device::Action,
    },
    /// Format or lint Swift sources.
    Format {
        #[command(subcommand)]
        action: commands::format::Action,
    },
    /// Work with `project.pbxproj` files (semantic git-conflict merge).
    Pbxproj {
        #[command(subcommand)]
        action: commands::pbxproj::Action,
    },
    /// Work with SwiftPM `Package.resolved` files (semantic git-conflict merge).
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
    DerivedData {
        #[command(flatten)]
        target: ContainerArgs,
        #[command(subcommand)]
        action: commands::derived_data::Action,
    },
    /// Diagnose the local Xcode/Swift toolchain.
    Doctor,
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
}

/// Entry point for the CLI half of the binary. `argv` is the full process
/// argument vector minus `argv[0]` (clap re-prepends the program name).
#[must_use]
pub fn run(argv: &[String]) -> ExitCode {
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
    let config = match config::Config::load() {
        Ok(c) => c,
        Err(e) => {
            out.error(&format!("failed to load config: {e}"));
            return ExitCode::FAILURE;
        }
    };
    let state = state::State::load().unwrap_or_default();

    // Completions need nothing from config/state — emit and return.
    if let Resource::Completions { shell } = &cli.resource {
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
    };

    let result = match cli.resource {
        Resource::Scheme { target, action } => {
            ctx.targeting = target.into();
            commands::scheme::run(&mut ctx, &action)
        }
        Resource::Destination { action } => commands::destination::run(&mut ctx, &action),
        Resource::Project { target, action } => {
            ctx.targeting = target.into();
            commands::project::run(&mut ctx, &action)
        }
        Resource::Settings { target, action } => {
            ctx.targeting = target.into();
            commands::settings::run(&mut ctx, &action)
        }
        Resource::Simulator { action } => commands::simulator::run(&mut ctx, &action),
        Resource::Build { target, action } => {
            ctx.targeting = target.into();
            commands::build::run(&mut ctx, &action)
        }
        Resource::Test { target, action } => {
            ctx.targeting = target.into();
            commands::test::run(&mut ctx, &action)
        }
        Resource::App { target, action } => {
            ctx.targeting = target.into();
            commands::app::run(&mut ctx, &action)
        }
        Resource::Device { action } => commands::device::run(&mut ctx, &action),
        Resource::Format { action } => commands::format::run(&mut ctx, &action),
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
        Resource::Doctor => commands::doctor::run(&mut ctx),
        Resource::Completions { .. } => unreachable!("handled above"),
    };

    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            ctx.out.error(&e.to_string());
            ExitCode::FAILURE
        }
    }
}

/// The error type every command returns. A thin string-backed wrapper for now;
/// can grow structured variants (for richer `--json` error objects) as
/// commands are implemented.
#[derive(Debug)]
pub struct CliError(pub String);

impl std::fmt::Display for CliError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for CliError {}

impl CliError {
    pub fn new(msg: impl Into<String>) -> Self {
        Self(msg.into())
    }
}

/// Convenience alias for command results.
pub type CliResult = Result<(), CliError>;
