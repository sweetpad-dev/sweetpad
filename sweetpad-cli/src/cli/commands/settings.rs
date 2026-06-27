//! `sweetpad settings …` — show resolved build settings, computed in-process by
//! the resolver ([`sweetpad_core::build_settings`]) — the engine's specialty.

use clap::Subcommand;

use crate::cli::output::Output;
use crate::cli::resolve::{self, Container};
use crate::cli::{CliError, CommandResult, Context, Render, Rendered};
use sweetpad_core::build_settings::{BuildSettingsOptions, TargetSettings, resolve_build_settings};

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

pub fn run(ctx: &mut Context, action: &Action) -> CommandResult {
    match action {
        Action::Show { target, key } => show(ctx, target.as_deref(), key.as_deref()),
    }
}

/// The resolved build settings for the query: a `# target:` block per target in
/// human mode, or `{ "targets": [ { "target", "settings" } ] }` in the JSON
/// envelope. When empty, `empty_note` carries the reason to print in human mode
/// (a Swift-package explanation, or "no build settings resolved").
struct SettingsResult {
    targets: Vec<TargetSettings>,
    empty_note: String,
}

impl Render for SettingsResult {
    fn human(&self, out: &Output) {
        if self.targets.is_empty() {
            out.note(&self.empty_note);
            return;
        }
        for (i, t) in self.targets.iter().enumerate() {
            if i > 0 {
                out.line("");
            }
            out.line(&format!("# target: {}", t.target));
            for (k, v) in &t.settings {
                out.line(&format!("{k} = {v}"));
            }
        }
    }

    fn json(&self) -> serde_json::Value {
        let targets: Vec<serde_json::Value> = self
            .targets
            .iter()
            .map(|t| serde_json::json!({ "target": t.target, "settings": t.settings }))
            .collect();
        serde_json::json!({ "targets": targets })
    }
}

fn show(ctx: &mut Context, target: Option<&str>, key: Option<&str>) -> CommandResult {
    let resolved = resolve::resolve(ctx)?;

    let (project, workspace) = match &resolved.container {
        Container::Project(p) => (Some(p.clone()), None),
        Container::Workspace(p) => (None, Some(p.clone())),
        Container::SwiftPackage(p) => {
            // SwiftPM packages have no pbxproj/xcconfig for the resolver to
            // compute settings from — surface that rather than erroring.
            return Ok(Rendered::data(SettingsResult {
                targets: Vec::new(),
                empty_note: format!(
                    "settings show is not available for Swift packages ({}); \
                     SwiftPM has no xcconfig/pbxproj build settings to resolve",
                    p.display()
                ),
            }));
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
        .and_then(sweetpad_lib::destination::parse_destination_arg);

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

    Ok(Rendered::data(SettingsResult {
        targets: results,
        empty_note: "no build settings resolved".to_string(),
    }))
}
