//! Turning a saved `.swift` file into a loadable `.dylib` the injection client
//! can `dlopen` and patch in. Both strategies converge on running **one
//! `swift-frontend -primary-file` job** for the changed file (single-file speed)
//! and linking it into a dylib; they differ only in where that frontend command
//! comes from (CLI_DESIGN §9d):
//!
//! - **`Resolver` (F, default):** ask the crate's own `compiler_args` for the
//!   target's `swiftc` *driver* invocation, then ask the user's own toolchain
//!   driver to print the *frontend* jobs it would run (`swiftc -###`, a dry run)
//!   and pick the one for the changed file. Robust — no build-log dependency, no
//!   Xcode-version log-format drift — and uses the active toolchain (so the
//!   driver/frontend/version all match). Whole-module `swiftc -emit-library` is
//!   the fallback if `-###` recovery fails.
//! - **`BuildLog` (A, switchable):** recover the same single-file command from
//!   the `--hot` build's transcript (`EMIT_FRONTEND_COMMAND_LINES`) — free, since
//!   the build already emitted it.
//!
//! Recovered commands are **cached per source** (they're stable until the file
//! set / settings change), so the only per-save cost is the compile + link.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use crate::build_settings::{BuildSettingsOptions, resolve_compiler_arguments};
use crate::cli::resolve::Container;
use crate::compiler_args::TargetCompilerArguments;

/// Which recompile strategy `--hot` uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// F — single-file frontend recovered from the resolver via `swiftc -###`
    /// (whole-module `-emit-library` fallback). The default.
    Resolver,
    /// A — single-file `swift-frontend` recovered from the build log.
    BuildLog,
}

impl Mode {
    /// Parse the `--hot-recompiler` flag / config value.
    #[must_use]
    pub fn parse(s: &str) -> Option<Mode> {
        match s.to_ascii_lowercase().as_str() {
            "resolver" | "f" => Some(Mode::Resolver),
            "buildlog" | "build-log" | "a" => Some(Mode::BuildLog),
            _ => None,
        }
    }
}

/// Everything the recompiler needs across a `--hot` session.
pub struct Recompiler {
    pub mode: Mode,
    /// Temp dir for `eval_injection_*.o` / `.dylib`.
    pub out_dir: PathBuf,
    /// Active Xcode `Contents/Developer` (for the `.xcodePath` command + linking).
    pub developer_dir: String,
    /// `-sdk` short name conditionals bind to (e.g. `iphonesimulator`).
    pub sdk: String,
    /// Arch conditionals bind to and we link for (`arm64` / `x86_64`).
    pub arch: String,

    // Resolver (F) inputs:
    project: Option<PathBuf>,
    workspace: Option<PathBuf>,
    scheme: String,
    configuration: String,
    /// Cached per-target compiler args; rebuilt on a miss (e.g. a new file).
    resolved: Mutex<Option<Vec<TargetCompilerArguments>>>,
    /// Recovered single-file frontend commands, keyed by canonical source path.
    /// Stable across saves, so we run `-###` / parse the log at most once per file.
    frontend_cache: Mutex<HashMap<PathBuf, Vec<String>>>,

    // BuildLog (A) input: the `--hot` build's captured transcript.
    build_log: Option<PathBuf>,

    counter: AtomicUsize,
}

impl Recompiler {
    #[allow(clippy::too_many_arguments)]
    #[must_use]
    pub fn new(
        mode: Mode,
        container: &Container,
        scheme: String,
        configuration: String,
        sdk: String,
        arch: String,
        developer_dir: String,
        build_log: Option<PathBuf>,
        out_dir: PathBuf,
    ) -> Recompiler {
        let (project, workspace) = match container {
            Container::Project(p) => (Some(p.clone()), None),
            Container::Workspace(p) => (None, Some(p.clone())),
            // SPM has no pbxproj for the resolver; resolver mode won't apply.
            Container::SwiftPackage(_) => (None, None),
        };
        Recompiler {
            mode,
            out_dir,
            developer_dir,
            sdk,
            arch,
            project,
            workspace,
            scheme,
            configuration,
            resolved: Mutex::new(None),
            frontend_cache: Mutex::new(HashMap::new()),
            build_log,
            counter: AtomicUsize::new(0),
        }
    }

    /// Recompile `source` into a fresh loadable dylib, returning its path.
    pub fn recompile(&self, source: &Path) -> Result<PathBuf, String> {
        std::fs::create_dir_all(&self.out_dir).map_err(|e| format!("create inject dir: {e}"))?;
        let n = self.counter.fetch_add(1, Ordering::Relaxed);
        // The `eval_injection_` prefix is what the client's image scan expects.
        let dylib = self.out_dir.join(format!("eval_injection_{n}.dylib"));
        let object = self.out_dir.join(format!("eval_injection_{n}.o"));
        let source_str = source.to_string_lossy().into_owned();

        // Single-file frontend command (cached); both modes converge here.
        match self.frontend_tokens(source) {
            Ok(tokens) => {
                run(
                    "",
                    &single_file_command(&tokens, &source_str, &object)?,
                    "compile",
                )?;
                run("", &link_command(&tokens, &object, &dylib)?, "link")?;
                Ok(dylib)
            }
            // Resolver mode degrades to a robust whole-module build if single-file
            // recovery (`-###`) failed for any reason.
            Err(single_err) if self.mode == Mode::Resolver => {
                self.recompile_wholemodule(source, &dylib).map_err(|wm| {
                    format!("single-file recovery failed ({single_err}); whole-module fallback failed: {wm}")
                })?;
                Ok(dylib)
            }
            Err(e) => Err(e),
        }
    }

    /// The cached single-file frontend command for `source` (raw job tokens,
    /// before output geometry). Populates the cache on a miss.
    fn frontend_tokens(&self, source: &Path) -> Result<Vec<String>, String> {
        let canon = std::fs::canonicalize(source).unwrap_or_else(|_| source.to_path_buf());
        if let Some(tokens) = self.frontend_cache.lock().unwrap().get(&canon) {
            return Ok(tokens.clone());
        }
        match self.mode {
            Mode::BuildLog => {
                let tokens = self.buildlog_tokens(source)?;
                self.frontend_cache
                    .lock()
                    .unwrap()
                    .insert(canon, tokens.clone());
                Ok(tokens)
            }
            Mode::Resolver => {
                self.populate_resolver_cache(source)?;
                self.frontend_cache
                    .lock()
                    .unwrap()
                    .get(&canon)
                    .cloned()
                    .ok_or_else(|| {
                        format!(
                            "swiftc -### produced no frontend job for {}",
                            source.display()
                        )
                    })
            }
        }
    }

    // ---- F: single-file frontend recovered from `swiftc -###` ----

    /// Run the toolchain driver as a dry run (`swiftc -###`) for the target that
    /// owns `source`, parse the per-file frontend jobs it would spawn, and cache
    /// each by its `-primary-file` path. One spawn populates the whole module.
    fn populate_resolver_cache(&self, source: &Path) -> Result<(), String> {
        let swift = self.resolve_swift_for(source)?;
        // `-disable-batch-mode` makes the driver emit one frontend job per file
        // (a clean single `-primary-file` each), which is exactly what we want.
        let mut argv: Vec<String> =
            vec!["swiftc".into(), "-###".into(), "-disable-batch-mode".into()];
        argv.extend(sanitize_driver_args(&swift.arguments));
        argv.extend(swift.input_files.clone());
        let output = capture_combined("xcrun", &argv)?;

        let mut cache = self.frontend_cache.lock().unwrap();
        let mut found = 0;
        for tokens in parse_frontend_jobs(&output) {
            if let Some(primary) = primary_file(&tokens) {
                let key =
                    std::fs::canonicalize(&primary).unwrap_or_else(|_| PathBuf::from(&primary));
                cache.insert(key, tokens);
                found += 1;
            }
        }
        if found == 0 {
            return Err("no frontend compile jobs in swiftc -### output".into());
        }
        Ok(())
    }

    /// Whole-module fallback: `swiftc -emit-library` over all the module's files.
    fn recompile_wholemodule(&self, source: &Path, dylib: &Path) -> Result<(), String> {
        let swift = self.resolve_swift_for(source)?;
        let mut argv: Vec<String> = sanitize_driver_args(&swift.arguments);
        argv.extend(swift.input_files.clone());
        argv.extend(
            [
                "-emit-library",
                "-Xlinker",
                "-interposable",
                "-Xlinker",
                "-undefined",
                "-Xlinker",
                "dynamic_lookup",
                "-o",
            ]
            .map(String::from),
        );
        argv.push(dylib.to_string_lossy().into_owned());
        run("xcrun", &prepend("swiftc", &argv), "emit-library")
    }

    /// Resolve (and cache) the target whose module owns `source`, returning its
    /// `swiftc` invocation. A cache miss (new file, or first call) re-resolves.
    fn resolve_swift_for(
        &self,
        source: &Path,
    ) -> Result<crate::compiler_args::ToolInvocation, String> {
        let canon = std::fs::canonicalize(source).unwrap_or_else(|_| source.to_path_buf());
        let name = source
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or_default();

        let mut guard = self.resolved.lock().unwrap();
        for attempt in 0..2 {
            if guard.is_none() || attempt == 1 {
                *guard = Some(self.resolve_all()?);
            }
            if let Some(targets) = guard.as_ref() {
                for t in targets {
                    if let Some(swift) = &t.swift
                        && swift.input_files.iter().any(|f| {
                            std::fs::canonicalize(f)
                                .map(|c| c == canon)
                                .unwrap_or(false)
                                || f.ends_with(name)
                        })
                    {
                        return Ok(swift.clone());
                    }
                }
            }
            // Not found on the cached set — drop it and re-resolve once.
            *guard = None;
        }
        Err(format!(
            "no target's Swift module contains {} (resolver mode)",
            source.display()
        ))
    }

    fn resolve_all(&self) -> Result<Vec<TargetCompilerArguments>, String> {
        let opts = BuildSettingsOptions {
            project: self.project.clone(),
            workspace: self.workspace.clone(),
            scheme: Some(self.scheme.clone()),
            configuration: self.configuration.clone(),
            sdk: self.sdk.clone(),
            arch: self.arch.clone(),
            ..Default::default()
        };
        resolve_compiler_arguments(&opts)
    }

    // ---- A: single-file frontend recovered from the build log ----

    /// Recover the `swift-frontend -primary-file <source>` command for `source`
    /// from the captured `--hot` build transcript.
    fn buildlog_tokens(&self, source: &Path) -> Result<Vec<String>, String> {
        let log = self
            .build_log
            .as_ref()
            .ok_or("build-log recompiler needs the captured --hot build transcript")?;
        let source_str = source.to_string_lossy().into_owned();
        let name = source
            .file_name()
            .and_then(|s| s.to_str())
            .ok_or("bad source path")?;
        let text = std::fs::read_to_string(log).map_err(|e| format!("read build log: {e}"))?;

        let is_primary_line = |l: &str| {
            l.split_whitespace()
                .collect::<Vec<_>>()
                .windows(2)
                .any(|w| w[0] == "-primary-file" && (w[1].ends_with(name) || w[1] == source_str))
        };
        let line = text
            .lines()
            .find(|l| l.contains("-primary-file") && is_primary_line(l))
            .or_else(|| {
                text.lines()
                    .find(|l| l.contains("swift-frontend") && l.contains(name))
            })
            .ok_or_else(|| {
                format!("no swift-frontend -primary-file command for {name} in build log")
            })?;

        // Transcript args are shell-escaped; we exec argv directly, so unescape.
        Ok(line.split_whitespace().map(unescape).collect())
    }

    /// The `Xcode.app` path the client wants for `.xcodePath` (drops the
    /// `/Contents/Developer` suffix the developer dir carries).
    #[must_use]
    pub fn xcode_app_path(&self) -> String {
        self.developer_dir
            .strip_suffix("/Contents/Developer")
            .unwrap_or(&self.developer_dir)
            .to_string()
    }
}

// ---- shared command helpers (ported from the validated spike) ----

/// Run `prog argv`, returning stdout+stderr combined (the `-###` dry run prints
/// its jobs to stderr; we don't care which stream).
fn capture_combined(prog: &str, argv: &[String]) -> Result<String, String> {
    let out = Command::new(prog)
        .args(argv)
        .output()
        .map_err(|e| format!("spawn {prog}: {e}"))?;
    let mut s = String::from_utf8_lossy(&out.stdout).into_owned();
    s.push_str(&String::from_utf8_lossy(&out.stderr));
    Ok(s)
}

/// Parse `swiftc -###` output into the per-file frontend *compile* jobs (each a
/// token vector). `-###` prints each job on a line with every argument
/// double-quoted; we keep the lines that are frontend invocations carrying a
/// `-primary-file` and a compile flag (skipping the module-merge / link jobs).
fn parse_frontend_jobs(text: &str) -> Vec<Vec<String>> {
    let mut jobs = Vec::new();
    for line in text.lines() {
        if !line.contains("-frontend") {
            continue;
        }
        let tokens = parse_quoted_tokens(line);
        let has_primary = tokens.iter().any(|t| t == "-primary-file");
        let is_compile = tokens.iter().any(|t| t == "-c" || t == "-emit-object");
        if has_primary && is_compile {
            jobs.push(tokens);
        }
    }
    jobs
}

/// Tokenize a `-###` job line. `-###` wraps every argument in double quotes, so
/// the tokens are the odd-indexed `"`-split segments; fall back to whitespace
/// splitting for any unquoted line.
fn parse_quoted_tokens(line: &str) -> Vec<String> {
    let line = line.trim();
    if line.contains('"') {
        line.split('"')
            .enumerate()
            .filter(|(i, _)| i % 2 == 1)
            .map(|(_, s)| s.to_string())
            .collect()
    } else {
        line.split_whitespace().map(str::to_string).collect()
    }
}

/// Drop resolver driver flags that the build system only emits alongside build
/// *geometry* (output-file-maps, module-cache session files, explicit-module
/// scanner output) which `compiler_args` deliberately omits. For a standalone
/// recompile these orphans make `swiftc` reject the args (e.g.
/// `-validate-clang-modules-once` without `-clang-build-session-file`) or emit a
/// frontend command referencing missing files. We drop them and let swiftc build
/// modules implicitly — the same approach InjectionLite's recompiler takes.
fn sanitize_driver_args(args: &[String]) -> Vec<String> {
    const DROP_FLAG: &[&str] = &[
        "-validate-clang-modules-once", // needs -clang-build-session-file
        "-explicit-module-build",       // needs the module scanner's output
        "-experimental-emit-module-separately", // module emission; not for a -c recompile
        "-emit-const-values",           // needs -emit-const-values-path
        "-use-frontend-parseable-output", // output-format noise
    ];
    const DROP_PAIR: &[&str] = &["-clang-build-session-file"];
    let mut out = Vec::with_capacity(args.len());
    let mut i = 0;
    while i < args.len() {
        let a = args[i].as_str();
        if DROP_FLAG.contains(&a) {
            i += 1;
        } else if DROP_PAIR.contains(&a) {
            i += 2;
        } else {
            out.push(args[i].clone());
            i += 1;
        }
    }
    out
}

/// The argument following `-primary-file` in a job, if any.
fn primary_file(tokens: &[String]) -> Option<String> {
    tokens
        .iter()
        .position(|t| t == "-primary-file")
        .and_then(|i| tokens.get(i + 1))
        .cloned()
}

fn prepend(first: &str, rest: &[String]) -> Vec<String> {
    let mut v = Vec::with_capacity(rest.len() + 1);
    v.push(first.to_string());
    v.extend_from_slice(rest);
    v
}

#[allow(clippy::similar_names)] // prog/program/argv/args are the natural names here
fn run(prog: &str, argv: &[String], what: &str) -> Result<(), String> {
    if argv.is_empty() {
        return Err(format!("{what}: empty command"));
    }
    // `prog == ""` means argv is self-contained (argv[0] is the program).
    let (program, args) = if prog.is_empty() {
        (argv[0].as_str(), &argv[1..])
    } else {
        (prog, argv)
    };
    let out = Command::new(program)
        .args(args)
        .output()
        .map_err(|e| format!("spawn {what}: {e}"))?;
    if out.status.success() {
        Ok(())
    } else {
        Err(format!(
            "{what} failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ))
    }
}

/// Strip shell-escaping backslashes from a transcript token (`a\=b` -> `a=b`).
fn unescape(t: &str) -> String {
    let mut out = String::with_capacity(t.len());
    let mut chars = t.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            if let Some(n) = chars.next() {
                out.push(n);
            }
        } else {
            out.push(c);
        }
    }
    out
}

const DROP_WITH_NEXT: &[&str] = &[
    "-o",
    "-output-file-map",
    "-supplementary-output-file-map",
    "-serialize-diagnostics-path",
    "-emit-dependencies-path",
    "-emit-reference-dependencies-path",
    "-emit-module-path",
    "-emit-module-doc-path",
    "-emit-module-source-info-path",
    "-emit-objc-header-path",
    "-emit-const-values-path",
    "-index-store-path",
    "-index-unit-output-path",
    "-pch-output-dir",
    "-const-gather-protocols-file",
];

const DROP_STANDALONE: &[&str] = &["-frontend-parseable-output", "-emit-module"];

/// Rewrite the recovered frontend command to compile only `source` into one
/// object: drop output geometry, keep a single `-primary-file <source>` (other
/// primaries become plain secondary inputs), append `-c -o <object>`.
fn single_file_command(
    tokens: &[String],
    source: &str,
    object: &Path,
) -> Result<Vec<String>, String> {
    let mut out: Vec<String> = Vec::with_capacity(tokens.len());
    let mut i = 0;
    let mut kept_primary = false;
    while i < tokens.len() {
        let t = tokens[i].as_str();
        if DROP_WITH_NEXT.contains(&t) {
            i += 2;
            continue;
        }
        if DROP_STANDALONE.contains(&t) {
            i += 1;
            continue;
        }
        if t == "-primary-file" {
            let file = tokens.get(i + 1).ok_or("dangling -primary-file")?;
            if file.ends_with(source) || source.ends_with(file.as_str()) {
                out.push("-primary-file".into());
                out.push(file.clone());
                kept_primary = true;
            } else {
                out.push(file.clone());
            }
            i += 2;
            continue;
        }
        out.push(t.to_string());
        i += 1;
    }
    if !kept_primary {
        return Err(format!(
            "source {source} was not a -primary-file in the command"
        ));
    }
    if !out.iter().any(|t| t == "-c" || t == "-emit-object") {
        out.push("-c".into());
    }
    out.push("-o".into());
    out.push(object.to_string_lossy().into_owned());
    if !out[0].contains('/') {
        out = prepend("xcrun", &out);
    }
    Ok(out)
}

fn token_after<'a>(tokens: &'a [String], flag: &str) -> Option<&'a str> {
    tokens
        .iter()
        .position(|t| t == flag)
        .and_then(|i| tokens.get(i + 1))
        .map(String::as_str)
}

/// Build the `clang` link line for a loadable simulator dylib, reusing the
/// build's own `-target`/`-sdk` so the ABI matches.
fn link_command(tokens: &[String], object: &Path, dylib: &Path) -> Result<Vec<String>, String> {
    let triple = token_after(tokens, "-target").ok_or("no -target in frontend command")?;
    let sdk = token_after(tokens, "-sdk")
        .map(str::to_string)
        .or_else(|| {
            Command::new("xcrun")
                .args(["--sdk", "iphonesimulator", "--show-sdk-path"])
                .output()
                .ok()
                .filter(|o| o.status.success())
                .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        })
        .ok_or("could not resolve an SDK path")?;
    Ok([
        "xcrun",
        "clang",
        "-target",
        triple,
        "-isysroot",
        &sdk,
        "-dynamiclib",
        "-undefined",
        "dynamic_lookup",
        "-Xlinker",
        "-interposable",
        &object.to_string_lossy(),
        "-o",
        &dylib.to_string_lossy(),
    ]
    .map(String::from)
    .to_vec())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mode_parses_aliases() {
        assert_eq!(Mode::parse("resolver"), Some(Mode::Resolver));
        assert_eq!(Mode::parse("F"), Some(Mode::Resolver));
        assert_eq!(Mode::parse("buildlog"), Some(Mode::BuildLog));
        assert_eq!(Mode::parse("a"), Some(Mode::BuildLog));
        assert_eq!(Mode::parse("nope"), None);
    }

    #[test]
    fn unescape_strips_backslashes() {
        assert_eq!(
            unescape(r"-enforce-exclusivity\=checked"),
            "-enforce-exclusivity=checked"
        );
        assert_eq!(unescape("plain"), "plain");
    }

    #[test]
    fn single_file_keeps_our_primary_and_demotes_others() {
        let line = "/x/swift-frontend -frontend -c -primary-file /p/ContentView.swift \
                    -primary-file /p/App.swift -o /d/x.o -target arm64-apple-ios16.0-simulator";
        let toks: Vec<String> = line.split_whitespace().map(unescape).collect();
        let cmd =
            single_file_command(&toks, "/p/ContentView.swift", Path::new("/t/eval.o")).unwrap();
        // ContentView stays primary; App.swift demoted to a bare input; one -o added.
        assert!(
            cmd.windows(2)
                .any(|w| w[0] == "-primary-file" && w[1].ends_with("ContentView.swift"))
        );
        assert!(cmd.contains(&"/p/App.swift".to_string()));
        assert!(
            !cmd.windows(2)
                .any(|w| w[0] == "-primary-file" && w[1].ends_with("App.swift"))
        );
        assert_eq!(cmd.last().unwrap(), "/t/eval.o");
        // The original -o /d/x.o was dropped.
        assert!(!cmd.contains(&"/d/x.o".to_string()));
    }

    #[test]
    fn single_file_errors_when_source_not_primary() {
        let toks: Vec<String> = "swift-frontend -c -primary-file /p/Other.swift /p/X.swift"
            .split_whitespace()
            .map(unescape)
            .collect();
        assert!(single_file_command(&toks, "/p/X.swift", Path::new("/t/e.o")).is_err());
    }

    #[test]
    fn sanitize_drops_orphaned_geometry_flags() {
        let args: Vec<String> = [
            "-module-name",
            "App",
            "-validate-clang-modules-once",
            "-clang-build-session-file",
            "/x/session",
            "-explicit-module-build",
            "-emit-const-values",
            "-sdk",
            "/SDK",
        ]
        .map(String::from)
        .to_vec();
        let out = sanitize_driver_args(&args);
        assert_eq!(out, vec!["-module-name", "App", "-sdk", "/SDK"]);
    }

    #[test]
    fn parses_quoted_dash_dash_dash_tokens() {
        let line = r#"  "/x/swift-frontend" "-frontend" "-c" "-primary-file" "/p/A.swift" "-target" "arm64-apple-ios16.0-simulator""#;
        let t = parse_quoted_tokens(line);
        assert_eq!(t[0], "/x/swift-frontend");
        assert!(t.contains(&"-primary-file".to_string()));
        assert!(t.contains(&"/p/A.swift".to_string()));
        // Unquoted fallback still splits on whitespace.
        assert_eq!(parse_quoted_tokens("a b c"), vec!["a", "b", "c"]);
    }

    #[test]
    fn parse_frontend_jobs_picks_compile_jobs_only() {
        let out = concat!(
            "\"/x/swift-frontend\" \"-frontend\" \"-c\" \"-primary-file\" \"/p/A.swift\" \"/p/B.swift\" \"-o\" \"/t/A.o\"\n",
            "\"/x/swift-frontend\" \"-frontend\" \"-c\" \"/p/A.swift\" \"-primary-file\" \"/p/B.swift\" \"-o\" \"/t/B.o\"\n",
            "\"/x/swift-frontend\" \"-frontend\" \"-emit-module\" \"/p/A.swift\" \"/p/B.swift\"\n",
            "\"/x/ld\" \"/t/A.o\" \"/t/B.o\" \"-o\" \"/t/app\"\n",
        );
        let jobs = parse_frontend_jobs(out);
        // Two compile jobs (A, B); the emit-module and link lines are skipped.
        assert_eq!(jobs.len(), 2);
        assert_eq!(primary_file(&jobs[0]).as_deref(), Some("/p/A.swift"));
        assert_eq!(primary_file(&jobs[1]).as_deref(), Some("/p/B.swift"));
    }

    #[test]
    fn link_command_uses_build_target_and_sdk() {
        let toks: Vec<String> =
            "swift-frontend -target arm64-apple-ios16.0-simulator -sdk /SDKs/Sim.sdk"
                .split_whitespace()
                .map(unescape)
                .collect();
        let cmd = link_command(&toks, Path::new("/t/e.o"), Path::new("/t/e.dylib")).unwrap();
        assert!(cmd.contains(&"arm64-apple-ios16.0-simulator".to_string()));
        assert!(cmd.contains(&"/SDKs/Sim.sdk".to_string()));
        assert!(cmd.contains(&"-dynamiclib".to_string()));
        assert!(
            cmd.windows(2)
                .any(|w| w[0] == "-Xlinker" && w[1] == "-interposable")
        );
    }
}
