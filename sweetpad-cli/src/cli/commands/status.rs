//! `sweetpad status` — the at-a-glance project view: the resolved container,
//! the effective build context *with provenance* (config vs remembered state vs
//! default), and the last launched app. Also what a bare `sweetpad` prints when
//! run inside a project, `git status`-style.

use crate::cli::output::Output;
use crate::cli::resolve::{self, Container};
use crate::cli::state::LastLaunchedApp;
use crate::cli::{CommandResult, Context, Render, Rendered};

/// One context row: variable, effective value (if any), and where it came from.
struct Row {
    name: &'static str,
    value: Option<String>,
    source: &'static str,
}

/// The status payload: container + rows + last launch.
struct StatusReport {
    container: String,
    kind: &'static str,
    rows: Vec<Row>,
    last_launched: Option<LastLaunchedApp>,
}

impl Render for StatusReport {
    fn human(&self, out: &Output) {
        out.line(&format!("{} ({})", self.container, self.kind));
        for row in &self.rows {
            let source = if row.source.is_empty() {
                String::new()
            } else {
                format!("  ({})", row.source)
            };
            out.line(&format!(
                "  {:<13} {}{source}",
                row.name,
                row.value.as_deref().unwrap_or("(will prompt)")
            ));
        }
        if let Some(app) = &self.last_launched {
            out.line(&format!(
                "  {:<13} {} ({})",
                "last launched", app.bundle_identifier, app.kind
            ));
        }
        out.note("run `sweetpad app run` to build and launch");
    }

    fn json(&self) -> serde_json::Value {
        let mut context = serde_json::Map::new();
        for row in &self.rows {
            context.insert(
                row.name.to_string(),
                serde_json::json!({
                    "value": row.value,
                    "source": if row.source.is_empty() { None } else { Some(row.source) },
                }),
            );
        }
        serde_json::json!({
            "container": self.container,
            "kind": self.kind,
            "context": context,
            "lastLaunchedApp": serde_json::to_value(&self.last_launched).unwrap_or_default(),
        })
    }
}

pub fn run(ctx: &mut Context) -> CommandResult {
    let resolved = resolve::resolve(ctx)?;
    let key = resolved.container.key();
    let cfg = ctx.config.for_project(&key);
    let pf_scheme = ctx.project_file(&resolved.container).scheme.clone();
    let pf_configuration = ctx.project_file(&resolved.container).configuration.clone();
    let pf_destination = ctx.project_file(&resolved.container).destination.clone();
    let pf_sdk = ctx.project_file(&resolved.container).sdk.clone();
    let st = ctx.state.projects.get(&key).cloned().unwrap_or_default();

    // flag > config > sweetpad.toml > remembered — mirrors the resolver; a
    // value present in `resolved` but in none of the layers must have come
    // from a flag/env.
    let provenance = |resolved_value: &Option<String>,
                      cfg_value: &Option<String>,
                      project_value: &Option<String>,
                      state_value: &Option<String>|
     -> (Option<String>, &'static str) {
        match resolved_value {
            None => (None, ""),
            Some(v) => {
                if cfg_value.as_deref() == Some(v) {
                    (Some(v.clone()), "config")
                } else if project_value.as_deref() == Some(v) {
                    (Some(v.clone()), "sweetpad.toml")
                } else if state_value.as_deref() == Some(v) {
                    (Some(v.clone()), "remembered")
                } else {
                    (Some(v.clone()), "flag/env")
                }
            }
        }
    };

    let mut rows = Vec::new();
    let (value, source) = provenance(&resolved.scheme, &cfg.scheme, &pf_scheme, &st.scheme);
    rows.push(Row {
        name: "scheme",
        value,
        source,
    });

    let (value, source) = provenance(
        &resolved.configuration,
        &cfg.configuration,
        &pf_configuration,
        &st.configuration,
    );
    // An unset configuration falls to `Debug` only when the project has one
    // (otherwise the picker runs) — show which of those will happen.
    let (value, source) = if let Some(v) = value {
        (Some(v), source)
    } else {
        let has_debug = resolve::configurations(&resolved.container)
            .map(|c| c.is_empty() || c.iter().any(|x| x == "Debug"))
            .unwrap_or(true);
        if has_debug {
            (Some("Debug".to_string()), "default")
        } else {
            (None, "")
        }
    };
    rows.push(Row {
        name: "configuration",
        value,
        source,
    });

    let (value, source) = provenance(
        &resolved.destination,
        &cfg.destination,
        &pf_destination,
        &st.destination,
    );
    rows.push(Row {
        name: "destination",
        value,
        source,
    });

    // `--on`/`SWEETPAD_ON` outranks every destination layer at build time;
    // surface it so the shown context is the one a build will actually use.
    if let Some(on) = &ctx.targeting.on {
        rows.push(Row {
            name: "on",
            value: Some(format!("{on} (overrides destination)")),
            source: "flag/env",
        });
    }

    if resolved.sdk.is_some() {
        let (value, source) = provenance(&resolved.sdk, &cfg.sdk, &pf_sdk, &st.sdk);
        rows.push(Row {
            name: "sdk",
            value,
            source,
        });
    }

    Ok(Rendered::data(StatusReport {
        container: resolved.container.path().display().to_string(),
        kind: match &resolved.container {
            Container::Workspace(_) => "workspace",
            Container::Project(_) => "project",
            Container::SwiftPackage(_) => "package",
        },
        rows,
        last_launched: st.last_launched_app,
    }))
}
