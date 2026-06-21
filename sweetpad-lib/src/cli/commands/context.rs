//! `sweetpad context …` — view and manage the project's remembered build
//! context: the scheme, configuration, sdk, and destination that `build` and
//! `app` reuse, plus a separate testing context (`--testing`) for `test`, the
//! recently-used destinations, and the last launched app. These live in the
//! machine-managed state file ([`crate::cli::state`]); this command is the
//! first-class way to inspect and change them, instead of hand-editing that
//! file or relying on a build command's prompt-and-remember side effect.

use clap::{Subcommand, ValueEnum};

use crate::cli::output::Output;
use crate::cli::resolve::Container;
use crate::cli::state::{ProjectState, State, TestingState};
use crate::cli::{
    CliError, CommandResult, Context, ErrorKind, Render, Rendered, resolve, simctl,
};

/// A single context variable. `sdk` exists only in the build context and
/// `target` only in the testing context; the rest exist in both.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum Variable {
    Scheme,
    Configuration,
    Sdk,
    Destination,
    Target,
}

impl Variable {
    /// The variables `select` sets when given no specific one: the common core,
    /// shared by both contexts. `sdk`/`target` are set only when named.
    const CORE: [Variable; 3] = [
        Variable::Scheme,
        Variable::Configuration,
        Variable::Destination,
    ];

    /// The variable's name, as shown and as the `select`/`remove` argument.
    fn name(self) -> &'static str {
        match self {
            Variable::Scheme => "scheme",
            Variable::Configuration => "configuration",
            Variable::Sdk => "sdk",
            Variable::Destination => "destination",
            Variable::Target => "target",
        }
    }

    /// Whether this variable belongs to the given context.
    fn in_scope(self, scope: Scope) -> bool {
        match scope {
            Scope::Build => self != Variable::Target,
            Scope::Testing => self != Variable::Sdk,
        }
    }
}

/// Which context a `select`/`remove` acts on.
#[derive(Debug, Clone, Copy)]
enum Scope {
    Build,
    Testing,
}

impl Scope {
    fn from_flag(testing: bool) -> Self {
        if testing {
            Scope::Testing
        } else {
            Scope::Build
        }
    }

    fn label(self) -> &'static str {
        match self {
            Scope::Build => "build",
            Scope::Testing => "testing",
        }
    }
}

#[derive(Debug, Subcommand)]
pub enum Action {
    /// Show the project's saved build and testing context.
    Show,
    /// Set a context variable interactively; with no variable, set the core
    /// (scheme, configuration, destination).
    Select {
        /// Which variable to set; omit to set the core variables.
        variable: Option<Variable>,
        /// Act on the testing context instead of the build context.
        #[arg(long)]
        testing: bool,
    },
    /// Clear a saved context variable; `--all` clears the whole context.
    Remove {
        /// Which variable to clear.
        variable: Option<Variable>,
        /// Clear the entire context (the project entry, or just the testing
        /// sub-context with `--testing`).
        #[arg(long)]
        all: bool,
        /// Act on the testing context instead of the build context.
        #[arg(long)]
        testing: bool,
    },
}

pub fn run(ctx: &mut Context, action: &Action) -> CommandResult {
    match action {
        Action::Show => show(ctx),
        Action::Select { variable, testing } => select(ctx, *variable, Scope::from_flag(*testing)),
        Action::Remove {
            variable,
            all,
            testing,
        } => remove(ctx, *variable, *all, Scope::from_flag(*testing)),
    }
}

/// The saved build and testing context, recents, and last launched app. Renders
/// as the scope/recents/last-launched blocks, or
/// `{container, build, testing, recentDestinations, lastLaunchedApp}` in the JSON
/// envelope. Carries the container path and a snapshot of the project state.
struct ContextReport {
    container: String,
    state: ProjectState,
}

impl Render for ContextReport {
    fn human(&self, out: &Output) {
        let st = &self.state;
        print_scope(out, st, Scope::Build);
        if !st.testing.is_empty() {
            out.line("");
            print_scope(out, st, Scope::Testing);
        }
        print_recents(out, st);
        print_last_launched(out, st);
    }

    fn json(&self) -> serde_json::Value {
        let st = &self.state;
        serde_json::json!({ "container": self.container,
            "build": scope_json(st, Scope::Build),
            "testing": scope_json(st, Scope::Testing),
            "recentDestinations": recents_json(st),
            "lastLaunchedApp": serde_json::to_value(&st.last_launched_app).unwrap_or_default(),
        })
    }
}

/// Build the context report for the resolved container — the payload both `show`
/// and `select` render after their work.
fn report(ctx: &mut Context) -> CommandResult {
    let container = resolve::container(ctx)?;
    let key = container.key();
    let state = ctx.state.projects.get(&key).cloned().unwrap_or_default();
    Ok(Rendered::data(ContextReport {
        container: container.path().display().to_string(),
        state,
    }))
}

/// Show the saved build and testing context, recents, and last launched app.
fn show(ctx: &mut Context) -> CommandResult {
    report(ctx)
}

/// Interactively set one variable (or the core), persisting to state, then show
/// the updated context.
fn select(ctx: &mut Context, variable: Option<Variable>, scope: Scope) -> CommandResult {
    let container = resolve::container(ctx)?;
    let key = container.key();

    let vars: Vec<Variable> = match variable {
        Some(v) => {
            ensure_in_scope(v, scope)?;
            vec![v]
        }
        None => Variable::CORE.to_vec(),
    };
    for v in vars {
        let value = prompt_value(ctx, &container, &key, scope, v)?;
        set_field(ctx.state.project_mut(&key), scope, v, Some(value));
    }
    ctx.state.save().map_err(CliError::new)?;
    report(ctx)
}

/// What `remove` cleared, for rendering the note and the JSON `removed` field.
struct RemoveReport {
    /// The whole context was cleared (`--all`); otherwise a single variable.
    cleared_all: bool,
    /// The variable cleared, when not `--all`.
    variable: Option<&'static str>,
    scope: &'static str,
}

impl Render for RemoveReport {
    fn human(&self, out: &Output) {
        if self.cleared_all {
            out.note(&format!("cleared the {} context", self.scope));
        } else if let Some(v) = self.variable {
            out.note(&format!("removed {} {}", self.scope, v));
        }
    }

    fn json(&self) -> serde_json::Value {
        let removed = if self.cleared_all {
            "all"
        } else {
            self.variable.unwrap_or_default()
        };
        serde_json::json!({ "removed": removed, "scope": self.scope })
    }
}

/// Clear one variable, or the whole context with `--all`.
fn remove(ctx: &mut Context, variable: Option<Variable>, all: bool, scope: Scope) -> CommandResult {
    let key = resolve::container(ctx)?.key();

    if all {
        match scope {
            // Clearing "all" of the build context drops the whole entry —
            // recents, usage, and last-launched included.
            Scope::Build => {
                ctx.state.projects.remove(&key);
            }
            Scope::Testing => {
                if let Some(st) = ctx.state.projects.get_mut(&key) {
                    st.testing = TestingState::default();
                }
                prune(&mut ctx.state, &key);
            }
        }
        ctx.state.save().map_err(CliError::new)?;
        return Ok(Rendered::data(RemoveReport {
            cleared_all: true,
            variable: None,
            scope: scope.label(),
        }));
    }

    let Some(v) = variable else {
        return Err(CliError::new(
            "specify a variable (scheme, configuration, sdk, destination, target) or --all",
        ));
    };
    ensure_in_scope(v, scope)?;
    if let Some(st) = ctx.state.projects.get_mut(&key) {
        set_field(st, scope, v, None);
    }
    prune(&mut ctx.state, &key);
    ctx.state.save().map_err(CliError::new)?;
    Ok(Rendered::data(RemoveReport {
        cleared_all: false,
        variable: Some(v.name()),
        scope: scope.label(),
    }))
}

/// Reject a variable that doesn't belong to the chosen context.
fn ensure_in_scope(v: Variable, scope: Scope) -> Result<(), CliError> {
    if v.in_scope(scope) {
        Ok(())
    } else {
        Err(CliError::new(format!(
            "{} is not a {} context variable",
            v.name(),
            scope.label()
        )))
    }
}

/// Prompt for a variable's value using the same pickers the build flow uses;
/// `sdk`/`target` have no candidate list, so they're free-text (prefilled).
fn prompt_value(
    ctx: &mut Context,
    container: &Container,
    key: &str,
    scope: Scope,
    v: Variable,
) -> Result<String, CliError> {
    match v {
        Variable::Scheme => {
            let candidates = resolve::schemes(container)?;
            resolve::choose(ctx, "scheme", None, &candidates)
        }
        Variable::Configuration => {
            let candidates = resolve::configurations(container)?;
            resolve::choose(ctx, "configuration", None, &candidates)
        }
        Variable::Destination => resolve::pick_destination(ctx, key, &simctl::list()?),
        Variable::Sdk | Variable::Target => {
            if !ctx.out.is_interactive() {
                return Err(resolve::missing(v.name()));
            }
            let current = ctx
                .state
                .projects
                .get(key)
                .and_then(|st| get(st, scope, v).map(String::from));
            input(v.name(), current.as_deref(), ctx.out.use_color())
        }
    }
}

/// A free-text prompt prefilled with the current value (for `sdk`/`target`).
/// Honors `--no-color` with a plain theme.
fn input(label: &str, current: Option<&str>, color: bool) -> Result<String, CliError> {
    use dialoguer::theme::{ColorfulTheme, SimpleTheme, Theme};
    let colorful = ColorfulTheme::default();
    let simple = SimpleTheme;
    let theme: &dyn Theme = if color { &colorful } else { &simple };
    let mut builder =
        dialoguer::Input::<String>::with_theme(theme).with_prompt(format!("Enter {label}"));
    if let Some(c) = current {
        builder = builder.with_initial_text(c);
    }
    builder
        .interact_text()
        .map_err(|e| CliError::new(format!("input cancelled: {e}")).kind(ErrorKind::UserCancel))
}

/// Read a variable's value in the given context.
fn get(st: &ProjectState, scope: Scope, v: Variable) -> Option<&str> {
    match scope {
        Scope::Build => match v {
            Variable::Scheme => st.scheme.as_deref(),
            Variable::Configuration => st.configuration.as_deref(),
            Variable::Sdk => st.sdk.as_deref(),
            Variable::Destination => st.destination.as_deref(),
            Variable::Target => None,
        },
        Scope::Testing => match v {
            Variable::Scheme => st.testing.scheme.as_deref(),
            Variable::Configuration => st.testing.configuration.as_deref(),
            Variable::Target => st.testing.target.as_deref(),
            Variable::Destination => st.testing.destination.as_deref(),
            Variable::Sdk => None,
        },
    }
}

/// Write a variable's value (or clear it with `None`) in the given context.
fn set_field(st: &mut ProjectState, scope: Scope, v: Variable, value: Option<String>) {
    match scope {
        Scope::Build => match v {
            Variable::Scheme => st.scheme = value,
            Variable::Configuration => st.configuration = value,
            Variable::Sdk => st.sdk = value,
            Variable::Destination => st.destination = value,
            Variable::Target => {}
        },
        Scope::Testing => match v {
            Variable::Scheme => st.testing.scheme = value,
            Variable::Configuration => st.testing.configuration = value,
            Variable::Target => st.testing.target = value,
            Variable::Destination => st.testing.destination = value,
            Variable::Sdk => {}
        },
    }
}

/// Drop a project entry once nothing is left in it.
fn prune(state: &mut State, key: &str) {
    if state.projects.get(key).is_some_and(ProjectState::is_empty) {
        state.projects.remove(key);
    }
}

/// The variables shown for a context, in display order.
fn scope_vars(scope: Scope) -> [Variable; 4] {
    match scope {
        Scope::Build => [
            Variable::Scheme,
            Variable::Configuration,
            Variable::Sdk,
            Variable::Destination,
        ],
        Scope::Testing => [
            Variable::Scheme,
            Variable::Configuration,
            Variable::Target,
            Variable::Destination,
        ],
    }
}

fn print_scope(out: &Output, st: &ProjectState, scope: Scope) {
    out.line(scope.label());
    for v in scope_vars(scope) {
        let shown = get(st, scope, v).unwrap_or("(not set)");
        out.line(&format!("  {:<13} {shown}", v.name()));
    }
}

fn print_recents(out: &Output, st: &ProjectState) {
    if st.destination_recents.is_empty() {
        return;
    }
    // Most-used first, matching the picker order.
    let mut recents: Vec<_> = st.destination_recents.iter().collect();
    recents
        .sort_by_key(|d| std::cmp::Reverse(st.destination_usage.get(&d.id).copied().unwrap_or(0)));
    out.line("");
    out.line("recent destinations");
    for d in recents {
        let uses = st.destination_usage.get(&d.id).copied().unwrap_or(0);
        out.line(&format!("  {} ({uses})", d.name));
    }
}

fn print_last_launched(out: &Output, st: &ProjectState) {
    if let Some(app) = &st.last_launched_app {
        out.line("");
        out.line("last launched");
        out.line(&format!("  {} {}", app.kind, app.bundle_identifier));
        out.line(&format!("  {}", app.app_path));
    }
}

fn scope_json(st: &ProjectState, scope: Scope) -> serde_json::Value {
    let mut map = serde_json::Map::new();
    for v in scope_vars(scope) {
        map.insert(v.name().to_string(), get(st, scope, v).into());
    }
    serde_json::Value::Object(map)
}

fn recents_json(st: &ProjectState) -> serde_json::Value {
    let items: Vec<serde_json::Value> = st
        .destination_recents
        .iter()
        .map(|d| {
            serde_json::json!({
                "id": d.id,
                "kind": d.kind,
                "name": d.name,
                "usage": st.destination_usage.get(&d.id).copied().unwrap_or(0),
            })
        })
        .collect();
    serde_json::Value::Array(items)
}

#[cfg(test)]
mod tests {
    use super::*;

    const KEY: &str = "/proj";

    fn project(state: &mut State) -> &mut ProjectState {
        state.project_mut(KEY)
    }

    #[test]
    fn set_field_routes_to_the_right_context() {
        let mut state = State::default();
        set_field(
            project(&mut state),
            Scope::Build,
            Variable::Scheme,
            Some("App".into()),
        );
        set_field(
            project(&mut state),
            Scope::Testing,
            Variable::Scheme,
            Some("AppTests".into()),
        );
        let st = state.projects.get(KEY).unwrap();
        assert_eq!(get(st, Scope::Build, Variable::Scheme), Some("App"));
        assert_eq!(get(st, Scope::Testing, Variable::Scheme), Some("AppTests"));
    }

    #[test]
    fn sdk_and_target_are_scoped() {
        assert!(Variable::Sdk.in_scope(Scope::Build));
        assert!(!Variable::Sdk.in_scope(Scope::Testing));
        assert!(Variable::Target.in_scope(Scope::Testing));
        assert!(!Variable::Target.in_scope(Scope::Build));
    }

    #[test]
    fn prune_drops_an_emptied_entry_but_keeps_recents() {
        let mut state = State::default();
        set_field(
            project(&mut state),
            Scope::Build,
            Variable::Scheme,
            Some("App".into()),
        );
        set_field(project(&mut state), Scope::Build, Variable::Scheme, None);
        prune(&mut state, KEY);
        assert!(!state.projects.contains_key(KEY));

        // An entry with only recents is not pruned.
        project(&mut state)
            .destination_usage
            .insert("UDID".into(), 1);
        prune(&mut state, KEY);
        assert!(state.projects.contains_key(KEY));
    }
}
