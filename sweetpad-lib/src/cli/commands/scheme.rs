//! `sweetpad scheme …` — inspect schemes.

use clap::Subcommand;

use crate::cli::{CliResult, Context, resolve};

#[derive(Debug, Subcommand)]
pub enum Action {
    /// List the schemes available in the resolved project/workspace.
    List,
}

pub fn run(ctx: &mut Context, action: &Action) -> CliResult {
    match action {
        Action::List => list(ctx),
    }
}

/// Enumerate schemes for the resolved container and render them. The currently
/// selected scheme (flag > config > remembered state) is marked.
fn list(ctx: &mut Context) -> CliResult {
    let resolved = resolve::resolve(ctx)?;
    let schemes = resolve::schemes(&resolved.container)?;
    let selected = resolved.scheme.as_deref();

    if ctx.out.is_json() {
        let items: Vec<serde_json::Value> = schemes
            .iter()
            .map(|name| {
                serde_json::json!({
                    "name": name,
                    "selected": Some(name.as_str()) == selected,
                })
            })
            .collect();
        ctx.out.json_value(&serde_json::json!({
            "container": resolved.container.path().display().to_string(),
            "selected": selected,
            "schemes": items,
        }));
        return Ok(());
    }

    if schemes.is_empty() {
        ctx.out.note("no schemes found");
        return Ok(());
    }
    for name in &schemes {
        ctx.out.item(name, Some(name.as_str()) == selected);
    }
    Ok(())
}
