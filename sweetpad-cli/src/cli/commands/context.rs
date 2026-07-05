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
use crate::cli::{CliError, CommandResult, Context, ErrorKind, Render, Rendered, resolve, simctl};

/// A single context variable. `sdk` exists only in the build context and
/// `target` only in the testing context; the rest exist in both.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum Variable {
    /// The scheme to build/test.
    Scheme,
    /// The build configuration (e.g. Debug).
    Configuration,
    /// The -sdk override for builds and settings (build context only).
    Sdk,
    /// The -destination specifier (picked from the simulator list).
    Destination,
    /// The default test target `-only-testing:` narrows to (testing only).
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
    /// Set a context variable interactively.
    ///
    /// With no variable, sets the core three: scheme, configuration, and
    /// destination. Use `context set` to assign a value without a prompt.
    Select {
        /// Which variable to set; omit to set the core variables.
        variable: Option<Variable>,
        /// Act on the testing context instead of the build context.
        #[arg(long)]
        testing: bool,
    },
    /// Set a context variable to VALUE without prompting (script/CI-friendly).
    Set {
        /// Which variable to set.
        variable: Variable,
        /// The value to assign (validated for scheme/configuration).
        value: String,
        /// Act on the testing context instead of the build context.
        #[arg(long)]
        testing: bool,
    },
    /// Name a destination: `context alias work-phone <UDID>`, then
    /// `--on work-phone` anywhere.
    Alias {
        /// The alias name.
        name: String,
        /// Any `--on` reference (UDID, fuzzy name, booted, mac, …); omit with
        /// --remove.
        #[arg(required_unless_present = "remove")]
        reference: Option<String>,
        /// Remove the alias instead.
        #[arg(long, conflicts_with = "reference")]
        remove: bool,
    },
    /// Clear a saved context variable; `--all` clears the whole context.
    Remove {
        /// Which variable to clear.
        #[arg(required_unless_present = "all", conflicts_with = "all")]
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
        Action::Set {
            variable,
            value,
            testing,
        } => set(ctx, *variable, value, Scope::from_flag(*testing)),
        Action::Alias {
            name,
            reference,
            remove,
        } => alias(ctx, name, reference.as_deref(), *remove),
        Action::Remove {
            variable,
            all,
            testing,
        } => remove(ctx, *variable, *all, Scope::from_flag(*testing)),
    }
}

/// The saved build and testing context, recents, and last launched app. Renders
/// as the scope/recents/last-launched blocks — each variable with its effective
/// value *and provenance* (config file vs remembered state), so "why is it
/// building the wrong thing" is a glance, not a bug report. JSON:
/// `{container, build, testing, effective, recentDestinations, lastLaunchedApp}`.
struct ContextReport {
    container: String,
    state: ProjectState,
    /// The authored config defaults for this project — the layer above state.
    cfg: crate::cli::config::Defaults,
    /// The committed sweetpad.toml's targeting keys — between cfg and state.
    project: crate::cli::config::Defaults,
}

impl Render for ContextReport {
    fn human(&self, out: &Output) {
        let st = &self.state;
        print_scope(out, st, &self.cfg, &self.project, Scope::Build);
        if !st.testing.is_empty()
            || !testing_cfg_is_empty(&self.cfg)
            || !testing_cfg_is_empty(&self.project)
        {
            out.line("");
            print_scope(out, st, &self.cfg, &self.project, Scope::Testing);
        }
        print_recents(out, st);
        print_last_launched(out, st);
    }

    fn json(&self) -> serde_json::Value {
        let st = &self.state;
        serde_json::json!({ "container": self.container,
            "build": scope_json(st, Scope::Build),
            "testing": scope_json(st, Scope::Testing),
            "effective": {
                "build": effective_json(st, &self.cfg, &self.project, Scope::Build),
                "testing": effective_json(st, &self.cfg, &self.project, Scope::Testing),
            },
            "recentDestinations": recents_json(st),
            "lastLaunchedApp": serde_json::to_value(&st.last_launched_app).unwrap_or_default(),
        })
    }
}

/// Whether the config's testing sub-table pins anything for this project.
fn testing_cfg_is_empty(cfg: &crate::cli::config::Defaults) -> bool {
    cfg.testing.scheme.is_none()
        && cfg.testing.configuration.is_none()
        && cfg.testing.destination.is_none()
        && cfg.testing.target.is_none()
}

/// Build the context report for the resolved container — the payload both `show`
/// and `select` render after their work.
fn report(ctx: &mut Context) -> CommandResult {
    let container = resolve::container(ctx)?;
    let key = container.key();
    let state = ctx.state.projects.get(&key).cloned().unwrap_or_default();
    let cfg = ctx.config.for_project(&key);
    let pf = ctx.project_file(&container);
    let project = crate::cli::config::Defaults {
        scheme: pf.scheme.clone(),
        configuration: pf.configuration.clone(),
        destination: pf.destination.clone(),
        sdk: pf.sdk.clone(),
        testing: pf.testing.clone(),
    };
    Ok(Rendered::data(ContextReport {
        container: container.path().display().to_string(),
        state,
        cfg,
        project,
    }))
}

/// Show the saved build and testing context, recents, and last launched app.
fn show(ctx: &mut Context) -> CommandResult {
    report(ctx)
}

/// Non-interactive setter: `context set scheme MyApp` — the script/CI path the
/// prompt-only `select` can't serve. Scheme/configuration values are validated
/// against the project's candidates (with the build path's did-you-mean);
/// sdk/target/destination are taken verbatim (a destination may name hardware
/// that isn't currently attached).
fn set(ctx: &mut Context, v: Variable, value: &str, scope: Scope) -> CommandResult {
    let container = resolve::container(ctx)?;
    let key = container.key();
    ensure_in_scope(v, scope)?;
    match v {
        Variable::Scheme => {
            resolve::validate_choice("scheme", value, &resolve::schemes(&container)?)?;
        }
        Variable::Configuration => {
            let candidates = resolve::configurations(&container).unwrap_or_default();
            resolve::validate_choice("configuration", value, &candidates)?;
        }
        Variable::Sdk | Variable::Destination | Variable::Target => {}
    }
    set_field(
        ctx.state.project_mut(&key),
        scope,
        v,
        Some(value.to_string()),
    );
    ctx.state.save().map_err(CliError::new)?;
    report(ctx)
}

/// The `--on` reference forms an alias may not shadow: alias lookup runs
/// before the keyword ladder, so `context alias mac …` would silently
/// redefine `--on mac` for the project.
const RESERVED_ON_WORDS: [&str; 11] = [
    "mac", "macos", "booted", "device", "ios", "iphone", "ipad", "watchos", "tvos", "visionos",
    "xros",
];

/// Name (or drop) a destination alias for `--on`.
fn alias(ctx: &mut Context, name: &str, reference: Option<&str>, remove: bool) -> CommandResult {
    if !remove
        && RESERVED_ON_WORDS
            .iter()
            .any(|w| name.eq_ignore_ascii_case(w))
    {
        return Err(CliError::new(format!(
            "{name:?} is a built-in --on reference and can't be an alias name"
        )));
    }
    let key = resolve::container(ctx)?.key();
    let st = ctx.state.project_mut(&key);
    if remove {
        st.destination_aliases.remove(name);
    } else if let Some(reference) = reference {
        st.destination_aliases
            .insert(name.to_string(), reference.to_string());
    }
    prune(&mut ctx.state, &key);
    ctx.state.save().map_err(CliError::new)?;
    report(ctx)
}

/// Interactively set one variable (or the core), persisting to state, then show
/// the updated context.
fn select(ctx: &mut Context, variable: Option<Variable>, scope: Scope) -> CommandResult {
    // The generic strict-mode hint ("pass --scheme…") names flags this
    // command doesn't have — fail up front with the spelling that works
    // here.
    if !ctx.out.is_interactive() {
        return Err(CliError::new(
            "context select prompts interactively and the terminal is not interactive; use \
             `sweetpad context set <variable> <value>` instead",
        )
        .kind(ErrorKind::TargetResolution));
    }
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
        // Unreachable: clap requires VARIABLE unless --all (exit 2 up front).
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
            input(v.name(), current.as_deref(), ctx.out.use_color_stderr())
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

/// A scope's view of one Defaults-shaped table.
fn defaults_value(cfg: &crate::cli::config::Defaults, scope: Scope, v: Variable) -> Option<&str> {
    match scope {
        Scope::Build => match v {
            Variable::Scheme => cfg.scheme.as_deref(),
            Variable::Configuration => cfg.configuration.as_deref(),
            Variable::Sdk => cfg.sdk.as_deref(),
            Variable::Destination => cfg.destination.as_deref(),
            Variable::Target => None,
        },
        Scope::Testing => match v {
            Variable::Scheme => cfg.testing.scheme.as_deref(),
            Variable::Configuration => cfg.testing.configuration.as_deref(),
            Variable::Target => cfg.testing.target.as_deref(),
            Variable::Destination => cfg.testing.destination.as_deref(),
            Variable::Sdk => None,
        },
    }
}

/// The effective value of a variable in a scope, plus where it came from —
/// authored config beats the committed sweetpad.toml beats remembered state
/// (the same order the resolver uses; flags/env aren't inputs to
/// `context show` itself).
fn effective<'a>(
    st: &'a ProjectState,
    cfg: &'a crate::cli::config::Defaults,
    project: &'a crate::cli::config::Defaults,
    scope: Scope,
    v: Variable,
) -> (Option<&'a str>, &'static str) {
    if let Some(value) = defaults_value(cfg, scope, v) {
        return (Some(value), "config");
    }
    if let Some(value) = defaults_value(project, scope, v) {
        return (Some(value), "sweetpad.toml");
    }
    if let Some(value) = get(st, scope, v) {
        return (Some(value), "remembered");
    }
    // `sweetpad test` falls back to the *build* context for scheme /
    // configuration / destination (`resolve_testing`'s documented order), so
    // the effective view must show that value — reporting "(not set)" here
    // would tell a script the opposite of what `test` will actually use.
    if matches!(scope, Scope::Testing)
        && matches!(
            v,
            Variable::Scheme | Variable::Configuration | Variable::Destination
        )
    {
        let (value, source) = effective(st, cfg, project, Scope::Build, v);
        if value.is_some() {
            let source = match source {
                "config" => "build config",
                "sweetpad.toml" => "build sweetpad.toml",
                "remembered" => "build remembered",
                other => other,
            };
            return (value, source);
        }
    }
    (None, "")
}

fn print_scope(
    out: &Output,
    st: &ProjectState,
    cfg: &crate::cli::config::Defaults,
    project: &crate::cli::config::Defaults,
    scope: Scope,
) {
    out.line(scope.label());
    for v in scope_vars(scope) {
        let (value, source) = effective(st, cfg, project, scope, v);
        let source = if source.is_empty() {
            String::new()
        } else {
            format!("  ({source})")
        };
        out.line(&format!(
            "  {:<13} {}{source}",
            v.name(),
            value.unwrap_or("(not set)")
        ));
    }
}

/// The effective view for JSON: `{value, source}` per variable.
fn effective_json(
    st: &ProjectState,
    cfg: &crate::cli::config::Defaults,
    project: &crate::cli::config::Defaults,
    scope: Scope,
) -> serde_json::Value {
    let mut map = serde_json::Map::new();
    for v in scope_vars(scope) {
        let (value, source) = effective(st, cfg, project, scope, v);
        map.insert(
            v.name().to_string(),
            serde_json::json!({
                "value": value,
                "source": if source.is_empty() { None } else { Some(source) },
            }),
        );
    }
    serde_json::Value::Object(map)
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
