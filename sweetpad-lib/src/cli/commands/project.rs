//! `sweetpad project …` — inspect the project.

use clap::Subcommand;

use crate::cli::resolve::{self, Container};
use crate::cli::{CliError, CliResult, Context};

#[derive(Debug, Subcommand)]
pub enum Action {
    /// Show targets, configurations, and schemes for the resolved project.
    Info,
}

pub fn run(ctx: &mut Context, action: &Action) -> CliResult {
    match action {
        Action::Info => info(ctx),
    }
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
    ctx.out.line(&format!("  path: {}", resolved.container.path().display()));
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
            let ws = crate::workspace::open(p)
                .map_err(|e| CliError::new(format!("failed to read workspace {}: {e}", p.display())))?;
            Ok(Info {
                kind: "workspace",
                name: ws.name.clone(),
                targets: ws.merged_targets(),
                configurations: ws.merged_configurations(),
                schemes: ws.merged_schemes(),
            })
        }
        Container::Project(p) => {
            let proj = crate::project::open(p)
                .map_err(|e| CliError::new(format!("failed to read project {}: {e}", p.display())))?;
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
