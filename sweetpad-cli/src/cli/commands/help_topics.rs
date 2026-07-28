//! `sweetpad help <topic>` — the design doc's best sections, shipped into the
//! binary: configuration, environment variables, exit codes, destinations, and
//! hot reload. `sweetpad help` lists the topics; an unknown topic points at
//! `sweetpad <command> --help` for command help.

use crate::cli::output::Output;
use crate::cli::{CliError, CommandResult, Context, ErrorKind, Render, Rendered};

/// One help topic: its name, a one-line summary for the listing, and the body.
struct Topic {
    name: &'static str,
    summary: &'static str,
    body: &'static str,
}

const TOPICS: [Topic; 5] = [
    Topic {
        name: "config",
        summary: "the config file: location, keys, per-project overrides",
        body: "\
CONFIGURATION (hand-authored; sweetpad never writes it)

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

Resolution precedence, highest first:

  explicit flag > env var > config file > sweetpad.toml > remembered state
  > auto-discovery

A committed 'sweetpad.toml' is the team-shared defaults layer
(scheme/configuration/destination/sdk, 'developer_dir', '[run]', '[format]',
'[testing]') — personal config beats it, remembered picks yield to it. It is
found by walking up from the working directory to the git root, so one file
serves the whole checkout.

When the project is not a sibling of that file, name it with 'workspace' or
'project', relative to the file itself:

  # App/sweetpad.toml
  project = \"Sources/App.xcodeproj\"

Every command then works from anywhere in the checkout, with no '-C'. The
container resolves as: --workspace/--project > this key > auto-discovery.

Auto-discovery searches upward to the git root, then up to two levels down
(skipping Pods, node_modules, Carthage, vendor, DerivedData, build and
dotfiles), so nested layouts like 'ios/App.xcodeproj' need no setup at all.
Projects tied at the same depth — say 'ios/' and 'macos/' — are reported as an
error listing each one, rather than picked for you; name the one you want with
--project or the 'project' key above.

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

  SWEETPAD_HOTRELOAD_DYLIB  override the injection client dylib path (CI)",
    },
    Topic {
        name: "exit-codes",
        summary: "what each process exit code means",
        body: "\
EXIT CODES

  0   success
  1   generic failure
  2   usage error (bad flags/arguments; rendered by clap)
  3   build or test failure
  4   target resolution failed (unknown/missing scheme, destination,
      simulator, device, …)
  5   a required tool is missing (xcodebuild, simctl, …)
  6   cancelled by the user (a declined prompt, Ctrl-C in a session)

A SIGINT/SIGTERM that kills the process exits 128+signo (130/143) after the
handler restores the terminal and reaps children. Under --json, errors are
'{\"schema\":1,\"ok\":false,\"error\":{code,message}}' on stderr, where 'code'
is the same taxonomy: generic, build_failure, target_resolution, tool_missing,
user_cancel.

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

'sweetpad destination list' prints every runnable target with a ready
specifier; 'simulator list' and 'device list' show each pool.

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
                .kind(ErrorKind::Generic));
            };
            Ok(Rendered::data(HelpText {
                body: topic.body.to_string(),
                json: serde_json::json!({ "topic": topic.name, "text": topic.body }),
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
                "hot-reload"
            ]
        );
    }
}
