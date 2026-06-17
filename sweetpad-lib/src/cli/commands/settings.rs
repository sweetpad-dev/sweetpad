//! `sweetpad settings …` — show resolved build settings, computed in-process by
//! the resolver ([`crate::build_settings`]) — the engine's specialty.

use clap::Subcommand;

use crate::build_settings::{BuildSettingsOptions, resolve_build_settings};
use crate::cli::resolve::{self, Container};
use crate::cli::{CliError, CliResult, Context};

#[derive(Debug, Subcommand)]
pub enum Action {
    /// Show resolved build settings for the resolved scheme/target.
    Show {
        /// Resolve a single target instead of the scheme's buildables.
        #[arg(long)]
        target: Option<String>,

        /// Show only this one setting key.
        #[arg(long)]
        key: Option<String>,
    },
}

pub fn run(ctx: &mut Context, action: &Action) -> CliResult {
    match action {
        Action::Show { target, key } => show(ctx, target.as_deref(), key.as_deref()),
    }
}

fn show(ctx: &mut Context, target: Option<&str>, key: Option<&str>) -> CliResult {
    let resolved = resolve::resolve(ctx)?;

    let (project, workspace) = match &resolved.container {
        Container::Project(p) => (Some(p.clone()), None),
        Container::Workspace(p) => (None, Some(p.clone())),
        Container::SwiftPackage(p) => {
            // SwiftPM packages have no pbxproj/xcconfig for the resolver to
            // compute settings from — surface that rather than erroring.
            if ctx.out.is_json() {
                ctx.out.json_value(&serde_json::json!({ "targets": [] }));
            } else {
                ctx.out.note(&format!(
                    "settings show is not available for Swift packages ({}); \
                     SwiftPM has no xcconfig/pbxproj build settings to resolve",
                    p.display()
                ));
            }
            return Ok(());
        }
    };

    // A `--target` query bypasses scheme resolution; otherwise settle a scheme.
    let scheme = if target.is_some() {
        None
    } else {
        let schemes = resolve::schemes(&resolved.container)?;
        Some(resolve::choose(
            ctx,
            "scheme",
            resolved.scheme.clone(),
            &schemes,
        )?)
    };

    let configuration = resolved
        .configuration
        .clone()
        .unwrap_or_else(|| "Debug".to_string());
    let destination = resolved
        .destination
        .as_deref()
        .and_then(crate::destination::parse_destination_arg);

    let opts = BuildSettingsOptions {
        project,
        workspace,
        scheme,
        target: target.map(str::to_string),
        configuration,
        sdk: String::new(),
        arch: String::new(),
        destination,
        xcconfig: None,
        xcode: None,
        xcspec_root: None,
        sdksettings_root: None,
        catalog_cache: None,
        derived_data_path: None,
        keys: key.map(|k| vec![k.to_string()]),
    };

    let results = resolve_build_settings(&opts).map_err(CliError::new)?;

    if ctx.out.is_json() {
        let targets: Vec<serde_json::Value> = results
            .iter()
            .map(|t| serde_json::json!({ "target": t.target, "settings": t.settings }))
            .collect();
        ctx.out
            .json_value(&serde_json::json!({ "targets": targets }));
        return Ok(());
    }

    if results.is_empty() {
        ctx.out.note("no build settings resolved");
        return Ok(());
    }
    for (i, t) in results.iter().enumerate() {
        if i > 0 {
            ctx.out.line("");
        }
        ctx.out.line(&format!("# target: {}", t.target));
        for (k, v) in &t.settings {
            ctx.out.line(&format!("{k} = {v}"));
        }
    }
    Ok(())
}
