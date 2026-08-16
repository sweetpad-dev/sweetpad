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
    /// Human-only annotation; never part of the machine value.
    note: &'static str,
}

/// The status payload: container + rows + last launch.
struct StatusReport {
    container: String,
    kind: &'static str,
    rows: Vec<Row>,
    last_launched: Option<LastLaunchedApp>,
    /// The file a detached macOS launch captured stdout/stderr to, when the last
    /// launch was macOS and the file is present — so `app logs` reading it back
    /// is discoverable without catching the one launch line that named it.
    detached_log: Option<String>,
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
            let note = if row.note.is_empty() {
                String::new()
            } else {
                format!(" ({})", row.note)
            };
            out.line(&format!(
                "  {:<13} {}{note}{source}",
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
        if let Some(log) = &self.detached_log {
            out.line(&format!("  {:<13} {log}", "detached log"));
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
            "detachedLog": self.detached_log,
        })
    }
}

#[allow(clippy::too_many_lines)] // a row per context variable, plus the launch summary
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
        note: "",
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
        note: "",
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
        note: "",
    });

    // `--on`/`SWEETPAD_ON` outranks every destination layer at build time;
    // surface it so the shown context is the one a build will actually use.
    // The machine `value` stays the bare reference; the annotation is prose.
    if let Some(on) = &ctx.targeting.on {
        rows.push(Row {
            name: "on",
            value: Some(on.clone()),
            source: "flag/env",
            note: "overrides destination",
        });
    }

    if resolved.sdk.is_some() {
        let (value, source) = provenance(&resolved.sdk, &cfg.sdk, &pf_sdk, &st.sdk);
        rows.push(Row {
            name: "sdk",
            value,
            source,
            note: "",
        });
    }

    // A committed `[xcodebuild] args` shapes every build in this project from
    // a file the person running the command may never have opened. Show it, or
    // the difference it makes has no visible cause.
    let pf_xcodebuild = ctx
        .project_file(&resolved.container)
        .xcodebuild
        .args
        .clone();
    if !pf_xcodebuild.is_empty() {
        rows.push(Row {
            name: "xcodebuild",
            value: Some(pf_xcodebuild.join(" ")),
            source: "sweetpad.toml",
            note: "added to every build",
        });
    }

    let detached_log = detached_log_for(st.last_launched_app.as_ref());
    Ok(Rendered::data(StatusReport {
        container: resolved.container.path().display().to_string(),
        kind: match &resolved.container {
            Container::Workspace(_) => "workspace",
            Container::Project(_) => "project",
            Container::SwiftPackage(_) => "package",
        },
        rows,
        last_launched: st.last_launched_app,
        detached_log,
    }))
}

/// The captured stdout/stderr file of the last launch, when it was a macOS app
/// and the file is present — see [`StatusReport::detached_log`].
fn detached_log_for(app: Option<&LastLaunchedApp>) -> Option<String> {
    let app = app?;
    (app.kind == "macos")
        .then(|| super::app::detached_log_path(&app.bundle_identifier))
        .flatten()
        .filter(|p| p.exists())
        .map(|p| p.display().to_string())
}
