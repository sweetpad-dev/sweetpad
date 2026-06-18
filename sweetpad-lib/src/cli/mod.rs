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

pub mod backend;
pub mod buildlog;
pub mod config;
pub mod devicectl;
pub mod inject;
pub mod merge;
pub mod output;
pub mod plan;
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

/// Flags accepted on every command. Resolution flags follow the layered
/// precedence documented in [`resolve`] (flag > env > config > auto-discovery).
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

    /// Path to the `.xcworkspace` to operate on (overrides auto-discovery).
    #[arg(long, global = true, env = "SWEETPAD_WORKSPACE")]
    pub workspace: Option<std::path::PathBuf>,

    /// Path to the `.xcodeproj` to operate on (overrides auto-discovery).
    #[arg(long, global = true, env = "SWEETPAD_PROJECT")]
    pub project: Option<std::path::PathBuf>,

    /// Scheme to use (overrides config and remembered selection).
    #[arg(long, global = true, env = "SWEETPAD_SCHEME")]
    pub scheme: Option<String>,

    /// Build configuration to use (e.g. Debug, Release).
    #[arg(long, global = true, env = "SWEETPAD_CONFIGURATION")]
    pub configuration: Option<String>,

    /// Destination specifier (e.g. "platform=iOS Simulator,name=iPhone 15").
    #[arg(long, global = true, env = "SWEETPAD_DESTINATION")]
    pub destination: Option<String>,

    /// Build backend to use (e.g. "xcodebuild", "swiftpm"). Overrides config;
    /// defaults to auto-selection by project type.
    #[arg(long, global = true, env = "SWEETPAD_BACKEND")]
    pub backend: Option<String>,
}

/// Top-level resources. Each is a noun; actions are its subcommands.
#[derive(Debug, Subcommand)]
pub enum Resource {
    /// Inspect schemes.
    Scheme {
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
        #[command(subcommand)]
        action: commands::project::Action,
    },
    /// Show resolved build settings.
    Settings {
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
        #[command(subcommand)]
        action: commands::build::Action,
    },
    /// Run the project's tests.
    Test {
        #[command(subcommand)]
        action: commands::test::Action,
    },
    /// Run, install, and manage the built app's lifecycle.
    App {
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
        #[command(subcommand)]
        action: commands::bsp::Action,
    },
    /// Inspect and purge Xcode's DerivedData.
    DerivedData {
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
        config,
        state,
        out,
    };

    let result = match cli.resource {
        Resource::Scheme { action } => commands::scheme::run(&mut ctx, &action),
        Resource::Destination { action } => commands::destination::run(&mut ctx, &action),
        Resource::Project { action } => commands::project::run(&mut ctx, &action),
        Resource::Settings { action } => commands::settings::run(&mut ctx, &action),
        Resource::Simulator { action } => commands::simulator::run(&mut ctx, &action),
        Resource::Build { action } => commands::build::run(&mut ctx, &action),
        Resource::Test { action } => commands::test::run(&mut ctx, &action),
        Resource::App { action } => commands::app::run(&mut ctx, &action),
        Resource::Device { action } => commands::device::run(&mut ctx, &action),
        Resource::Format { action } => commands::format::run(&mut ctx, &action),
        Resource::Pbxproj { action } => commands::pbxproj::run(&mut ctx, &action),
        Resource::Spm { action } => commands::spm::run(&mut ctx, &action),
        Resource::Merge { action } => commands::merge::run(&mut ctx, &action),
        Resource::Bsp { action } => commands::bsp::run(&mut ctx, &action),
        Resource::DerivedData { action } => commands::derived_data::run(&mut ctx, &action),
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
