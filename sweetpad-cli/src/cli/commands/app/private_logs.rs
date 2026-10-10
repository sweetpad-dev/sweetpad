//! A macOS app's private log values (CLI_DESIGN §9v).
//!
//! On a Mac the unified log records a dynamic value, such as a `String`
//! interpolated into a `Logger` message, as `<private>`, and `log stream` reads
//! what it records. An app opts a subsystem in from its own Info.plist:
//!
//! ```text
//! OSLogPreferences → <subsystem> → DEFAULT-OPTIONS → Enable-Private-Data = true
//! ```
//!
//! A key at the top level does nothing, and a log with no subsystem, NSLog's
//! or a `Logger()`'s, can't be opted in at all. So after each debuggable build
//! of the app, its subsystems are added to the built Info.plist and the bundle
//! is re-signed by the identity that signed it. The subsystems are the bundle
//! identifier, which Apple recommends as one, and every subsystem the log
//! stream has shown from the app before ([`Watch`] records them).

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use sweetpad_lib::pbxproj::Value;

use super::Senders;
use crate::cli::output::Output;
use crate::cli::state::{LogSubsystems, Recorded};
use crate::cli::xcodebuild::AppBundle;
use crate::cli::{CliError, oslog, process};

const PLIST_BUDDY: &str = "/usr/libexec/PlistBuddy";

/// The built app's Info.plist, for a macOS bundle.
fn info_plist(app: &AppBundle) -> Option<PathBuf> {
    Some(app.path.join("Contents/Info.plist")).filter(|p| p.is_file())
}

/// Opt `app`'s subsystems in, returning the ones added. A subsystem whose
/// `Enable-Private-Data` the app's own Info.plist sets, on or off, is left as
/// the app has it, and nothing is re-signed when nothing is added. A bundle
/// that isn't debuggable, a Release build say, is left alone: Xcode shows
/// private values only for an app a debugger may attach to, and so does this.
pub(super) fn opt_in(app: &AppBundle) -> Result<Vec<String>, CliError> {
    let Some(plist) = info_plist(app) else {
        return Ok(Vec::new());
    };
    let set = private_data(&plist)?;
    let mut wanted = LogSubsystems::load().of(&app.bundle_id);
    if !app.bundle_id.is_empty() {
        wanted.insert(app.bundle_id.clone());
    }
    let added: Vec<String> = wanted
        .into_iter()
        .filter(|s| addressable(s) && !set.contains_key(s))
        .collect();
    if added.is_empty() || !super::signed_entitlements(&app.path).debuggable {
        return Ok(Vec::new());
    }
    let original = std::fs::read(&plist)
        .map_err(|e| CliError::new(format!("reading {}: {e}", plist.display())))?;
    let mut args = Vec::new();
    for subsystem in &added {
        args.push("-c".to_string());
        args.push(format!(
            "Add :OSLogPreferences:{subsystem}:DEFAULT-OPTIONS:Enable-Private-Data bool true"
        ));
    }
    args.push(plist.display().to_string());
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    // PlistBuddy saves the commands that worked even when one fails, so a
    // failure at either step puts the Info.plist back, and the signature
    // covers it again: the app launches as it was built.
    if let Err(e) = run(PLIST_BUDDY, &args) {
        let _ = std::fs::write(&plist, &original);
        return Err(e);
    }
    if let Err(e) = resign(&app.path) {
        let _ = std::fs::write(&plist, &original);
        let _ = resign(&app.path);
        return Err(e);
    }
    Ok(added)
}

/// Whether PlistBuddy can name `subsystem` in a command: it splits a key path
/// on `:` and a command on whitespace, and reads quotes and backslashes.
/// Reverse-DNS names hold none of them.
fn addressable(subsystem: &str) -> bool {
    !subsystem.is_empty()
        && !subsystem.contains(|c: char| c.is_whitespace() || matches!(c, ':' | '"' | '\'' | '\\'))
}

/// What the Info.plist at `plist` sets `Enable-Private-Data` to, by subsystem.
/// A subsystem it configures without that key isn't in the map.
fn private_data(plist: &Path) -> Result<BTreeMap<String, bool>, CliError> {
    // plutil reads either plist format, and writes the binary one the bplist
    // reader takes; its output is bytes, not text, so it's read raw.
    let output = std::process::Command::new("plutil")
        .args(["-convert", "binary1", "-o", "-"])
        .arg(plist)
        .output()
        .map_err(|e| process::spawn_error("plutil", &e))?;
    if !output.status.success() {
        return Err(CliError::new(format!(
            "plutil can't read {}: {}",
            plist.display(),
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    let root = sweetpad_lib::bplist::parse(&output.stdout)
        .map_err(|e| CliError::new(format!("reading {}: {e}", plist.display())))?;
    Ok(read_private_data(&root))
}

/// [`private_data`] from a parsed Info.plist, where the bplist reader spells
/// `true` as `YES`.
fn read_private_data(root: &Value) -> BTreeMap<String, bool> {
    let Some(preferences) = root.get("OSLogPreferences").and_then(Value::as_dict) else {
        return BTreeMap::new();
    };
    preferences
        .iter()
        .filter_map(|(subsystem, options)| {
            let value = options
                .get("DEFAULT-OPTIONS")?
                .get("Enable-Private-Data")?
                .as_str()?;
            Some((subsystem.clone(), value == "YES"))
        })
        .collect()
}

/// Re-sign `app` with the identity that signed it, keeping its identifier,
/// entitlements, flags and hardened runtime, then check the result.
fn resign(app: &Path) -> Result<(), CliError> {
    let identity = signing_identity(app)?;
    let app_arg = app.display().to_string();
    run(
        "codesign",
        &[
            "--force",
            "--sign",
            &identity,
            "--preserve-metadata=identifier,entitlements,flags,runtime",
            "--timestamp=none",
            &app_arg,
        ],
    )?;
    run("codesign", &["--verify", &app_arg]).map(|_| ())
}

/// The identity that signed `app`: `-` for an ad-hoc signature, otherwise the
/// SHA-1 of the leaf certificate the signature carries. codesign takes the
/// hash in place of a name, and unlike the name it can't match two
/// certificates, which a renewed development certificate leaves behind.
fn signing_identity(app: &Path) -> Result<String, CliError> {
    let app_arg = app.display().to_string();
    // codesign writes what it displays to stderr, which the capture holds.
    let info = process::run_captured("codesign", &["-d", "--verbose=2", &app_arg], None)?;
    if info
        .combined
        .lines()
        .any(|line| line.trim() == "Signature=adhoc")
    {
        return Ok("-".to_string());
    }
    let dir = sweetpad_core::scratch::ScratchDir::new("sweetpad-certs")
        .map_err(|e| CliError::new(format!("creating a scratch directory: {e}")))?;
    let prefix = dir.join("cert");
    run(
        "codesign",
        &[
            "-d",
            &format!("--extract-certificates={}", prefix.display()),
            &app_arg,
        ],
    )?;
    let leaf = prefix.with_file_name("cert0");
    run("shasum", &["-a", "1", &leaf.display().to_string()])?
        .split_whitespace()
        .next()
        .filter(|hash| hash.len() == 40)
        .map(str::to_string)
        .ok_or_else(|| CliError::new(format!("{} carries no signing certificate", app.display())))
}

/// Run `program` to completion, returning what it printed, or its last lines
/// as the error.
fn run(program: &str, args: &[&str]) -> Result<String, CliError> {
    let run = process::run_captured(program, args, None)?;
    if !run.success {
        return Err(CliError::new(format!(
            "{program} failed: {}",
            run.tail.trim()
        )));
    }
    Ok(run.combined)
}

/// What the running build of an app sets up, read the first time a note
/// needs it.
struct Build {
    /// Its Info.plist's `Enable-Private-Data`, by subsystem.
    private_data: BTreeMap<String, bool>,
    /// A debugger may attach to it, which the opt-in needs.
    debuggable: bool,
}

impl Build {
    fn read(app: &AppBundle) -> Self {
        Build {
            private_data: info_plist(app)
                .and_then(|plist| private_data(&plist).ok())
                .unwrap_or_default(),
            debuggable: super::signed_entitlements(&app.path).debuggable,
        }
    }
}

/// The kinds of `<private>` value a session explains, each once.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Kind {
    NSLog,
    /// Logged with no subsystem: a `Logger()` or `os_log` to the default log.
    Unnamed,
    Subsystem,
}

impl Kind {
    fn of(line: &oslog::Line) -> Self {
        match (&line.subsystem, line.nslog) {
            (_, true) => Kind::NSLog,
            (None, false) => Kind::Unnamed,
            (Some(_), false) => Kind::Subsystem,
        }
    }
}

/// What the log stream's reader does with a macOS app's entries besides
/// showing them: it records each subsystem the app logs to, for the next build
/// to opt in, and explains the first `<private>` value of each [`Kind`] the
/// session shows.
pub(super) struct Watch {
    app: AppBundle,
    /// Subsystems recorded for the app.
    known: BTreeSet<String>,
    /// Subsystems seen but not recorded, which no build opts in: the app has
    /// as many as the file keeps, or the file couldn't be written.
    unrecorded: BTreeSet<String>,
    /// Whether the plan opts the app's builds in.
    plan_opts_in: bool,
    senders: Senders,
    out: Output,
    build: Option<Build>,
    explained: BTreeSet<Kind>,
}

impl Watch {
    pub(super) fn new(app: &AppBundle, plan_opts_in: bool, senders: Senders, out: Output) -> Self {
        Watch {
            app: app.clone(),
            known: LogSubsystems::load().of(&app.bundle_id),
            unrecorded: BTreeSet::new(),
            plan_opts_in,
            senders,
            out,
            build: None,
            explained: BTreeSet::new(),
        }
    }

    /// Look at one entry, which the live filter `shown` or hid. A hidden one
    /// still teaches a subsystem but explains nothing. Apple's subsystems
    /// aren't recorded: in the app's process they come from Apple's code, such
    /// as the stub a debug build launches through.
    pub(super) fn saw(&mut self, line: &oslog::Line, shown: bool) {
        if let Some(subsystem) = &line.subsystem
            && !subsystem.starts_with("com.apple.")
            && !self.known.contains(subsystem)
            && !self.unrecorded.contains(subsystem)
        {
            match LogSubsystems::record(&self.app.bundle_id, subsystem) {
                Ok(Recorded::New | Recorded::Known) => self.known.insert(subsystem.clone()),
                Ok(Recorded::Full) | Err(_) => self.unrecorded.insert(subsystem.clone()),
            };
        }
        if !shown || !line.redacted {
            return;
        }
        let kind = Kind::of(line);
        if self.explained.contains(&kind) {
            return;
        }
        if let Some(note) = self.explain(line) {
            self.explained.insert(kind);
            self.out.note(&note);
        }
    }

    /// Why `line` shows a `<private>` value, and what shows it. `None` when
    /// there is nothing to say: NSLog without a debugger, which doesn't reach
    /// the stream, Apple's subsystems, and one the build opts in already.
    fn explain(&mut self, line: &oslog::Line) -> Option<String> {
        if line.nslog {
            return (self.senders == Senders::AppAndNSLog).then(|| {
                "macOS shows NSLog text as <private> while a debugger is attached. To see it \
                 here, log with Logger and mark the value privacy: .public."
                    .to_string()
            });
        }
        let Some(subsystem) = line.subsystem.as_deref() else {
            return Some(format!(
                "macOS shows values logged without a subsystem as <private>, and no Info.plist \
                 key covers them. To see them, create the Logger with a subsystem, such as \
                 Logger(subsystem: \"{}\", category: \"app\"), or mark the value privacy: \
                 .public.",
                self.app.bundle_id
            ));
        };
        if subsystem.starts_with("com.apple.") {
            return None;
        }
        let build = self.build.get_or_insert_with(|| Build::read(&self.app));
        match build.private_data.get(subsystem) {
            Some(true) => None,
            None if self.plan_opts_in
                && build.debuggable
                && addressable(subsystem)
                && !self.unrecorded.contains(subsystem) =>
            {
                Some(format!(
                    "macOS shows values logged to {subsystem} as <private>. sweetpad shows them \
                     after its next build of the app."
                ))
            }
            _ => Some(format!(
                "macOS shows values logged to {subsystem} as <private>. To see them, set \
                 OSLogPreferences > {subsystem} > DEFAULT-OPTIONS > Enable-Private-Data to true \
                 in the app's Info.plist."
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plist_buddy_can_address_reverse_dns_names_only() {
        assert!(addressable("com.example.app"));
        for name in ["", "a:b", "a b", "a\"b", "a'b", "a\\b"] {
            assert!(!addressable(name), "{name:?}");
        }
    }

    #[test]
    fn private_data_reads_each_subsystems_setting() {
        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<plist version="1.0"><dict>
<key>OSLogPreferences</key><dict>
  <key>com.example.on</key><dict><key>DEFAULT-OPTIONS</key><dict><key>Enable-Private-Data</key><true/></dict></dict>
  <key>com.example.off</key><dict><key>DEFAULT-OPTIONS</key><dict><key>Enable-Private-Data</key><false/></dict></dict>
  <key>com.example.level</key><dict><key>Level</key><dict><key>Enable</key><string>Debug</string></dict></dict>
  <key>com.example.dated</key><dict><key>Since</key><date>2026-01-01T00:00:00Z</date></dict>
</dict>
</dict></plist>
"#;
        let dir = sweetpad_core::scratch::ScratchDir::new("sweetpad-private-data").unwrap();
        let plist = dir.join("Info.plist");
        std::fs::write(&plist, xml).unwrap();
        let set = private_data(&plist).unwrap();
        assert_eq!(
            set.into_iter().collect::<Vec<_>>(),
            [
                ("com.example.off".to_string(), false),
                ("com.example.on".to_string(), true)
            ]
        );
    }

    /// A watch for a build that sets `private_data` and is debuggable, which
    /// already knows every subsystem the tests log from, so nothing is
    /// recorded to the state directory.
    fn watch(plan_opts_in: bool, senders: Senders, private_data: &[(&str, bool)]) -> Watch {
        let global = crate::cli::GlobalArgs {
            chdir: None,
            developer_dir: None,
            output: None,
            json: false,
            non_interactive: true,
            no_color: true,
            verbose: false,
            quiet: true,
            gh_annotations: false,
        };
        Watch {
            app: AppBundle {
                path: PathBuf::from("/nonexistent/App.app"),
                bundle_id: "com.example.App".into(),
                executable: PathBuf::from("/nonexistent/App.app/Contents/MacOS/App"),
            },
            known: ["com.example.new", "com.example.App", "my app"]
                .into_iter()
                .map(str::to_string)
                .collect(),
            unrecorded: BTreeSet::new(),
            plan_opts_in,
            senders,
            out: Output::new(&global),
            build: Some(Build {
                private_data: private_data
                    .iter()
                    .map(|(s, on)| ((*s).to_string(), *on))
                    .collect(),
                debuggable: true,
            }),
            explained: BTreeSet::new(),
        }
    }

    fn redacted(subsystem: Option<&str>, nslog: bool) -> oslog::Line {
        oslog::Line {
            level: oslog::Level::Info,
            text: String::new(),
            subsystem: subsystem.map(str::to_string),
            nslog,
            redacted: true,
        }
    }

    #[test]
    fn a_subsystem_the_next_build_opts_in_says_so() {
        let mut w = watch(true, Senders::App, &[("com.example.App", true)]);
        let note = w
            .explain(&redacted(Some("com.example.new"), false))
            .unwrap();
        assert!(note.contains("after its next build"), "{note}");
        // Opted in already, or Apple's: nothing to say.
        assert_eq!(w.explain(&redacted(Some("com.example.App"), false)), None);
        assert_eq!(w.explain(&redacted(Some("com.apple.stub"), false)), None);
    }

    #[test]
    fn a_subsystem_no_build_opts_in_gets_the_info_plist_key() {
        let plist_key = |w: &mut Watch, subsystem: &str| {
            let note = w.explain(&redacted(Some(subsystem), false)).unwrap();
            assert!(
                note.contains(&format!("OSLogPreferences > {subsystem} >")),
                "{note}"
            );
        };
        // Opted out in sweetpad.toml.
        plist_key(&mut watch(false, Senders::App, &[]), "com.example.new");
        // Turned off by the app itself.
        plist_key(
            &mut watch(true, Senders::App, &[("com.example.new", false)]),
            "com.example.new",
        );
        // A name PlistBuddy can't address.
        plist_key(&mut watch(true, Senders::App, &[]), "my app");
        // Past what the file keeps.
        let mut full = watch(true, Senders::App, &[]);
        full.unrecorded.insert("com.example.extra".into());
        plist_key(&mut full, "com.example.extra");
    }

    #[test]
    fn a_log_without_a_subsystem_asks_for_one() {
        let note = watch(true, Senders::App, &[])
            .explain(&redacted(None, false))
            .unwrap();
        assert!(
            note.contains("Logger(subsystem: \"com.example.App\""),
            "{note}"
        );
    }

    #[test]
    fn nslog_is_explained_only_with_a_debugger_attached() {
        let line = redacted(None, true);
        assert_eq!(watch(true, Senders::App, &[]).explain(&line), None);
        let note = watch(true, Senders::AppAndNSLog, &[])
            .explain(&line)
            .unwrap();
        assert!(note.contains("privacy: .public"), "{note}");
    }

    /// An ad-hoc signed `App.app` around a copy of `/usr/bin/true`, carrying
    /// `get-task-allow` when `debuggable`, and `extra` inside its Info.plist's
    /// top-level dict.
    fn signed_bundle(dir: &Path, debuggable: bool, extra: &str) -> AppBundle {
        let bundle_id = "dev.sweetpad.test.PrivateLogs";
        let app = dir.join("App.app");
        let macos = app.join("Contents/MacOS");
        std::fs::create_dir_all(&macos).unwrap();
        std::fs::copy("/usr/bin/true", macos.join("App")).unwrap();
        std::fs::write(
            app.join("Contents/Info.plist"),
            format!(
                "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<plist version=\"1.0\"><dict>\
                 <key>CFBundleIdentifier</key><string>{bundle_id}</string>\
                 <key>CFBundleExecutable</key><string>App</string>{extra}</dict></plist>\n"
            ),
        )
        .unwrap();
        let entitlements = dir.join("app.entitlements");
        std::fs::write(
            &entitlements,
            format!(
                "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<plist version=\"1.0\"><dict>\
                 <key>com.apple.security.get-task-allow</key><{debuggable}/></dict></plist>\n"
            ),
        )
        .unwrap();
        run(
            "codesign",
            &[
                "--force",
                "--sign",
                "-",
                "--entitlements",
                &entitlements.display().to_string(),
                &app.display().to_string(),
            ],
        )
        .unwrap();
        AppBundle {
            path: app,
            bundle_id: bundle_id.into(),
            executable: macos.join("App"),
        }
    }

    fn verifies(app: &AppBundle) -> bool {
        run("codesign", &["--verify", &app.path.display().to_string()]).is_ok()
    }

    #[test]
    #[cfg_attr(not(target_os = "macos"), ignore = "signs with codesign")]
    fn opting_in_adds_the_bundle_id_and_keeps_a_valid_signature() {
        let dir = sweetpad_core::scratch::ScratchDir::new("sweetpad-private-logs").unwrap();
        let app = signed_bundle(&dir, true, "");
        assert_eq!(opt_in(&app).unwrap(), std::slice::from_ref(&app.bundle_id));
        let plist = info_plist(&app).unwrap();
        assert_eq!(
            private_data(&plist).unwrap().get(&app.bundle_id),
            Some(&true)
        );
        assert!(verifies(&app));
        assert!(super::super::signed_entitlements(&app.path).debuggable);
        // Already opted in: nothing to add, nothing re-signed.
        assert!(opt_in(&app).unwrap().is_empty());
    }

    #[test]
    #[cfg_attr(not(target_os = "macos"), ignore = "signs with codesign")]
    fn a_failed_edit_leaves_the_built_app_as_it_was() {
        let dir = sweetpad_core::scratch::ScratchDir::new("sweetpad-private-logs").unwrap();
        // A string where the subsystems' dict belongs: PlistBuddy can't add
        // to it.
        let app = signed_bundle(
            &dir,
            true,
            "<key>OSLogPreferences</key><string>not a dict</string>",
        );
        let plist = info_plist(&app).unwrap();
        let before = std::fs::read(&plist).unwrap();
        assert!(opt_in(&app).is_err());
        assert_eq!(std::fs::read(&plist).unwrap(), before);
        assert!(verifies(&app));
    }

    #[test]
    #[cfg_attr(not(target_os = "macos"), ignore = "signs with codesign")]
    fn a_build_no_debugger_may_attach_to_is_left_alone() {
        let dir = sweetpad_core::scratch::ScratchDir::new("sweetpad-private-logs").unwrap();
        let app = signed_bundle(&dir, false, "");
        let before = std::fs::read(app.path.join("Contents/Info.plist")).unwrap();
        assert!(opt_in(&app).unwrap().is_empty());
        assert_eq!(
            std::fs::read(app.path.join("Contents/Info.plist")).unwrap(),
            before
        );
    }

    #[test]
    fn each_kind_is_explained_once_and_only_when_shown() {
        let mut w = watch(true, Senders::AppAndNSLog, &[]);
        let logger = redacted(Some("com.example.new"), false);
        w.saw(&logger, false);
        assert!(w.explained.is_empty(), "a hidden line explains nothing");
        w.saw(&redacted(Some("com.apple.stub"), false), true);
        assert!(
            w.explained.is_empty(),
            "Apple's lines leave the note unused"
        );
        w.saw(&logger, true);
        w.saw(&redacted(None, true), true);
        w.saw(&redacted(None, false), true);
        assert_eq!(
            w.explained.iter().copied().collect::<Vec<_>>(),
            [Kind::NSLog, Kind::Unnamed, Kind::Subsystem]
        );
    }
}
