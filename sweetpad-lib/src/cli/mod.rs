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
pub mod pymobiledevice3;
pub mod progress;
pub mod rawmode;
pub mod render;
pub mod resolve;
pub mod scaffold;
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
#[allow(clippy::struct_excessive_bools)] // independent CLI toggles, not a state machine
pub struct GlobalArgs {
    /// Emit machine-readable JSON instead of human output.
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
}

/// Tier 1 — which project container to act on. Flattened into every command
/// that locates a workspace/project. The flags are `global` *within* the
/// resource they're flattened into, so they parse on either side of the action
/// token (`sweetpad project --project App.xcodeproj info` and
/// `sweetpad project info --project App.xcodeproj` both work), while staying
/// scoped to the resources that actually consume them — a resource that doesn't
/// flatten this tier never advertises `--project`/`--workspace`.
#[derive(Debug, clap::Args)]
pub struct ContainerArgs {
    /// Path to the `.xcworkspace` to operate on (overrides auto-discovery).
    #[arg(long, env = "SWEETPAD_WORKSPACE", global = true)]
    pub workspace: Option<std::path::PathBuf>,

    /// Path to the `.xcodeproj` to operate on (overrides auto-discovery).
    #[arg(long, env = "SWEETPAD_PROJECT", global = true)]
    pub project: Option<std::path::PathBuf>,
}

/// Tier 2 — container plus a scheme. For commands that need to know *which*
/// scheme but not a full build target (e.g. `scheme list`).
#[derive(Debug, clap::Args)]
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
#[derive(Debug, clap::Args)]
pub struct BuildTargetArgs {
    #[command(flatten)]
    pub scheme: SchemeArgs,

    /// Build configuration to use (e.g. Debug, Release).
    #[arg(long, env = "SWEETPAD_CONFIGURATION", global = true)]
    pub configuration: Option<String>,

    /// Destination specifier (e.g. "platform=iOS Simulator,name=iPhone 15").
    #[arg(long, env = "SWEETPAD_DESTINATION", global = true)]
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
    /// Show, select, or clear the project's remembered build context.
    Context {
        #[command(flatten)]
        target: ContainerArgs,
        #[command(subcommand)]
        action: commands::context::Action,
    },
    /// Inspect the project: targets, configurations, schemes.
    Project {
        #[command(flatten)]
        target: ContainerArgs,
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
            out.error(&CliError::new(format!("failed to load config: {e}")));
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
        Resource::Context { target, action } => {
            ctx.targeting = target.into();
            commands::context::run(&mut ctx, &action)
        }
        Resource::Project { target, action } => {
            ctx.targeting = target.into();
            commands::project::run(&mut ctx, &action)
        }
        Resource::Dependency { target, action } => {
            ctx.targeting = target.into();
            commands::dependency::run(&mut ctx, &action)
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
        Ok(Rendered::Data { payload, exit }) => {
            // The single success-render site: human view, or the JSON envelope
            // (`json_value` wraps the payload's data in `{schema, ok, data}`).
            if ctx.out.is_json() {
                ctx.out.json_value(&payload.json());
            } else {
                payload.human(&ctx.out);
            }
            ExitCode::from(exit)
        }
        // The command streamed its own output (or self-emitted); nothing to render.
        Ok(Rendered::Streamed) => ExitCode::SUCCESS,
        Err(e) => {
            ctx.out.error(&e);
            ExitCode::from(e.error_kind().exit_code())
        }
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
