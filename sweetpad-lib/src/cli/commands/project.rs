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

    // On a TTY the wizard collects the answers (and lets the user step back to
    // change earlier ones); off a TTY it's flags + defaults, strictly.
    let answers = if interactive {
        run_wizard(args, &cwd)?
    } else {
        default_answers(args, &cwd)?
    };

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

/// Non-interactive resolution: flags win, else defaults. A missing name is the
/// one hard error — there's no sensible default without `--current-dir`.
fn default_answers(args: &NewArgs, cwd: &Path) -> Result<Answers, CliError> {
    let current_dir = args.current_dir;
    let name = match &args.name {
        Some(name) => name.clone(),
        None => current_dir
            .then(|| dir_basename(cwd))
            .flatten()
            .ok_or_else(|| {
                CliError::new(
                    "a project name is required (pass it as an argument, or use \
                     --current-dir to name the project after the current directory)",
                )
            })?,
    };
    let platform = args.platform.unwrap_or(scaffold::Platform::Ios);
    Ok(Answers {
        current_dir,
        bundle_id: args
            .bundle_id
            .clone()
            .unwrap_or_else(|| format!("com.example.{name}")),
        deployment_target: args
            .deployment_target
            .clone()
            .unwrap_or_else(|| platform.default_deployment_target().to_string()),
        platform,
        git: !args.no_git,
        name,
    })
}

// Wizard sentinels and trailing menu entries.
const SKIP: &str = "*"; // text steps: accept this default and all remaining ones
const BACK: &str = "<"; // text steps: return to the previous step
const USE_DEFAULTS: &str = "Use defaults for everything else"; // Select equivalent of `*`
const GO_BACK: &str = "← Back"; // Select equivalent of `<`

/// The ordered wizard steps. Each maps to one resolved [`Answers`] field.
#[derive(Clone, Copy)]
enum Step {
    Location,
    Name,
    Platform,
    BundleId,
    DeploymentTarget,
    Git,
}

/// How a step wants to move: forward, back to the previous step, or jump to the
/// end filling everything still unset from defaults.
enum Hop {
    Next,
    Back,
    Defaults,
}

/// Running wizard state. Flags pre-seed it; the steps fill the rest. A stored
/// answer doubles as the pre-filled default when a step is revisited via Back.
struct Wizard<'a> {
    cwd: &'a Path,
    current_dir: bool,
    name: Option<String>,
    platform: Option<scaffold::Platform>,
    bundle_id: Option<String>,
    deployment_target: Option<String>,
    git: bool,
}

/// Drive the steps a flag hasn't already pinned, honoring Back/Defaults, then
/// fold the collected state into [`Answers`].
fn run_wizard(args: &NewArgs, cwd: &Path) -> Result<Answers, CliError> {
    let mut wiz = Wizard {
        cwd,
        current_dir: args.current_dir,
        name: args.name.clone(),
        platform: args.platform,
        bundle_id: args.bundle_id.clone(),
        deployment_target: args.deployment_target.clone(),
        git: !args.no_git,
    };

    let mut steps = Vec::new();
    if !args.current_dir {
        steps.push(Step::Location);
    }
    if args.name.is_none() {
        steps.push(Step::Name);
    }
    if args.platform.is_none() {
        steps.push(Step::Platform);
    }
    if args.bundle_id.is_none() {
        steps.push(Step::BundleId);
    }
    if args.deployment_target.is_none() {
        steps.push(Step::DeploymentTarget);
    }
    if !args.no_git {
        steps.push(Step::Git);
    }

    let mut i = 0;
    while i < steps.len() {
        let can_back = i > 0;
        let hop = match steps[i] {
            Step::Location => wiz.ask_location(can_back)?,
            Step::Name => wiz.ask_name(can_back)?,
            Step::Platform => wiz.ask_platform(can_back)?,
            Step::BundleId => wiz.ask_bundle_id(can_back)?,
            Step::DeploymentTarget => wiz.ask_deployment_target(can_back)?,
            Step::Git => wiz.ask_git(can_back)?,
        };
        match hop {
            Hop::Next => i += 1,
            Hop::Back => i = i.saturating_sub(1),
            Hop::Defaults => break, // remaining fields stay None → defaulted below
        }
    }

    wiz.into_answers()
}

impl Wizard<'_> {
    fn into_answers(self) -> Result<Answers, CliError> {
        // Name is always set here: it's flag-pinned or a step that precedes every
        // "use defaults" escape (those start at Platform), so a break can't skip it.
        let name = self
            .name
            .ok_or_else(|| CliError::new("a project name is required"))?;
        let platform = self.platform.unwrap_or(scaffold::Platform::Ios);
        Ok(Answers {
            current_dir: self.current_dir,
            bundle_id: self
                .bundle_id
                .unwrap_or_else(|| format!("com.example.{name}")),
            deployment_target: self
                .deployment_target
                .unwrap_or_else(|| platform.default_deployment_target().to_string()),
            platform,
            git: self.git,
            name,
        })
    }

    fn ask_location(&mut self, can_back: bool) -> Result<Hop, CliError> {
        let mut items = vec![
            "Create a new project directory".to_string(),
            "Use the current directory".to_string(),
        ];
        let back = add_back(&mut items, can_back);
        let idx = select("Location", &items, usize::from(self.current_dir))?;
        if Some(idx) == back {
            return Ok(Hop::Back);
        }
        self.current_dir = idx == 1;
        Ok(Hop::Next)
    }

    fn ask_name(&mut self, can_back: bool) -> Result<Hop, CliError> {
        // Revisit shows the prior answer; first visit offers the directory
        // basename only when scaffolding in place.
        let default = self
            .name
            .clone()
            .or_else(|| self.current_dir.then(|| dir_basename(self.cwd)).flatten());
        match ask_text(
            "Project name",
            default.as_deref(),
            scaffold::validate_name,
            can_back,
            false,
        )? {
            TextHop::Value(value) => {
                self.name = Some(value);
                Ok(Hop::Next)
            }
            TextHop::Back => Ok(Hop::Back),
            TextHop::Defaults => unreachable!("name step has no defaults escape"),
        }
    }

    fn ask_platform(&mut self, can_back: bool) -> Result<Hop, CliError> {
        let choices = [scaffold::Platform::Ios, scaffold::Platform::Macos];
        let mut items: Vec<String> = choices.iter().map(|p| p.label().to_string()).collect();
        let defaults_idx = items.len();
        items.push(USE_DEFAULTS.to_string());
        let back = add_back(&mut items, can_back);
        let default = usize::from(self.platform == Some(scaffold::Platform::Macos));
        let idx = select("Platform", &items, default)?;
        if Some(idx) == back {
            return Ok(Hop::Back);
        }
        if idx == defaults_idx {
            return Ok(Hop::Defaults);
        }
        self.platform = Some(choices[idx]);
        Ok(Hop::Next)
    }

    fn ask_bundle_id(&mut self, can_back: bool) -> Result<Hop, CliError> {
        let default = self
            .bundle_id
            .clone()
            .unwrap_or_else(|| format!("com.example.{}", self.name.as_deref().unwrap_or("App")));
        self.text_step(
            "Bundle identifier",
            &default,
            scaffold::validate_bundle_id,
            can_back,
            |wiz, value| wiz.bundle_id = Some(value),
        )
    }

    fn ask_deployment_target(&mut self, can_back: bool) -> Result<Hop, CliError> {
        let platform = self.platform.unwrap_or(scaffold::Platform::Ios);
        let default = self
            .deployment_target
            .clone()
            .unwrap_or_else(|| platform.default_deployment_target().to_string());
        let label = format!("{} deployment target", platform.label());
        self.text_step(
            &label,
            &default,
            scaffold::validate_deployment_target,
            can_back,
            |wiz, value| wiz.deployment_target = Some(value),
        )
    }

    fn ask_git(&mut self, can_back: bool) -> Result<Hop, CliError> {
        let mut items = vec!["Yes".to_string(), "No".to_string()];
        let back = add_back(&mut items, can_back);
        let idx = select(
            "Initialize a git repository?",
            &items,
            usize::from(!self.git),
        )?;
        if Some(idx) == back {
            return Ok(Hop::Back);
        }
        self.git = idx == 0;
        Ok(Hop::Next)
    }

    /// Shared body for the optional text steps (bundle id, deployment target):
    /// both honor the `*` defaults escape and the `<` back escape.
    fn text_step(
        &mut self,
        label: &str,
        default: &str,
        validate: fn(&str) -> Result<(), String>,
        can_back: bool,
        store: impl FnOnce(&mut Self, String),
    ) -> Result<Hop, CliError> {
        match ask_text(label, Some(default), validate, can_back, true)? {
            TextHop::Value(value) => {
                store(self, value);
                Ok(Hop::Next)
            }
            TextHop::Back => Ok(Hop::Back),
            TextHop::Defaults => Ok(Hop::Defaults),
        }
    }
}

/// Append a "← Back" entry when going back is possible; return its index.
fn add_back(items: &mut Vec<String>, can_back: bool) -> Option<usize> {
    can_back.then(|| {
        items.push(GO_BACK.to_string());
        items.len() - 1
    })
}

/// A `Select` step returning the chosen index.
fn select(prompt: &str, items: &[String], default: usize) -> Result<usize, CliError> {
    dialoguer::Select::new()
        .with_prompt(prompt)
        .items(items)
        .default(default)
        .interact()
        .map_err(|e| CliError::new(format!("prompt failed: {e}")))
}

/// What a text step resolved to.
enum TextHop {
    Value(String),
    Back,
    Defaults,
}

/// A text wizard step: pre-filled default, inline validation (re-asks until
/// accepted), plus the `<` back escape and — when `allow_defaults` — the `*`
/// defaults escape. The escapes bypass validation.
fn ask_text(
    label: &str,
    default: Option<&str>,
    validate: fn(&str) -> Result<(), String>,
    can_back: bool,
    allow_defaults: bool,
) -> Result<TextHop, CliError> {
    let mut hints = Vec::new();
    if allow_defaults {
        hints.push("* = defaults");
    }
    if can_back {
        hints.push("< = back");
    }
    let prompt = if hints.is_empty() {
        label.to_string()
    } else {
        format!("{label} ({})", hints.join(", "))
    };

    let mut input = dialoguer::Input::<String>::new().with_prompt(prompt);
    if let Some(default) = default {
        input = input.default(default.to_string());
    }
    let value = input
        .validate_with(move |s: &String| {
            if (allow_defaults && s == SKIP) || (can_back && s == BACK) {
                Ok(())
            } else {
                validate(s)
            }
        })
        .interact_text()
        .map_err(|e| CliError::new(format!("prompt failed: {e}")))?;

    if can_back && value == BACK {
        Ok(TextHop::Back)
    } else if allow_defaults && value == SKIP {
        Ok(TextHop::Defaults)
    } else {
        Ok(TextHop::Value(value))
    }
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
