//! `sweetpad project …` — inspect and scaffold projects.

use std::path::{Path, PathBuf};

use clap::Subcommand;

use crate::cli::output::Output;
use crate::cli::resolve::{self, Container};
use crate::cli::scaffold::{self, ProjectSpec, ScaffoldFile};
use crate::cli::{CliError, CliResult, CommandResult, Context, ErrorKind, Render, Rendered};

#[derive(Debug, Subcommand)]
pub enum Action {
    /// Show targets, configurations, and schemes for the resolved project.
    Info,
    /// Create a new minimal SwiftUI app project (iOS or macOS).
    ///
    /// Run with no flags on a terminal to be walked through a short wizard;
    /// pass flags (or `--json`) to skip the prompts and use defaults.
    New(NewArgs),
}

/// Flags for `project new`. Anything left unset is filled by the interactive
/// wizard on a TTY, or by its default otherwise.
#[derive(Debug, clap::Args)]
pub struct NewArgs {
    /// Project name — used for the `.xcodeproj`, target, and product. Optional
    /// with `--current-dir`, where it defaults to the directory's name.
    pub name: Option<String>,

    /// Scaffold into the current directory instead of creating `./<Name>/`.
    #[arg(long)]
    pub current_dir: bool,

    /// Bundle identifier (default: `com.example.<Name>`).
    #[arg(long)]
    pub bundle_id: Option<String>,

    /// iOS deployment target (default: `17.0`).
    #[arg(long)]
    pub deployment_target: Option<String>,

    /// Target platform (default: `ios`).
    #[arg(long, value_enum)]
    pub platform: Option<scaffold::Platform>,

    /// Skip `git init` in the new project.
    #[arg(long)]
    pub no_git: bool,

    /// Allow scaffolding into a non-empty directory.
    #[arg(long)]
    pub force: bool,
}

pub fn run(ctx: &mut Context, action: &Action) -> CommandResult {
    match action {
        Action::Info => info(ctx),
        Action::New(args) => new(ctx, args),
    }
}

fn new(ctx: &mut Context, args: &NewArgs) -> CommandResult {
    let interactive = ctx.out.is_interactive();
    let color = ctx.out.use_color();
    let cwd = std::env::current_dir()
        .map_err(|e| CliError::new(format!("cannot read current directory: {e}")))?;

    // On a TTY each still-unset field is prompted; off a TTY it's flags +
    // defaults, strictly. Both paths share the same defaults.
    let answers = gather_answers(args, &cwd, interactive, color)?;

    let spec = ProjectSpec::new(
        &answers.name,
        &answers.bundle_id,
        &answers.deployment_target,
        answers.platform,
    )
    .map_err(CliError::new)?;

    let root = if answers.current_dir {
        cwd
    } else {
        cwd.join(&spec.name)
    };
    ensure_writable(&root, args.force, interactive, color)?;

    let files = scaffold::scaffold(&spec);
    let written = write_files(&root, &files)?;
    let git_initialized = answers.git && init_git(ctx, &root);

    Ok(Rendered::data(created(
        &spec,
        &root,
        &written,
        answers.current_dir,
        git_initialized,
    )))
}

/// The fully-resolved choices a `project new` run operates on, however they were
/// obtained (flags, the wizard, or defaults).
struct Answers {
    current_dir: bool,
    name: String,
    platform: scaffold::Platform,
    bundle_id: String,
    deployment_target: String,
    git: bool,
}

fn dir_basename(dir: &Path) -> Option<String> {
    dir.file_name().map(|n| n.to_string_lossy().into_owned())
}

/// Resolve every `project new` choice in one pass. Each field takes its flag
/// when set; otherwise it's prompted on a TTY and falls back to its default off
/// one. Defaults are computed here once, so the interactive and headless paths
/// can't drift apart.
fn gather_answers(
    args: &NewArgs,
    cwd: &Path,
    interactive: bool,
    color: bool,
) -> Result<Answers, CliError> {
    let current_dir = if args.current_dir {
        true
    } else if interactive {
        confirm("Scaffold into the current directory?", false, color)?
    } else {
        false
    };

    // A missing name is the one hard error off a TTY — there's no sensible
    // default unless we can borrow the current directory's name.
    let name_default = current_dir.then(|| dir_basename(cwd)).flatten();
    let name = match &args.name {
        Some(name) => name.clone(),
        None if interactive => input(
            "Project name",
            name_default.as_deref(),
            scaffold::validate_name,
            color,
        )?,
        None => name_default.ok_or_else(|| {
            CliError::new(
                "a project name is required (pass it as an argument, or use \
                 --current-dir to name the project after the current directory)",
            )
        })?,
    };

    let platform = match args.platform {
        Some(platform) => platform,
        None if interactive => select_platform(scaffold::Platform::Ios, color)?,
        None => scaffold::Platform::Ios,
    };

    let bundle_default = format!("com.example.{}", scaffold::bundle_id_segment(&name));
    let bundle_id = match &args.bundle_id {
        Some(bundle_id) => bundle_id.clone(),
        None if interactive => input(
            "Bundle identifier",
            Some(&bundle_default),
            scaffold::validate_bundle_id,
            color,
        )?,
        None => bundle_default,
    };

    let target_default = platform.default_deployment_target().to_string();
    let deployment_target = match &args.deployment_target {
        Some(target) => target.clone(),
        None if interactive => input(
            &format!("{} deployment target", platform.label()),
            Some(&target_default),
            scaffold::validate_deployment_target,
            color,
        )?,
        None => target_default,
    };

    let git = if args.no_git {
        false
    } else if interactive {
        confirm("Initialize a git repository?", true, color)?
    } else {
        true
    };

    Ok(Answers {
        current_dir,
        name,
        platform,
        bundle_id,
        deployment_target,
        git,
    })
}

/// The dialoguer theme honoring `--no-color`: colorful on a color terminal, a
/// plain theme otherwise. Boxed so the prompts can take `&dyn Theme`.
fn prompt_theme(color: bool) -> Box<dyn dialoguer::theme::Theme> {
    if color {
        Box::new(dialoguer::theme::ColorfulTheme::default())
    } else {
        Box::new(dialoguer::theme::SimpleTheme)
    }
}

/// Prompt for free text with a pre-filled default (Enter accepts it) and inline
/// validation that re-asks until the value passes.
fn input(
    prompt: &str,
    default: Option<&str>,
    validate: fn(&str) -> Result<(), String>,
    color: bool,
) -> Result<String, CliError> {
    let theme = prompt_theme(color);
    let mut input = dialoguer::Input::<String>::with_theme(theme.as_ref()).with_prompt(prompt);
    if let Some(default) = default {
        input = input.default(default.to_string());
    }
    input
        .validate_with(|s: &String| validate(s))
        .interact_text()
        .map_err(|e| CliError::new(format!("prompt failed: {e}")).kind(ErrorKind::UserCancel))
}

/// Pick a platform from the supported set, pre-selecting `default`.
fn select_platform(
    default: scaffold::Platform,
    color: bool,
) -> Result<scaffold::Platform, CliError> {
    let platforms = [scaffold::Platform::Ios, scaffold::Platform::Macos];
    let labels: Vec<&str> = platforms.iter().map(|p| p.label()).collect();
    let default_idx = platforms.iter().position(|&p| p == default).unwrap_or(0);
    Ok(platforms[select("Platform", &labels, default_idx, color)?])
}

/// A `Select` menu returning the chosen index.
fn select(prompt: &str, items: &[&str], default: usize, color: bool) -> Result<usize, CliError> {
    let theme = prompt_theme(color);
    dialoguer::Select::with_theme(theme.as_ref())
        .with_prompt(prompt)
        .items(items)
        .default(default)
        .interact()
        .map_err(|e| CliError::new(format!("prompt failed: {e}")).kind(ErrorKind::UserCancel))
}

/// A yes/no prompt with a default that Enter accepts.
fn confirm(prompt: &str, default: bool, color: bool) -> Result<bool, CliError> {
    let theme = prompt_theme(color);
    dialoguer::Confirm::with_theme(theme.as_ref())
        .with_prompt(prompt)
        .default(default)
        .interact()
        .map_err(|e| CliError::new(format!("prompt failed: {e}")).kind(ErrorKind::UserCancel))
}

/// Refuse to scaffold over an existing non-empty directory. `--force` waives the
/// check outright; on a TTY without it, the user is asked (default no); off a
/// TTY it's a hard error.
fn ensure_writable(root: &Path, force: bool, interactive: bool, color: bool) -> CliResult {
    let mut entries = match std::fs::read_dir(root) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => {
            return Err(CliError::new(format!(
                "cannot inspect {}: {e}",
                root.display()
            )));
        }
    };
    if entries.next().is_none() || force {
        return Ok(());
    }
    if interactive
        && confirm(
            &format!("{} is not empty. Scaffold into it anyway?", root.display()),
            false,
            color,
        )?
    {
        return Ok(());
    }
    Err(CliError::new(format!(
        "{} already exists and is not empty (use --force to scaffold into it anyway)",
        root.display()
    )))
}

/// Write each generated file under `root`, creating parent directories.
fn write_files(root: &Path, files: &[ScaffoldFile]) -> Result<Vec<PathBuf>, CliError> {
    let mut written = Vec::with_capacity(files.len());
    for file in files {
        let full = root.join(&file.path);
        if let Some(parent) = full.parent() {
            std::fs::create_dir_all(parent).map_err(|e| {
                CliError::new(format!("failed to create {}: {e}", parent.display()))
            })?;
        }
        std::fs::write(&full, &file.contents)
            .map_err(|e| CliError::new(format!("failed to write {}: {e}", full.display())))?;
        written.push(full);
    }
    Ok(written)
}

/// Best-effort `git init`. A missing git or a non-zero exit is a soft note —
/// the project is already written and usable without a repository.
fn init_git(ctx: &Context, root: &Path) -> bool {
    match crate::cli::process::run("git", &["init", "--quiet"], Some(root), true) {
        Ok(true) => true,
        Ok(false) => {
            ctx.out
                .note("git init failed; skipping repository initialization");
            false
        }
        Err(_) => {
            ctx.out
                .note("git not found on PATH; skipping repository initialization");
            false
        }
    }
}

/// The outcome of a `project new` scaffold: the created project's facts. JSON
/// reports the full object; human mode prints a "Created … / Next steps" summary.
struct Created {
    name: String,
    path: String,
    xcodeproj: String,
    scheme: String,
    platform: &'static str,
    bundle_id: String,
    deployment_target: String,
    git_initialized: bool,
    files: Vec<String>,
    /// Whether we scaffolded into the current directory — drops the `cd` step.
    current_dir: bool,
}

impl Render for Created {
    fn human(&self, out: &Output) {
        out.line(&format!("Created {} at {}", self.name, self.path));
        out.line("");
        out.line("Next steps:");
        if !self.current_dir {
            out.line(&format!("  cd {}", self.name));
        }
        out.line("  sweetpad app run");
    }

    fn json(&self) -> serde_json::Value {
        serde_json::json!({
            "name": self.name,
            "path": self.path,
            "xcodeproj": self.xcodeproj,
            "scheme": self.scheme,
            "platform": self.platform,
            "bundleId": self.bundle_id,
            "deploymentTarget": self.deployment_target,
            "gitInitialized": self.git_initialized,
            "files": self.files,
        })
    }
}

/// Assemble the `Created` payload from the scaffold result. Derives the
/// `.xcodeproj`/`.xcscheme` paths the same way the report always has.
fn created(
    spec: &ProjectSpec,
    root: &Path,
    written: &[PathBuf],
    current_dir: bool,
    git_initialized: bool,
) -> Created {
    let xcodeproj = root.join(format!("{}.xcodeproj", spec.name));
    let scheme = xcodeproj
        .join("xcshareddata")
        .join("xcschemes")
        .join(format!("{}.xcscheme", spec.name));

    Created {
        name: spec.name.clone(),
        path: root.display().to_string(),
        xcodeproj: xcodeproj.display().to_string(),
        scheme: scheme.display().to_string(),
        platform: spec.platform.label(),
        bundle_id: spec.bundle_id.clone(),
        deployment_target: spec.deployment_target.clone(),
        git_initialized,
        files: written.iter().map(|p| p.display().to_string()).collect(),
        current_dir,
    }
}

/// Gathered project facts, container-agnostic so workspace and project render
/// the same way. Renders as a name/path header with sectioned lists, or
/// `{kind, name, path, targets, configurations, schemes}` in the JSON envelope.
struct Info {
    kind: &'static str,
    name: String,
    path: String,
    targets: Vec<String>,
    configurations: Vec<String>,
    schemes: Vec<String>,
}

impl Render for Info {
    fn human(&self, out: &Output) {
        out.line(&format!("{} ({})", self.name, self.kind));
        out.line(&format!("  path: {}", self.path));
        section(out, "targets", &self.targets);
        section(out, "configurations", &self.configurations);
        section(out, "schemes", &self.schemes);
    }

    fn json(&self) -> serde_json::Value {
        serde_json::json!({
            "kind": self.kind,
            "name": self.name,
            "path": self.path,
            "targets": self.targets,
            "configurations": self.configurations,
            "schemes": self.schemes,
        })
    }
}

fn info(ctx: &mut Context) -> CommandResult {
    let container = resolve::container(ctx)?;
    let info = gather(&container)?;
    Ok(Rendered::data(info))
}

fn section(out: &Output, title: &str, items: &[String]) {
    out.line(&format!("  {title}:"));
    if items.is_empty() {
        out.line("    (none)");
    } else {
        for item in items {
            out.line(&format!("    {item}"));
        }
    }
}

fn gather(container: &Container) -> Result<Info, CliError> {
    let path = container.path().display().to_string();
    match container {
        Container::Workspace(p) => {
            let ws = sweetpad_lib::workspace::open(p).map_err(|e| {
                CliError::new(format!("failed to read workspace {}: {e}", p.display()))
            })?;
            Ok(Info {
                kind: "workspace",
                name: ws.name.clone(),
                path,
                targets: ws.merged_targets(),
                configurations: ws.merged_configurations(),
                schemes: ws.merged_schemes(),
            })
        }
        Container::Project(p) => {
            let proj = sweetpad_lib::project::open(p).map_err(|e| {
                CliError::new(format!("failed to read project {}: {e}", p.display()))
            })?;
            Ok(Info {
                kind: "project",
                name: proj.name.clone(),
                path,
                targets: proj.targets.iter().map(|t| t.name.clone()).collect(),
                configurations: proj.configurations.clone(),
                schemes: proj.schemes.clone(),
            })
        }
        Container::SwiftPackage(_) => {
            // No pbxproj to read; evaluate the manifest instead. Targets are
            // every declared target; schemes mirror the synthesized set
            // (products, or non-test targets). SwiftPM builds are debug/release.
            let manifest = crate::cli::swiftpm::manifest(container)?;
            Ok(Info {
                kind: "package",
                name: manifest.name.clone(),
                path,
                targets: manifest.targets.iter().map(|t| t.name.clone()).collect(),
                configurations: vec!["Debug".to_string(), "Release".to_string()],
                schemes: manifest.scheme_names(),
            })
        }
    }
}
