//! `sweetpad scheme …` — inspect schemes.

use clap::Subcommand;
use serde::Serialize;

use crate::cli::output::Output;
use crate::cli::{CommandResult, Context, Render, Rendered, resolve};

#[derive(Debug, Subcommand)]
pub enum Action {
    /// List the schemes available in the resolved project/workspace.
    List,
}

pub fn run(ctx: &mut Context, action: &Action) -> CommandResult {
    match action {
        Action::List => list(ctx),
    }
}

/// The scheme list: a marked human list, or the `data` of the JSON envelope.
#[derive(Serialize)]
struct SchemeList {
    container: String,
    selected: Option<String>,
    schemes: Vec<SchemeEntry>,
}

#[derive(Serialize)]
struct SchemeEntry {
    name: String,
    selected: bool,
}

impl Render for SchemeList {
    fn human(&self, out: &Output) {
        if self.schemes.is_empty() {
            out.note("no schemes found");
            return;
        }
        for s in &self.schemes {
            out.item(&s.name, s.selected);
        }
    }

    fn json(&self) -> serde_json::Value {
        serde_json::to_value(self).unwrap_or(serde_json::Value::Null)
    }
}

/// Enumerate schemes for the resolved container. The currently selected scheme
/// (flag > config > remembered state) is marked.
fn list(ctx: &mut Context) -> CommandResult {
    let resolved = resolve::resolve(ctx)?;
    let schemes = resolve::schemes(&resolved.container)?;
    let selected = resolved.scheme.clone();
    let entries = schemes
        .into_iter()
        .map(|name| {
            let is_selected = Some(&name) == selected.as_ref();
            SchemeEntry {
                name,
                selected: is_selected,
            }
        })
        .collect();
    Ok(Rendered::data(SchemeList {
        container: resolved.container.path().display().to_string(),
        selected,
        schemes: entries,
    }))
}
