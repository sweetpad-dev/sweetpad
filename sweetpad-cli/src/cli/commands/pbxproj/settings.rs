//! `sweetpad pbxproj settings …` — the *stored* settings layer: show, set,
//! and unset raw `XCBuildConfiguration.buildSettings` entries (CLI_DESIGN
//! §9f/§9g).
//!
//! `show` here prints what the file says, per configuration — the layer
//! `set`/`unset` edit. The everyday question ("what will the build use") is
//! porcelain: top-level `sweetpad settings show`, which resolves xcconfig
//! files, SDK defaults, and `$(inherited)` chains on top of this layer —
//! and is why a value can survive an `unset` in the resolved view.
//!
//! Mutations ride [`sweetpad_lib::settings_pbxproj`] and never guess: an
//! ambiguous workspace, unknown target, or unknown configuration is a hard
//! error — interactive or not. Setting `INFOPLIST_FILE` to a path inside a
//! target's synchronized folder also records the membership exception Xcode
//! itself would write (without it the plist doubles as a bundle resource — a
//! "Multiple commands produce" build failure on flat-bundle platforms).

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::Path;

use clap::{Args, Subcommand};

use crate::cli::output::Output;
use crate::cli::pbxedit;
use crate::cli::{CliError, CommandResult, ContainerArgs, Context, Render, Rendered};
use sweetpad_core::build_settings::{BuildSettingsOptions, resolve_build_settings};
use sweetpad_lib::settings_pbxproj::{self, Assignment, Change, Op, Scope, Setting};
use sweetpad_lib::sync_pbxproj::{self, ExcludeOutcome};

#[derive(Debug, Subcommand)]
pub enum Action {
    /// Show the stored `buildSettings` entries, per configuration.
    Show {
        #[command(flatten)]
        container: ContainerArgs,

        /// Show one build target's configurations instead of the project
        /// level plus every target.
        #[arg(long)]
        target: Option<String>,

        /// Show only this one setting key — printed as the bare value(s).
        #[arg(long)]
        key: Option<String>,
    },
    /// Set stored build settings.
    Set(SetArgs),
    /// Remove stored build settings.
    Unset(UnsetArgs),
}

/// Flags for `pbxproj settings set`.
#[derive(Debug, Args)]
pub struct SetArgs {
    /// Assignments: `KEY=VALUE` sets, `KEY+=VALUE` appends to the value's
    /// element list, and repeating a key builds an array in argument order.
    /// Conditional keys (`KEY[sdk=iphoneos*]`) pass through verbatim.
    #[arg(required = true, value_name = "KEY=VALUE")]
    pub assignments: Vec<String>,

    #[command(flatten)]
    pub container: ContainerArgs,

    /// Build target whose configurations to edit (repeatable). Without it,
    /// the project-level configurations — inherited by every target — are
    /// edited.
    #[arg(long = "target", value_name = "TARGET")]
    pub targets: Vec<String>,

    /// Configuration to touch (repeatable; default: all of them).
    #[arg(long = "configuration", value_name = "NAME")]
    pub configurations: Vec<String>,

    /// Edit a generated project (XcodeGen/Tuist) anyway — the change is
    /// deliberate and will be lost on the next regenerate.
    #[arg(long)]
    pub force: bool,
}

/// Flags for `pbxproj settings unset`.
#[derive(Debug, Args)]
pub struct UnsetArgs {
    /// Keys to remove. Exact match only — conditional variants
    /// (`KEY[sdk=…]`) are separate keys and stay untouched.
    #[arg(required = true, value_name = "KEY")]
    pub keys: Vec<String>,

    #[command(flatten)]
    pub container: ContainerArgs,

    /// Build target whose configurations to edit (repeatable). Without it,
    /// the project-level configurations are edited.
    #[arg(long = "target", value_name = "TARGET")]
    pub targets: Vec<String>,

    /// Configuration to touch (repeatable; default: all of them).
    #[arg(long = "configuration", value_name = "NAME")]
    pub configurations: Vec<String>,

    /// Edit a generated project (XcodeGen/Tuist) anyway — the change is
    /// deliberate and will be lost on the next regenerate.
    #[arg(long)]
    pub force: bool,
}

pub fn run(ctx: &mut Context, action: &Action) -> CommandResult {
    match action {
        Action::Show {
            container,
            target,
            key,
        } => show(ctx, container, target.as_deref(), key.as_deref()),
        Action::Set(args) => set(ctx, args),
        Action::Unset(args) => unset(ctx, args),
    }
}

/// One reported edit: raw before/after for a (scope, configuration, key).
struct ChangeRow {
    target: Option<String>,
    configuration: String,
    key: String,
    old: Option<Setting>,
    new: Option<Setting>,
}

/// A membership exception recorded alongside an `INFOPLIST_FILE` set.
struct ExceptionRow {
    target: String,
    root_dir: String,
    exception: String,
    already: bool,
}

/// A post-write re-resolution of a touched key — the *effect* of the edit.
struct ResolvedRow {
    target: String,
    configuration: String,
    key: String,
    value: Option<String>,
}

/// The `set`/`unset` report: per-configuration before → after, the recorded
/// Info.plist exceptions, xcspec/xcconfig warnings, and the re-resolved
/// values (JSON only — human mode shows the raw edit).
struct MutationResult {
    file: String,
    unset: bool,
    changes: Vec<ChangeRow>,
    exceptions: Vec<ExceptionRow>,
    warnings: Vec<String>,
    resolved: Vec<ResolvedRow>,
}

impl Render for MutationResult {
    fn human(&self, out: &Output) {
        out.line(&self.file);
        for c in &self.changes {
            let scope = match &c.target {
                Some(t) => format!("{t}/{}", c.configuration),
                None => c.configuration.clone(),
            };
            let line = match (&c.old, &c.new) {
                (None, Some(new)) => format!("  {scope}: {} = {}", c.key, new.display()),
                (Some(old), Some(new)) => format!(
                    "  {scope}: {} = {} (was {})",
                    c.key,
                    new.display(),
                    old.display()
                ),
                (Some(old), None) => {
                    format!("  {scope}: {} removed (was {})", c.key, old.display())
                }
                (None, None) => format!("  {scope}: {} was not set", c.key),
            };
            out.line(&line);
        }
        for e in &self.exceptions {
            if e.already {
                continue;
            }
            out.note(&format!(
                "excepted {} from synchronized folder {} (target {}) so the custom \
                 Info.plist isn't also copied as a resource",
                e.exception, e.root_dir, e.target
            ));
        }
        for w in &self.warnings {
            out.warn(w);
        }
    }

    fn json(&self) -> serde_json::Value {
        let changes: Vec<serde_json::Value> = self
            .changes
            .iter()
            .map(|c| {
                serde_json::json!({
                    "target": c.target,
                    "configuration": c.configuration,
                    "key": c.key,
                    "old": setting_json(c.old.as_ref()),
                    "new": setting_json(c.new.as_ref()),
                })
            })
            .collect();
        let exceptions: Vec<serde_json::Value> = self
            .exceptions
            .iter()
            .map(|e| {
                serde_json::json!({
                    "target": e.target,
                    "root": e.root_dir,
                    "exception": e.exception,
                    "alreadyExcluded": e.already,
                })
            })
            .collect();
        let resolved: Vec<serde_json::Value> = self
            .resolved
            .iter()
            .map(|r| {
                serde_json::json!({
                    "target": r.target,
                    "configuration": r.configuration,
                    "key": r.key,
                    "value": r.value,
                })
            })
            .collect();
        serde_json::json!({
            "file": self.file,
            "action": if self.unset { "unset" } else { "set" },
            "changes": changes,
            "exceptions": exceptions,
            "resolved": resolved,
            "warnings": self.warnings,
        })
    }
}

fn setting_json(setting: Option<&Setting>) -> serde_json::Value {
    match setting {
        None => serde_json::Value::Null,
        Some(Setting::String(s)) => serde_json::Value::String(s.clone()),
        Some(Setting::List(items)) => serde_json::Value::Array(
            items
                .iter()
                .cloned()
                .map(serde_json::Value::String)
                .collect(),
        ),
    }
}

fn set(ctx: &mut Context, args: &SetArgs) -> CommandResult {
    ctx.targeting = args.container.clone().into();
    let container = crate::cli::resolve::container(ctx)?;
    let xcodeproj = pbxedit::mutation_xcodeproj(ctx, &container, &args.targets)?;
    pbxedit::guard_generated(ctx.project_file(&container), &xcodeproj, args.force)?;
    let mut root = pbxedit::parse_owned(&xcodeproj)?;

    let assignments = parse_assignments(&args.assignments)?;
    let scopes = scopes_of(&args.targets);
    let mut changes = Vec::new();
    for scope in &scopes {
        changes.extend(
            settings_pbxproj::set(&mut root, scope, &args.configurations, &assignments)
                .map_err(CliError::new)?,
        );
    }

    // A custom Info.plist inside a synchronized folder needs a membership
    // exception (verified live — a hard build failure on iOS without it). A
    // project-level set applies to every target, so each one is checked.
    let mut exceptions = Vec::new();
    if let Some(plist) = assigned_value(&assignments, "INFOPLIST_FILE") {
        let affected: Vec<String> = if args.targets.is_empty() {
            settings_pbxproj::target_names(&root)
        } else {
            args.targets.clone()
        };
        for target in affected {
            let outcome = sync_pbxproj::ensure_infoplist_exception(&mut root, &target, &plist)
                .map_err(CliError::new)?;
            match outcome {
                Some(ExcludeOutcome::Added {
                    root_dir,
                    exception,
                }) => exceptions.push(ExceptionRow {
                    target,
                    root_dir,
                    exception,
                    already: false,
                }),
                Some(ExcludeOutcome::AlreadyExcluded {
                    root_dir,
                    exception,
                }) => exceptions.push(ExceptionRow {
                    target,
                    root_dir,
                    exception,
                    already: true,
                }),
                None => {}
            }
        }
    }

    let mut warnings = xcspec_warnings(&assignments);
    let keys: Vec<String> = assignments.iter().map(|a| a.key.clone()).collect();
    warnings.extend(xcconfig_warnings(
        &root,
        &xcodeproj,
        &scopes,
        &args.configurations,
        &keys,
    ));

    pbxedit::write_pbxproj(&xcodeproj, &root)?;

    let resolved = resolve_effects(
        &xcodeproj,
        &root,
        &args.targets,
        &changes,
        &keys,
        &mut warnings,
    );
    Ok(Rendered::data(MutationResult {
        file: xcodeproj.join("project.pbxproj").display().to_string(),
        unset: false,
        changes: changes.into_iter().map(change_row).collect(),
        exceptions,
        warnings,
        resolved,
    }))
}

fn unset(ctx: &mut Context, args: &UnsetArgs) -> CommandResult {
    ctx.targeting = args.container.clone().into();
    let container = crate::cli::resolve::container(ctx)?;
    let xcodeproj = pbxedit::mutation_xcodeproj(ctx, &container, &args.targets)?;
    pbxedit::guard_generated(ctx.project_file(&container), &xcodeproj, args.force)?;
    let mut root = pbxedit::parse_owned(&xcodeproj)?;

    let keys = parse_keys(&args.keys)?;
    let scopes = scopes_of(&args.targets);
    let mut changes = Vec::new();
    for scope in &scopes {
        changes.extend(
            settings_pbxproj::unset(&mut root, scope, &args.configurations, &keys)
                .map_err(CliError::new)?,
        );
    }

    let mut warnings = xcconfig_warnings(&root, &xcodeproj, &scopes, &args.configurations, &keys);

    let touched = changes.iter().any(|c| c.old.is_some());
    if touched {
        pbxedit::write_pbxproj(&xcodeproj, &root)?;
    }

    let resolved = if touched {
        resolve_effects(
            &xcodeproj,
            &root,
            &args.targets,
            &changes,
            &keys,
            &mut warnings,
        )
    } else {
        Vec::new()
    };
    Ok(Rendered::data(MutationResult {
        file: xcodeproj.join("project.pbxproj").display().to_string(),
        unset: true,
        changes: changes.into_iter().map(change_row).collect(),
        exceptions: Vec::new(),
        warnings,
        resolved,
    }))
}

fn change_row(c: Change) -> ChangeRow {
    ChangeRow {
        target: c.target,
        configuration: c.configuration,
        key: c.key,
        old: c.old,
        new: c.new,
    }
}

fn scopes_of(targets: &[String]) -> Vec<Scope> {
    if targets.is_empty() {
        vec![Scope::Project]
    } else {
        targets.iter().cloned().map(Scope::Target).collect()
    }
}

/// The effective (last) value assigned to `key`, for the Info.plist hook.
fn assigned_value(assignments: &[Assignment], key: &str) -> Option<String> {
    let assignment = assignments.iter().find(|a| a.key == key)?;
    match &assignment.op {
        Op::Assign(values) | Op::Append(values) => values.last().cloned(),
    }
}

/// Parse and fold the positional `KEY=VALUE` / `KEY+=VALUE` pairs: one
/// [`Assignment`] per key, repeated occurrences extending its element list in
/// argument order. The key/value split honors bracket conditionals, so
/// `KEY[sdk=iphoneos*]=v` splits after the `]`.
fn parse_assignments(inputs: &[String]) -> Result<Vec<Assignment>, CliError> {
    let mut order: Vec<Assignment> = Vec::new();
    let mut index: HashMap<String, usize> = HashMap::new();
    for input in inputs {
        let (key, append, value) = split_assignment(input)?;
        if let Some(&i) = index.get(&key) {
            match &mut order[i].op {
                Op::Assign(values) | Op::Append(values) => values.push(value),
            }
        } else {
            index.insert(key.clone(), order.len());
            let op = if append {
                Op::Append(vec![value])
            } else {
                Op::Assign(vec![value])
            };
            order.push(Assignment { key, op });
        }
    }
    Ok(order)
}

/// Split one `KEY=VALUE` / `KEY+=VALUE` argument at the first `=` outside
/// `[…]` conditionals. Returns `(key, is_append, value)`.
fn split_assignment(input: &str) -> Result<(String, bool, String), CliError> {
    let mut depth = 0usize;
    for (i, c) in input.char_indices() {
        match c {
            '[' => depth += 1,
            ']' => depth = depth.saturating_sub(1),
            '=' if depth == 0 => {
                let (raw_key, append) = match input[..i].strip_suffix('+') {
                    Some(key) => (key, true),
                    None => (&input[..i], false),
                };
                let key = raw_key.trim();
                validate_key(key, input)?;
                return Ok((key.to_string(), append, input[i + 1..].to_string()));
            }
            _ => {}
        }
    }
    Err(CliError::new(format!(
        "`{input}` is not a KEY=VALUE assignment (use KEY=VALUE, or KEY+=VALUE to append)"
    )))
}

fn parse_keys(inputs: &[String]) -> Result<Vec<String>, CliError> {
    inputs
        .iter()
        .map(|k| {
            let key = k.trim();
            // A bare `=` marks a stray value; one inside `[…]` is a
            // conditional (`KEY[sdk=iphoneos*]`) and part of the key.
            let mut depth = 0usize;
            for c in key.chars() {
                match c {
                    '[' => depth += 1,
                    ']' => depth = depth.saturating_sub(1),
                    '=' if depth == 0 => {
                        return Err(CliError::new(format!(
                            "`{k}` names a key to remove — pass the key only, without a value"
                        )));
                    }
                    _ => {}
                }
            }
            validate_key(key, k)?;
            Ok(key.to_string())
        })
        .collect()
}

fn validate_key(key: &str, input: &str) -> Result<(), CliError> {
    if key.is_empty() {
        return Err(CliError::new(format!(
            "`{input}` has no setting key before the `=`"
        )));
    }
    if key.chars().any(char::is_whitespace) {
        return Err(CliError::new(format!(
            "setting key `{key}` contains whitespace"
        )));
    }
    Ok(())
}

/// Warn when a known key is set to a value outside the domain Xcode's xcspec
/// enumerates for it. The domain is the option's declared `Values` list
/// (Enumerations like `SWIFT_OPTIMIZATION_LEVEL`) unioned with its
/// `CommandLineArgs` map keys when that map is closed — the map admits legacy
/// spellings (`-Owholemodule`) the `Values` list omits. Warnings only —
/// unknown keys are legal (user-defined settings), and xcspec coverage isn't
/// total. Values with `$(…)` references are skipped: their final form isn't
/// knowable here.
fn xcspec_warnings(assignments: &[Assignment]) -> Vec<String> {
    let Ok(catalog) = sweetpad_lib::catalog_cache::embedded() else {
        return Vec::new();
    };
    let mut domains: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();
    for options in catalog.compiler_options.values() {
        for option in options {
            let mut domain: BTreeSet<&str> = option.values.iter().map(String::as_str).collect();
            if let Some(sweetpad_lib::xcspec::CliArgs::ByValue { map, otherwise }) = &option.args {
                // An open map (`<<otherwise>>`) constrains nothing by itself,
                // but its keys still extend an explicit `Values` domain.
                if otherwise.is_none() || !domain.is_empty() {
                    domain.extend(map.keys().map(String::as_str));
                }
                if otherwise.is_some() && option.values.is_empty() {
                    domain.clear();
                }
            }
            if !domain.is_empty() {
                domains
                    .entry(option.name.as_str())
                    .or_default()
                    .extend(domain);
            }
        }
    }
    let mut warnings = Vec::new();
    for assignment in assignments {
        let base = assignment.key.split('[').next().unwrap_or(&assignment.key);
        let Some(allowed) = domains.get(base) else {
            continue;
        };
        let (Op::Assign(values) | Op::Append(values)) = &assignment.op;
        // Enum/Boolean settings are scalars; a multi-element list is a
        // different shape entirely and gets no verdict.
        let [value] = values.as_slice() else { continue };
        if value.contains("$(") || allowed.contains(value.as_str()) {
            continue;
        }
        let mut expected: Vec<&str> = allowed.iter().copied().collect();
        expected.sort_unstable();
        warnings.push(format!(
            "{base} = {value} is not a value Xcode's xcspec lists for it \
             (expected one of: {})",
            expected.join(", ")
        ));
    }
    warnings
}

/// Warn when a touched configuration's base xcconfig also assigns one of the
/// edited keys: the pbxproj layer outranks it, so a `set` silently shadows
/// the xcconfig value and an `unset` hands the wheel back to it.
fn xcconfig_warnings(
    root: &sweetpad_lib::pbxproj::Value,
    xcodeproj: &Path,
    scopes: &[Scope],
    configurations: &[String],
    keys: &[String],
) -> Vec<String> {
    let project_dir = xcodeproj.parent().unwrap_or_else(|| Path::new("."));
    let mut warnings = Vec::new();
    let mut seen: BTreeSet<(String, String)> = BTreeSet::new();
    for scope in scopes {
        let Ok(bases) = settings_pbxproj::base_xcconfigs(root, scope, configurations) else {
            continue;
        };
        for (configuration, rel_path) in bases {
            let Ok(xcconfig) = sweetpad_lib::xcconfig::parse_file(&project_dir.join(&rel_path))
            else {
                continue;
            };
            for key in keys {
                let base_key = key.split('[').next().unwrap_or(key);
                let assigns = xcconfig.entries.iter().any(|e| {
                    matches!(e, sweetpad_lib::xcconfig::Entry::Assignment(a) if a.key == base_key)
                });
                if assigns && seen.insert((rel_path.clone(), key.clone())) {
                    warnings.push(format!(
                        "{configuration}'s base xcconfig ({rel_path}) also assigns {key}; \
                         the pbxproj layer outranks it"
                    ));
                }
            }
        }
    }
    warnings
}

/// Re-resolve the touched keys after the write — the *effect* of the edit,
/// per (target, configuration). A project-level edit applies to every target.
/// Resolution failures degrade to a warning; the mutation itself stands.
fn resolve_effects(
    xcodeproj: &Path,
    root: &sweetpad_lib::pbxproj::Value,
    targets: &[String],
    changes: &[Change],
    keys: &[String],
    warnings: &mut Vec<String>,
) -> Vec<ResolvedRow> {
    let affected: Vec<String> = if targets.is_empty() {
        settings_pbxproj::target_names(root)
    } else {
        targets.to_vec()
    };
    let configurations: BTreeSet<&String> = changes.iter().map(|c| &c.configuration).collect();
    let mut rows = Vec::new();
    for target in &affected {
        for configuration in &configurations {
            let opts = BuildSettingsOptions {
                project: Some(xcodeproj.to_path_buf()),
                workspace: None,
                scheme: None,
                target: Some(target.clone()),
                configuration: (*configuration).clone(),
                sdk: String::new(),
                arch: String::new(),
                destination: None,
                xcconfig: None,
                xcode: None,
                xcspec_root: None,
                sdksettings_root: None,
                catalog_cache: None,
                derived_data_path: None,
                read_xcode_locations: true,
                keys: Some(keys.to_vec()),
            };
            match resolve_build_settings(&opts) {
                Ok(resolved) => {
                    for t in resolved {
                        for key in keys {
                            rows.push(ResolvedRow {
                                target: t.target.clone(),
                                configuration: (*configuration).clone(),
                                key: key.clone(),
                                value: t.settings.get(key).cloned(),
                            });
                        }
                    }
                }
                Err(e) => {
                    warnings.push(format!(
                        "could not re-resolve {target}/{configuration} to report the \
                         effective values: {e}"
                    ));
                }
            }
        }
    }
    rows
}

/// One scope's stored settings: the project, or one target.
struct RawScope {
    target: Option<String>,
    configurations: Vec<settings_pbxproj::ConfigSettings>,
}

/// The `pbxproj settings show` payload: the stored layer per scope,
/// optionally filtered to one `--key` (bare values in human mode, like the
/// porcelain `--key`).
struct RawResult {
    scopes: Vec<RawScope>,
    bare_key: Option<String>,
}

impl Render for RawResult {
    fn human(&self, out: &Output) {
        if let Some(key) = &self.bare_key {
            for scope in &self.scopes {
                for config in &scope.configurations {
                    for (k, v) in &config.settings {
                        if k == key {
                            out.line(&v.display());
                        }
                    }
                }
            }
            return;
        }
        let mut first = true;
        for scope in &self.scopes {
            for config in &scope.configurations {
                if !first {
                    out.line("");
                }
                first = false;
                match &scope.target {
                    Some(t) => out.line(&format!("# target {t} — {}", config.configuration)),
                    None => out.line(&format!("# project — {}", config.configuration)),
                }
                for (key, value) in &config.settings {
                    out.line(&format!("{key} = {}", value.display()));
                }
            }
        }
    }

    fn json(&self) -> serde_json::Value {
        let scopes: Vec<serde_json::Value> = self
            .scopes
            .iter()
            .map(|scope| {
                let configurations: Vec<serde_json::Value> = scope
                    .configurations
                    .iter()
                    .map(|config| {
                        let settings: serde_json::Map<String, serde_json::Value> = config
                            .settings
                            .iter()
                            .map(|(k, v)| (k.clone(), setting_json(Some(v))))
                            .collect();
                        serde_json::json!({
                            "configuration": config.configuration,
                            "settings": settings,
                        })
                    })
                    .collect();
                serde_json::json!({
                    "target": scope.target,
                    "configurations": configurations,
                })
            })
            .collect();
        serde_json::json!({ "raw": true, "scopes": scopes })
    }
}

fn show(
    ctx: &mut Context,
    container: &ContainerArgs,
    target: Option<&str>,
    key: Option<&str>,
) -> CommandResult {
    let target_owned = target.map(str::to_string);
    let (_, root) = super::open_project(ctx, container, target_owned.as_ref())?;

    let mut scopes = Vec::new();
    let wanted: Vec<Scope> = match target {
        Some(t) => vec![Scope::Target(t.to_string())],
        None => std::iter::once(Scope::Project)
            .chain(
                settings_pbxproj::target_names(&root)
                    .into_iter()
                    .map(Scope::Target),
            )
            .collect(),
    };
    for scope in wanted {
        let mut configurations = settings_pbxproj::raw(&root, &scope).map_err(CliError::new)?;
        if let Some(key) = key {
            for config in &mut configurations {
                config.settings.retain(|(k, _)| k == key);
            }
        }
        scopes.push(RawScope {
            target: scope.target().map(str::to_string),
            configurations,
        });
    }
    Ok(Rendered::data(RawResult {
        scopes,
        bare_key: key.map(str::to_string),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_assignments_and_appends() {
        assert_eq!(
            split_assignment("SWIFT_VERSION=5.0").unwrap(),
            ("SWIFT_VERSION".into(), false, "5.0".into())
        );
        assert_eq!(
            split_assignment("OTHER_LDFLAGS+=-lz").unwrap(),
            ("OTHER_LDFLAGS".into(), true, "-lz".into())
        );
        // The value keeps its own `=`s.
        assert_eq!(
            split_assignment("GCC_PREPROCESSOR_DEFINITIONS=DEBUG=1").unwrap(),
            (
                "GCC_PREPROCESSOR_DEFINITIONS".into(),
                false,
                "DEBUG=1".into()
            )
        );
        // A conditional key splits after the bracket, not inside it.
        assert_eq!(
            split_assignment("CODE_SIGN_IDENTITY[sdk=iphoneos*]=iPhone Developer").unwrap(),
            (
                "CODE_SIGN_IDENTITY[sdk=iphoneos*]".into(),
                false,
                "iPhone Developer".into()
            )
        );
        // An empty value clears to the empty string.
        assert_eq!(
            split_assignment("DEVELOPMENT_TEAM=").unwrap(),
            ("DEVELOPMENT_TEAM".into(), false, String::new())
        );
        assert!(split_assignment("NO_EQUALS_HERE").is_err());
        assert!(split_assignment("=value").is_err());
        assert!(split_assignment("BAD KEY=1").is_err());
    }

    #[test]
    fn folds_repeated_keys_into_arrays() {
        let folded = parse_assignments(&[
            "LD_RUNPATH_SEARCH_PATHS=$(inherited)".into(),
            "SWIFT_VERSION=5.0".into(),
            "LD_RUNPATH_SEARCH_PATHS=@executable_path/Frameworks".into(),
        ])
        .unwrap();
        assert_eq!(folded.len(), 2);
        assert_eq!(folded[0].key, "LD_RUNPATH_SEARCH_PATHS");
        assert_eq!(
            folded[0].op,
            Op::Assign(vec![
                "$(inherited)".into(),
                "@executable_path/Frameworks".into()
            ])
        );
        assert_eq!(folded[1].op, Op::Assign(vec!["5.0".into()]));
    }

    #[test]
    fn append_first_occurrence_keeps_append_semantics() {
        let folded = parse_assignments(&[
            "OTHER_LDFLAGS+=-framework".into(),
            "OTHER_LDFLAGS+=Metal".into(),
        ])
        .unwrap();
        assert_eq!(
            folded[0].op,
            Op::Append(vec!["-framework".into(), "Metal".into()])
        );
    }

    #[test]
    fn unset_keys_reject_values() {
        assert!(parse_keys(&["SWIFT_VERSION=5.0".into()]).is_err());
        assert_eq!(
            parse_keys(&["CODE_SIGN_IDENTITY[sdk=iphoneos*]".into()]).unwrap(),
            vec!["CODE_SIGN_IDENTITY[sdk=iphoneos*]".to_string()]
        );
    }
}
