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

    // Each step takes the flag if it was given, else asks on a TTY (with a
    // default that Enter accepts), else falls back to the default off a TTY.
    let current_dir = if args.current_dir {
        true
    } else if interactive {
        confirm("Create the project in the current directory?", false)?
    } else {
        false
    };

    // With --current-dir an absent name falls back to the directory's basename.
    let dir_name = if current_dir {
        cwd.file_name().map(|n| n.to_string_lossy().into_owned())
    } else {
        None
    };

    let name = resolve_name(args, dir_name.as_deref(), interactive)?;
    let platform = resolve_platform(args, interactive)?;
    let bundle_id = resolve_bundle_id(args, &name, interactive)?;
    let deployment_target = resolve_deployment_target(args, platform, interactive)?;

    let spec =
        ProjectSpec::new(&name, &bundle_id, &deployment_target, platform).map_err(CliError::new)?;

    let root = if current_dir {
        cwd
    } else {
        cwd.join(&spec.name)
    };
    ensure_writable(&root, args.force, interactive)?;

    // --no-git short-circuits to "don't init"; otherwise ask / default to yes.
    let want_git = if args.no_git {
        false
    } else if interactive {
        confirm("Initialize a git repository?", true)?
    } else {
        true
    };

    let files = scaffold::scaffold(&spec);
    let written = write_files(&root, &files)?;
    let git_initialized = want_git && init_git(ctx, &root);

    report(ctx, &spec, &root, &written, current_dir, git_initialized);
    Ok(())
}

/// Name resolution: explicit flag/arg > directory basename (with
/// `--current-dir`) > interactive prompt > hard error in non-TTY contexts.
fn resolve_name(
    args: &NewArgs,
    dir_name: Option<&str>,
    interactive: bool,
) -> Result<String, CliError> {
    if let Some(name) = &args.name {
        return Ok(name.clone());
    }
    if interactive {
        return prompt("Project name", dir_name, scaffold::validate_name);
    }
    dir_name.map(str::to_string).ok_or_else(|| {
        CliError::new(
            "a project name is required (pass it as an argument, or use \
             --current-dir to name the project after the current directory)",
        )
    })
}

fn resolve_bundle_id(args: &NewArgs, name: &str, interactive: bool) -> Result<String, CliError> {
    if let Some(bundle_id) = &args.bundle_id {
        return Ok(bundle_id.clone());
    }
    let default = format!("com.example.{name}");
    if interactive {
        return prompt(
            "Bundle identifier",
            Some(&default),
            scaffold::validate_bundle_id,
        );
    }
    Ok(default)
}

/// Platform resolution: explicit flag > wizard pick (default iOS) > iOS.
fn resolve_platform(args: &NewArgs, interactive: bool) -> Result<scaffold::Platform, CliError> {
    if let Some(platform) = args.platform {
        return Ok(platform);
    }
    if !interactive {
        return Ok(scaffold::Platform::Ios);
    }
    let choices = [scaffold::Platform::Ios, scaffold::Platform::Macos];
    let labels: Vec<&str> = choices.iter().map(|p| p.label()).collect();
    let index = dialoguer::Select::new()
        .with_prompt("Platform")
        .items(&labels)
        .default(0)
        .interact()
        .map_err(|e| CliError::new(format!("prompt failed: {e}")))?;
    Ok(choices[index])
}

fn resolve_deployment_target(
    args: &NewArgs,
    platform: scaffold::Platform,
    interactive: bool,
) -> Result<String, CliError> {
    if let Some(target) = &args.deployment_target {
        return Ok(target.clone());
    }
    let default = platform.default_deployment_target();
    if interactive {
        let label = format!("{} deployment target", platform.label());
        return prompt(&label, Some(default), scaffold::validate_deployment_target);
    }
    Ok(default.to_string())
}

/// One wizard step: a text prompt with an optional pre-filled default and
/// inline validation (re-asks until the value is accepted).
fn prompt(
    label: &str,
    default: Option<&str>,
    validate: fn(&str) -> Result<(), String>,
) -> Result<String, CliError> {
    let mut input = dialoguer::Input::<String>::new().with_prompt(label);
    if let Some(default) = default {
        input = input.default(default.to_string());
    }
    input
        .validate_with(move |value: &String| validate(value))
        .interact_text()
        .map_err(|e| CliError::new(format!("prompt failed: {e}")))
}

/// A yes/no wizard step with a default that Enter accepts.
fn confirm(prompt: &str, default: bool) -> Result<bool, CliError> {
    dialoguer::Confirm::new()
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
    ctx.out.line("  sweetpad build start");
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
    let resolved = resolve::resolve(ctx)?;
    let info = gather(&resolved.container)?;

    if ctx.out.is_json() {
        ctx.out.json_value(&serde_json::json!({
            "kind": info.kind,
            "name": info.name,
            "path": resolved.container.path().display().to_string(),
            "targets": info.targets,
            "configurations": info.configurations,
            "schemes": info.schemes,
        }));
        return Ok(());
    }

    ctx.out.line(&format!("{} ({})", info.name, info.kind));
    ctx.out
        .line(&format!("  path: {}", resolved.container.path().display()));
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
        Container::SwiftPackage(p) => Err(CliError::new(format!(
            "project info is not supported for Swift packages ({}); the resolver \
             reads Xcode project files, not Package.swift",
            p.display()
        ))),
    }
}
