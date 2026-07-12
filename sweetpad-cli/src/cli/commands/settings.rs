//! `sweetpad settings …` — show resolved build settings, computed in-process
//! by the resolver ([`sweetpad_core::build_settings`]) — the engine's
//! specialty.
//!
//! This is the porcelain view: what the build will actually use, after
//! xcconfig files, SDK defaults, and `$(inherited)` chains. The *stored*
//! pbxproj layer — and everything that edits it — is plumbing:
//! `sweetpad pbxproj settings show/set/unset` (CLI_DESIGN §9g).

use std::path::PathBuf;

use clap::Subcommand;

use crate::cli::output::Output;
use crate::cli::resolve::{self, Container};
use crate::cli::{BuildTargetArgs, CliError, CommandResult, Context, Render, Rendered};
use sweetpad_core::build_settings::{BuildSettingsOptions, TargetSettings, resolve_build_settings};

#[derive(Debug, Subcommand)]
pub enum Action {
    /// Show resolved build settings for the resolved scheme/target.
    Show {
        #[command(flatten)]
        build: BuildTargetArgs,

        /// Resolve a single target instead of the scheme's buildables.
        #[arg(long)]
        target: Option<String>,

        /// Show only this one setting key — printed as the bare value, ready
        /// for `$(…)` capture.
        #[arg(long)]
        key: Option<String>,
    },
}

pub fn run(ctx: &mut Context, action: &Action) -> CommandResult {
    match action {
        Action::Show { build, target, key } => {
            ctx.targeting = build.clone().into();
            show(ctx, target.as_deref(), key.as_deref())
        }
    }
}

/// The resolved build settings for the query: a `# target:` block per target in
/// human mode, or `{ "targets": [ { "target", "settings" } ] }` in the JSON
/// envelope. A single-`--key` query prints the bare value(s) instead, so
/// `P=$(sweetpad settings show --key PRODUCT_NAME)` needs no sed. When empty,
/// `empty_note` carries the reason to print in human mode (a Swift-package
/// explanation, or "no build settings resolved").
struct SettingsResult {
    targets: Vec<TargetSettings>,
    empty_note: String,
    /// Set when the query was `--key X`: human mode prints only the values.
    bare_key: Option<String>,
}

impl Render for SettingsResult {
    fn human(&self, out: &Output) {
        if self.targets.is_empty() {
            out.note(&self.empty_note);
            return;
        }
        if let Some(key) = &self.bare_key {
            for t in &self.targets {
                if let Some(v) = t.settings.get(key) {
                    out.line(v);
                }
            }
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
    let mut resolved = resolve::resolve(ctx)?;

    let (project, workspace): (Option<PathBuf>, Option<PathBuf>) = match &resolved.container {
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
                bare_key: key.map(str::to_string),
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

    let configuration = resolve::settle_configuration(ctx, &mut resolved, false)?;
    // `--on` resolves to a concrete specifier here like it does for builds;
    // an explicit --destination (the resolved layer) otherwise applies. Both
    // typed is the same conflict it is for build/archive.
    resolve::reject_on_destination_conflict(ctx)?;
    let on_specifier = match ctx.targeting.on.clone() {
        Some(reference) => {
            let key = resolved.container.key();
            if resolve::on_is_mac(ctx, &key, &reference) {
                Some("platform=macOS".to_string())
            } else {
                let sims = crate::cli::simctl::list()?;
                Some(resolve::resolve_on(ctx, &key, &reference, &sims)?.specifier())
            }
        }
        None => None,
    };
    let destination = on_specifier
        .as_deref()
        .or(resolved.destination.as_deref())
        .and_then(sweetpad_lib::destination::parse_destination_arg);

    let opts = BuildSettingsOptions {
        project,
        workspace,
        scheme,
        target: target.map(str::to_string),
        configuration,
        sdk: resolved.sdk.clone().unwrap_or_default(),
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
        bare_key: key.map(str::to_string),
    }))
}
