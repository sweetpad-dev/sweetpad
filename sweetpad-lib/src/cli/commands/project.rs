//! `sweetpad project …` — inspect and scaffold projects.

use std::path::{Path, PathBuf};

use clap::Subcommand;

use crate::cli::resolve::{self, Container};
use crate::cli::scaffold::{self, ProjectSpec, ScaffoldFile};
use crate::cli::{CliError, CliResult, Context};

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

pub fn run(ctx: &mut Context, action: &Action) -> CliResult {
    match action {
        Action::Info => info(ctx),
        Action::New(args) => new(ctx, args),
    }
}

fn new(ctx: &mut Context, args: &NewArgs) -> CliResult {
    let interactive = ctx.out.is_interactive();
    let cwd = std::env::current_dir()
        .map_err(|e| CliError::new(format!("cannot read current directory: {e}")))?;

    // On a TTY each still-unset field is prompted; off a TTY it's flags +
    // defaults, strictly. Both paths share the same defaults.
    let answers = gather_answers(args, &cwd, interactive)?;

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
    ensure_writable(&root, args.force, interactive)?;

    let files = scaffold::scaffold(&spec);
    let written = write_files(&root, &files)?;
    let git_initialized = answers.git && init_git(ctx, &root);

    report(
        ctx,
        &spec,
        &root,
        &written,
        answers.current_dir,
        git_initialized,
    );
    Ok(())
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
fn gather_answers(args: &NewArgs, cwd: &Path, interactive: bool) -> Result<Answers, CliError> {
    let current_dir = if args.current_dir {
        true
    } else if interactive {
        confirm("Scaffold into the current directory?", false)?
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
        None if interactive => select_platform(scaffold::Platform::Ios)?,
        None => scaffold::Platform::Ios,
    };

    let bundle_default = format!("com.example.{name}");
    let bundle_id = match &args.bundle_id {
        Some(bundle_id) => bundle_id.clone(),
        None if interactive => input(
            "Bundle identifier",
            Some(&bundle_default),
            scaffold::validate_bundle_id,
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
        )?,
        None => target_default,
    };

    let git = if args.no_git {
        false
    } else if interactive {
        confirm("Initialize a git repository?", true)?
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

/// A shared dialoguer theme so every prompt (`input`, `select`, `confirm`)
/// renders consistently.
fn theme() -> dialoguer::theme::ColorfulTheme {
    dialoguer::theme::ColorfulTheme::default()
}

/// Prompt for free text with a pre-filled default (Enter accepts it) and inline
/// validation that re-asks until the value passes.
fn input(
    prompt: &str,
    default: Option<&str>,
    validate: fn(&str) -> Result<(), String>,
) -> Result<String, CliError> {
    let theme = theme();
    let mut input = dialoguer::Input::<String>::with_theme(&theme).with_prompt(prompt);
    if let Some(default) = default {
        input = input.default(default.to_string());
    }
    input
        .validate_with(|s: &String| validate(s))
        .interact_text()
        .map_err(|e| CliError::new(format!("prompt failed: {e}")))
}

/// Pick a platform from the supported set, pre-selecting `default`.
fn select_platform(default: scaffold::Platform) -> Result<scaffold::Platform, CliError> {
    let platforms = [scaffold::Platform::Ios, scaffold::Platform::Macos];
    let labels: Vec<&str> = platforms.iter().map(|p| p.label()).collect();
    let default_idx = platforms.iter().position(|&p| p == default).unwrap_or(0);
    Ok(platforms[select("Platform", &labels, default_idx)?])
}

/// A `Select` menu returning the chosen index.
fn select(prompt: &str, items: &[&str], default: usize) -> Result<usize, CliError> {
    dialoguer::Select::with_theme(&theme())
        .with_prompt(prompt)
        .items(items)
        .default(default)
        .interact()
        .map_err(|e| CliError::new(format!("prompt failed: {e}")))
}

/// A yes/no prompt with a default that Enter accepts.
fn confirm(prompt: &str, default: bool) -> Result<bool, CliError> {
    dialoguer::Confirm::with_theme(&theme())
        .with_prompt(prompt)
        .default(default)
        .interact()
        .map_err(|e| CliError::new(format!("prompt failed: {e}")))
}

/// Refuse to scaffold over an existing non-empty directory. `--force` waives the
/// check outright; on a TTY without it, the user is asked (default no); off a
/// TTY it's a hard error.
fn ensure_writable(root: &Path, force: bool, interactive: bool) -> CliResult {
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

fn report(
    ctx: &Context,
    spec: &ProjectSpec,
    root: &Path,
    written: &[PathBuf],
    current_dir: bool,
    git_initialized: bool,
) {
    let xcodeproj = root.join(format!("{}.xcodeproj", spec.name));
    let scheme = xcodeproj
        .join("xcshareddata")
        .join("xcschemes")
        .join(format!("{}.xcscheme", spec.name));

    if ctx.out.is_json() {
        ctx.out.json_value(&serde_json::json!({
            "name": spec.name,
            "path": root.display().to_string(),
            "xcodeproj": xcodeproj.display().to_string(),
            "scheme": scheme.display().to_string(),
            "platform": spec.platform.label(),
            "bundleId": spec.bundle_id,
            "deploymentTarget": spec.deployment_target,
            "gitInitialized": git_initialized,
            "files": written
                .iter()
                .map(|p| p.display().to_string())
                .collect::<Vec<_>>(),
        }));
        return;
    }

    ctx.out
        .line(&format!("Created {} at {}", spec.name, root.display()));
    ctx.out.line("");
    ctx.out.line("Next steps:");
    if !current_dir {
        ctx.out.line(&format!("  cd {}", spec.name));
    }
    ctx.out.line("  sweetpad app run");
}

/// Gathered project facts, container-agnostic so workspace and project render
/// the same way.
struct Info {
    kind: &'static str,
    name: String,
    targets: Vec<String>,
    configurations: Vec<String>,
    schemes: Vec<String>,
}

fn info(ctx: &mut Context) -> CliResult {
    let container = resolve::container(ctx)?;
    let info = gather(&container)?;

    if ctx.out.is_json() {
        ctx.out.json_value(&serde_json::json!({
            "kind": info.kind,
            "name": info.name,
            "path": container.path().display().to_string(),
            "targets": info.targets,
            "configurations": info.configurations,
            "schemes": info.schemes,
        }));
        return Ok(());
    }

    ctx.out.line(&format!("{} ({})", info.name, info.kind));
    ctx.out
        .line(&format!("  path: {}", container.path().display()));
    section(ctx, "targets", &info.targets);
    section(ctx, "configurations", &info.configurations);
    section(ctx, "schemes", &info.schemes);
    Ok(())
}

fn section(ctx: &Context, title: &str, items: &[String]) {
    ctx.out.line(&format!("  {title}:"));
    if items.is_empty() {
        ctx.out.line("    (none)");
    } else {
        for item in items {
            ctx.out.line(&format!("    {item}"));
        }
    }
}

fn gather(container: &Container) -> Result<Info, CliError> {
    match container {
        Container::Workspace(p) => {
            let ws = crate::workspace::open(p).map_err(|e| {
                CliError::new(format!("failed to read workspace {}: {e}", p.display()))
            })?;
            Ok(Info {
                kind: "workspace",
                name: ws.name.clone(),
                targets: ws.merged_targets(),
                configurations: ws.merged_configurations(),
                schemes: ws.merged_schemes(),
            })
        }
        Container::Project(p) => {
            let proj = crate::project::open(p).map_err(|e| {
                CliError::new(format!("failed to read project {}: {e}", p.display()))
            })?;
            Ok(Info {
                kind: "project",
                name: proj.name.clone(),
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
                targets: manifest.targets.iter().map(|t| t.name.clone()).collect(),
                configurations: vec!["Debug".to_string(), "Release".to_string()],
                schemes: manifest.scheme_names(),
            })
        }
    }
}
