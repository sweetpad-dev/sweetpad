//! Hand-authored configuration: `~/.config/sweetpad/config.toml`.
//!
//! Global settings plus optional per-project overrides keyed by canonicalized
//! project path. The file is the user's: the one write is `feedback off|on`
//! setting `[feedback] enabled` through [`set_feedback_enabled`], which edits
//! that key in place and keeps every other line, comments included.
//! Machine-written remembered selections live separately in
//! [`crate::cli::state`].
//!
//! Honors `XDG_CONFIG_HOME`, falling back to `~/.config`.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::Deserialize;
use sweetpad_core::xcodebuild_args::{self, has_flag};

/// Parsed `config.toml`. Missing file ⇒ [`Config::default`] (all empty).
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct Config {
    /// Global defaults applied to every project unless overridden.
    pub defaults: Defaults,
    /// Per-project overrides, keyed by absolute project/workspace path.
    pub projects: BTreeMap<String, Defaults>,
    /// `[feedback]`: whether `feedback submit` may send a report.
    pub feedback: FeedbackConfig,
    /// Lint findings from [`load`](Config::load): unknown keys (typos parse
    /// cleanly and are silently ignored otherwise) and `[projects."…"]` tables
    /// whose key can't match a real container. Surfaced as warnings by the
    /// dispatcher; never serialized.
    #[serde(skip)]
    pub warnings: Vec<String>,
}

/// The override knobs, shared by the global `[defaults]` table and each
/// `[projects."…"]` table.
#[derive(Debug, Default, Clone, Deserialize)]
#[serde(default)]
pub struct Defaults {
    pub scheme: Option<String>,
    pub configuration: Option<String>,
    pub destination: Option<String>,
    /// SDK for builds and `settings show` (e.g. `iphonesimulator`). Rarely
    /// needed — the destination usually implies it.
    pub sdk: Option<String>,
    /// Test-action overrides, layered over the build defaults for `test` only
    /// (e.g. a `TEST-Debug` configuration while the app builds `UAT-Debug`).
    pub testing: TestingDefaults,
}

/// Test-action config overrides — the `[…​.testing]` sub-table. Mirrors the
/// extension's `sweetpad.testing.*` settings. Unset fields fall back to the
/// build defaults during test resolution.
#[derive(Debug, Default, Clone, Deserialize)]
#[serde(default)]
pub struct TestingDefaults {
    pub scheme: Option<String>,
    pub configuration: Option<String>,
    pub destination: Option<String>,
    /// Default test target: `test run` narrows to `-only-testing:<target>`
    /// when no explicit `--only-testing` selector is given.
    pub target: Option<String>,
}

/// The `[feedback]` table. Unset means on.
#[derive(Debug, Default, Clone, Deserialize)]
#[serde(default)]
pub struct FeedbackConfig {
    pub enabled: Option<bool>,
}

impl Config {
    /// Standard config path, honoring `XDG_CONFIG_HOME`.
    #[must_use]
    pub fn path() -> Option<PathBuf> {
        config_dir().map(|d| d.join("sweetpad").join("config.toml"))
    }

    /// Load and parse the config file. A missing file is not an error
    /// (returns defaults); a malformed file is. Unknown keys and dead
    /// `[projects."…"]` keys parse cleanly but are collected into
    /// [`warnings`](Config::warnings) so a typo (`schme = "App"`, `[default]`)
    /// isn't silently ignored.
    pub fn load() -> Result<Self, String> {
        let Some(path) = Self::path() else {
            return Ok(Self::default());
        };
        match std::fs::read_to_string(&path) {
            Ok(text) => Self::parse(&text).map_err(|e| format!("{}: {e}", path.display())),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(format!("{}: {e}", path.display())),
        }
    }

    /// Parse config text, linting for unknown keys and dead project keys.
    fn parse(text: &str) -> Result<Self, toml::de::Error> {
        let mut cfg: Config = toml::from_str(text)?;
        // Lint against the raw document so unknown keys are visible (serde has
        // already dropped them from the typed struct).
        if let Ok(raw) = toml::from_str::<toml::Value>(text) {
            cfg.warnings = lint(&raw);
        }
        cfg.warnings
            .extend(cfg.projects.keys().filter_map(|k| lint_project_key(k)));
        Ok(cfg)
    }

    /// Effective defaults for `project_key`: per-project overrides layered on
    /// top of the global defaults.
    #[must_use]
    pub fn for_project(&self, project_key: &str) -> Defaults {
        let mut merged = self.defaults.clone();
        if let Some(over) = self.projects.get(project_key) {
            layer(&mut merged.scheme, over.scheme.as_ref());
            layer(&mut merged.configuration, over.configuration.as_ref());
            layer(&mut merged.destination, over.destination.as_ref());
            layer(&mut merged.sdk, over.sdk.as_ref());
            layer(&mut merged.testing.scheme, over.testing.scheme.as_ref());
            layer(
                &mut merged.testing.configuration,
                over.testing.configuration.as_ref(),
            );
            layer(
                &mut merged.testing.destination,
                over.testing.destination.as_ref(),
            );
            layer(&mut merged.testing.target, over.testing.target.as_ref());
        }
        merged
    }
}

/// What [`set_feedback_enabled`] did to `config.toml`.
#[derive(Debug)]
pub struct FeedbackEdit {
    pub path: PathBuf,
    /// Whether the file was written.
    pub changed: bool,
}

/// Set `[feedback] enabled` in the user's `config.toml`, editing that key and
/// keeping the rest of the file as written.
///
/// Turning feedback off writes `enabled = false`, creating the table, and the
/// file, when they are missing. Turning it on sets an `enabled` key that is
/// there to true and otherwise leaves the file alone, since unset means on, so
/// `on` never creates a config file.
///
/// The write goes to a temporary file beside the real one and is renamed over
/// it, so an interrupted write leaves the old file. A symlinked config (a
/// dotfiles checkout) is written at the link's target, with its permissions.
pub fn set_feedback_enabled(enabled: bool) -> Result<FeedbackEdit, String> {
    let path = Config::path()
        .ok_or("can't locate config.toml: neither XDG_CONFIG_HOME nor HOME is set")?;
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(format!("{}: {e}", path.display())),
    };
    let edited =
        edit_feedback_enabled(&text, enabled).map_err(|e| format!("{}: {e}", path.display()))?;
    let Some(edited) = edited.filter(|new| *new != text) else {
        return Ok(FeedbackEdit {
            path,
            changed: false,
        });
    };
    let target = std::fs::canonicalize(&path).unwrap_or_else(|_| path.clone());
    let write = || -> std::io::Result<()> {
        if let Some(dir) = target.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let tmp = target.with_file_name(format!(
            ".{}.sweetpad-{}",
            target
                .file_name()
                .map_or_else(|| "config.toml".into(), |n| n.to_string_lossy()),
            std::process::id()
        ));
        std::fs::write(&tmp, &edited)?;
        if let Ok(meta) = std::fs::metadata(&target) {
            std::fs::set_permissions(&tmp, meta.permissions())?;
        }
        std::fs::rename(&tmp, &target).inspect_err(|_| {
            let _ = std::fs::remove_file(&tmp);
        })
    };
    write().map_err(|e| format!("{}: {e}", target.display()))?;
    Ok(FeedbackEdit {
        path,
        changed: true,
    })
}

/// `text` with `[feedback] enabled` set to `enabled`, or `None` when turning
/// feedback on needs no edit. A trailing comment on an existing `enabled`
/// line stays on it.
fn edit_feedback_enabled(text: &str, enabled: bool) -> Result<Option<String>, String> {
    let mut doc: toml_edit::DocumentMut = text
        .parse()
        .map_err(|e: toml_edit::TomlError| e.to_string())?;
    match doc.get_mut("feedback") {
        None if enabled => return Ok(None),
        None => {
            let mut table = toml_edit::Table::new();
            table.insert("enabled", toml_edit::value(false));
            doc.insert("feedback", toml_edit::Item::Table(table));
        }
        Some(item) => {
            let Some(table) = item.as_table_like_mut() else {
                return Err("'feedback' must be a table: [feedback]".to_string());
            };
            match table
                .get_mut("enabled")
                .and_then(toml_edit::Item::as_value_mut)
            {
                Some(value) => {
                    let decor = value.decor().clone();
                    *value = toml_edit::Value::from(enabled);
                    *value.decor_mut() = decor;
                }
                None if enabled => return Ok(None),
                None => {
                    table.insert("enabled", toml_edit::value(false));
                }
            }
        }
    }
    Ok(Some(doc.to_string()))
}

/// Overlay a per-project override onto a base value, keeping the base when the
/// override is unset.
fn layer(base: &mut Option<String>, over: Option<&String>) {
    if let Some(v) = over {
        *base = Some(v.clone());
    }
}

/// The keys a `[defaults]` / `[projects."…"]` table accepts.
const DEFAULTS_KEYS: [&str; 5] = ["scheme", "configuration", "destination", "sdk", "testing"];
/// The keys a `[….testing]` sub-table accepts.
const TESTING_KEYS: [&str; 4] = ["scheme", "configuration", "destination", "target"];

/// Walk the raw document and report every key serde would silently drop.
fn lint(raw: &toml::Value) -> Vec<String> {
    let mut warnings = Vec::new();
    let Some(top) = raw.as_table() else {
        return warnings;
    };
    for (key, value) in top {
        match key.as_str() {
            "defaults" => lint_defaults(value, "[defaults]", &mut warnings),
            "projects" => {
                if let Some(projects) = value.as_table() {
                    for (proj, table) in projects {
                        lint_defaults(table, &format!("[projects.\"{proj}\"]"), &mut warnings);
                    }
                }
            }
            "feedback" => {
                if let Some(table) = value.as_table() {
                    for key in table.keys().filter(|k| *k != "enabled") {
                        warnings.push(format!("config: unknown key '{key}' in [feedback]"));
                    }
                }
            }
            other => warnings.push(format!(
                "config: unknown key '{other}' (did you mean 'defaults', 'projects' or 'feedback'?)"
            )),
        }
    }
    warnings
}

/// Lint one defaults-shaped table (and its `testing` sub-table) at `at`.
fn lint_defaults(value: &toml::Value, at: &str, warnings: &mut Vec<String>) {
    let Some(table) = value.as_table() else {
        return;
    };
    for (key, sub) in table {
        if key == "testing" {
            if let Some(testing) = sub.as_table() {
                for tkey in testing.keys() {
                    if !TESTING_KEYS.contains(&tkey.as_str()) {
                        warnings.push(format!("config: unknown key '{tkey}' in {at} testing"));
                    }
                }
            }
        } else if !DEFAULTS_KEYS.contains(&key.as_str()) {
            let hint = suggest(key, &DEFAULTS_KEYS)
                .map(|s| format!(" (did you mean '{s}'?)"))
                .unwrap_or_default();
            warnings.push(format!("config: unknown key '{key}' in {at}{hint}"));
        }
    }
}

/// A `[projects."…"]` key that can't match: the key must be the canonicalized
/// **container** path (`/…/App.xcodeproj`), not the directory holding it — a
/// directory key parses cleanly and then silently never applies.
fn lint_project_key(key: &str) -> Option<String> {
    let path = std::path::Path::new(key);
    if is_container_path(path) {
        // Right shape — flag it only if the canonical spelling differs. The
        // lookup is by raw *string*, so the comparison must be too: `Path`
        // equality would forgive a trailing slash or a `//`, exactly the
        // silently-dead keys this lint exists to catch.
        let canonical = std::fs::canonicalize(path).ok()?;
        (canonical.to_string_lossy() != key).then(|| {
            format!(
                "config: [projects.\"{key}\"] won't match — keys are canonicalized paths; use \"{}\"",
                canonical.display()
            )
        })
    } else if path.is_dir() {
        // The doc-example mistake: keyed by the project *directory*.
        let suggestion = crate::cli::resolve::discover(path)
            .map(|c| c.key())
            .map_or_else(String::new, |k| format!("; did you mean \"{k}\"?"));
        Some(format!(
            "config: [projects.\"{key}\"] matches no project — keys are the \
             canonicalized container path (the .xcworkspace/.xcodeproj/Package.swift \
             itself){suggestion}"
        ))
    } else {
        Some(format!(
            "config: [projects.\"{key}\"] matches no project on disk — keys are the \
             canonicalized container path"
        ))
    }
}

/// Whether a path names a container (workspace/project/manifest), existing or not.
fn is_container_path(path: &std::path::Path) -> bool {
    matches!(
        path.extension().and_then(|e| e.to_str()),
        Some("xcworkspace" | "xcodeproj")
    ) || path.file_name().and_then(|f| f.to_str()) == Some("Package.swift")
}

/// The closest known key within an edit distance of 2, for did-you-mean hints.
fn suggest<'a>(unknown: &str, known: &[&'a str]) -> Option<&'a str> {
    known
        .iter()
        .map(|k| (edit_distance(unknown, k), *k))
        .filter(|(d, _)| *d <= 2)
        .min_by_key(|(d, _)| *d)
        .map(|(_, k)| k)
}

/// Levenshtein distance, small-string sized (config keys).
pub(crate) fn edit_distance(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.iter().enumerate() {
        let mut row = vec![i + 1];
        for (j, cb) in b.iter().enumerate() {
            let cost = usize::from(ca != cb);
            row.push((prev[j] + cost).min(prev[j + 1] + 1).min(row[j] + 1));
        }
        prev = row;
    }
    prev[b.len()]
}

/// A committed, hand-authored `sweetpad.toml` at the project root — how a team
/// shares defaults (`~/.config/sweetpad/config.toml` stays personal). Read-only
/// like the user config; its precedence slot is between the user config and
/// remembered state. Besides the targeting keys it houses the tool defaults
/// that previously had flags but no config: `[run] hot`/`hot_recompiler` and
/// `[format] tool`.
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct ProjectFile {
    /// Names the `.xcworkspace` this file belongs to, relative to the file's
    /// own directory — so a `sweetpad.toml` at the repo root can point at a
    /// container nested below it and every command works from anywhere in the
    /// tree. Wins over `project` when both are set.
    pub workspace: Option<String>,
    /// Names the `.xcodeproj`, relative to the file's own directory. See
    /// [`workspace`](Self::workspace).
    pub project: Option<String>,
    pub scheme: Option<String>,
    pub configuration: Option<String>,
    pub destination: Option<String>,
    pub sdk: Option<String>,
    /// Pin the Xcode used for this project (sets DEVELOPER_DIR).
    pub developer_dir: Option<String>,
    /// Declares the `.xcodeproj` as generated (`"xcodegen"` / `"tuist"` /
    /// a free-form tool name) — pbxproj mutations then require `--force`
    /// even when no spec file sits next to the project (CLI_DESIGN §9g).
    pub generator: Option<String>,
    pub testing: TestingDefaults,
    pub run: RunDefaults,
    pub format: FormatDefaults,
    pub xcodebuild: XcodebuildDefaults,
}

/// `[run]` — `app run` defaults for this project.
#[derive(Debug, Default, Clone, Deserialize)]
#[serde(default)]
pub struct RunDefaults {
    /// Default `app run` to hot reload (`--no-hot` opts out per run).
    pub hot: Option<bool>,
    /// Default hot-reload recompiler: `resolver` or `buildlog`.
    pub hot_recompiler: Option<String>,
    /// Whether a `--hot` macOS build may strip the App Sandbox from an
    /// explicit entitlements file, ephemerally, to make injection possible
    /// (CLI_DESIGN §9d). Default true; `false` opts the project out
    /// (`--keep-sandbox` does it per run).
    pub auto_unsandbox: Option<bool>,
}

/// `[format]` — `format` defaults for this project.
#[derive(Debug, Default, Clone, Deserialize)]
#[serde(default)]
pub struct FormatDefaults {
    /// Default formatter: `swift-format` or `swiftlint`.
    pub tool: Option<String>,
}

/// `[xcodebuild]` — arguments every command in this project that spawns
/// `xcodebuild` adds to the invocation, so a repo-wide flag is written down
/// once instead of typed after `--` on each command.
#[derive(Debug, Default, Clone, Deserialize)]
#[serde(default)]
pub struct XcodebuildDefaults {
    /// `xcodebuild` flags and/or `KEY=VALUE` build-setting overrides, in the
    /// order they should appear.
    pub args: Vec<String>,
}

/// The effective `xcodebuild` passthrough for one invocation, and the file's
/// arguments the typed tail replaced in it.
#[derive(Debug, PartialEq, Eq)]
pub struct MergedXcodebuildArgs {
    pub args: Vec<String>,
    /// Each file argument left out for a flag the tail also gives, as
    /// `[flag, value]`: see [`SINGLE_USE_FLAGS`].
    pub replaced: Vec<[String; 2]>,
}

/// The effective `xcodebuild` passthrough for one invocation: the committed
/// `[xcodebuild] args` first, then the `--` tail typed on the command line, so
/// a typed argument wins under `xcodebuild`'s last-one-wins. A flag
/// `xcodebuild` takes only once ([`SINGLE_USE_FLAGS`]) has no last one to
/// win, so when the tail gives it, the file's copy and its value are left out.
/// Both lists are read with [`xcodebuild_args::read`]: a value spelled like
/// a flag (`-xcconfig -jobs`) is the value, and no copy of that flag.
///
/// The file's arguments are refused when they name something the CLI already
/// owns — [`configured_arg_refusal`] explains each case. Refusing is an error
/// rather than a warning because the alternative is handing `xcodebuild` two
/// answers to one question and building whichever it picks.
pub fn effective_xcodebuild_args(
    configured: &[String],
    tail: &[String],
) -> Result<MergedXcodebuildArgs, String> {
    if let Some((arg, fix)) = xcodebuild_args::read(configured)
        .find_map(|a| configured_arg_refusal(a.word).map(|fix| (a.word, fix)))
    {
        return Err(format!(
            "sweetpad.toml: '{arg}' in [xcodebuild] args — {fix}"
        ));
    }
    // Merged, the tail would hand the flag its value, so the file has to
    // give it one.
    if let Some(flag) = xcodebuild_args::dangling_flag(configured) {
        return Err(format!(
            "sweetpad.toml: '{flag}' in [xcodebuild] args — add its value after it"
        ));
    }
    let mut args = Vec::with_capacity(configured.len() + tail.len());
    let mut replaced = Vec::new();
    for arg in xcodebuild_args::read(configured) {
        if SINGLE_USE_FLAGS.contains(&arg.word) && has_flag(tail, arg.word) {
            let value = arg.value.unwrap_or_default();
            replaced.push([arg.word.to_string(), value.to_string()]);
        } else {
            args.extend(arg.words().map(String::from));
        }
    }
    args.extend(tail.iter().cloned());
    Ok(MergedXcodebuildArgs { args, replaced })
}

/// The `xcodebuild` flags that fail a second copy ("option '-xcconfig' may
/// only be provided once"), each of which takes a value, as Xcode 27 refuses
/// them. The single-use flags [`configured_arg_refusal`] keeps out of the file
/// (`-scheme`, `-derivedDataPath`, …) are not listed, and neither are the
/// value flags a build may repeat (`-destination`, `-arch`, `-toolchain`,
/// `-packageCachePath`).
const SINGLE_USE_FLAGS: [&str; 31] = [
    "-xcconfig",
    "-jobs",
    "-destination-timeout",
    "-clonedSourcePackagesDirPath",
    "-resultStreamPath",
    "-resultBundleVersion",
    "-xctestrun",
    "-testProductsPath",
    "-enableCodeCoverage",
    "-enableAddressSanitizer",
    "-enableThreadSanitizer",
    "-enableUndefinedBehaviorSanitizer",
    "-enablePerformanceTestsDiagnostics",
    "-enableCodesizeProfile",
    "-codesizeProfileOutputDir",
    "-testLanguage",
    "-testRegion",
    "-test-iterations",
    "-test-repetition-relaunch-enabled",
    "-test-timeouts-enabled",
    "-default-test-execution-time-allowance",
    "-maximum-test-execution-time-allowance",
    "-parallel-testing-enabled",
    "-parallel-testing-worker-count",
    "-maximum-parallel-testing-workers",
    "-maximum-concurrent-test-device-destinations",
    "-maximum-concurrent-test-simulator-destinations",
    "-authenticationKeyPath",
    "-authenticationKeyID",
    "-authenticationKeyIssuerID",
    "-scmProvider",
];

/// Why a given argument can't live in a committed `[xcodebuild] args`, if it
/// can't. Four groups: the inputs the resolver settles and passes itself (a
/// second copy makes the build depend on which `xcodebuild` honors), the
/// result bundle `test` writes and then reads back, the paths `archive`
/// names for its archive and export, and `-derivedDataPath`.
/// Only the builds, `clean` and the app locator read this file, so a
/// committed location would split them from `clean --purge`, `derived-data`
/// and the BSP index, which keep the default one. A relative value would also
/// resolve against the directory holding the project, where `xcodebuild`
/// runs, while the file's `workspace`/`project` keys resolve against the file.
fn configured_arg_refusal(arg: &str) -> Option<&'static str> {
    Some(match arg {
        "-workspace" | "-project" => "name the container with the 'workspace'/'project' key",
        "-scheme" => "use the 'scheme' key",
        "-configuration" => "use the 'configuration' key",
        "-destination" => "use the 'destination' key",
        "-sdk" => "use the 'sdk' key",
        "-derivedDataPath" => {
            "'clean --purge', 'derived-data' and the editor index would keep using the \
             DerivedData location Xcode's settings name, and a relative value resolves against \
             the project's directory, not this file's; pass it per command instead"
        }
        "-resultBundlePath" => {
            "'sweetpad test' writes and reads back its own result bundle; name one per run \
             with 'test --result-bundle', or after '--' on a build"
        }
        "-archivePath" | "-exportPath" => {
            "'sweetpad archive' names its own; use 'archive --output-file'"
        }
        "-exportOptionsPlist" => "'sweetpad archive' names its own; use 'archive --export-options'",
        _ => return None,
    })
}

impl ProjectFile {
    /// Load the `sweetpad.toml` next to `container_dir` (the container's
    /// parent). Missing file ⇒ defaults. A malformed or typo'd file is
    /// *warned about* rather than fatal — a broken committed file must not
    /// brick every teammate's CLI — and the warnings ride back for the caller
    /// to surface once.
    #[must_use]
    pub fn load_for(container_dir: &std::path::Path) -> (Self, Vec<String>) {
        let path = container_dir.join("sweetpad.toml");
        let text = match std::fs::read_to_string(&path) {
            Ok(text) => text,
            // Only a *missing* file is silently defaults. A present-but-
            // unreadable committed file (permissions, I/O error on a network
            // mount) must warn like a malformed one — silently dropping the
            // team's pinned defaults would build with the wrong Xcode.
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return (Self::default(), Vec::new());
            }
            Err(e) => {
                return (
                    Self::default(),
                    vec![format!("{}: {e} (file ignored)", path.display())],
                );
            }
        };
        match toml::from_str::<ProjectFile>(&text) {
            Ok(pf) => {
                let mut warnings = Vec::new();
                if let Ok(raw) = toml::from_str::<toml::Value>(&text) {
                    lint_project_file(&raw, &mut warnings);
                }
                (pf, warnings)
            }
            Err(e) => (
                Self::default(),
                vec![format!("{}: {e} (file ignored)", path.display())],
            ),
        }
    }
}

/// A `sweetpad.toml` located by walking up from the working directory, paired
/// with the directory holding it.
///
/// The walk is what lets one file serve a whole checkout: the file may sit
/// beside the container (the common case) or above it at the repo root, where
/// its [`workspace`](ProjectFile::workspace)/[`project`](ProjectFile::project)
/// key names a container nested below. Paths in the file resolve against
/// [`dir`](Self::dir) rather than the cwd, so the same file works from every
/// directory in the tree and needs no absolute paths.
#[derive(Debug)]
pub struct RootFile {
    /// The directory holding the file — the base for its relative paths.
    pub dir: PathBuf,
    pub file: ProjectFile,
}

impl RootFile {
    /// The nearest `sweetpad.toml` at or above `start`. The walk stops at the
    /// git root, the same boundary container discovery honors, so a checkout
    /// above this one never donates its defaults; it also stops at the
    /// filesystem root. Returns the file's lint warnings for the caller to
    /// surface once.
    #[must_use]
    pub fn find_upward(start: &Path) -> Option<(Self, Vec<String>)> {
        let mut dir = start.to_path_buf();
        loop {
            if dir.join("sweetpad.toml").exists() {
                let (file, warnings) = ProjectFile::load_for(&dir);
                return Some((Self { dir, file }, warnings));
            }
            if dir.join(".git").exists() {
                return None;
            }
            dir = dir.parent()?.to_path_buf();
        }
    }

    /// The container this file names, resolved against [`dir`](Self::dir).
    /// `workspace` wins when both keys are set (matching `--workspace` beating
    /// `--project`, and auto-discovery preferring a workspace). The path is
    /// returned unchecked — a declared container that doesn't exist is an
    /// error the resolver reports, not a reason to fall back to discovery and
    /// build something else.
    #[must_use]
    pub fn declared(&self) -> Option<crate::cli::resolve::Container> {
        use crate::cli::resolve::Container;
        if let Some(ws) = &self.file.workspace {
            return Some(Container::Workspace(self.dir.join(ws)));
        }
        self.file
            .project
            .as_ref()
            .map(|p| Container::Project(self.dir.join(p)))
    }

    /// Whether this file is `container`'s project file.
    ///
    /// Naming a container is a statement of identity: a file with a
    /// `workspace`/`project` key covers that container and no other, so a
    /// `--project` pointing somewhere else doesn't inherit a scheme meant for
    /// the declared one — even when the two sit side by side. Without a
    /// declaration the file covers the container beside it, which is both the
    /// long-standing layout and what keeps that file from being read and
    /// linted a second time.
    #[must_use]
    pub fn covers(&self, container: &crate::cli::resolve::Container) -> bool {
        let same = |a: &Path, b: &Path| match (std::fs::canonicalize(a), std::fs::canonicalize(b)) {
            (Ok(a), Ok(b)) => a == b,
            _ => false,
        };
        if let Some(declared) = self.declared() {
            return same(declared.path(), container.path());
        }
        container
            .path()
            .parent()
            .is_some_and(|parent| same(parent, &self.dir))
    }
}

/// The keys a `sweetpad.toml` accepts at the top level.
const PROJECT_FILE_KEYS: [&str; 12] = [
    "workspace",
    "project",
    "scheme",
    "configuration",
    "destination",
    "sdk",
    "developer_dir",
    "generator",
    "testing",
    "run",
    "format",
    "xcodebuild",
];

/// Report every `sweetpad.toml` key serde would silently drop.
fn lint_project_file(raw: &toml::Value, warnings: &mut Vec<String>) {
    let Some(top) = raw.as_table() else { return };
    if top.contains_key("workspace") && top.contains_key("project") {
        warnings.push(
            "sweetpad.toml: 'workspace' and 'project' are both set; using 'workspace'".to_string(),
        );
    }
    for (key, value) in top {
        match key.as_str() {
            // A committed file that names its container with an absolute path
            // resolves to nothing on every other machine — the one mistake
            // that makes the file worse than no file at all.
            "workspace" | "project" => {
                if value
                    .as_str()
                    .is_some_and(|s| std::path::Path::new(s).is_absolute())
                {
                    warnings.push(format!(
                        "sweetpad.toml: '{key}' is an absolute path, which won't resolve for \
                         anyone else with this repo — make it relative to sweetpad.toml"
                    ));
                }
            }
            "testing" => {
                if let Some(t) = value.as_table() {
                    for tkey in t.keys() {
                        if !TESTING_KEYS.contains(&tkey.as_str()) {
                            warnings
                                .push(format!("sweetpad.toml: unknown key '{tkey}' in [testing]"));
                        }
                    }
                }
            }
            "run" => {
                if let Some(t) = value.as_table() {
                    for rkey in t.keys() {
                        if !["hot", "hot_recompiler", "auto_unsandbox"].contains(&rkey.as_str()) {
                            warnings.push(format!("sweetpad.toml: unknown key '{rkey}' in [run]"));
                        }
                    }
                }
            }
            "format" => {
                if let Some(t) = value.as_table() {
                    for fkey in t.keys() {
                        if fkey != "tool" {
                            warnings
                                .push(format!("sweetpad.toml: unknown key '{fkey}' in [format]"));
                        }
                    }
                }
            }
            "xcodebuild" => {
                if let Some(t) = value.as_table() {
                    for xkey in t.keys() {
                        if xkey != "args" {
                            warnings.push(format!(
                                "sweetpad.toml: unknown key '{xkey}' in [xcodebuild]"
                            ));
                        }
                    }
                }
            }
            other if !PROJECT_FILE_KEYS.contains(&other) => {
                let hint = suggest(other, &PROJECT_FILE_KEYS)
                    .map(|s| format!(" (did you mean '{s}'?)"))
                    .unwrap_or_default();
                warnings.push(format!("sweetpad.toml: unknown key '{other}'{hint}"));
            }
            _ => {}
        }
    }
}

/// `$XDG_CONFIG_HOME` or `$HOME/.config`.
fn config_dir() -> Option<PathBuf> {
    if let Some(xdg) = std::env::var_os("XDG_CONFIG_HOME")
        && !xdg.is_empty()
    {
        return Some(PathBuf::from(xdg));
    }
    home_dir().map(|h| h.join(".config"))
}

pub(crate) fn home_dir() -> Option<PathBuf> {
    sweetpad_core::paths::home_dir()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::testdir::TempDir;

    #[test]
    fn for_project_layers_overrides_on_defaults() {
        let cfg: Config = toml::from_str(
            r#"
            [defaults]
            configuration = "Debug"
            scheme = "Global"

            [projects."/work/App.xcodeproj"]
            scheme = "App"
            destination = "platform=iOS Simulator,name=iPhone 15"
            "#,
        )
        .unwrap();

        let d = cfg.for_project("/work/App.xcodeproj");
        // Per-project scheme/destination win; configuration falls through.
        assert_eq!(d.scheme.as_deref(), Some("App"));
        assert_eq!(d.configuration.as_deref(), Some("Debug"));
        assert_eq!(
            d.destination.as_deref(),
            Some("platform=iOS Simulator,name=iPhone 15")
        );

        // Unknown project gets only the global defaults.
        let other = cfg.for_project("/other");
        assert_eq!(other.scheme.as_deref(), Some("Global"));
        assert_eq!(other.destination, None);
    }

    #[test]
    fn empty_config_is_default() {
        let cfg: Config = toml::from_str("").unwrap();
        assert!(cfg.projects.is_empty());
        assert_eq!(cfg.for_project("/x").scheme, None);
    }

    #[test]
    fn unknown_keys_are_warned_not_ignored() {
        // `[default]` instead of `[defaults]`, and a `schme` typo — both parse
        // cleanly (serde drops them), so the lint is the only signal.
        let cfg = Config::parse("[default]\nscheme = \"App\"\n").unwrap();
        assert!(cfg.warnings.iter().any(|w| w.contains("'default'")));

        let cfg = Config::parse("[defaults]\nschme = \"App\"\n").unwrap();
        assert!(
            cfg.warnings
                .iter()
                .any(|w| w.contains("'schme'") && w.contains("did you mean 'scheme'?")),
            "warnings: {:?}",
            cfg.warnings
        );

        let cfg = Config::parse("[defaults.testing]\nsdk = \"x\"\n").unwrap();
        assert!(cfg.warnings.iter().any(|w| w.contains("'sdk'")));

        // A clean config produces no warnings.
        let cfg = Config::parse("[defaults]\nscheme = \"App\"\n").unwrap();
        assert!(cfg.warnings.is_empty(), "warnings: {:?}", cfg.warnings);
    }

    #[test]
    fn project_key_that_is_a_directory_is_warned() {
        // The CLI_DESIGN doc-example mistake: keying by the project's directory
        // instead of the container path. It parses, then silently never matches.
        let dir = TempDir::new("sweetpad-cfg");
        std::fs::create_dir_all(dir.join("App.xcodeproj")).unwrap();
        let text = format!("[projects.\"{}\"]\nscheme = \"App\"\n", dir.display());
        let cfg = Config::parse(&text).unwrap();
        assert!(
            cfg.warnings
                .iter()
                .any(|w| w.contains("matches no project") && w.contains("App.xcodeproj")),
            "warnings: {:?}",
            cfg.warnings
        );
    }

    #[test]
    fn container_shaped_project_keys_are_accepted() {
        // A key with the right shape (even if absent on this machine's disk at
        // lint time it canonicalize-fails → no crash, no false "matches no
        // project on disk" for the container-shaped case).
        let dir = TempDir::new("sweetpad-cfg2");
        let proj = dir.join("App.xcodeproj");
        std::fs::create_dir_all(&proj).unwrap();
        let key = std::fs::canonicalize(&proj).unwrap();
        let text = format!("[projects.\"{}\"]\nscheme = \"App\"\n", key.display());
        let cfg = Config::parse(&text).unwrap();
        assert!(cfg.warnings.is_empty(), "warnings: {:?}", cfg.warnings);
    }

    #[test]
    fn feedback_off_adds_the_key_and_keeps_the_rest_of_the_file() {
        let text = "# my defaults\n[defaults]\nscheme = \"App\"   # the main one\n\n\
                    [projects.\"/work/App.xcodeproj\"]\nconfiguration = \"Debug\"\n";
        let off = edit_feedback_enabled(text, false).unwrap().unwrap();
        assert!(off.starts_with(text), "{off}");
        assert!(off.ends_with("[feedback]\nenabled = false\n"), "{off}");
        let cfg = Config::parse(&off).unwrap();
        assert_eq!(cfg.feedback.enabled, Some(false));
        assert!(cfg.warnings.is_empty(), "{:?}", cfg.warnings);

        // On sets the same key back, comment and all; a second off is the
        // file it started as.
        let with_comment = off.replace("enabled = false", "enabled = false # quiet, please");
        let on = edit_feedback_enabled(&with_comment, true).unwrap().unwrap();
        assert!(on.contains("enabled = true # quiet, please"), "{on}");
        assert_eq!(on.replace("true", "false"), with_comment);
        assert_eq!(
            edit_feedback_enabled(&on, false).unwrap().unwrap(),
            with_comment
        );
    }

    #[test]
    fn feedback_on_needs_no_file_and_no_table() {
        assert_eq!(edit_feedback_enabled("", true).unwrap(), None);
        assert_eq!(
            edit_feedback_enabled("[defaults]\nscheme = \"A\"\n", true).unwrap(),
            None
        );
        // Off on an empty file is the table alone.
        assert_eq!(
            edit_feedback_enabled("", false).unwrap().unwrap(),
            "[feedback]\nenabled = false\n"
        );
        // A dotted key and an inline table are edited where they are.
        assert_eq!(
            edit_feedback_enabled("feedback.enabled = true\n", false)
                .unwrap()
                .unwrap(),
            "feedback.enabled = false\n"
        );
        assert_eq!(
            edit_feedback_enabled("feedback = { enabled = false }\n", true)
                .unwrap()
                .unwrap(),
            "feedback = { enabled = true }\n"
        );
    }

    #[test]
    fn feedback_edits_refuse_a_file_they_cant_read_as_toml() {
        let err = edit_feedback_enabled("[defaults\n", false).unwrap_err();
        assert!(!err.is_empty());
        let err = edit_feedback_enabled("feedback = 1\n", false).unwrap_err();
        assert!(err.contains("[feedback]"), "{err}");
        // An unknown key in the table is a lint warning, like any other.
        let cfg = Config::parse("[feedback]\nenable = false\n").unwrap();
        assert!(
            cfg.warnings
                .iter()
                .any(|w| w.contains("'enable' in [feedback]")),
            "{:?}",
            cfg.warnings
        );
    }

    #[test]
    fn edit_distance_powers_did_you_mean() {
        assert_eq!(edit_distance("schme", "scheme"), 1);
        assert_eq!(edit_distance("scheme", "scheme"), 0);
        assert!(edit_distance("destination", "scheme") > 2);
    }

    #[test]
    fn generator_key_parses_and_lints_clean() {
        let pf: ProjectFile = toml::from_str("generator = \"xcodegen\"\n").unwrap();
        assert_eq!(pf.generator.as_deref(), Some("xcodegen"));
        let raw: toml::Value = toml::from_str("generator = \"tuist\"\n").unwrap();
        let mut warnings = Vec::new();
        lint_project_file(&raw, &mut warnings);
        assert!(warnings.is_empty(), "{warnings:?}");
    }

    #[test]
    fn run_auto_unsandbox_parses_and_lints_clean() {
        let pf: ProjectFile =
            toml::from_str("[run]\nhot = true\nauto_unsandbox = false\n").unwrap();
        assert_eq!(pf.run.auto_unsandbox, Some(false));
        // The linter knows the key — no unknown-key warning.
        let raw: toml::Value =
            toml::from_str("[run]\nauto_unsandbox = false\nbogus = 1\n").unwrap();
        let mut warnings = Vec::new();
        lint_project_file(&raw, &mut warnings);
        assert_eq!(warnings.len(), 1, "{warnings:?}");
        assert!(warnings[0].contains("bogus"), "{warnings:?}");
    }

    #[test]
    fn xcodebuild_args_parse_and_lint_clean() {
        let pf: ProjectFile =
            toml::from_str("[xcodebuild]\nargs = [\"-skipMacroValidation\", \"FOO=1\"]\n").unwrap();
        assert_eq!(pf.xcodebuild.args, ["-skipMacroValidation", "FOO=1"]);

        // An absent table is an empty list, not a parse error.
        let pf: ProjectFile = toml::from_str("scheme = \"App\"\n").unwrap();
        assert!(pf.xcodebuild.args.is_empty());

        let raw: toml::Value =
            toml::from_str("[xcodebuild]\nargs = [\"-x\"]\nbogus = 1\n").unwrap();
        let mut warnings = Vec::new();
        lint_project_file(&raw, &mut warnings);
        assert_eq!(warnings.len(), 1, "{warnings:?}");
        assert!(warnings[0].contains("bogus"), "{warnings:?}");
    }

    #[test]
    fn the_file_supplies_arguments_before_the_typed_tail() {
        let s = |args: &[&str]| args.iter().map(|a| (*a).to_string()).collect::<Vec<_>>();

        // Committed first, typed second — xcodebuild takes the last one, so a
        // typed argument beats the file's.
        assert_eq!(
            effective_xcodebuild_args(&s(&["-skipMacroValidation"]), &s(&["FOO=1"]))
                .unwrap()
                .args,
            ["-skipMacroValidation", "FOO=1"]
        );
        // Either side alone.
        assert_eq!(
            effective_xcodebuild_args(&s(&["-a"]), &[]).unwrap().args,
            ["-a"]
        );
        assert_eq!(
            effective_xcodebuild_args(&[], &s(&["-b"])).unwrap().args,
            ["-b"]
        );
        assert!(effective_xcodebuild_args(&[], &[]).unwrap().args.is_empty());
    }

    #[test]
    fn a_typed_single_use_flag_replaces_the_files_copy() {
        let s = |args: &[&str]| args.iter().map(|a| (*a).to_string()).collect::<Vec<_>>();

        // Xcode 27 fails a second copy: "xcodebuild: error: option '-xcconfig'
        // may only be provided once". The typed one is the one the caller
        // asked for, so the file's goes, value and all.
        let merged = effective_xcodebuild_args(
            &s(&["-xcconfig", "a.xcconfig", "-skipMacroValidation", "FOO=1"]),
            &s(&["-xcconfig", "b.xcconfig"]),
        )
        .unwrap();
        assert_eq!(
            merged.args,
            ["-skipMacroValidation", "FOO=1", "-xcconfig", "b.xcconfig"]
        );
        assert_eq!(
            merged.replaced,
            [["-xcconfig".to_string(), "a.xcconfig".to_string()]]
        );

        // Every listed flag, and only when the tail gives it.
        for flag in SINGLE_USE_FLAGS {
            let merged =
                effective_xcodebuild_args(&s(&[flag, "file"]), &s(&[flag, "typed"])).unwrap();
            assert_eq!(merged.args, [flag, "typed"], "{flag}");
            let kept = effective_xcodebuild_args(&s(&[flag, "file"]), &s(&["FOO=1"])).unwrap();
            assert_eq!(kept.args, [flag, "file", "FOO=1"], "{flag}");
            assert!(kept.replaced.is_empty(), "{flag}");
        }

        // A value flag a build may repeat keeps both copies, the way
        // xcodebuild takes them.
        let merged =
            effective_xcodebuild_args(&s(&["-arch", "arm64"]), &s(&["-arch", "x86_64"])).unwrap();
        assert_eq!(merged.args, ["-arch", "arm64", "-arch", "x86_64"]);
        assert!(merged.replaced.is_empty());
    }

    /// A value spelled like a single-use flag is its flag's value, in the
    /// file and in the tail, as xcodebuild reads it.
    #[test]
    fn a_value_spelled_like_a_single_use_flag_is_no_copy_of_it() {
        let s = |args: &[&str]| args.iter().map(|a| (*a).to_string()).collect::<Vec<_>>();

        // The file's '-jobs' names its xcconfig, so the typed '-jobs'
        // replaces nothing.
        let merged =
            effective_xcodebuild_args(&s(&["-xcconfig", "-jobs"]), &s(&["-jobs", "4"])).unwrap();
        assert_eq!(merged.args, ["-xcconfig", "-jobs", "-jobs", "4"]);
        assert!(merged.replaced.is_empty());
        // The typed '-jobs' names the tail's xcconfig, so the file's stays.
        let merged =
            effective_xcodebuild_args(&s(&["-jobs", "4"]), &s(&["-xcconfig", "-jobs"])).unwrap();
        assert_eq!(merged.args, ["-jobs", "4", "-xcconfig", "-jobs"]);
        assert!(merged.replaced.is_empty());

        // Each takes a value, which leaves with it.
        for flag in SINGLE_USE_FLAGS {
            assert!(xcodebuild_args::takes_value(flag), "{flag}");
        }
    }

    #[test]
    fn the_files_flags_take_their_values_inside_the_file() {
        let s = |args: &[&str]| args.iter().map(|a| (*a).to_string()).collect::<Vec<_>>();

        // Merged, the tail's first argument would become the file's value.
        for tail in [&[][..], &["FOO=1"]] {
            let err = effective_xcodebuild_args(&s(&["-quiet", "-xcconfig"]), &s(tail))
                .expect_err("a dangling flag");
            assert_eq!(
                err,
                "sweetpad.toml: '-xcconfig' in [xcodebuild] args — add its value after it"
            );
        }
        assert!(effective_xcodebuild_args(&s(&["-xcconfig", "a.xcconfig"]), &[]).is_ok());
        assert!(effective_xcodebuild_args(&s(&["-xcconfig", "-quiet"]), &[]).is_ok());
    }

    #[test]
    fn the_file_cannot_carry_the_arguments_the_cli_settles_itself() {
        let s = |args: &[&str]| args.iter().map(|a| (*a).to_string()).collect::<Vec<_>>();

        for (arg, hint) in [
            ("-scheme", "'scheme' key"),
            ("-configuration", "'configuration' key"),
            ("-destination", "'destination' key"),
            ("-sdk", "'sdk' key"),
            ("-workspace", "'workspace'/'project' key"),
            ("-project", "'workspace'/'project' key"),
            ("-derivedDataPath", "per command"),
            // The fix names the flags that work: a build takes a typed
            // '-resultBundlePath' after '--', and 'test' has its own flag.
            (
                "-resultBundlePath",
                "'test --result-bundle', or after '--' on a build",
            ),
            ("-archivePath", "'archive --output-file'"),
            ("-exportPath", "'archive --output-file'"),
            ("-exportOptionsPlist", "'archive --export-options'"),
        ] {
            let err = effective_xcodebuild_args(&s(&[arg, "value"]), &[])
                .expect_err("a refused argument must not merge");
            assert!(err.contains(arg) && err.contains(hint), "{arg}: {err}");
        }

        // The reason has to hold: xcodebuild runs from the project's directory,
        // so a relative value never meant the caller's working directory. What
        // a committed one would break is every command that doesn't read it.
        let err = effective_xcodebuild_args(&s(&["-derivedDataPath", "dd"]), &[]).unwrap_err();
        assert!(!err.contains("working directory"), "{err}");
        assert!(err.contains("project's directory"), "{err}");
        assert!(
            err.contains("'clean --purge'") && err.contains("'derived-data'"),
            "{err}"
        );

        // Typing one is still the caller's own business — only the committed
        // file is policed, since everyone else inherits it unseen.
        assert_eq!(
            effective_xcodebuild_args(&[], &s(&["-derivedDataPath", "/tmp/dd"]))
                .unwrap()
                .args,
            ["-derivedDataPath", "/tmp/dd"]
        );

        // A value spelled like a refused flag is a value, as xcodebuild
        // reads it: this names an xcconfig called '-scheme'.
        assert!(effective_xcodebuild_args(&s(&["-xcconfig", "-scheme"]), &[]).is_ok());
        let err = effective_xcodebuild_args(&s(&["-quiet", "-scheme", "App"]), &[]).unwrap_err();
        assert!(err.contains("'-scheme'"), "{err}");
    }

    #[test]
    fn testing_section_layers_separately_from_build() {
        // The #219 setup: build on UAT-Debug, test on TEST-Debug.
        let cfg: Config = toml::from_str(
            r#"
            [projects."/work/App.xcodeproj"]
            configuration = "UAT-Debug"
            [projects."/work/App.xcodeproj".testing]
            configuration = "TEST-Debug"
            scheme = "AppTests"
            "#,
        )
        .unwrap();

        let d = cfg.for_project("/work/App.xcodeproj");
        assert_eq!(d.configuration.as_deref(), Some("UAT-Debug"));
        assert_eq!(d.testing.configuration.as_deref(), Some("TEST-Debug"));
        assert_eq!(d.testing.scheme.as_deref(), Some("AppTests"));
        // A testing field left unset stays None (test resolution falls back to build).
        assert_eq!(d.testing.destination, None);
    }

    /// Lay `root` out like the reported case: a git root holding the file, a
    /// sibling directory to run from, and the project one level down.
    fn lay_out_repo(root: &Path) {
        std::fs::create_dir_all(root.join(".git")).unwrap();
        std::fs::create_dir_all(root.join("Scripts")).unwrap();
        std::fs::create_dir_all(root.join("Sources/App.xcodeproj")).unwrap();
    }

    /// A scratch directory laid out by [`lay_out_repo`].
    fn nested_repo(tag: &str) -> TempDir {
        let root = TempDir::new(&format!("sweetpad-rootfile-{tag}"));
        lay_out_repo(&root);
        root
    }

    #[test]
    fn find_upward_reaches_the_file_from_a_nested_directory() {
        let root = nested_repo("nested");
        std::fs::write(
            root.join("sweetpad.toml"),
            "project = \"Sources/App.xcodeproj\"",
        )
        .unwrap();

        // Found from the root itself and from a sibling directory below it.
        for start in [root.to_path_buf(), root.join("Scripts")] {
            let (found, warnings) = RootFile::find_upward(&start).expect("file found");
            assert!(warnings.is_empty(), "{warnings:?}");
            assert_eq!(found.dir, *root);
            // Relative to the file, not to the directory the walk started in.
            let declared = found.declared().expect("declares a project");
            assert_eq!(declared.path(), root.join("Sources/App.xcodeproj"));
        }
    }

    #[test]
    fn find_upward_stops_at_the_git_root() {
        // A file above the repository must not donate its defaults.
        let above = TempDir::new("sweetpad-rootfile-above");
        std::fs::write(above.join("sweetpad.toml"), "scheme = \"Stray\"").unwrap();
        let outer = above.join("repo");
        lay_out_repo(&outer);

        assert!(RootFile::find_upward(&outer.join("Scripts")).is_none());
    }

    #[test]
    fn declared_prefers_workspace_and_absolute_paths_pass_through() {
        let root = nested_repo("both");
        std::fs::write(
            root.join("sweetpad.toml"),
            "workspace = \"Sources/App.xcworkspace\"\nproject = \"Sources/App.xcodeproj\"\n",
        )
        .unwrap();

        let (found, warnings) = RootFile::find_upward(&root).unwrap();
        assert!(
            warnings.iter().any(|w| w.contains("both set")),
            "{warnings:?}"
        );
        assert!(matches!(
            found.declared(),
            Some(crate::cli::resolve::Container::Workspace(_))
        ));

        // An absolute value warns but still resolves, `join` yielding the value.
        std::fs::write(
            root.join("sweetpad.toml"),
            "project = \"/abs/Other.xcodeproj\"",
        )
        .unwrap();
        let (found, warnings) = RootFile::find_upward(&root).unwrap();
        assert!(
            warnings.iter().any(|w| w.contains("absolute path")),
            "{warnings:?}"
        );
        assert_eq!(
            found.declared().unwrap().path(),
            Path::new("/abs/Other.xcodeproj")
        );
    }

    #[test]
    fn covers_the_container_it_names_and_its_own_siblings() {
        let root = nested_repo("covers");
        std::fs::write(
            root.join("sweetpad.toml"),
            "project = \"Sources/App.xcodeproj\"",
        )
        .unwrap();
        let (found, _) = RootFile::find_upward(&root).unwrap();

        // The named container, reached by a different spelling of the path.
        let spelled = root.join("Scripts/../Sources/App.xcodeproj");
        assert!(found.covers(&crate::cli::resolve::Container::Project(spelled)));

        // An unrelated container in the same checkout is not this file's —
        // including one sitting right beside the file, since naming a
        // container rules out every other.
        std::fs::create_dir_all(root.join("Other/Other.xcodeproj")).unwrap();
        std::fs::create_dir_all(root.join("Beside.xcodeproj")).unwrap();
        assert!(!found.covers(&crate::cli::resolve::Container::Project(
            root.join("Other/Other.xcodeproj")
        )));
        assert!(!found.covers(&crate::cli::resolve::Container::Project(
            root.join("Beside.xcodeproj")
        )));

        // A sibling of the file is covered once nothing is declared.
        std::fs::write(root.join("sweetpad.toml"), "scheme = \"App\"").unwrap();
        let (plain, _) = RootFile::find_upward(&root).unwrap();
        assert!(plain.covers(&crate::cli::resolve::Container::Project(
            root.join("Beside.xcodeproj")
        )));
    }
}
