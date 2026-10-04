//! `sweetpad help <topic>` — the design doc's best sections, shipped into the
//! binary: configuration, environment variables, exit codes, destinations,
//! hot reload, and feedback reports. `sweetpad help` lists the topics; an
//! unknown topic points at `sweetpad <command> --help` for command help.

use crate::cli::output::Output;
use crate::cli::{CliError, CommandResult, Context, ErrorKind, Render, Rendered};

/// One help topic: its name, a one-line summary for the listing, and the body.
struct Topic {
    name: &'static str,
    summary: &'static str,
    body: &'static str,
}

/// What `help feedback` says once the user has turned reports off, in place
/// of the guidance for sending one.
const FEEDBACK_OFF: &str = "\
FEEDBACK

The user turned off feedback reports ('sweetpad feedback off'). Don't offer to
send one, and don't bring it up.

The user can turn them back on with 'sweetpad feedback on'.";

const TOPICS: [Topic; 6] = [
    Topic {
        name: "config",
        summary: "the config file: location, keys, per-project overrides",
        body: "\
CONFIGURATION (hand-authored; sweetpad edits only its [feedback] table)

  ~/.config/sweetpad/config.toml        (honors XDG_CONFIG_HOME)

Global defaults plus per-project overrides. Project keys are the canonicalized
CONTAINER path — the .xcworkspace / .xcodeproj / Package.swift itself, not the
directory holding it:

  [defaults]
  configuration = \"Debug\"

  [projects.\"/Users/me/code/MyApp/MyApp.xcodeproj\"]
  scheme = \"MyApp\"
  destination = \"platform=iOS Simulator,name=iPhone 15\"
  sdk = \"iphonesimulator\"          # rarely needed; the destination implies it

  [projects.\"/Users/me/code/MyApp/MyApp.xcodeproj\".testing]
  configuration = \"Test\"           # test-only overrides; falls back to build
  target = \"MyAppTests\"            # default -only-testing: target

Unknown keys and project keys that can't match a real container are warned
about on every run — a typo is never silently ignored.

'sweetpad feedback off' writes the one key sweetpad sets itself, and
'sweetpad feedback on' sets it back to true. Both change only that key and
keep the rest of the file, comments included:

  [feedback]
  enabled = false                  # agents don't offer problem reports

Resolution precedence, highest first:

  explicit flag > env var > config file > sweetpad.toml > remembered state
  > auto-discovery

A committed 'sweetpad.toml' is the team-shared defaults layer
(scheme/configuration/destination/sdk, 'developer_dir', '[run]', '[format]',
'[testing]', '[xcodebuild]') — personal config beats it, remembered picks
yield to it. It is found by walking up from the working directory to the git
root, so one file serves the whole checkout.

'[xcodebuild] args' is the repo-wide version of the '--' tail — a list added
to every command that builds, so a flag the project always needs is written
down once instead of typed each time:

  [xcodebuild]
  args = [\"-skipMacroValidation\"]

A typed '--' tail is appended after it, so it wins for anything both set
(xcodebuild takes the last value). A flag xcodebuild takes only once, such
as -xcconfig, is left out of the file's list when the tail gives it, and -v
says so. A flag only testing takes (-enableCodeCoverage, -testPlan,
-testLanguage, -testRegion, -testProductsPath) reaches only 'test' and 'test
build', since build, archive and clean fail on it, and -v names what a
command left out. 'sweetpad status' prints the effective
list. The arguments sweetpad settles itself are refused there, naming the key
to use instead: -scheme, -configuration, -destination, -sdk, -workspace,
-project, -resultBundlePath (use 'test --result-bundle'), -archivePath,
-exportPath and -exportOptionsPlist (use 'archive --output-file' and
'--export-options'), and -derivedDataPath (a relative value would
mean a different directory depending on where the command ran — pass that one
per command). Swift packages ignore the table: they build with 'swift build'.

The '--' tail can't repeat what sweetpad passes itself either, since
xcodebuild fails on a second copy: -scheme, -configuration, -sdk, -workspace
and -project there are a usage error naming the flag to use, as are test's
-resultBundlePath and archive's own paths. A build writes a typed
-resultBundlePath in place of its own.

When the project is not a sibling of that file, name it with 'workspace' or
'project', relative to the file itself:

  # App/sweetpad.toml
  project = \"Sources/App.xcodeproj\"

Every command then works from anywhere in the checkout, with no '-C'. The
container resolves as: --workspace/--project > this key > auto-discovery.

Auto-discovery searches upward to the git root, then up to two levels down
(skipping Pods, node_modules, Carthage, vendor, DerivedData, build,
SourcePackages and dotfiles), so nested layouts like 'ios/App.xcodeproj' need no
setup at all. Projects tied at the same depth — say 'ios/' and 'macos/' — are
reported as an error listing each one, rather than picked for you; name the one
you want with --project or the 'project' key above.

Remembered state lives separately in
~/.local/state/sweetpad/state.toml (machine-managed — inspect and edit it
with 'sweetpad context', not by hand).",
    },
    Topic {
        name: "environment",
        summary: "every SWEETPAD_* variable, and the color/CI controls",
        body: "\
ENVIRONMENT VARIABLES

Value-carrying (folded into the flag layer; a typed flag still wins, and a
variable set to the empty string means unset):

  SWEETPAD_WORKSPACE        path to the .xcworkspace
  SWEETPAD_PROJECT          path to the .xcodeproj
  SWEETPAD_SCHEME           scheme name
  SWEETPAD_CONFIGURATION    build configuration
  SWEETPAD_DESTINATION      raw -destination specifier
  SWEETPAD_ON               human destination reference (see 'help
                            destinations'); overrides SWEETPAD_DESTINATION
  SWEETPAD_SDK              -sdk override
  DEVELOPER_DIR             the Xcode every spawned tool uses (the
                            --developer-dir flag; a sweetpad.toml
                            developer_dir pins it per project)

Boolean (truthy parsing: 0 / false / no / off / empty mean OFF):

  SWEETPAD_NONINTERACTIVE   never prompt; missing values become errors
  CI                        same as SWEETPAD_NONINTERACTIVE (set by CI runners)

Color:

  NO_COLOR                  disable color when set non-empty (no-color.org)
  CLICOLOR_FORCE / FORCE_COLOR
                            force color on even when piped; an explicit
                            --no-color / NO_COLOR still wins

Hot reload:

  SWEETPAD_HOTRELOAD_DYLIB  override the injection client dylib path (CI)

Debug adapter ('sweetpad dap'):

  SWEETPAD_LLDB_DAP         the lldb-dap to start in place of 'xcrun lldb-dap'
  SWEETPAD_DAP_LOG          file that records every message between the
                            editor and lldb-dap",
    },
    Topic {
        name: "exit-codes",
        summary: "what each process exit code means",
        body: "\
EXIT CODES

  0   success
  1   generic failure
  2   usage error (bad flags/arguments, a flag the command refuses, like
      '--failed' on 'test build', or a flag a prompt needs off a terminal,
      like '--yes' for 'derived-data purge')
  3   build or test failure
  4   target resolution failed (unknown/missing scheme, destination,
      simulator, device, …)
  5   a required tool is missing (xcodebuild, simctl, …)
  6   cancelled by the user (a declined prompt, or Ctrl-C while a run session
      builds)

A SIGINT/SIGTERM that kills the process exits 128+signo (130/143) after the
handler restores the terminal and reaps children.

A run session ('sweetpad run' at a terminal) exits 6 when Ctrl-C stops one
of its builds, even if the app ran before. Quitting ('q', or Ctrl-C or Ctrl-D
at the prompt) exits 0 once the app has run. If it never ran, the exit is 3
when the last build failed, or 1 when the app built but didn't launch. Under --json, errors are
'{\"schema\":1,\"ok\":false,\"error\":{code,message}}' on stderr, where 'code'
is the same taxonomy: generic, usage_error, build_failure, target_resolution,
tool_missing, user_cancel. A flag clap can't parse is reported in clap's own
text, even under --json.

'ok: true' means \"the command executed\", not \"the outcome was good\": a red
test suite exits 3 with 'data.passed: false'; read the payload's own status
field.",
    },
    Topic {
        name: "destinations",
        summary: "destination specifiers, and how the picker chooses",
        body: "\
DESTINATIONS

A destination tells xcodebuild where to build for / run on. The raw specifier
forms (usable with --destination or SWEETPAD_DESTINATION):

  platform=iOS Simulator,name=iPhone 16 Pro
  platform=iOS Simulator,id=<UDID>
  platform=iOS,id=<device UDID>
  platform=macOS

'sweetpad devices' prints every runnable target with a ready specifier;
'sweetpad simulator list' and 'sweetpad device list' show each pool, and
'sweetpad device info' connects to a physical device to say whether it is
ready.

When no destination is given, an interactive terminal gets the picker:
ordered most-used-first (per project), then booted (marked ●), then the
resolver's platform/OS ordering — your habitual target sits on top. The pick
is remembered per project (see 'sweetpad context show'); one-off --destination
values and 'app run --mac'/'--device' targets are never remembered.",
    },
    Topic {
        name: "hot-reload",
        summary: "app run --hot: requirements, recompilers, SwiftUI notes",
        body: "\
HOT RELOAD ('sweetpad app run --hot')

Each Swift save is recompiled and injected into the running app — no relaunch,
state preserved. Targets: the iOS Simulator, and native macOS apps
('--mac --hot'). Physical devices strip DYLD_INSERT_LIBRARIES and can't inject.

How it works: the app is built with -Xlinker -interposable, launched with the
injection client dylib, and a watcher recompiles saved files and streams the
result into the process. 'r' still does a full rebuild+relaunch; 'q' quits.

The injection client: release builds of sweetpad (Homebrew) bundle one for
the iOS Simulator and macOS. A sweetpad built from source has one only if
'sweetpad-cli/vendor/injection-client/build.sh' ran before 'cargo build'.
Without a client, '--hot' stops before building and says so. Pointing
SWEETPAD_HOTRELOAD_DYLIB at a client dylib also works, as does an installed
InjectionNext.app.

macOS: the hot build also passes ENABLE_HARDENED_RUNTIME=NO and
ENABLE_APP_SANDBOX=NO so the Debug product is injectable (dyld honors the
insert; ad-hoc recompiled dylibs load). An App Sandbox set in an explicit
.entitlements file can't be overridden — turn it off for Debug; the preflight
tells you when that's the case. Prefs/files of an unsandboxed hot run live in
~/Library instead of the container.

Recompilers (--hot-recompiler):

  resolver    whole-module compile via the in-process build-settings resolver
              (default; robust)
  buildlog    single-file compile recovered from the build transcript (fast)

SwiftUI: views need the Inject package to redraw on injection — add
https://github.com/krzysztofzablocki/Inject and annotate views with
@ObserveInjection + .enableInjection(). UIKit/AppKit apps need nothing.

One session owns port 8887, so a session that died without unwinding leaves the
listener bound and later runs fail with 'Address already in use'. 'sweetpad hot
status' names the holder; 'sweetpad hot reset' ends it when it is a sweetpad
process, and takes --force for anything else (e.g. InjectionNext.app).

CI: SWEETPAD_HOTRELOAD_DYLIB overrides the client dylib; the hidden
--hot-selfcheck FILE flag drives the end-to-end injection test.",
    },
    Topic {
        name: "feedback",
        summary: "for agents: when and how to send a sweetpad problem report",
        body: "\
FEEDBACK (problem reports about sweetpad, sent by an agent)

This topic is for an AI agent that drives sweetpad for a user. When sweetpad
itself goes wrong, you can offer to send its maintainer a short report. It
goes to the maintainer's Sentry project as user feedback. sweetpad never
prompts: the user approves twice, once to write a report and once to send it.

Offer a report when:

  - sweetpad crashed, hung, or reported an internal error
  - its output contradicted itself, or contradicted Xcode
  - a tip or command it suggested didn't work
  - it lacked something, so you fell back to raw xcodebuild, simctl or
    devicectl
  - its help or docs disagreed with what it did

Don't offer one for problems that are the user's to fix: compile errors,
failing tests, code signing setup, missing simulator runtimes, a mistyped
command that sweetpad refused clearly, or an environment problem it
reported correctly.

Offer once per issue, and don't press. If the user says no, don't offer
again for that issue.

Sending one:

  1. Ask the user whether they'd like to send a report about the issue.
  2. If they would, write the report to a file (the format is below) and run
       sweetpad feedback submit <file> --dry-run
     This sends nothing. It prints the exact payload and its digest.
  3. Show the user all of that output, and ask whether to send it.
  4. Only if they say yes, run
       sweetpad feedback submit <file> --approve <digest>
     It sends only if the payload still has that digest. After any change to
     the file, run the dry run again and show the user the new output.

Clean the report before the dry run. sweetpad sends the text as written and
doesn't scrub it. Remove these, or replace them with a placeholder such as
<scheme>:

  - project, workspace, scheme, target and package names
  - bundle ids and team ids
  - home directories and other absolute paths
  - device names and UDIDs
  - user names and host names
  - email addresses
  - private URLs
  - tokens, keys and passwords

Keep what the maintainer needs to reproduce the problem: versions, the
command with its values replaced by placeholders ('sweetpad run --scheme
<scheme> --on <simulator>'), and the exact error text with the names
replaced.

The report file is one entry in the sweetpad-feedback log format:

  ## 2026-09-27T10:00Z · bug · medium
  - **Context:** an iOS app in a workspace; the user asked to run it
  - **Command:** sweetpad run --on <simulator> --no-logs
  - **Expected:** the app launches and the command returns
  - **Actual:** exit 1: error: couldn't find the built app for <scheme>
  - **Assumption or gap:** the build succeeded, so the app exists
  - **Fix idea:** look for the app where the build wrote it

The heading gives the kind (bug, gap, unclear, skill-wrong, docs-wrong or
friction) and the severity (low, medium or high). Its timestamp is optional
and isn't sent. Every field needs a value except 'Fix idea', and a value may
run over several lines. A 'Seen' line is read and isn't sent.

sweetpad adds its own version, the Xcode version, the macOS version and the
Mac's architecture. It adds no IP address, user name, host name or path.

The user can turn reports off with 'sweetpad feedback off'.",
    },
];

/// The topic listing / body payload.
struct HelpText {
    body: String,
    /// Machine-readable form: topic name(s) and text.
    json: serde_json::Value,
}

impl Render for HelpText {
    fn human(&self, out: &Output) {
        out.line(&self.body);
    }

    fn json(&self) -> serde_json::Value {
        self.json.clone()
    }
}

pub fn run(_ctx: &mut Context, topic: Option<&str>) -> CommandResult {
    match topic {
        None => Ok(Rendered::data(listing())),
        Some(name) => {
            let Some(topic) = TOPICS.iter().find(|t| t.name == name) else {
                let names: Vec<&str> = TOPICS.iter().map(|t| t.name).collect();
                return Err(CliError::new(format!(
                    "unknown help topic {name:?} (topics: {}) — for command help, run \
                     'sweetpad {name} --help'",
                    names.join(", ")
                ))
                .kind(ErrorKind::Usage));
            };
            // A config that doesn't parse has already warned at startup, and
            // 'feedback submit' refuses on it; the guidance still reads here.
            let body = if topic.name == "feedback"
                && crate::cli::commands::feedback::enabled() == Ok(false)
            {
                FEEDBACK_OFF
            } else {
                topic.body
            };
            Ok(Rendered::data(HelpText {
                body: body.to_string(),
                json: serde_json::json!({ "topic": topic.name, "text": body }),
            }))
        }
    }
}

/// `sweetpad help` — the topic index.
fn listing() -> HelpText {
    use std::fmt::Write as _;
    let mut body = String::from("Help topics (run 'sweetpad help <topic>'):\n\n");
    for t in &TOPICS {
        let _ = writeln!(body, "  {:<13} {}", t.name, t.summary);
    }
    body.push_str("\nFor command help, run 'sweetpad <command> --help'.");
    let topics: Vec<serde_json::Value> = TOPICS
        .iter()
        .map(|t| serde_json::json!({ "topic": t.name, "summary": t.summary }))
        .collect();
    HelpText {
        body,
        json: serde_json::json!({ "topics": topics }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_topic_resolves_and_unknown_errors() {
        for t in &TOPICS {
            assert!(!t.body.is_empty() && !t.summary.is_empty());
        }
        let names: Vec<&str> = TOPICS.iter().map(|t| t.name).collect();
        assert_eq!(
            names,
            vec![
                "config",
                "environment",
                "exit-codes",
                "destinations",
                "hot-reload",
                "feedback"
            ]
        );
    }

    /// The text an agent reads once reports are off says only that, and
    /// how the user turns them back on: none of the guidance for sending one.
    #[test]
    fn the_feedback_topic_turned_off_carries_no_guidance() {
        assert!(FEEDBACK_OFF.contains("turned off feedback reports"));
        assert!(FEEDBACK_OFF.contains("Don't offer"));
        assert!(FEEDBACK_OFF.contains("'sweetpad feedback on'"));
        assert!(!FEEDBACK_OFF.contains("submit"));
    }

    /// A command a topic points at is one `--help` lists: a hidden alias
    /// works, but a reader who looks for it in the help finds nothing.
    #[test]
    fn the_topics_point_at_visible_commands() {
        use clap::CommandFactory;
        let root = crate::cli::Cli::command();
        for topic in &TOPICS {
            // Apostrophes rule out pairing every quote, so each quoted
            // command runs from its "'sweetpad " to the next quote.
            for (at, _) in topic.body.match_indices("'sweetpad ") {
                let rest = &topic.body[at + 1..];
                let quoted = &rest[..rest.find('\'').unwrap_or(rest.len())];
                let words = &quoted["sweetpad ".len()..];
                let mut cmd = &root;
                for word in words.split_whitespace() {
                    if word.starts_with(['-', '<']) {
                        break;
                    }
                    let Some(sub) = cmd.find_subcommand(word) else {
                        panic!(
                            "topic {} names '{quoted}', and '{word}' is no command",
                            topic.name
                        );
                    };
                    assert!(
                        !sub.is_hide_set() && sub.get_name() == word,
                        "topic {} names '{quoted}', and '{word}' is hidden or an alias",
                        topic.name
                    );
                    cmd = sub;
                }
            }
        }
    }
}
