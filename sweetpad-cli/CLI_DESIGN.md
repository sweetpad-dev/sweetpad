# SweetPad CLI — design

The `sweetpad` binary's standalone, headless command set: **"xcodebuild for
humans."** A pure native front-end to the `sweetpad-lib` Rust engine for
running, building, and exploring Xcode projects without an editor.

It lives in the **same `sweetpad` binary** as the existing `vscode` namespace
(a generic JSON-RPC client that controls the VS Code extension — see
`src/vscode_cli.rs`). `vscode` stands on its own; the new resources sit beside it.

> Status: committed design goals. Implementation tracked in §8.

---

## 1. Positioning

- **Standalone & headless.** Drive Xcode projects from a terminal or CI with no
  editor and no Node runtime. Contrast with the previous CLI iteration, which
  only *controlled* the VS Code extension (now the `vscode` namespace).
- **For humans, not just scripts.** Friendlier than raw `xcodebuild`/`xcrun`:
  sane discovery, readable output, interactive pickers — while staying fully
  scriptable.
- **Backed by `sweetpad-lib`.** Scheme/destination/build-setting resolution
  comes from the existing Rust engine; the CLI is a thin, well-factored layer
  on top.

## 2. Command grammar

**Noun-verb, resource-first:** `sweetpad <resource> <action> [flags]`.
Consistent and discoverable (like `kubectl`/`docker`/`gh`). Resources live at
the **top level**; `vscode` is just one more top-level entry.

**Verb shortcuts for the daily loop.** The hot-path commands also work without
their action token — `sweetpad build` (= `build start`), `sweetpad test`
(= `test run`), `sweetpad app` / the flagship `sweetpad run` (= `app run`),
`sweetpad format` (= `format run`) — plus `sweetpad clean` and the aggregated
`sweetpad devices` view. Bare `sweetpad` prints the status view inside a
project. Aliases: `dep`, `sim`, `dd`, `fmt`.

**Flag policy.** `--yes` skips a confirmation prompt (the action is unchanged);
`--force` overrides a safety check (e.g. `project new --force` scaffolds into a
non-empty directory; `merge`-family `--force` redoes work git considers done).
`--all` widens scope to "everything in this store" (`derived-data --all`,
`context remove --all`); commands that act on "every item" by default express
it by *omitting* a positional (`dependency update`), not with `--all`.

**Deletion verbs are domain-faithful, not uniform:** `simulator erase` (Apple's
term for factory-reset), `context remove`/`dependency remove` (take something
out of a collection), `derived-data purge` (delete stored data wholesale).

**`--target` glossary.** The word is overloaded by the domain itself: a build
target (`settings show --target`), a link target (`dependency add --target`),
the default *test* target (`context select target --testing`), and the
positional simulator argument named TARGET on `simulator` verbs. Each command's
help says which one it means.

## 3. Command surface (v1)

v1 scope is **explore + build/run** — the minimum to actually develop headless.

```
sweetpad scheme list                 list schemes
sweetpad destination list            list build destinations
sweetpad project info                targets, configurations, schemes
sweetpad project new <Name>          scaffold a new minimal SwiftUI iOS app
sweetpad settings show               resolved build settings (lib's specialty)
sweetpad simulator list              list simulators
sweetpad simulator boot              boot a simulator
sweetpad build start                 compile only

sweetpad app run                     build + install + launch
sweetpad app install                 build + install, no launch
sweetpad app launch                  launch an already-installed app (--mac: detached)
sweetpad app logs                    stream app logs (macOS: os_log + captured stdout; §9h)
sweetpad app stop                    kill the running app

sweetpad vscode <method> [--flag …]  control the VS Code extension (JSON-RPC)
```

`build` stays purely "compile"; the full run/install/launch/logs/stop lifecycle
groups under `app`, the noun it acts on.

Out of scope for v1 (later iterations): `test`, `format`, `device` (physical)
management, `bsp` (autocomplete config), `tools` (Homebrew).

### The surface as shipped (post-audit, July 2026)

The audit pass grew the tree beyond the v1 sketch:

```
sweetpad                          status view in a project; help outside one
sweetpad run [--on X] [--hot]     the flagship loop (= app run)
sweetpad build [--clean|--watch|--show-command] [-- XCODEBUILD_ARGS]
sweetpad build diagnostics        last build's errors/warnings, no rebuild
sweetpad test [--failed|--retry-flaky N|--coverage|--junit P|--watch]
sweetpad test build               compile the test targets, run none, §9p
sweetpad test attachments         export the last run's screenshots/dumps, §9l
sweetpad test output              what the last run's tests printed, §9l
sweetpad clean [--purge]          xcodebuild clean; --purge adds DerivedData
sweetpad archive [--export-method M] [--no-export]
sweetpad devices                  everything runnable, specifier-ready
sweetpad device <list|info>       paired physical devices; info checks one is ready (§9q)
sweetpad status / open / doctor / self-update / help <topic>
sweetpad simulator <boot|create|delete|clone|push|privacy|status-bar|
                    location|media-add|record|screenshot|…>   (alias: sim)
sweetpad app <run|install|launch|debug|diagnose|uninstall|logs|stop|open-url|
              container|screenshot|sample|ui> (screenshot: simulator or macOS window, §9h;
                                        sample: main-thread verdict, §9r;
                                        ui: drive a macOS app's UI, §9i;
                                        debug --batch / diagnose: scriptable lldb, §9j;
                                        logs: os_log + captured stdout on macOS, --source/--last, §9h;
                                        container: the app's data/.app/App Group paths, §9o;
                                        logs --exits: why the app's processes ended, §9j)
sweetpad merge <install|run>      semantic conflict resolution (pbxproj/spm
                                  are hidden aliases)
sweetpad context <show|select|set|alias|remove>
sweetpad settings show [-- XCODEBUILD_ARGS]   resolved build settings (porcelain; §9f/§9g)
sweetpad pbxproj <resolve|settings|folder|membership|fileref|group>  plumbing (§9g)
sweetpad feedback <submit|off|on|status>  an agent's problem report to the maintainer (§9t)
```

Destination selection is `--on <ref>` (fuzzy name / `booted` / `mac` /
`device` / platform word / UDID / a `context alias` name), with
`--destination` as the raw escape hatch. `-o json|ndjson` is the machine
surface (§4).

The `-- XCODEBUILD_ARGS` tail follows the build. `build`, `test`, `archive`,
and the `app` verbs that spawn `xcodebuild` — `run`, `install`, `debug`,
`diagnose` — all take it; the verbs that only act on an already-installed app
(`launch`, `uninstall`, `logs`, `stop`) refuse it, because args that reach no
`xcodebuild` would be accepted and silently dropped. A passthrough
`-derivedDataPath` is read back out and handed to the in-process resolver, so
the app the CLI installs is the one the build just wrote, and `app launch`
takes the same location as `--derived-data-path` (§9s). The resolver takes
the tail's `KEY=VALUE` settings and `-xcconfig` too, so a `SYMROOT=`,
`OBJROOT=` or `CONFIGURATION_BUILD_DIR=` that moves the product is followed
like any other setting (§9s). `settings show` takes the tail too, which
reaches no `xcodebuild` but previews what a build given it resolves. A
project that always needs the same argument writes it in `sweetpad.toml`'s
`[xcodebuild] args` instead of typing it each time (§6).

The tail can't name what sweetpad passes `xcodebuild` itself, because
`xcodebuild` fails a second copy ("option '-scheme' may only be provided
once") and the failure would read as a broken build. So `-scheme`,
`-configuration`, `-sdk`, `-workspace` and `-project` in the tail are a
usage error naming the sweetpad flag to use, checked before the command
resolves a scheme or destination. So are `test`'s `-resultBundlePath`
(`--result-bundle`) and `archive`'s `-archivePath`, `-exportPath`
(`--output-file`) and `-exportOptionsPlist` (`--export-options`), and on
`test` a `-enableCodeCoverage` beside `--coverage` or a `-test-iterations` beside
`--retry-flaky`. `-destination` stays: `xcodebuild` takes several and builds
or tests for each. A build is the one place that adopts the typed copy.
Nothing reads the build's own result bundle back (it is passed only so
`xcodebuild` writes the activity log the editor's index reads), so a typed
`-resultBundlePath` replaces it. That bundle is the caller's. It is kept, and
it is cleared only when an earlier build of the same process wrote it (the
next `--watch` round, a run-session rebuild). One that was there first is left
for `xcodebuild` to refuse.

A tail that ends with a flag still waiting for its value (`-- -xcconfig`) is
a usage error too, checked at the same point. `xcodebuild` refuses it
("option '-xcconfig' requires an argument"), but `settings show` and the
locator never spawn it, and would read an earlier `-xcconfig` or none. A
`sweetpad.toml` list that ends that way is refused as well, since the merge
would hand the flag the tail's first argument as its value. A Swift
package's tail skips these checks, `test`'s check for a copy of what
`--coverage` or `--retry-flaky` passes among them. It goes to `swift build`
or `swift test`, which take none of `xcodebuild`'s flags, and a compiler flag
it forwards can be spelled like one of them (`-Xswiftc -sdk -Xswiftc <path>`).
The CLI and the BSP server read these arguments with one walker,
sweetpad-core's `xcodebuild_args::read`: a flag that takes a value takes the
argument after it, dashes and all, as `xcodebuild` reads it. The checks above,
the `sweetpad.toml` merge (§6) and every read of a flag's value go through
it, and the last copy of a flag counts. The server can't refuse the
extension's `bsp.json`, so a `buildArgs` ending that way is logged as a
warning, and the index reads the rest, the copy before it included.

The walker's `VALUE_FLAGS` lists each flag that shapes a build, test or
archive and takes a value in Xcode 27, as probed against it. That includes the
testing flags such as `-enableCodeCoverage`, `-test-iterations` and the
two-word `-only-testing X`. Each fails at the end of a command line and takes
the next argument even when it starts with a dash. A flag that checks its
value (`-enableCodeCoverage YES|NO`) then refuses it. `SWITCHES` beside it
lists the flags that take no value. The flags of `xcodebuild`'s other modes,
such as `-exportLocalizations`' `-exportLanguage`, are in neither.

clap rejects an unknown flag on a verb with a tail, and its stock tip ("to
pass '--bogus' as a value, use '-- --bogus'") would hand the flag to
`xcodebuild`, which spells no flag with two dashes. sweetpad drops that tip
(`hint_tail_flag`). A flag `xcodebuild` takes, typed ahead of `--` with one
dash or two, gets a tip that shows it after `--` with one: `xcodebuild flags
go after '--': 'sweetpad build -- -allowProvisioningUpdates'`. clap splits a
one-dash word into short flags and names a single letter (`-a`), so the error
names the whole word instead. clap can also take the rest of the word as a
value: `-only-testing:App/Tests` reads as `-o nly-testing:App/Tests`, an
invalid output format. `-hideShellScriptEnvironment` reads as `-h` and prints
the verb's help. When the whole word is an `xcodebuild` flag, that error or
help gives way to the same unknown-flag error and tip. The help stays for
`-h`, `--help` and `-help`, and for any command line where another word asks
for it too. For a flag sweetpad passes itself, the tip names the sweetpad
flag, as the tail's refusal does. So it does for a flag with a value when the
verb has a flag of the same name (`pass '--destination' instead of
'-destination'`, and `test`'s `-skip-testing`). A flag nothing takes gets no
tip. The flags it knows are `VALUE_FLAGS` and `SWITCHES`:
what `xcodebuild -help` shows for Xcode 27, less the flags that make it do
something other than build, test or archive (`-showBuildSettings`, `-list`).
The verbs with a hidden tail (`build diagnostics`, `test output`) refuse one,
so they get none of these tips.

## 3a. `project new` — scaffolding

`project new` creates a fresh, buildable **minimal SwiftUI iOS app** with no
external tools. The `.xcodeproj` is generated **natively**: a
[`crate::pbxproj`] object graph assembled in [`cli::scaffold`] and serialized by
the crate's own [`crate::pbxproj_writer`], with the shared `.xcscheme` built as
a [`crate::xcscheme::Element`]. This keeps the CLI standalone — no XcodeGen
dependency — and on-policy with DOCS §3 (hand-roll Apple's project formats).

```
sweetpad project new <Name> [flags]
```

- **One command, new directory by default.** Creates `./<Name>/`; `--current-dir`
  scaffolds into the working directory instead (name then defaults to its
  basename).
- **Interactive wizard.** On a TTY, any value not supplied as a flag is prompted
  for — location (current dir?), name, platform, bundle id, deployment target,
  and git init — each with a default that **Enter accepts** (the name has no
  universal default, so new-directory mode requires typing it). A non-empty
  target additionally prompts to continue (the `--force` question). Non-TTY /
  `--json` runs stay strict: flags and defaults only, and a missing name or a
  non-empty target without `--force` is a usage error (§4). Declining the
  `--force` question is a cancel (exit 6).
- **Inline "use defaults" escape.** Every step after the name carries its own
  way to accept the remaining defaults without more questions: the platform
  picker has a trailing *"Use defaults for everything else"* entry, and the text
  steps accept a lone `*`. Choosing it fills that field and all later ones from
  defaults and finishes — no separate "proceed?" question.
- **Back-navigation.** The wizard is a step machine over the un-flagged fields,
  so any step past the first can go back to change an earlier answer — a `← Back`
  entry on the `Select` steps and a lone `<` on the text steps. A revisited step
  is pre-filled with the prior answer, and dependent defaults (bundle id from the
  name, deployment target from the platform) recompute when their input changes.
- **Platform.** `--platform ios|macos` (default `ios`); the wizard offers a
  picker. Switching platform swaps `SDKROOT`, the deployment-target key and its
  default (`17.0` iOS / `14.0` macOS), the framework runpath, and the iOS-only
  Info.plist keys (launch screen, orientations, device family).
- **Flags:** `--bundle-id` (default `com.example.<Name>`), `--deployment-target`
  (platform default), `--platform`, `--no-git` (git init runs by default),
  `--force` (allow a non-empty target), `--json` (emits the created paths).
- **Generated tree:** `<Name>.xcodeproj` (pbxproj + inner `.xcworkspace` + shared
  scheme), `<Name>/<Name>App.swift`, `<Name>/ContentView.swift`, `.gitignore`.
- **Sources are a synchronized root group** (`objectVersion = 77`, the fresh
  Xcode 16 template shape, §9f): the pbxproj carries no per-file objects, so
  adding a source file is creating it on disk — the project file never changes
  as the app grows. `sweetpad pbxproj folder`/`membership` (§9g) manage
  folders and exceptions.
- **Names** must be plain identifiers (letters/digits/underscore) so they're safe
  as a Swift type, target, and product name in one.

Generation is a **pure** function (spec → list of files), unit-tested by
round-tripping the pbxproj through the parser and resolving it with
`project::open_from_value`; the `cli-smoke` job then scaffolds a project and
builds it with real `xcodebuild`.

## 4. Output model

- **Human/colored by default** — tables, spinners, formatted build logs.
- **`--json`** on any command emits stable, machine-readable JSON for
  scripting/CI.
- Color **auto-disables** when stdout is not a TTY or when `NO_COLOR` is set
  (non-empty, per the no-color.org spec); `--no-color` forces it off;
  `CLICOLOR_FORCE`/`FORCE_COLOR` force it back on when piped (an explicit
  `--no-color`/`NO_COLOR` still wins).
- Errors: human messages on **stderr** by default; **structured error objects**
  under `--json`. Meaningful exit codes. A failed build or test run's
  streamed log closes on its `✗` line. When xcodebuild printed no
  `** BUILD FAILED **` (a destination error stops it before any build
  starts), sweetpad prints the line itself. When the log already showed the
  error, output ends there and no trailing error repeats it. The exit code
  still reports the failure, and a failure with no parsed error still prints
  its message. That holds while stdout and stderr are one file (a terminal,
  or `2>&1`), because the streamed log goes to stdout. When they are
  different files (`2>err.log`, or stdout piped away), the error on stderr
  repeats the first three distinct errors, one per line, and counts the rest.
  `build`, `test`, `test build` and the run session's build then close on
  `tip: run 'sweetpad build diagnostics' to see every error and warning` (a
  device's own tip wins). A `test` records its build step the way `build`
  does, so the command reads back the latest build: a failed one's errors,
  or, once the tests ran, a clean one's warnings. Only the lines before the
  run's first test line count, because after it XCTest writes each failed
  assertion as `<file>:<line>: error: …`.
- Help, errors, warnings and notes quote a command or value with 'single
  quotes', as clap does; a terminal prints backticks literally. A unit test
  walks the clap tree and fails on a backtick in any help text. Messages are
  built in code, so a second test reads the string literals out of the
  sources of `sweetpad-cli`, `sweetpad-core` and `sweetpad-lib` (the
  libraries' `String` errors reach the terminal verbatim), skipping comments
  and `#[cfg(test)]` code, and fails on a backtick in any of them.
- Help, errors, warnings and notes never cite this document: someone reading
  `--help` has no `§9g` to look up. A reference that helps the next
  maintainer goes in a plain `//` comment, which clap doesn't show. The same
  tree walk fails on `§` or `CLI_DESIGN` in any help text, and the source
  scan in any message.

### The JSON envelope

- Success: `{"schema": 1, "ok": true, "data": …}` on **stdout**,
  pretty-printed. Errors: `{"schema": 1, "ok": false, "error": {code,
  message}}` on **stderr**, compact single-line by design (robust to scrape
  even when child-process stderr interleaves). A build failure adds the
  parsed `diagnostics`; a failure that points at a next command adds it as
  `tip`, which human output prints as its closing `tip:` line even when the
  streamed log already showed the error.
- **`ok` means "the command executed"**, not "the outcome was good": a red
  test suite is `ok: true` with `data.passed: false` (exit 3), `doctor` with
  problems is `ok: true` with per-check statuses (exit 1), `device info` on a
  device that is not ready is `ok: true` with `data.ready: false` (exit 1),
  `format --check` reports findings in `data` (exit 3). Every such payload
  carries its own status field — read that, not `ok`.
- **`schema` bump policy:** additive fields never bump it; a removed or
  re-typed field bumps it. Consumers should tolerate unknown fields.
- **Exceptions:** `app run` rejects `--json` only in the forms that stream —
  the log-following session, `--hot`, and an SPM executable (whose output is
  the program's own). `--no-logs` and `--detach` build, install, launch and
  exit, so they carry a payload (`{built, bundleId, destination, pid,
  detached}`); the refusal is keyed on what the invocation does, not on the
  verb, because `--no-logs` is exactly the agent-facing form. `app logs --json`
  emits a *stream* of raw
  `log stream` NDJSON events (one JSON object per line, no envelope; on macOS,
  captured stdout/stderr lines are `{"source":"stdout",…}`), except under
  `--exits`, whose finite report takes the envelope like any other;
  `completions` ignores it. Clap usage errors (exit 2) print clap's human
  text on stderr regardless of `--json`.

### The NDJSON stream (`-o ndjson`)

- **stdout carries only events**: one compact JSON object per line, each with
  an `event` discriminator (`task`, `diagnostic`, `test`, `suite`), and the
  stream ends with exactly **one** terminal line —
  `{"event":"result","ok":true,"data":…}` on success, or
  `{"event":"result","ok":false,"error":{code,message}}` on failure (the
  compact stderr envelope is also emitted, as the machine-parsed error
  surface). Non-streaming commands degenerate to just the result line.
- A `test` event says `passed`, `failed` or `skipped` for one test, and a
  `suite` event names a suite as it starts. They come from the lines the run
  prints, read by `sweetpad_core::test_markers`: XCTest's, Swift Testing's,
  and the `Test case '…' passed on '<runner>'` form `xcodebuild` uses for a
  parallel run. A serial Swift Testing line names the test without its suite.
  The counts in the result come from the result bundle.
- Human chatter stays on stderr; child tools are run captured/quiet so their
  raw stdout never interleaves with the events.
- **Exceptions:** `app run` rejects ndjson exactly where it rejects `--json`
  (the streaming forms), and emits the same terminal result line for
  `--no-logs`/`--detach`;
  `--watch` is refused under both machine modes (a rerun-forever loop has no
  terminal result); `app logs` passes through the raw `log stream` events
  (plus `{"source":"stdout",…}` lines for a macOS app's captured console).
- `--gh-annotations` conflicts with both machine modes (workflow commands and
  the envelope/event stream both claim stdout) and is rejected up front.

### Exit codes

```
0  success                      4  target resolution failed (no/unknown
1  generic failure                 scheme, destination, simulator, …)
2  usage error                  5  required tool missing (xcodebuild, …)
3  build or test failure        6  cancelled by the user (a declined or
                                   Esc'd prompt, Ctrl-C while a run
                                   session builds)
```

A SIGINT/SIGTERM that kills the process exits `128 + signo` (130/143), after
the handler restores the terminal and reaps children.

A run session (`app run` at a terminal, plain or `--hot`) exits by how it
ended, not by the last thing that happened in it. Ctrl-C while one of its
builds runs cancels the session: 6, whether or not the app ran earlier, since
it interrupted work in hand. Every other ending is a quit: `q`, Ctrl-C or
Ctrl-D at the prompt (raw mode turns Ctrl-C into a key, and at the prompt it
means what `q` means), `d`, or stdin closing. A quit exits 0 once the app has
run in the session, however the last rebuild went. A session that never got
the app running exits with its last build's code instead, 3 if it failed or 1
if it built but didn't launch, so a wrapper still sees that nothing ran. A
`--hot` session whose first build or launch fails ends there with the same
codes. `session_result` in `commands/app.rs` holds the rule for both. A quit
that can't stop the app (a terminate that fails, or times out on a wedged
simulator) exits 1 instead of 0, since the app may still be running. A
session already exiting non-zero keeps its code and prints the stop's error
too (`quit_result`).

Exit 2 has two sources. clap reports what it can't parse, in its own text
even under `--json`. The command reports what it parses but refuses: a flag
on a verb it means nothing to (`test build --failed`, `build diagnostics
--clean`), two flags that can't go together (`--on` with `--destination`,
`--gh-annotations` with `-o json`, `app debug --batch` with `-o json`,
`test --coverage -- -enableCodeCoverage NO`), a `--` argument naming what
sweetpad passes `xcodebuild` itself (`build -- -scheme App`; §3), a
flag value out of range (`--pid 0`, `--nth 0`, `archive --on toaster`), an
argument no project could make valid (`pbxproj membership add` naming no
file, a `pbxproj settings set` argument with no `=`, a `project new` name
with a space, `context alias mac`, an unknown `help` topic, `feedback submit`
with neither `--dry-run` nor `--approve`, a report file missing a field).
Those take the envelope with `code: "usage_error"`. Where the command line alone settles
one, the command checks it before looking for a project, so the refusal is
the same from any directory. The typed flags alone decide it, and a
committed default never causes one: `[run] hot = true` yields to the flags
a hot session would refuse (`--no-logs`, `--detach`, `--wait-for-debugger`)
with a one-line note, since the flags were typed for this run and the
default was not, and only a typed `--hot` refuses them. So the agent-facing
`run --no-logs` works in every project. A flag refused because
of what the project or destination turns out to be (`--scheme` on a Swift
package, `--keep-sandbox` off macOS) keeps its own code, since the same flags
work in another project.

A refusal that stands in for a prompt is a usage error too. Off a terminal
(piped, `--json`, `--non-interactive`, `CI`), a command that would have asked
refuses and names the flag that gives the answer: `derived-data purge` and
`simulator delete` without `--yes`, `dependency add` without `--product` and
`--target`, `project new` with no name or into a non-empty directory without
`--force`, and `context select`, which only prompts and names `context set`.
The command line alone is what falls short, since adding the flag makes the
same run go through, so it exits 2 with `code: "usage_error"`. The check stays
where the prompt was: a purge with nothing to delete asks nothing and needs no
`--yes`. Target resolution is the exception. A missing scheme or destination
off a terminal keeps exit 4 (§5), with a hint naming `--scheme` or
`--destination`, since whether one is needed depends on how many the project
has.

## 5. Target resolution

What the command acts on (workspace/project, scheme, configuration,
destination) resolves by **layered precedence**:

```
explicit flag  >  env var  >  config file  >  remembered state  >  auto-discovery
```

- **Auto-discovery:** find the `.xcworkspace` / `.xcodeproj` / `Package.swift`
  in the working directory (deterministic: same-kind siblings resolve to the
  alphabetically first, with a warning; pass `--workspace`/`--project` to
  disambiguate). The search walks *up* toward the git root, so a command works
  from a nested source directory.
- **Container resolution** runs on its own shorter ladder — `--workspace` /
  `--project` > a `sweetpad.toml` `workspace`/`project` key > auto-discovery —
  because every layer below it (per-project config, remembered state) is
  *keyed by* the container and so can't take part in finding it.
  `xcodebuild` runs in the container's directory, so a relative container
  reaches it by file name: `--project ios/App.xcodeproj` runs `xcodebuild
  -project App.xcodeproj` in `ios`. A relative path in the `--` tail is read
  from there too.
- **Downward scan**, once the upward walk finds nothing: look below the working
  directory, then below the repository root, at most two levels down. This is
  what makes the common nested layouts work with no configuration at all —
  `ios/App.xcodeproj`, `Sources/App.xcodeproj`, `apps/ios/App.xcodeproj` — none
  of which the upward walk can reach from the repository root. Build output and
  vendored trees (`Pods`, `node_modules`, `Carthage`, `vendor`, `DerivedData`,
  `build`, `SourcePackages`, dotfiles) are never entered, so
  `Pods/Pods.xcodeproj` is never a candidate, and symlinks are never followed.
  The walk is `sweetpad_lib::discover`, which the extension's project picker
  and its `workspace.detect` RPC take too, four and three levels down.
  - The **shallowest** level holding anything wins outright, and within a
    single directory the usual workspace > project > package ordering applies
    — so a `ios/` holding both a workspace and a project resolves silently to
    the workspace, as CocoaPods and React Native layouts want.
  - Directories **tied at that depth** (Flutter's `ios/` and `macos/`, a
    monorepo's `apps/*`) are a real choice, not a tiebreak: the command errors
    with every candidate listed and names both ways to settle it. This is a
    deliberate split from same-kind siblings *in* the working directory, which
    warn and take the alphabetically first — standing in a directory is a
    signal of intent, a hit two levels down is not.
  - A scan-resolved container is **announced** (`using Sources/App.xcodeproj
    (found below the current directory)`), since the user never named it.
  - Bare `sweetpad` treats a tie as "no container" and prints the help wall:
    the status-or-help probe has nowhere to put a question, and `sweetpad
    status` reports the ambiguity properly a keystroke later.
- **Env vars:** `SWEETPAD_SCHEME`, `SWEETPAD_DESTINATION`,
  `SWEETPAD_CONFIGURATION`, `SWEETPAD_SDK`, `SWEETPAD_PROJECT` /
  `SWEETPAD_WORKSPACE` (value-carrying, folded into the flag layer — but an
  explicitly *typed* flag still beats an exported env var, so
  `SWEETPAD_WORKSPACE` can't override a typed `--project`), and
  `SWEETPAD_NONINTERACTIVE` (boolean). Boolean `SWEETPAD_*` vars parse
  truthiness: `0`/`false`/`no`/`off`/empty mean **off**.
- **`--on` and `--destination` versus the mode flags:** `--mac`, `--device`
  and `--device-id` name a destination the way `--on` and `--destination` do,
  so every `app` verb that takes them settles them with one check, `run` and
  the lifecycle stages alike, before any project is looked for: a typed mode
  flag beats an exported `SWEETPAD_ON` or `SWEETPAD_DESTINATION`, and a typed
  `--on` or `--destination` beside one is a usage error, whether it names the
  same place (`--on mac --mac`) or another (`--on "iPhone 17" --mac`,
  `--destination "platform=iOS Simulator,name=Nope" --mac`). `build`, `test`
  and `test build` take `--mac` as a spelling of `--on mac` and settle it
  with the same check, so the Mac is named the same way on every command
  that builds. `--device` and `--device-id` stay on the `app` verbs.
- **Reading a raw destination:** every command reads a `-destination`
  specifier through `sweetpad-lib`'s `DestinationSpec`, and so do core's
  app locator and the extension's addon. One platform table maps its label
  to the SDK, to the `SUPPORTED_PLATFORMS` tokens `SupportedPlatforms`
  filters on, and to the OS names simctl and devicectl use (simctl's runtime
  spells visionOS `xrOS`). Platform labels match
  without regard to case, as xcodebuild's do. When an `app` verb installs
  from a `name=` specifier, the simulator is matched the way xcodebuild
  matches it: the exact name, on the specifier's platform, at its exact
  `OS=` (or the newest for `OS=latest`), booted first. Xcode keeps a
  default device set for every runtime, so a name alone can match one
  simulator per installed iOS.
- **Remembered state:** the last interactive picks, saved per project, feed the
  layer just above auto-discovery so the daily loop doesn't re-prompt (§6).
  Only picker-settled values are remembered — a one-off flag/env/config
  override never rewrites the stored context, and `--mac`/`--device`
  destinations are never remembered. The cost shows up as `no scheme
  specified` on the command after a `build --scheme X`, so that error names
  the scheme the project's last recorded build ran when it is still one of
  the project's (the build record carries it), and otherwise lists the
  schemes; both forms name `--scheme` and `context set scheme`. It names
  rather than uses: the record is a fact about a build, not a selection.
- **Interactive fallback:** when something is ambiguous/unset **and the
  terminal is interactive** (stderr is a TTY, no `--json`, no
  `--non-interactive`/`SWEETPAD_NONINTERACTIVE`), drop to a fuzzy picker
  (choose a scheme/destination from a menu). **Non-interactive/CI stays
  strict** and errors instead of prompting.
- **Validation:** an explicitly-requested scheme/configuration is checked
  against the project's candidates up front, with a did-you-mean hint — and
  the `Debug` configuration default applies only when the project actually has
  a `Debug`; otherwise the configuration picker runs.
- **Testing:** the `test` action resolves a *separate* testing context — testing
  config/state layered over the build context — so tests can pin their own
  scheme/configuration/destination (§6, "Context").

## 6. Configuration & state

**No files are written to the project root.** Two distinct stores, kept apart so
the tool never clobbers hand-authored config:

### Config — hand-authored only
- `~/.config/sweetpad/config.toml` (honoring `XDG_CONFIG_HOME`).
- **Global settings** plus optional **per-project overrides** keyed by the
  canonicalized **container** path — the `.xcworkspace`/`.xcodeproj`/
  `Package.swift` itself, *not* the directory holding it:
  `[projects."/abs/path/to/Proj.xcodeproj"]`.
- The tool **reads** this and **never rewrites** it, with one exception:
  `feedback off`/`on` set `[feedback] enabled` (§9t) through `toml_edit`, which
  changes that key and keeps every other line and comment as written.
  Unknown keys and `[projects."…"]` keys that can't match a real container are
  **warned about** on load (with a did-you-mean where possible), never
  silently ignored.

```toml
# ~/.config/sweetpad/config.toml
[defaults]
configuration = "Debug"

[projects."/Users/me/code/MyApp/MyApp.xcodeproj"]
scheme = "MyApp"
destination = "platform=iOS Simulator,name=iPhone 15"

# Test-action overrides, layered over the build values for `test` only.
[projects."/Users/me/code/MyApp/MyApp.xcodeproj".testing]
configuration = "Test"
```

- Keys per table: `scheme`, `configuration`, `destination`, `sdk`. Each table
  also accepts a `[….testing]` sub-table — `scheme`/`configuration`/
  `destination`/`target` overrides used by `test`, falling back to the build
  values when unset (mirrors the extension's `sweetpad.testing.*` settings);
  `target` narrows the run to `-only-testing:<target>` when no explicit
  selector is given.
- A top-level `[feedback]` table holds `enabled` (default true), the switch
  for `feedback submit` (§9t).

### Project file — committed, team-shared
- An *optional, hand-authored* `sweetpad.toml` at the project root. This is how
  a team standardizes: it travels with the clone, needs no absolute paths, and
  sweetpad still **never writes** to the project root — the file is read-only
  to the tool, like the user config.
- Precedence slot: **user config > sweetpad.toml > remembered state**.
- **Located by walking up** from the working directory to the git root, so one
  file serves the whole checkout. It may sit beside the container (the common
  case) or above it, where `workspace`/`project` names a container nested
  below — the layout `xcodebuild` can't be pointed at without `-C`:

```
App/
  sweetpad.toml          project = "Sources/App.xcodeproj"
  Scripts/               `sweetpad build` works from here too
  Sources/App.xcodeproj
```

- Keys: `workspace`/`project` (which container this file belongs to, relative
  to the file — `workspace` wins if both are set), `scheme`, `configuration`,
  `destination`, `sdk`, `developer_dir`, a `[testing]` sub-table
  (`scheme`/`configuration`/`destination`/`target`), plus the tool defaults
  that previously had flags but no home:

```toml
# sweetpad.toml (committed)
project = "Sources/MyApp.xcodeproj"   # only when it isn't a sibling
scheme = "MyApp"
configuration = "Debug"

[testing]
configuration = "Test"

[run]
hot = true                 # `app run` defaults to hot reload (--no-hot opts out)
hot_recompiler = "resolver"

[format]
tool = "swiftlint"

[xcodebuild]
args = ["-skipMacroValidation"]   # added to every command that builds
```

- **`developer_dir`** pins the project's Xcode: sweetpad sets
  `DEVELOPER_DIR` to it for every tool it spawns, unless `--developer-dir` or
  the environment already chose one. sweetpad loads the file as soon as it
  resolves the container, so a manifest read or package resolve that runs
  before anything asks the file for a default uses the pinned Xcode too.
- **`[xcodebuild] args`** is the committed form of the `-- XCODEBUILD_ARGS`
  tail (§3): a list joined onto every `xcodebuild` this project spawns —
  `build`, `test`, `archive`, and the builds inside `app
  run`/`install`/`debug`/`diagnose`. `clean` takes it too, so a `SYMROOT=`
  or an `-xcconfig` that moves the products sends `xcodebuild clean` where
  the build wrote. The list has no per-action form, so each action leaves out
  the file's flags it fails on, with a `-v` note naming each one. The testing
  flags `-enableCodeCoverage`, `-testPlan`, `-testLanguage`, `-testRegion`
  and `-testProductsPath` fail `build`, `archive` and `clean` ("only
  supported when testing"), so they reach only `test` and `test build`.
  `-resultStreamPath` fails without a `-resultBundlePath`, so it stays out of
  `clean` and of an `archive` that wasn't typed one. Both lists are read the
  way `xcodebuild` reads them: the argument after a flag that takes a value is
  that value, so `-xcconfig -enableCodeCoverage` keeps an xcconfig of that
  name, and a `-resultBundlePath` that is another flag's value names no
  bundle. The rest of the testing
  flags (`-test-iterations`, `-parallel-testing-enabled`, `-only-testing:`, …)
  pass every action as Xcode 27 probes them. A flag typed after `--` is never
  left out: it was typed for this run, and `xcodebuild`'s refusal names it. A
  repo-wide `-skipMacroValidation` is a
  property of the project, not a decision to re-make per command; this is where
  it lives. The typed tail is appended *after* the file's arguments, so typing
  one wins under xcodebuild's last-one-wins, and `status` prints the effective
  list — a build shaped by a file the caller never opened must still say where
  that came from. Some flags have no last one to win: `xcodebuild` fails a
  second `-xcconfig` ("option '-xcconfig' may only be provided once"), and
  Xcode 27 refuses a repeat of nearly every flag that takes a value
  (`-jobs`, `-enableCodeCoverage`, `-test-iterations`, …; `-destination`,
  `-arch`, `-toolchain` and `-packageCachePath` repeat). When the tail gives
  one of those, the merge leaves out the file's copy and its value, and `-v`
  names what it left out, so the typed tail still wins. A value spelled like
  one of them is no copy of it: a file's `-xcconfig -jobs` names an xcconfig
  and stays beside a typed `-jobs 4`. The `app` verbs that find an already-built product instead
  of building one (`launch`, `stop`, `uninstall`, `logs`, `container`,
  `screenshot`, `sample`, `ui`) plan with the same list, so a file `build`
  refuses stops them too, and a setting that moves the product, such as
  `SYMROOT=build`, sends them where the build put it instead of to a stale
  `.app` in the default DerivedData.
- The arguments the CLI settles itself are **refused** in the file, naming the
  key to use instead: `-scheme`, `-configuration`, `-destination`, `-sdk`,
  `-workspace`, `-project` (a second copy makes the build depend on which one
  xcodebuild honors), `-resultBundlePath` (`test` writes and reads back its
  own; the refusal names `test --result-bundle`, and a build's `--` tail,
  which takes one per run), `-archivePath`, `-exportPath` and
  `-exportOptionsPlist` (`archive` names its own; `--output-file` and
  `--export-options` set them), and `-derivedDataPath`. `--coverage` and
  `--retry-flaky` pass `-enableCodeCoverage` and `-test-iterations`, which the
  file may carry, so under either flag `test` leaves the file's copy out and
  `-v` says so. That check reads flag values the same way, so a file's or a
  typed `-xcconfig -enableCodeCoverage` keeps its xcconfig beside
  `--coverage`. A refusal is an error rather than a warning:
  the alternative is handing xcodebuild two answers to one question. Swift
  packages ignore the table entirely, since `swift build` knows none of these
  flags.
- **`-derivedDataPath` stays refused (decided).** The file could carry it
  only if every consumer of DerivedData resolved a committed value to the
  same place, and they don't:
  - The two that read the file agree with each other. sweetpad runs
    xcodebuild from the directory holding the container, and the locator
    joins a relative value onto that same directory (§9s). But that is not
    the file's directory when `project` names a container below it, and
    every other path in the file resolves against the file, so `dd` would
    read as `App/dd` and land in `App/Sources/dd`.
  - `clean` passes `[xcodebuild] args` to its `xcodebuild clean`, but the
    rest never see the file: `derived-data` and `clean --purge` look where
    Xcode's own settings put DerivedData, and the BSP index takes its location
    from the `buildServer.json` the extension writes. A committed location
    would move every teammate's build away from all of them.

  It is passed per command instead: a build's `--` tail, and
  `--derived-data-path` for `app launch` (§9s). Lifting the refusal would
  take a value resolved against the file, and `clean --purge`,
  `derived-data` and `bsp` reading it too.

- Unknown keys are warned about; a malformed file is warned about and ignored
  (a broken committed file must not brick every teammate's CLI). An absolute
  `workspace`/`project` warns too — it resolves to nothing on every other
  machine, the one mistake that makes a committed file worse than none.
- A declared container that doesn't exist is an **error**, never a fallback to
  discovery: the file said which project this is, and quietly building a
  different one is worse than stopping. When `generator` is set, the error
  names the tool to run — the usual cause is a project that hasn't been
  generated yet.

### State — machine-managed
- `~/.local/state/sweetpad/state.toml` (honoring `XDG_STATE_HOME`).
- Holds the **remembered build context** (scheme, configuration, sdk,
  destination), a separate **testing context**, the **recent/most-used
  destinations**, and the **last launched app** — keyed by **project identity =
  canonicalized workspace/project path**.
- Churns freely; safe for the tool to rewrite. Manage it with `context` (below),
  not by hand.

Precedence note: an authored per-project override in `config.toml` outranks
remembered `state.toml` selections (config > auto, and remembered state feeds
the auto/last-used layer).

### Context — inspect & manage remembered state

`sweetpad context` is the first-class way to view and edit the remembered
selections, instead of hand-editing `state.toml` or relying on a build command's
prompt-and-remember side effect. It mirrors the richer context the VS Code
extension keeps in its workspace state (§7).

```
sweetpad context show                  print the build + testing context, recent
                                       destinations, and last launched app (--json)
sweetpad context select [VAR]          set a variable interactively (no VAR → the
                                       core: scheme, configuration, destination)
sweetpad context remove [VAR] [--all]  clear a variable, or the whole context
```

- **Variables:** `scheme`, `configuration`, `destination` (both contexts),
  `sdk` (build-only), `target` (testing-only). `scheme`/`configuration` reuse the
  project's pickers; `destination` uses the simulator picker below.
- **`--testing`** on `select`/`remove` acts on the testing context; otherwise the
  build context. `--all` clears the whole project entry (or just the testing
  sub-context with `--testing`); emptied entries are pruned.
- **Adaptive destination picker.** Picking a destination records it into the
  project's recents and usage counts; the picker then orders **most-used first,
  then booted (marked `●`), then a deterministic platform → newest-OS → family →
  natural-name sort** — so your habitual target sits on top.
- **Testing precedence.** `test` resolves each field as `flag > testing config >
  testing state > build config > build state`, so a pinned testing override wins
  and `test` otherwise follows the build selection.

## 7. Relationship to the VS Code extension

**Standalone now, adoptable later.** Build the CLI cleanly factored, with a
clear internal API, so the extension *could* later shell out to / drive the
`sweetpad` binary instead of its own TS build logic — but **no migration is
committed**. For now the CLI and extension share only `sweetpad-lib` (the
resolver). Build/simulator orchestration is implemented fresh in Rust for the
CLI.

## 8. Implementation notes

- **Crate layout:** command logic lives in **testable library modules** (a new
  `cli` module tree in `sweetpad-lib`); `src/bin/sweetpad.rs` stays a thin
  entry point dispatching to it. Gated behind the existing default `cli`
  feature.
- **Arg parsing:** `clap` (derive) — auto `--help`/usage, nested subcommands,
  "did you mean", shell completions, env-var binding. This is the first
  substantial dependency beyond `serde_json`; justified under the DOCS §3.1
  policy (don't reinvent *standard* things — a CLI parser is standard).
- **TOML:** a `toml` crate (read-only for config) plus `serde` for
  config/state (de)serialization.
- **Universal flags:** `--json`, `--no-color`, `-v/--verbose`, `-q/--quiet`,
  and `--non-interactive` are global — accepted on every command and its
  actions. `--quiet` mutes progress chatter (notes, spinners, step labels, the
  beautified build stream apart from diagnostics and failure banners) while
  errors and primary data/JSON still emit; it wins over `--verbose`. Both are
  sweetpad's, so their help points at `xcodebuild`'s own `-quiet` and
  `-verbose`, which go after `--`. `--non-interactive` (or `SWEETPAD_NONINTERACTIVE`) forces the strict no-
  prompt behavior at a TTY. The **targeting flags**
  (`--workspace`/`--project`, `--scheme`, `--configuration`, `--destination`,
  `--sdk`) are scoped to the commands that consume them, in three tiers —
  container-only (`project`, `bsp`, `derived-data`), container plus `--scheme`
  (`scheme`), and the full build target (`build`, `test`, `settings`, `app`).
  Within a resource they are global, so they parse on either side of the
  action token: both `sweetpad build --scheme App run` and
  `sweetpad build run --scheme App` work. A resource that doesn't consume a
  tier never advertises its flags (e.g. `project info` rejects
  `--destination`).
- **Help headings:** `--help` lists the targeting flags under "Target
  selection", along with `--mac`/`--device`/`--device-id` (the flag forms of
  `--on mac`/`--on device`). A command's own flags go under "Options" and the
  universal ones under "Global". Each targeting flag sets its heading itself.
  A struct-level `next_help_heading` would not stop at the struct: clap keeps
  it for every arg declared after the flatten, so `build`'s `--clean` and
  `test`'s `--failed` would land under "Target selection" too. A unit test
  walks the clap tree and fails on a flag under a heading its struct doesn't
  own.
- **Process orchestration:** spawn and stream `xcodebuild` / `xcrun simctl`;
  parse output for human and `--json` render paths.

## 9. v2 — completing the headless dev loop

Shipped on top of v1, same grammar and plumbing:

```
sweetpad test run [--only-testing ID]… [--skip-testing ID]…
                                     xcodebuild test; --json emits a pass/fail
                                     summary parsed from the .xcresult bundle
sweetpad format run [paths…] [--tool swift-format|swiftlint] [--check]
                                     formats in place (or lints with --check);
                                     each tool reads its own project config
sweetpad device list                 connected physical devices (xcrun devicectl)
sweetpad bsp init [--output-file PATH]  write buildServer.json for sourcekit-lsp
                                     (reuses the crate's bsp::write_config)
sweetpad completions <shell>          clap_complete-generated scripts
```

`app run` gains the full session experience:

- **`--device` / `--device-id <id>`** — build + install + launch on a physical
  device via `devicectl` (destination becomes `platform=iOS,id=<udid>`).
- **`--mac`** — build and run as a native macOS app: no install step, launch the
  built executable directly (`TARGET_BUILD_DIR/EXECUTABLE_PATH`).
- **inline logs by default** — after launching, follow the app's output:
  `simctl spawn … log stream` on a simulator, `devicectl … launch --console` on
  a device, the executable's own stdout/stderr for macOS. Disable with
  **`--no-logs`**.
- **interactive rebuild session** — at an interactive terminal, `app run` keeps
  the loop under the developer's control instead of auto-watching files: the app's
  output streams from a background child (sim `log stream`, device `--console`, or
  the macOS executable itself) while a single-key reader sits in front. **`r`**
  rebuilds + relaunches on demand; **`q`**, Ctrl-C, or Ctrl-D quit; **`h`** lists
  the other keys for the target. `s` (screenshot) and `o` (bring the app forward:
  the Simulator window, or the running macOS app itself) are offered only where
  there is a window to act on, so a device's list leaves them out and `o` there
  says why. `o` on macOS activates only an app that is still running, in the
  plain and `--hot` sessions alike, since `open` on a stopped bundle would
  launch it outside the session and without its arguments. On each `r`
  the running app is **terminated first** (`simctl`/`devicectl terminate`, or
  killing the macOS process) so the relaunch is always a fresh process picking up
  the new binary — `simctl launch` alone would just foreground the stale one — and
  the app is likewise terminated on quit. A failed rebuild keeps the session alive;
  fix and press `r` again. The os_log stream starts with the first launch, not
  the first build: a session whose build failed has no app to follow, so it
  runs no `log stream` (and `h` offers no level keys) until an `r` gets the app
  up. The `Filtering the log data using …` line `log stream` prints first is
  the tool restating sweetpad's predicate, so `oslog::render_ndjson_line` drops
  it wherever an os_log stream is rendered: the session, and `app logs`, where
  it can't satisfy an `--until` that names the app either. The
  `{"count":N,"finished":1}` object `log show` closes an `app logs --last`
  query with is the tool's too: rendered, it would read as an empty `N [?]`
  entry, so it is dropped there and from the raw `--json` stream. The exit code
  follows how the session ended (§4):
  Ctrl-C during any of its builds is 6, and a quit is 0 once the app has run,
  else the last build's code (3 failed, 1 built but not launched). A quit
  whose stop of the app failed is 1, since the app may still be running.

  The reader uses a hand-rolled raw mode (`libc`, unix-only) that flips only
  stdin's line discipline (`ICANON`/`ECHO`/`ISIG`/`IEXTEN`), leaving the terminal's
  output post-processing on so streamed logs still render cleanly; clearing `ISIG`
  routes Ctrl-C in as a byte we handle, so the RAII guard always restores the
  terminal on exit. Reads are a non-blocking `VTIME` poll, which lets a watcher
  thread keep reading stdin **during** a build: Ctrl-C there is forwarded as
  `SIGINT` to xcodebuild's process group (so a long build stays abortable without
  leaving raw mode), and any other key pressed mid-build is swallowed so it can't
  queue a spurious rebuild. The build runs `xcodebuild` in its own process group
  with piped stdout fed through the [`buildlog`] beautifier. Non-interactive /
  piped runs (and `--no-logs`) fall back to a one-shot launch + inline follow.
  On a simulator that launch attaches the console the session's does
  (`--console-pty`), so the app's `print` output streams next to its os_log.
  The console only watches: the app keeps running once the follow ends.

- **bounded simulator steps** — `simctl install`, `launch` and `terminate`
  each get two minutes (`simctl::STEP_TIMEOUT`). A wedged simulator accepts
  those calls and never answers, and an unbounded call would hold
  `run --no-logs` at "Launching app" for good, and an agent's loop with it.
  A healthy simulator answers in
  under a second (an install too, since APFS clones the bundle), so the
  bound only has to clear a freshly booted simulator's slow first launch.
  A step that runs out is killed and fails with exit 1 (the destination
  resolved; the simulator failed at it), naming the step, with a `tip`
  pointing at `simulator shutdown` and `simulator boot` for that UDID. This
  covers every verb that goes through those three, `app install`/`launch`/
  `stop` included. `simctl boot` stays unbounded, since a first boot after a
  runtime update can legitimately take minutes. The session's console launch
  (`--console-pty`) lives as long as the app does, so only its start is
  bounded: it has started once it prints, once it exits, or once the app's
  process shows up in the host's `ps` (a simulator app is a host process
  under the device's data directory, so this asks nothing of the simulator).
  A process of the app that was already running doesn't count, since a
  terminate that timed out can leave one behind. The session's own
  terminates, on `r` and on quit, go through the same bounded `terminate`
  under a `Terminating app` spinner, so a wedged simulator's two-minute wait
  after `q` shows what it waits on and ends with the same stuck error and
  restart tip. A failed stop on `r` is printed and the rebuild goes on. A
  failed stop on quit is the session's error, headed `couldn't stop <bundle
  id>, so it may still be running`, and a quit that would exit 0 exits 1 with
  it (`quit_result`). A device's `devicectl terminate` failing on quit does
  the same.

`destination list` aggregates **macOS + simulators + connected devices**, each
with a ready `-destination` specifier. SPM containers are supported for
`scheme`/`build`/`test`/`run`: schemes are read straight from the manifest via
`swift package dump-package` (the product names xcodebuild would synthesize —
no xcodebuild spawn, no pbxproj needed), plus the scheme files in the
package's `.swiftpm/xcode`, sorted the way `xcodebuild -list` sorts them. The
extension reads a package through the same code
(`sweetpad_core::package_members::standalone`, via the addon), so neither
writes a `.build/` into the package. A project or workspace container reads
the same manifests for the local packages around it, walking `.package(path:)`
from every package the container references; `sweetpad-lib/DOCS.md` §9 has the
rules and `sweetpad-core/src/package_members.rs` the cache that keeps the
spawns off the common path.

Notes / heuristics:
- A scheme's `.xcscheme` is found where `xcodebuild -list` reads it for the
  container (`sweetpad_lib::scheme::locate`): the container itself, then a
  workspace's member projects, then the `.swiftpm/xcode` of each local package
  it or they declare. Only the current user's `xcuserdata` counts, since Xcode
  never shows one user another's personal schemes. The app locator reads the
  Run action through it (`sweetpad_core::app_locator::find_scheme`), and so do
  the CLI, the build-settings resolver, the supported-platforms filter and the
  extension (launch settings, `scheme.reveal`).
- Autocreated schemes follow Xcode's per-target rule
  (`sweetpad_lib::project::listed_schemes`): each eligible target gets one
  unless a scheme file is named for it, or a scheme file under another name
  covers it (`sweetpad_lib::scheme::SchemeReferences`). A scheme covers an
  app, tool or extension by running it in its Launch or Profile action, and a
  framework or library by building it. A workspace's own scheme files count
  for its members, and a local package's for its products. Scheme files for
  other targets don't stand in the way, so `scheme list`, `settings show` and
  the build's product lookup agree on the set `xcodebuild -list` prints.
- `test run` exits non-zero on failures; the `--json` summary lands on stdout
  and the failure error on stderr, so both are independently consumable.
- simulator inline logs use a best-effort `processImagePath CONTAINS` log
  predicate; may need refinement per app.
- New deps (under the `cli` feature only): `clap_complete`, `dialoguer`, `libc`
  (the last just for the `app run` raw-mode key reader, unix-only).

A **`cli-smoke` GitHub Actions job** (macOS) generates a real iOS app with
XcodeGen (`ci/fixture-app/`) and runs the actual dev loop — `scheme/project/
settings/destination/simulator/bsp/completions`, then `build start`,
`test run`, `app run` — against live `xcodebuild`/`simctl`. This is the runtime
counterpart to the unit tests below.

## 9b. v3 — toolchain & maintenance commands

Quality-of-life commands on the same grammar and plumbing, aimed at the
everyday frictions raw `xcodebuild`/`xcrun` leave to the user:

```
sweetpad doctor                      diagnose the toolchain (flutter-doctor style):
                                     Xcode/xcodebuild/swift, simulator runtimes,
                                     devicectl, swift-format/swiftlint — each ok/
                                     warning/problem with a fix hint. A missing
                                     required tool is a non-zero exit.
sweetpad derived-data path [--all]   this project's DerivedData folder(s), or the
sweetpad derived-data size [--all]   whole store with --all (size is human + bytes)
sweetpad derived-data purge [--all] [--yes]
                                     delete DerivedData — this project by default
                                     (the safe default), or --all; confirms on a
                                     TTY unless --yes
sweetpad simulator shutdown [NAME]   shut down a sim (defaults to the booted one)
sweetpad simulator erase [NAME]      erase contents & settings (must be shut down)
sweetpad simulator open              open the simulator window
sweetpad simulator screenshot [NAME] [--output-file PATH]
                                     PNG of a booted sim (timestamped by default)
sweetpad simulator appearance <light|dark> [NAME]
                                     toggle a booted sim's UI appearance
sweetpad app open-url <URL> [--simulator NAME]
                                     drive deep / universal links in via
                                     `simctl openurl` (boots the sim if needed)
```

Notes / heuristics:
- `doctor` probes each tool with both stdio streams captured (so the report
  stays clean) and reports the first version line; the runtime-count, summary,
  status-glyph, and `first_line` helpers are pure and unit-tested. The Swift
  driver makes a `TemporaryDirectory.*` in `$TMPDIR` for each run. It removes
  the directory only after a run whose jobs it runs itself. When it hands the
  whole run to one `swift-frontend` that replaces it, the directory stays: on
  Swift 6.4 (Xcode 27.0) that happens for `--version`, `-print-target-info`,
  `-typecheck` and a single-file `-c`. A `-###` dry run and a driver that dies
  leave one too. So `swift --version` (here and in the Swift 6 check of
  `dependency add`) runs with a `TMPDIR` that sweetpad removes afterwards.
  Every `dump-package` does the same, since SwiftPM runs
  `swiftc -print-target-info` first: the one that reads a local package's
  schemes, and the one behind `project info` and the `dependency` verbs for a
  package. It also gets a scratch path of its own, so reading a manifest
  leaves no `.build/` in the package. The `swiftc` that a `bsp serve` prepare
  runs to emit a module cleans up after itself unless it dies first: killed,
  or stopped by a warning it writes to a pipe nobody reads once the server is
  gone. So those runs share a `TMPDIR` of the server's own. The server
  removes it when it exits, after it kills a `swiftc` that is still running.
- DerivedData scoping is by container, not by name. Every checkout, worktree
  and copy of a project writes its own `<Name>-<hash>` folder, so the
  container's file-stem (exact name or `<Name>-` prefix, tested against
  prefix collisions: `MyApp` must not match `MyAppHelper-…`) only finds the
  candidates. A candidate is this project's when its hash is the one Xcode
  takes over the container's standardized path (the `.xcodeproj` or
  `.xcworkspace`, or a package's directory), or when the `info.plist` Xcode
  writes into a folder it builds in names the container as its
  `WorkspacePath`. `path`, `size`, `purge`, `clean --purge` and `open dd` act
  on those alone. The rest are reported, never touched: `path` and `purge`
  list them as `others` in JSON, and human output counts them in a note
  (`kept 2 other 'MyApp-*' folder(s) from other checkouts or same-named
  projects`). A project named through its embedded
  `Foo.xcodeproj/project.xcworkspace` is keyed by the `.xcodeproj`, as
  `xcodebuild` keys it (`sweetpad_lib::derived_data::ContainerKey`, shared
  with the build locator and the BSP).
- The verbs find DerivedData the way the build locator does
  (`sweetpad_lib::derived_data`), so they follow Xcode's settings when those
  move it. `--all` is the app-wide store: `IDECustomDerivedDataLocation` in
  the Xcode preferences, else `~/Library/Developer/Xcode/DerivedData`. A
  project's own per-user workspace settings can move its folder out of that
  store (`DerivedDataLocationStyle` with `DerivedDataCustomLocation`): an
  absolute location keeps the `<Name>-<hash>` name, and a location relative
  to the project writes a bare `<Name>` folder, which is the project's own.
  A Swift package keeps these settings in
  `.swiftpm/xcode/package.xcworkspace`, and its relative location hangs off
  the package directory itself. The BSP finds its index store and the `SYMROOT`/`OBJROOT` of a `-target`
  prepare through the same locator.
- the side-effecting `simulator`/`app open-url` actions share one
  simulator picker (`resolve::select_simulator`): explicit name/UDID wins, else
  the lone booted sim, else prompt (booted set, or the full list) / strict
  error off a TTY.

## 9c. v4 — git conflict resolution (.pbxproj + Package.resolved)

`project.pbxproj` is the canonical git merge-conflict nightmare: a flat,
UUID-keyed plist where a line-based merge drops `<<<<<<<` markers in arbitrary
spots and usually yields an unparseable file. This crate already owns both ends
of the fix — a faithful parser ([`pbxproj`]) and a **byte-exact** writer
([`pbxproj_writer`], verified against the whole fixture corpus) — so a *semantic*
three-way merge is a thin layer between them.

Two file kinds are covered: Xcode's `project.pbxproj` (object-graph merge via
[`pbxproj_merge`] + the byte-exact [`pbxproj_writer`]) and SwiftPM's
`Package.resolved` (JSON pin merge via [`spm_resolved`]). Both run on demand
*and* automatically as git merge drivers; the shared plumbing lives in
[`cli::merge`].

```
sweetpad pbxproj resolve [PATHS…] [--force]
sweetpad spm resolve     [PATHS…] [--force]
                                     resolve conflicted .pbxproj / Package.resolved
                                     files mid-conflict. Defaults to every matching
                                     conflicted file in the repo; reads the three
                                     clean inputs from git's index stages (:1: base,
                                     :2: ours, :3: theirs), merges, writes the
                                     result, and `git add`s it. --force recovers the
                                     inputs from HEAD/MERGE_HEAD when git already
                                     auto-merged the file textually. Non-zero exit if
                                     anything is left unresolved.

sweetpad merge install [--global]    register both as git merge drivers
                                     (.gitattributes + `git config`) so plain
                                     `git merge` resolves them automatically.
sweetpad merge driver <KIND> %O %A %B %P
                                     the driver git itself invokes (hidden); reads
                                     git's three temp files and writes the merge over
                                     %A, exiting non-zero on a real conflict so git
                                     leaves the path unmerged (then `<kind> resolve`
                                     shows the structured report).
```

The pbxproj engine ([`pbxproj_merge`]) is pure (no git, no I/O, no Xcode) and
runs the standard three-way rule per UUID-keyed object and per field: identical
edits and one-sided changes resolve silently, disjoint object/array additions
union (reference lists like `children`/`files` are ordered sets, honoring
deletions), and only genuine contradictions — both sides setting the same scalar
differently, or modify-vs-delete — are reported. One side deleting an object
while the other starts naming it counts as modify-vs-delete too: a file one
branch deletes and the other adds to a target's build phase would otherwise
merge cleanly into a build file naming nothing, which builds the file for
neither branch. Two sides moving one node into different groups conflict as
well: the listings merge as sets, and taking both would list the node twice,
which Xcode 27.2 refuses to open. On any conflict the file is left untouched,
with a graph-path report (`objects/<UUID> (<isa>)/<field>`) of what collided. The SPM engine ([`spm_resolved`]) is the same shape over `serde_json`:
the `pins` array merges by `identity` (union disjoint pins, take one-sided version
bumps, conflict only on both-sides-bumped-differently), re-rendered to Xcode's
exact `Package.resolved` style (2-space indent, `" : "`, sorted keys, pins sorted
by identity). `originHash` is a derived digest Xcode regenerates, so it is never
treated as a conflict.

Notes / heuristics:
- Reads pristine blobs from git, never the marker-riddled working copy, so the
  textual conflict's placement is irrelevant. The same engines back both the
  on-demand `resolve` commands (index stages) and the `merge driver` (git's temp
  files), so behavior is identical either way.
- The merged pbxproj dict preserves base key order (then ours-only, then
  theirs-only additions) and the parser's single-line layout hint, keeping output
  Xcode-stable and low-churn.
- `merge install` writes the driver to `git config` (per-clone; collaborators run
  it once) and the attribute lines to the repo `.gitattributes` (commit it) — or,
  with `--global`, to global git config + `core.attributesFile`.
- Engines are unit-tested without a Mac (pbxproj: disjoint adds, one-sided delete,
  modify-delete, same-field conflict, array union+delete, layout-hint; spm:
  byte-exact serialize, pin union+sort, version bump, both-bump conflict, add/remove,
  originHash divergence); the end-to-end git driver path is exercised by real
  synthetic merges.
- Later: a `Package.resolved`-style driver for other regenerated lockfiles is the
  same pattern; a built-in `git merge`-driver self-test could pin the integration.

## 9d. v5 — built-in hot reload (`app run --hot`)

`app run --hot` adds **live code injection** to the interactive session: save a
Swift file and the running app picks up the change in-place, with state
preserved — no relaunch, no `r`. Targets: the **iOS Simulator** and **native
macOS apps** (`--mac --hot`; see the macOS subsection below). Physical devices
are out (codesigning strips `DYLD_INSERT_LIBRARIES`); watchOS ships no
injection dylib. The full-rebuild `r` path (§9c) stays as the always-available
fallback.

> Status: committed design; implementation tracked in the milestones below.

### Architecture — the CLI *is* the injection server

Hot reload (John Holdsworth's InjectionNext/InjectionLite lineage) is always two
halves: a small, stable **client** loaded into the running app, and a **server**
that watches sources, recompiles the changed file to a `.dylib`, and hands it
over. The injected app is the **TCP client** — its `+load` hook
(`ClientBoot.mm`) connects *out* to `127.0.0.1:8887`; whatever is listening
there is the server. `InjectionNext.app` is just one such listener.

**So `sweetpad` becomes the listener.** It binds `:8887` before launch and
serves the same prebuilt client the VS Code extension already injects
(`libiphonesimulatorInjection.dylib` via `DYLD_INSERT_LIBRARIES`) — no new
in-app code, and **`InjectionNext.app` is not required**. This is "Option Y":
the CLI owns the watch + recompile + serve loop itself, rather than delegating
to the menu-bar app or to the in-app standalone watcher.

### Wire protocol (grounded in the upstream `InjectionNextC` source)

- **Transport:** TCP, localhost, port `8887`. Framing is native little-endian:
  `int` = 4-byte `int32`; `string`/`data` = `int32` length then bytes; the EOF
  sentinel is `-1`. A command is an `int32` code then its optional payload
  (`SimpleSocket.mm`).
- **Handshake** — on connect the app pushes, and the server reads in order:
  `int` `INJECTION_VERSION` (4001, validate) · `string` home dir · then an
  `InjectionResponse` stream: `.platform`+string then a bare `string` arch ·
  `.projectRoot`+string (when `INJECTION_PROJECT_ROOT` is set) · `.tmpPath`+string
  · optionally `.executable`+string. These tell the server the platform/arch/sdk
  context to compile for.
- **Server → app** (`InjectionCommand`): the two that matter for v5 are
  `.load`+`string dylibPath` (app `dlopen`s that host path directly — works on
  the simulator, which shares the host filesystem) and `.inject`+`name`+`data`
  (ship the bytes; for devices, out of scope now). Optionally `.xcodePath`+string
  up front so the client's reloader knows the toolchain.
- **App → server** after a load: `.injected` / `.failed` / `.unhide` — surfaced
  as a session status line.

### Build & launch wiring

Two hooks, mirroring the extension's proven `hot-reload.ts` path:

- **Build flags** — `[`crate::cli::xcodebuild::BuildPlan`]` gains, under `--hot`:
  `OTHER_LDFLAGS=$(inherited) -Xlinker -interposable`
  (lets dyld swap symbols at runtime) and `EMIT_FRONTEND_COMMAND_LINES=YES`
  (needed to recover compile commands on Xcode 16.3+; see the recompiler below).
  Both are gated to `--hot` so ordinary `build`/`run` never pay for them. The
  list, with the macOS additions below, is `sweetpad_core::hot::build_settings`,
  which the extension's hot build reads through the addon
  (`hotReloadBuildSettings`), next to the client dylib and platform tables.
- **Launch env** — `[`crate::cli::simctl`]` gains an env-passing `launch`
  variant; `--hot` sets `SIMCTL_CHILD_DYLD_INSERT_LIBRARIES=<client dylib>`,
  `SIMCTL_CHILD_INJECTION_PROJECT_ROOT=<workspace root>`, and the XCTest
  `DYLD_FRAMEWORK_PATH`/`DYLD_LIBRARY_PATH` the client dylib's deps need
  (`simctl` forwards any `SIMCTL_CHILD_*` var into the launched process).

### macOS (`--mac --hot`)

The same server/recompiler/watcher drive a native mac app; only the launch and
the signing posture differ:

- **Injectability is settled at build time.** A macOS `--hot` build adds
  `ENABLE_HARDENED_RUNTIME=NO` and `ENABLE_APP_SANDBOX=NO` — command-line
  settings outrank project ones, so the hot Debug product is built without the
  two protections that break injection (the hardened runtime makes dyld strip
  `DYLD_INSERT_LIBRARIES` and library validation reject the ad-hoc recompiled
  dylibs; the sandbox blocks the client's socket and dlopen from outside the
  container). No post-build re-signing, no project mutation. Xcode 14+ mac
  templates declare both protections via exactly these settings, so a template
  app is injectable with zero setup.
- **Preflight.** A sandbox declared in an explicit `.entitlements` file (App
  Store projects) is beyond build settings — it is auto-stripped for the hot
  build (see *Zero-config sandbox stripping* below), and
  `[`crate::cli::inject::mac_preflight`]` stays as the safety net: it inspects
  the built product (`codesign -d`) and refuses with the exact fix when the
  sandbox survived (stripping disabled or failed) instead of launching a dead
  session. A hardened product (re-signed by a run-script phase) is refused the
  same way, unless it carries the `allow-dyld-environment-variables` +
  `disable-library-validation` entitlements pair that makes it injectable anyway.
- **Direct spawn, raw env.** The mac app is our own child process — the same
  injection env as the simulator's but unprefixed (no `SIMCTL_CHILD_`, no
  install step), stdout/stderr piped through the session console. `r` kills and
  respawns the child; `d` detaches leaving it running. The session holds the
  child the way the plain session holds its app (`Running`), so an app that
  exits on its own gets the same `✗ <bundle id> exited` notice from the same
  check, which also records the exit for `app logs --exits`. `o` asks the same
  check before `open`, so an app that has exited is not relaunched outside the
  session.
- **Bundled mac client.** `vendor/injection-client/build.sh` produces a second
  prebuilt (`SweetpadInjectionClientMac.dylib`, the upstream SPM product built
  for `generic/platform=macOS`), embedded alongside the simulator client and
  selected by SDK. `InjectionNext.app` stays the fallback.
- **Connect watchdog.** A mac app that hasn't dialed back within 15s is running
  uninjected (something undid the insert env); the session says so, with the
  `codesign` command that shows what the product carries.
- **Validated end-to-end** by `ci/hot-reload-e2e.sh`: the `--hot-selfcheck`
  nonce round-trip (edit → `.injected` → the new code's marker observed in the
  host unified log) passes for both recompilers against the fixture's
  `SweetpadCIMac` scheme.

**Beautifier interaction (`EMIT_FRONTEND_COMMAND_LINES` × §11).** The setting
prints the `swift-frontend` invocations into xcodebuild's *raw* transcript, but
those lines start with a tool path, not a task verb, so `[`buildlog::parse_line`]`
classifies them as `Event::Other`, which `[`buildlog::render`]` suppresses unless
`-v` — the same path that already swallows xcodebuild's per-task command echoes.
So the **beautified default stream is unchanged** (no extra verbosity, nothing
broken; they can't reach the diagnostic matcher, which requires `: error:`/
`: warning:`/`: note:` markers a command line never carries). The only cost is a
larger raw transcript, paid only under `--hot`. Because parsing is decoupled from
rendering, path A captures the **raw** frontend lines for the recompiler index in
parallel with (not instead of) beautification — both consume the same stream, so
there is no double-printing and no leakage into the pretty output.

The server must be listening on `:8887` before the app launches so the client's
`+load` connect succeeds.

### Zero-config sandbox stripping (`--hot` on macOS)

You cannot inject into a sandboxed app — dyld sanitizes the environment, the
container blocks the CLI's socket and dylib paths — so "hot reload on a
sandboxed project" can only ever mean *automating the un-sandboxing*; there
is no keep-the-sandbox variant to chase. The build-setting overrides above
already handle the common case, but when the sandbox comes from an explicit
`CODE_SIGN_ENTITLEMENTS` plist (the normal shape for App Store projects),
the plist wins at signing. Until v8 the only fixes were user-side and
permanent (edit the project, or hand-pass an override every run); now `--hot`
is zero-config:

1. **Resolve the effective entitlements** for the (scheme → app target,
   configuration, macOS) being hot-built, via the in-process build-settings
   resolver — the engine behind `settings show`, so `$(SRCROOT)` interpolation
   and conditional `CODE_SIGN_ENTITLEMENTS[sdk=macosx*]` spellings come for
   free. No explicit file ⇒ nothing to do (the `ENABLE_APP_SANDBOX=NO`
   override suffices).
2. **Strip ephemerally.** If the plist asserts `com.apple.security.app-sandbox`,
   copy it to the hot-reload cache
   (`~/.cache/sweetpad/hot-reload/entitlements/<projhash>/<config>-nosandbox.entitlements`),
   delete the sandbox key (everything else stays — network entitlements etc.
   become no-ops, keeping behavior close to sandboxed-minus-container), and
   ensure `com.apple.security.get-task-allow` for attach/injection. The edit
   runs through `plutil`/`PlistBuddy`, so binary plists work; any failure
   falls back to the preflight's guidance rather than guessing. The file is
   regenerated from the real plist on every hot run, so edits propagate.
3. **Override for the hot build only**: `CODE_SIGN_ENTITLEMENTS=<cache path>`
   rides next to the sandbox/hardened-runtime overrides. Nothing in the
   user's project is written — crash-safe by construction (`kill -9` leaves
   the repo pristine, and generated projects show no spec/`.xcodeproj` diff).
4. **Announce, don't ask.** One honest line
   (`hot reload: running un-sandboxed for injection …`) instead of a blocking
   prompt — zero-config by default, CI/headless friendly, and the Keychain/
   container behavior change is named. Un-sandboxed Debug means data lands in
   `~/Library` rather than the app container and sandbox-only bugs hide until
   a sandboxed build — which is why it's hot-Debug-only and announced.

Knobs: `--keep-sandbox` skips the strip (reproducing the preflight refusal —
the honest opt-out), `--hot-entitlements FILE` signs the hot build with a
caller-supplied plist instead of auto-deriving (apps needing specific
non-sandbox entitlements while injected), and a committed
`[run] auto_unsandbox = false` opts a whole project out. Mechanism
alternatives considered and rejected: a temp `-xcconfig` (equivalent, plus a
file), post-build re-signing (redundant second step when we already own the
build), and editing the real `.entitlements` with restore-on-exit (the
crash-footgun the ephemeral design exists to avoid).

### The recompiler — resolver-first (F), live-capture fallback (A)

Turning a saved `Foo.swift` into a loadable `.dylib` is the load-bearing risk.
The upstream approach (InjectionLite's `LogParser`/`Recompiler`) **scrapes the
build logs**: `gunzip` the newest `*.xcactivitylog` in DerivedData, `grep` for
the ` -primary-file Foo.swift ` frontend invocation, regex-rewrite it down to a
single-primary `-c -o eval.o`, then regex out `-sdk` to assemble a fixed
`clang -dylib -interposable …` link line. It works, but it rides an undocumented
log format that shifts every Xcode release and breaks under log pruning, Whole-
Module mode, and `COMPILATION_CACHE_ENABLE_CACHING`. We do **not** take that as
the primary path.

Both implemented strategies instead converge on running **one
`swift-frontend -primary-file` job** for the changed file (single-file speed) and
linking it into a dylib; they differ only in where that frontend command comes
from. Recovered commands are **cached per source** (stable until the file
set/settings change), so the per-save cost is just compile + link.

**(F) Default — resolver → frontend via `swiftc -###`.**
`[`crate::compiler_args`]` produces, from the resolved pbxproj/xcspec settings
(snapshot-tested against real `xcodebuild`), the target's **driver** `swift_arguments`.
But single-file compilation is a **frontend** (`-primary-file`) operation, and the
two flag vocabularies differ — so the recompiler asks the *user's own toolchain
driver* to translate: `xcrun swiftc -### -disable-batch-mode <driver args>
<module files>` is a **dry run** that prints the `swift-frontend` jobs it *would*
spawn (one `-primary-file` per file). We parse those, cache each by source, and
on a save run the changed file's job (rewritten to a single `-o eval.o`) then a
`clang -dynamiclib -interposable -undefined dynamic_lookup` link. No build-log
dependency, no Xcode-version log-format drift, and because `-###` uses the
*active* toolchain the driver/frontend/version all match by construction. If
`-###` recovery ever fails, it falls back to whole-module `swiftc -emit-library`.
(We deliberately do **not** link `swift-driver` as a library: a vendored driver
wouldn't match the user's Xcode — the same skew we avoid everywhere — and the
cached one-shot spawn makes per-save cost ~0 anyway.) Each `-###` dry run leaves
a `TemporaryDirectory.*` in `$TMPDIR`, which the driver never removes, so every
toolchain child of the recompiler (the dry run, the compile, the link and the
whole-module fallback) runs with a `TMPDIR` of the session's own, `tmp/` in the
work directory the session removes when it ends. It lasts as long as the cached jobs do, so a path a job names
inside it stays valid for every save.

`xcodebuild` itself keeps the user's `TMPDIR`, though a build leaves a
`TemporaryDirectory.*` there too (the build service runs `swiftc --version`
first), and every invocation that opens a project
(`-list`, `clean`, `-resolvePackageDependencies`, a build) writes a
`_Users_<user>_.swiftpm.lock`. SwiftPM keeps its cross-process locks in
`$TMPDIR`, each named for the path it guards (this one guards `~/.swiftpm`), and
a lock only excludes the other SwiftPM clients (Xcode itself, a second
`xcodebuild`, `swift build`) because they all take it in the same directory. A
private `TMPDIR` would let a sweetpad build and one of them write that shared
state at once. The build's children inherit `TMPDIR` as well: a Run Script phase and a macOS
test host (in its environment, though not in `NSTemporaryDirectory()`) see it,
so both would get a directory that is removed when the build ends.

So sweetpad cleans up after the build instead. Before the build it notes the
`TemporaryDirectory.*` entries in `TMPDIR`. After the build exits, it removes
each new one that holds only the driver's `.keep-directory`. The lock files
stay. A live driver's directory looks the same until its first job writes
there, so nothing is removed while a Swift driver runs with that `TMPDIR` (or
with none), while a running process names one of the directories, or when `ps`
can't list the processes. One helper does this, `scratch::TmpdirLeftovers` in
sweetpad-core, for every run that keeps the user's `TMPDIR`: a `bsp serve`
prepare that falls back to `xcodebuild`, and every `xcodebuild`, `swift` and
`swiftc` child of the CLI, since each one is spawned and reaped in
`process.rs`. A child given a `TMPDIR` of its own is left to whoever made that
directory. The CLI's runs cover `build`,
`test`, `archive`, `clean` and the builds of an `app run` session. They also
cover a package's `swift build`, `swift test` and `swift run`. SwiftPM leaves
several of these directories per build (five on a warm build of the CI package
with Xcode 27.0). `--show-command` runs nothing and touches nothing. A
signal that ends the CLI during a build can still leave the build's
directories.

**(A) Switchable — capture frontend command lines from our own build.**
Because the CLI *is* the builder, the `--hot` build tees the `swift-frontend`
invocations straight out of `xcodebuild`'s stdout (`EMIT_FRONTEND_COMMAND_LINES`)
— so the exact per-file command is a **free byproduct**, no `-###` spawn at all.
Same single-file/link path as (F), sourced from the transcript and cached per
source. Selected with `--hot-recompiler buildlog`.

### Module layout & session integration

A new `cli/inject/` tree, kept off the existing tool-spawning modules:
`protocol.rs` (the two enums + framing primitives), `socket.rs` (the `:8887`
TCP listener), `server.rs` (accept + handshake + command loop), `recompiler.rs`
(F + A), `watcher.rs` (debounced FS watch of the workspace root, ignoring build
output dirs). The server runs as a sidecar thread alongside the existing
`Running` struct in `[`crate::cli::commands::app`]`; the watcher becomes a third
event source next to the keypress reader and the log stream. `r` still does a
full rebuild+relaunch; `q`/Ctrl-C/Ctrl-D quit and tear the server down.

### Milestones

> **Milestone 1: ✅ validated** — run #5 of `hot-reload-spike.yaml` on a real
> arm64 simulator: the Rust server completed the `:8887` handshake (`version 4001`,
> `iPhoneSimulator arm64`, projectRoot/tmpPath/executable), recompiled the changed
> file, linked a dylib, sent `.load`, and the in-app client confirmed `.injected`.
> The novel socket protocol and the build→load→patch chain are proven.

1. **Socket spike — ✅ done.** Validated transport + a recompile/`.load`/`.injected`
   round-trip using the **(A)** live build-log command.
2. **Build-flag + launch-env plumbing — ✅ done.** `BuildPlan.hot` appends
   `-interposable` + `EMIT_FRONTEND_COMMAND_LINES`; `simctl::launch_opts`
   forwards the `SIMCTL_CHILD_*` injection vars (`app run --hot`, simulator-gated).
3. **Recompiler — ✅ done.** Both strategies in `cli/inject/recompiler.rs`
   converge on a cached single-file frontend command: **F** (default) recovers it
   from the resolver via `xcrun swiftc -###` (whole-module `-emit-library`
   fallback); **A** (`--hot-recompiler buildlog`) recovers it from the captured
   transcript. (F's `-###` path wants the macOS CI's confirmation; A is proven.)
4. **Watcher + session integration — ✅ done.** Polling watcher → `server.inject`;
   `run_hot_session` builds + serves + launches + watches; key loop keeps `r`
   (full rebuild, client reconnects) / `q`; `.injected`/`.failed` status lines.
5. **Bundled client — ✅ done & validated.** The client is built once from the
   pinned InjectionNext **SPM product** (XCTest-free — see Client distribution
   below; `vendor/injection-client`) and embedded into the binary via `build.rs`;
   `resolve_dylib` order is override → bundled (materialized under a content key) →
   `InjectionNext.app` fallback. Validated green by the `hot-reload-src` CI job,
   which runs the real `app run --hot` (no dylib override) and injects on **both
   Xcode 16 and 26** from one prebuilt — no clone, no per-Xcode build.
6. **Polish — ✅ mostly done.** "Inject package missing" advisory ported;
   teardown (watcher/server/app/cleanup) wired. (Config-level default for the
   recompiler mode — beyond the `--hot-recompiler` flag — is the remaining nicety.)

> **Implementation status:** the `cli/inject/` module + `app run --hot` are
> implemented and **validated end-to-end on real simulators** (Xcode 16 + 26),
> both recompilers, with the client bundled from the pinned InjectionNext SPM
> product. `clippy -D warnings`/`fmt` clean, unit tests on Linux, live e2e on the
> macOS matrix.

### Client distribution — bundle one prebuilt, built from the upstream SPM product (decided)

The client is **compiled once and bundled into the `sweetpad` binary**, not built
on the user's machine. `vendor/injection-client/build.sh` builds it from the
**pinned upstream InjectionNext SPM product** (MIT) for the iOS simulator, `build.rs`
embeds the result via `include_bytes!`, and on the first `--hot` the CLI writes it
to a content-addressed cache and `DYLD_INSERT_LIBRARIES`-injects it. No git clone,
no per-Xcode `xcodebuild`, no runtime network.

The key move is **dropping XCTest** — the one Xcode-versioned dependency in the
client. InjectionNext's *Xcode* `InjectionBundle` target links XCTest + Quick +
Nimble for its test-reload feature, and that ABI skew is what broke a *prebuilt*
binary under Xcode 16.4 (Milestone 1). But its *SPM* product references none of
them, and an SPM build defines `SWIFT_PACKAGE` — exactly the flag the engine's
`canImport(Nimble)` build sentinel keys on — so the product compiles the full
engine **without** Quick/Nimble/XCTest. The resulting dylib depends only on
ABI-stable OS/runtime libraries (`/usr/lib/swift/*`, `/System/...`), so **one
prebuilt is portable across Xcode versions** and can be shipped. (Verified: `otool
-L` shows zero XCTest, and the e2e injects on Xcode 16 + 26.) Test hot-reload is
dropped along with XCTest; the promoted feature is app UI/code reload (SwiftUI/UIKit).

- **Zero-edit wrapper, not a fork.** `vendor/injection-client/Package.swift` is a
  thin SPM package that depends on pinned InjectionNext and re-exposes its product
  as a `.dynamic` library (upstream ships only static ones) so it's loadable via
  `DYLD_INSERT_LIBRARIES`. Upstream is unpatched; bumping the client = bumping one
  `revision` pin. `-all_load` keeps the client's ObjC `+load` connect hook from
  being dead-stripped, and the extracted Mach-O is ad-hoc re-signed (a `.framework`
  signature doesn't survive extraction, and the simulator won't load a mismatched
  insert).
- **`xcodebuild` drives the SPM build**, targeting `generic/platform=iOS
  Simulator`. This sidesteps raw `swift build`'s finicky iOS-sim support and the
  dev-symlink snag from Milestone 1, while SPM still resolves InjectionNext's
  submodules automatically.
- **Not committed.** The ~4.7 MB dylib is gitignored; CI (`hot-reload-src`) and the
  release CLI scripts (`build:cli` / `build:cli:universal`) run `build.sh` before
  the cargo build, and `build.rs` embeds whatever is present. Builds without it
  compile fine (empty embed) and fall back to `InjectionNext.app` at runtime.
  `app run` looks for a client before it builds (`inject::client::check_available`:
  the override, the bundled client, then `InjectionNext.app`, writing nothing),
  so a typed `--hot` with none fails at once, exit 5 (`tool_missing`), naming
  `build.sh` or `SWEETPAD_HOTRELOAD_DYLIB` for the SDKs a release bundles and
  `InjectionNext.app` for the rest. A `[run] hot = true` default yields with a
  warning instead, as it does to a busy `:8887`. It yields with a note to
  `--no-logs`, `--detach` and `--wait-for-debugger` too, which ask for a run a
  hot session can't be (launch and return, or start suspended); a typed
  `--hot` refuses those, exit 2.
- **Drop-in UX preserved** — no project edit, no `InjectionNext.app` required. The
  SwiftUI `@ObserveInjection`/`.enableInjection()` annotations remain the user's to
  add (UIKit reloads without them).

## 9e. `dependency` — Swift Package Manager dependencies (`dep`)

View, add, remove, and resolve a project's SPM dependencies without opening
Xcode — the one package operation Xcode otherwise gates behind its GUI.

```
sweetpad dependency list [--transitive]      declared packages + locked versions
sweetpad dependency add <url> <requirement>  add a package, link a product
sweetpad dependency remove <pkg>             remove a package (or unlink a product)
sweetpad dependency update [<pkg>] [req]      bump pins, or change a requirement
sweetpad dependency resolve                  refresh Package.resolved
```

Works on all three containers. For an `.xcodeproj`/`.xcworkspace` there is no
Apple CLI for this, so the object graph is edited directly via
`crate::spm_pbxproj` (parse → mutate → `pbxproj_writer::serialize` → write,
byte-for-byte, like the scaffold/merge paths); for a `Package.swift` it drives
the Swift 6 `swift package add-dependency`/`add-target-dependency`/`resolve`.

- **`list`** shows each directly-declared package's requested requirement next to
  its locked version, correlated by SwiftPM identity against `Package.resolved`,
  plus its `product → target` links. `--transitive` adds the resolved-only pins.
- **`add`** takes one SPM-style requirement flag (`--from`/`--exact`/
  `--up-to-next-minor-from`/`--branch`/`--revision`, plus `--to` for a range),
  resolves the package to read its real products, then prompts for the
  product(s)/target(s) to link (or takes `--product`/`--target`; strict-errors
  off a TTY). When the package declares no products, `add` fails before the
  prompt with "the package declares no products to link" and rolls the
  package back out. Auto-resolves afterward unless `--no-resolve`. Supports
  remote git URLs and local paths (`XCLocalSwiftPackageReference`); the
  product is linked via a Frameworks `PBXBuildFile`, or a
  `PBXTargetDependency` for static-library targets.
- **`remove`** drops the whole package (reference, product dependencies, target
  links, build files, and its `Package.resolved` pin) by name/URL/identity, or
  narrows to unlinking one product from one target with `--product`/`--target`.
  Removing a *local* package also matches products Xcode wrote without a
  `package` back-ref (by the names its manifest declares). Naming a transitive
  pin yields a hint to change/remove the direct package that pulls it in.
- **`update`** with no requirement re-pins to the latest the current
  requirements allow — `swift package update [name]` for a package, or dropping
  the pin(s) (one, or the whole lockfile) and re-resolving for an xcodeproj.
  With a requirement (`dep update <pkg> --exact 6.0.0`) it rewrites that
  package's `requirement` in place — a bump, pin, or **downgrade** — then drops
  the stale pin and re-resolves.
- `add`/`update` discovery resolves **once**: it reads the package's products
  from the resolved checkout located precisely via SourcePackages'
  `workspace-state.json` (robust to monorepo sub-paths and case), so there's no
  second resolve and no checkout-name guessing.
- A workspace `add`/`remove`/`update` targets the member project that declares
  the package (for remove/update), else its sole member, else an interactive
  pick (strict `--project` error off a TTY). All `xcodebuild
  -resolvePackageDependencies` calls pass a `-scheme` (required for a workspace).

**Amendment: `dependency` reads and writes `project.xcproj` too.** The grammar
and the choice of member project are unchanged; the backend follows whichever
document the bundle holds. The JSON document names a product's package by the
name Xcode gives it (`swift-collections`, `LocalKit`) rather than pointing at
an object, so that name is the package's handle there, and an `add` whose name
is already taken is refused where a pbxproj would take a second reference. A
link is a `package-product-members` entry for the target's Frameworks phase, or
a `{ kind: package }` dependency for a static library or a target with no
Frameworks phase — the same split the pbxproj makes between a build file and a
target dependency. Removing drops the lists it empties, so `add` then `remove`
leaves the document byte for byte as it was. Verified end to end on Xcode 27.2
for a local and a remote package: `add`, a resolve and a build that imports the
product, `update`, and `remove`.

**Amendment: `update` resolves into an empty clone directory first.**
xcodebuild has no update, and on Xcode 27 pruning a pin (or the whole lockfile)
before a resolve doesn't make one: the resolve keeps a checkout already in
SourcePackages while it satisfies the requirement, and doesn't write the pruned
pin back. So `update` prunes, then resolves with `-clonedSourcePackagesDirPath`
pointed at an empty directory. That resolve has no checkout to keep, honours
the pins still in the lockfile, and writes the newest allowed versions to
`Package.resolved`. A normal resolve then checks them out in SourcePackages.
The empty directory also gets past Xcode 27 refusing to move a package from a
version that declares SwiftPM traits to one that declares none ("Disabled
default traits … on package … that declares no traits"). A plain resolve hits
that whenever a lockfile and a checkout of the old version both exist, with
either project format. A requirement change takes the newest version the new
requirement allows, and the report names every pin that moved (`changes` in
JSON). The price is a second clone of the package graph, which SwiftPM's
repository cache keeps short. Measured on Xcode 27.2 with a project of each
format and a workspace. In human output the renderer shows the indented lines
under an `xcodebuild: error:` header that ends in a colon, since that is where
xcodebuild puts the reason a resolve failed. Those lines go to stdout and the
error goes to stderr. When the two streams are different files (`2>err.log`,
or stdout piped away), the error repeats the lines as xcodebuild printed them.
On a terminal or behind `2>&1`, the reason already sits just above the error,
which stays one line.

> Supersedes the earlier "vendor full source, compile per Xcode" plan: the
> from-source per-Xcode build (and its `~/.cache/.../<xcode-build>/` cache) existed
> only to keep XCTest's ABI matched against the active Xcode. Building the
> XCTest-free SPM product removes that need, so one bundled prebuilt suffices. The
> earlier strip-the-bundle analysis (≈ ½ week) is moot — the SPM product is already
> XCTest-free with no patching.

### Open decisions

- **ABI match — A proven, F pending.** Path A (exact build-log command) injects
  cleanly (Milestone 1), so it is primary. The (F) resolver path's ABI match is
  still to confirm; until then F is an optimization, not the default.

### macOS test harness (permanent)

Hot reload needs macOS + Xcode + a simulator, so it's validated in CI by the
permanent **`xcode-tests.yaml`** workflow — a reusable matrix harness for any
Xcode/simulator-requiring test, across Xcode versions (16.x, 26.x; weekly + on
push/PR). Two jobs:

- **`cli`** — the full standalone-CLI e2e (`ci/smoke.sh`) on each Xcode.
- **`hot-reload-src`** — the injection e2e (`ci/hot-reload-e2e.sh`) on **both
  Xcode 16 and 26**: it builds the bundled client (`vendor/injection-client/build.sh`),
  generates the fixture app, and runs the *real* `sweetpad app run --hot
  --hot-selfcheck` (hidden flag) with **no** dylib override, so it exercises the
  client **bundled into the binary** (Milestone 5), for **both** recompilers
  (resolver + build-log). The self-check builds with the interposable/frontend
  flags, starts the `:8887` server, launches with the client injected, edits a
  Swift file once, and asserts `.injected` — exiting non-zero otherwise. Each
  save logs a line when the watcher hands it over (`recompiling…`) and one when
  the recompile ends (`recompiled in 1.1s, loading…`). A self-check that times
  out names the step the save stopped at: the watcher never reported the edit,
  the save ended before a load request, the recompile never finished, the load
  request was never sent, or the app never answered it. The watcher snapshots
  the tree before `HotSession::start` returns, so an edit made right after it
  returns still fires. (An
  earlier `hot-reload` job ran the same e2e against a *prebuilt-download* client
  that still linked XCTest, so it carried the per-Xcode ABI skew — flaky, and
  removed once the bundled XCTest-free client made one prebuilt portable.)

This supersedes the original throwaway spike (`hot-reload-spike.yaml`), whose
run #5 first proved the socket + recompile→load→inject chain end-to-end.

## 9f. v6 — project mutation: build settings & sync-group sources

Make the pbxproj itself directly drivable — the settings half of an XcodeGen
`project.yml` becomes `settings set` calls, and the `sources:` half becomes
Xcode 16 **synchronized root groups**, so per-file membership stops being a
problem anyone has to manage. Both ride the proven mutation pipeline
(parse → mutate → `pbxproj_writer::serialize`, byte-for-byte, GUIDs via
`fresh_guid` — the same path `dependency add` ships on). *Declined:* a
declarative spec file / `project sync` (that's re-implementing XcodeGen and
creates a second source of truth), XcodeGen interop, and an xcconfig write
mode. Idempotent imperative commands in a committed script *are* the spec.

**Mutations never guess.** Unlike the run/build resolution flow (TTY pickers),
`settings set`/`unset` and the `source` verbs hard-error on any ambiguity —
interactive or not — with the flag that disambiguates named in the message.
A mutation either applies exactly what was asked or changes nothing.

### `settings set` / `unset` / `show --raw`

```
sweetpad settings set KEY=VALUE [KEY=VALUE …] [--target T]… [--configuration C]…
sweetpad settings set KEY+=VALUE …                    append to a list setting
sweetpad settings unset KEY [KEY …] [--target T]… [--configuration C]…
sweetpad settings show --raw [--target T]             the stored pbxproj layer
```

- **Scope.** Project-level `XCBuildConfiguration`s by default; `--target`
  (repeatable) switches to those targets' configurations. No `--all-targets` —
  project-level *is* "all targets"; that's what inheritance is for. All
  configurations by default (XcodeGen `settings.base` semantics);
  `--configuration` (repeatable) narrows. Unknown target/configuration names
  are errors.
- **Multiple assignments, one write.** All pairs apply in a single
  parse → mutate → serialize pass (temp + rename): one diff, atomic.
- **Arrays.** Repeating a key builds an array in argument order
  (`set LD_RUNPATH_SEARCH_PATHS='$(inherited)' LD_RUNPATH_SEARCH_PATHS=…`).
  `KEY+=VALUE` appends: the prior value normalizes to its element list (arrays
  as-is; strings whitespace-split, matching how xcodebuild resolves list
  settings) and the new element lands at the end. Canonical on-disk form:
  pbxproj array for >1 element, plain string for 1.
- **Conditional keys** (`CODE_SIGN_IDENTITY[sdk=iphoneos*]`) pass through
  verbatim as part of the key. `set`/`unset` match the exact key only —
  conditional variants are separate keys, never implicitly swept.
- **`unset`** removes the key (true inheritance), not `$(inherited)`. Absent
  key → no-op with a note: re-runnable scripts stay green.
- **Validation, xcspec-backed.** A known key set to a value outside its xcspec
  domain (enum/boolean) gets a *warning*, never an error; unknown keys are
  accepted silently (user-defined settings are legal, xcspec coverage isn't
  total).
- **xcconfig interplay.** When a touched configuration has a
  `baseConfigurationReference` whose xcconfig also assigns the key, warn that
  the pbxproj value now shadows it. Writing xcconfig files is out of scope.
- **Workspaces.** `--target` maps the target to its owning member project and
  edits that pbxproj; the same target name in two members is an error naming
  `--project`. A project-level set resolves to the sole member, else requires
  `--project`. Swift packages: clean error (no pbxproj).
- **`show --raw`** prints what the pbxproj layer actually stores per
  target/configuration — the verification companion, and the answer to "why
  does `show` still have a value after `unset`" (inheritance).
- **Report** (JSON envelope): per (target, configuration): key, old raw value,
  new raw value, plus the re-resolved value (the *effect*, via the in-process
  resolver) and the file written.

### Sync-group sources

- **`project new` scaffolds a `PBXFileSystemSynchronizedRootGroup`** for
  `<Name>/` — no per-file `PBXFileReference`/`PBXBuildFile` objects,
  `objectVersion = 77` (Xcode 16+ floor, the fresh-template shape:
  `preferredProjectObjectVersion`, no `compatibilityVersion`). Adding a file to
  the app is `touch`; the pbxproj never changes as the project grows. The
  classic per-file scaffold shape is deleted, not flagged — one graph shape,
  one test suite. (Corpus already round-trips both sync-group generations:
  converted objectVersion-70 projects like ice-cubes and fresh
  objectVersion-77 templates.)
- **`source`** — the sync-group-era replacement for XcodeGen's `sources:` list:

  ```
  sweetpad source list [--target T]           roots + membership exceptions
  sweetpad source add <dir> --target T        attach a synchronized root
  sweetpad source remove <dir> --target T     detach a root
  sweetpad source exclude <path> --target T   membership exception (opt a file out)
  sweetpad source include <path> --target T   drop the exception
  ```

  `exclude`/`include` edit the root's
  `PBXFileSystemSynchronizedBuildFileExceptionSet`; `add` inserts one group
  object and lists it in the target's `fileSystemSynchronizedGroups`.
- **`settings set` auto-exception.** Setting `INFOPLIST_FILE` to a path inside
  a target's sync root also adds the membership exception. Investigated live
  (Xcode 26.5 / 17F42, scratch sync-root app, macOS + iphonesimulator) and
  against the corpus:
  - *Without* the exception, an in-root Info.plist is treated as an ordinary
    resource **and** processed as the Info.plist: on iOS the two outputs
    collide at the flat bundle root — `error: Multiple commands produce
    '….app/Info.plist'`, **build failure**; on macOS they don't (resources go
    to `Contents/Resources/`), so it's the "Copy Bundle Resources … contains
    this target's Info.plist" warning plus a stray duplicate plist shipped in
    the bundle. The auto-exception is correctness on iOS, hygiene on macOS.
  - *With* `membershipExceptions = (<path>)` both platforms build clean, the
    custom plist is the one processed, and no resource copy happens — exactly
    the objects Xcode itself persists (ice-cubes and NetNewsWire both carry
    `membershipExceptions = (Info.plist)` sets for every target whose
    `INFOPLIST_FILE` points into a sync root; Xcode does *not* special-case
    the filename at build time — any un-excepted `.plist` in a root is copied
    to Resources).
  - `CODE_SIGN_ENTITLEMENTS` needs **no** exception: a `.entitlements` file in
    a sync root joins no build phase (not copied, no warning) and is consumed
    via `ProcessProductPackaging` — so the auto-exception applies to
    `INFOPLIST_FILE` only, matching the corpus (no entitlements entries in any
    exception set).

> Superseded spellings: §9g moves this surface under the `pbxproj` plumbing
> namespace — `settings set/unset` → `pbxproj settings set/unset`,
> `show --raw` → `pbxproj settings show`, `source` → `pbxproj folder` (with
> `exclude`/`include` relocated to `pbxproj membership`). Everything else in
> this section — semantics, scoping, the auto-exception — is unchanged.

## 9g. v7 — the `pbxproj` plumbing namespace & explicit membership

Two decisions in one section: project-graph mutation is **plumbing**, visibly
separated from the everyday porcelain (git's plumbing/porcelain split); and
classic-project conversion ships as **explicit primitives, not a converter**.
These commands are for scripts and agents — it is better to run 200 explicit,
reviewable commands than one that decides everything silently. A monolithic
`project convert` is *declined*: its only real intelligence is a set
subtraction (folder contents − project membership), and the caller can do
that subtraction itself once the primitives expose the data. Every hard
conversion case (stray files, cross-target borrowing, per-file flags) becomes
a visible line in a script instead of a converter heuristic.

**The namespace.** `sweetpad pbxproj <resource> <verb>` — the CLI's first
three-level command path, justified by the boundary it draws: inside the
namespace you are thinking about `project.pbxproj` *objects*; outside it,
about tasks. The namespace already existed (hidden) for `pbxproj resolve`;
it becomes visible. `dependency` stays porcelain despite editing the pbxproj —
it's a GUI-equivalent daily task, not graph surgery. All namespace mutations
follow §9f law: never guess, hard-error on ambiguity, idempotent no-ops,
one atomic write per invocation.

```
sweetpad pbxproj resolve                            merge plumbing (§9c, unchanged)

sweetpad pbxproj settings show [--target T] [--key K]     the STORED layer
sweetpad pbxproj settings set KEY=VALUE … [--target T]… [--configuration C]…
sweetpad pbxproj settings unset KEY … [--target T]… [--configuration C]…

sweetpad pbxproj folder list [--target T]           synchronized folders + exceptions
sweetpad pbxproj folder add <dir> --target T
sweetpad pbxproj folder remove <dir> --target T

sweetpad pbxproj membership list [--target T]       everything a target builds
sweetpad pbxproj membership add <path>… [--fileref ID]… --target T --phase P
sweetpad pbxproj membership remove <path>… --target T     classic build-file entries
sweetpad pbxproj membership exclude <path> --target T     sync-folder exception
sweetpad pbxproj membership include <path> --target T     drop the exception

sweetpad pbxproj fileref list [--under PREFIX]      the files in the project
sweetpad pbxproj fileref add <path>… [--type T] [--source-tree ST] [--group G]
sweetpad pbxproj fileref remove <file> [--dangling]

sweetpad pbxproj group list                         the navigator tree
sweetpad pbxproj group add <name> [--parent G] [--path P] [--source-tree ST]
sweetpad pbxproj group remove <group> [--orphan-children]
sweetpad pbxproj group move <node> [--to G]         re-home a node
sweetpad pbxproj group attach <node> --group G      list a child (pbxproj only)
sweetpad pbxproj group detach <node> --group G      unlist a child (pbxproj only)
```

- **`settings` splits by layer, not by flag.** Top-level `sweetpad settings
  show` stays the porcelain question ("what will the build use" — resolved).
  `pbxproj settings show` answers "what does the file say" — the raw stored
  layer, per configuration; §9f's `--raw` flag dissolves into the namespace.
  `set`/`unset` live only here (semantics exactly as §9f).
- **`folder`** is §9f's `source` renamed to Xcode's own term (Xcode 16 UI:
  "New Folder", "Convert to Folder"). Same list/add/remove semantics.
- **`membership`** is the new resource, named for Xcode's File Inspector
  panel ("Target Membership" — the checkbox UI these verbs script). It spans
  both representations:
  - **`list`** reports everything a target builds with *provenance*: the
    classic build-file entries (resolved path, build phase — sources/
    resources/headers/frameworks/copy-with-name — plus per-file
    `COMPILER_FLAGS`, `ATTRIBUTES`, platform filters), and the synchronized
    folders with their exceptions. It does **not** enumerate the disk under
    sync folders — the folder + exceptions *is* the membership statement,
    and `ls` is the primitive for expanding it.
  - **`remove`** (batched paths, one write) deletes a target's classic
    build-file entries for the named files. When the last build file
    referencing a file reference goes, the reference is deleted and emptied
    ancestor groups are pruned — the same orphan-cleanup contract
    `folder remove` set. A reference the project still names otherwise stays:
    an app extension embedded by the target is still its own target's
    product, and deleting it would leave that target naming nothing. A file
    that isn't a member is a recorded no-op.
  - **`exclude`/`include`** are §9f's exception verbs, relocated: excluding
    a file *is* a membership edit (unchecking the box in Xcode writes
    exactly these exception sets). The set also holds the folder's per-file
    compiler flags, attributes and platform filters, so `include` deletes it
    with its last exception only when it holds nothing else.
  - **`add`** (batched, one write) gives a target a classic build-file entry
    for each file, in the phase `--phase` names. The phase is never derived
    from the extension, and the file reference has to exist already
    (`fileref add` makes one). The two things a smart `add` would have had to
    guess are the two things the caller states instead. A file a sync folder
    already builds is refused with the same cross-hint discipline as `remove`.
    Files are named by path or by `--fileref <ID>`, and the two mix in one
    invocation — see "Two ways to name a thing" below.
  - **Verbs are mechanism-specific and cross-hint.** `remove` on a file
    that's built via a sync folder errors with "use membership exclude";
    `exclude` on a file with a classic build-file entry errors with "use
    membership remove". The wrong verb never silently does the other thing.
- **`fileref` and `group` are the classic representation's other two axes**,
  kept apart because they answer different questions: a `PBXFileReference`
  says a file *exists* in the project, a `PBXGroup` entry says where it
  *appears* in the navigator, and membership says what *builds* it. Wiring a
  new file into a target is three explicit commands — `fileref add`, then
  `group attach` if the reference wasn't created under a group, then
  `membership add` — the way `git hash-object` and `git update-index` are two.
  - **No verb cascades into a neighbouring axis.** `fileref remove` refuses
    while a build file still points at the reference (`--dangling` overrides);
    `group remove` refuses while the group still lists children
    (`--orphan-children` overrides); `group detach` unlists a child without
    deleting the object. The single exception is referential integrity —
    deleting an object also drops it from every group's `children`, since a
    group naming a missing object is a corrupt file rather than a valid
    intermediate state — and every outcome reports what it took with it.
    Any other name for the object refuses the delete, whatever the override
    flag: the xcconfig a configuration is based on, a target's product, the
    navigator root, the Products group, a folder an xcconfig is anchored in.
    Xcode opens a pbxproj that names a missing xcconfig and builds without
    it, so this is the difference between an error and a silently wrong
    build. A `project.xcproj` refuses the same deletes for a stronger reason:
    Xcode 27.0 and 27.2 will not open one whose reference names nothing
    ("Invalid reference").
  - **Paths are anchored, not guessed.** A reference's `path` resolves through
    its `sourceTree` (`<group>`, `SOURCE_ROOT`, `<absolute>`), and each
    outcome returns the resolved on-disk path, so a wrong path/anchor pairing
    surfaces at the command rather than at the next build.

**Two ways to name a thing.** Separate axes mean two commands, and the cost of
two commands is naming the same file twice. Three rules keep that from being
friction, without collapsing the axes:

- **A group is named by id, by its navigator path, *or* by its resolved
  directory** (`--group Sources/App`), everywhere a group is selected:
  `fileref add --group`, `group add --parent`, `group move --to`,
  `group attach/detach --group`. Ids are unambiguous by construction, so an id
  that exists wins outright. A path is tried as a navigator path next, and as
  a directory only when it is no group's navigator path. The navigator path
  goes first because it is the one that tells apart organizational groups (a
  `name` with no `path`), which all resolve to their parent's directory, and
  it is the spelling a `project.xcproj` has (see the addressing amendment
  below). So `App` names the group shown as `App` even when another group's
  directory is also `App`. The navigator root's path is empty, and `""` and
  `/` both name it, in either format. Xcode keeps a group with neither a name
  nor a path as a navigator node with its children under it, and it adds an
  empty component to the paths below it. Xcode 27.2 spells them that way when
  it converts such a project: a product inside one at the root is
  `/Products/App.app`. `group list` prints those paths the same way
  (`/Products`, `App//Inner`), and a path is matched as typed before its
  slashes are trimmed. At the root such a group's own path is empty, like the
  root's, so there `""` is refused as naming two groups, and `/` still names
  the root. A `project.xcproj` uses these paths as its addresses: `fileref
  list` prints `/Products/App.app`, and a configuration's xcconfig named
  `Sources//Config/Base.xcconfig` resolves through the group with no name.
  Where two nodes share a navigator path, as the children of two such groups at
  the root do, Xcode 27.2 writes an `id` on the node and refers to it as
  `id:<id>`. A configuration's `file` and `anchor`, a target's `product` and
  the `products-group` all use that form. The xcconfig lookup reads it, and
  `id:<id>` also names the node as an argument. A group at the root with no
  name and no id cannot be the group a verb adds to or moves into. In either
  format, `group move` moves a group with neither a name nor a path only into
  a group whose directory is already its own. Anywhere else, keeping its
  children's files would give the group a path, and Xcode shows that path as
  its name, so the move is refused. A `project.pbxproj` group listed in two
  places has two paths, and either one selects it. Xcode 27.2 refuses to open
  such a project. Xcode 27.0 opens it with a warning and keeps the listing in
  the group it reads last, reading a group's children before the group itself:
  a listing in an ancestor wins, and of two sibling groups the later one wins.
  The same rule holds for a file listed twice, and a build compiles the file
  under the kept listing. `group list` shows the path of that listing, and the
  row's parent and directory, the files a target builds, the local packages
  the tree holds and an xcconfig's location all resolve the node under it too.
  A path that matches no group is an error, and so is one that matches two
  groups by the same spelling. That refusal lists each candidate's id with its
  navigator path and directory.
  `group list` prints all three on each row (`navigatorPath` in JSON, empty
  for the navigator root and null for a group nothing lists), so the miss
  error can point there. The human listing shows the root's navigator path as
  `/ (navigator root)`: `/` is the spelling that always selects it. This
  removes the `group list` lookup that otherwise preceded every add; it is a
  rule that errors, not a guess that picks.
- **`fileref add` is batched**, like every other mutating verb here.
  `--type`/`--source-tree`/`--group` apply to the whole batch, which is the
  case that actually recurs (a directory of new sources); files that disagree
  are separate calls. A bad path refuses the batch rather than half-applying
  it.
- **Membership is addressable by file-reference id** (`--fileref <ID>`), which
  is the `address` `fileref add` returns (a pbxproj addresses a node by its
  object id). This is the spelling that *composes* — the id
  flows from one command to the next and nothing is spelled twice, the way
  `git hash-object` feeds `git update-index`. It is also the only way to name
  a reference no group lists (no navigator path exists) or to disambiguate a
  path two references share, which is why the ambiguity error points at it.
  Paths remain the spelling for files that already exist.

The pair stays two commands, because a reference without membership is a real
state (`Info.plist` and `*.entitlements` are referenced by build settings and
compiled by nothing) and one verb cannot express "yes it exists, no it isn't
built". What these rules remove is the *bookkeeping* between the two, not the
distinction.

**Conversion as a recipe.** With these primitives, classic → sync-folder
conversion is a script the caller owns, one decision per line:

```
sweetpad pbxproj membership list --target App -o json   # the explicit truth
sweetpad pbxproj membership remove App/… --target App   # dismantle the list (batched)
sweetpad pbxproj folder add App --target App            # attach the folder
sweetpad pbxproj membership exclude App/Old.swift --target App  # strays stay out
sweetpad pbxproj settings show --target App             # verify the stored layer
```

Intermediate states are inconsistent (after `remove`, before `folder add`,
the file builds nowhere) — fine for scripts, nothing builds mid-sequence.
Constructs that don't map to sync folders (localization variant groups,
Core Data version groups) are simply visible in `list` and left classic —
a converter would have had to refuse; a script just doesn't touch them.

**Generated-project guard.** A `.xcodeproj` produced by XcodeGen or Tuist is
an *output*: any pbxproj edit is silently clobbered by the next
`xcodegen`/`tuist generate` (verified live — a probe setting vanished on
regenerate). So every pbxproj-mutating verb — `pbxproj settings set/unset`,
`folder add/remove`, `membership remove/exclude/include`, and `dependency
add/remove/update` when they edit a project — hard-errors when a generator
is detected, naming the spec to edit instead and the regenerate command that
would eat the change. `--force` says the ephemeral edit is deliberate
(what CI harnesses do); reads (`list`/`show`) are never guarded. Detection:
a `generator = "xcodegen" | "tuist" | <tool>` declaration in `sweetpad.toml`
wins, else a spec file next to the `.xcodeproj`
(`project.yml`/`project.yaml`/`project.json` → XcodeGen, `Project.swift` →
Tuist). Erroring (not warning) is deliberate: these commands are run by
scripts and agents that read exit codes, not stderr prose — a warning above
a success is exactly how the footgun fired in the first place.

The same detection drives a **staleness warning** on the resolution path: when
the spec is newer than the project's `project.pbxproj`, every command that
resolves a container says so once, naming the regenerate command. A file added
to the spec is invisible to the build until the project is regenerated, and the
build then fails with an ordinary `cannot find 'X' in scope` — a compile error
naming a symbol when the real cause is a stale project, which is the most
expensive kind of wrong answer to hand an agent. `project.pbxproj` is the
comparison target rather than the `.xcodeproj` directory because Xcode writes
`xcuserdata` and workspace state inside the bundle constantly, and any of that
would otherwise read as "freshly generated". This one warns rather than errors —
the opposite of the mutation guard above, and for the reason that distinguishes
them: the guard sits over a command that *succeeds* while doing the wrong thing,
where stderr prose gets missed, while staleness rides alongside a build that is
already failing and a caller already reading. Erroring would also be wrong on
its face, since a spec edited in a way that changes no file (a comment, a
setting) still builds correctly.

**Decision: the guard and the staleness warning are the whole generator story
(for now).** The CLI neither *regenerates* a project from an XcodeGen/Tuist spec
(no `project generate` passthrough — run `xcodegen`/`tuist generate` yourself)
nor *edits* those spec files on the user's behalf (no writing a
`settings set` through into `project.yml`). Detecting the spec and refusing
to fight it is the full extent of generator awareness. Rationale: the CLI's
own primitives (§3a scaffolding, §9f/§9g plumbing) make the `.xcodeproj`
itself a perfectly good source of truth, so the forward-looking answer to
"my project is generated" is migrating off the generator (the §9g recipe),
not deepening the CLI's entanglement with third-party spec formats and
their release cycles. Can be revisited if real demand shows up.

*Deliberately not built:* `project convert` (the recipe above, owned by the
caller), disk expansion in `membership list` (`ls` exists), and — per the
decision above — `project generate` / spec-file editing for XcodeGen/Tuist
projects.

**Amendment: a classic `membership add` ships, because the objection was to a
*smart* one.** The case against it was that creating file references and build
files by hand means guessing a file type, an anchor, a group, and a build
phase — inference Xcode does better. The plumbing verbs guess none of those:
`--type`, `--source-tree`, `--group`, and `--phase` are all stated by the
caller, and a path with no reference is an error naming `fileref add` rather
than an invented reference. What was declined was the inference, not the
capability, and refusing to infer removes the objection entirely. `folder add`
remains the forward-looking answer for a project that can adopt synchronized
folders; `fileref`/`group`/`membership add` are for the ones that cannot.

**Amendment: `settings` reads and writes `project.xcproj` too.** Xcode 27.2
writes a JSON project document in place of `project.pbxproj`, and the
namespace keeps its name and its spelling across both — a caller does not
learn which one the bundle holds. The grammar is unchanged because the
request is: a key, a scope, a set of configurations. Only the storage differs,
and the CLI states the picture it wants rather than the bytes:
`--configuration Debug` on a project whose key is stored once for everything
splits that key per configuration, and giving every configuration the same
value collapses it back, which is how Xcode writes the format.

`folder` and `membership` cross too, with the same grammar and two honest
differences in what they leave behind. `membership add` has nothing to create
first on a `project.xcproj` — the navigator node *is* the file — so it invents
nothing either: a path the tree does not hold is an error, and `--fileref`,
which names a pbxproj object, is refused. And `membership remove` there takes
the membership only, leaving the file listed; `folder remove` likewise leaves
the folder listed, building for nothing. A pbxproj deletes both, because there
a reference and a group exist to be pointed at, while in the JSON document
they are the navigator entry itself — which is exactly what Xcode writes for a
file or folder added for reference only.

**Amendment: `fileref` and `group` cross too, and a node is named by its
navigator path.** This was the one decision the two formats could not be given
the same answer to. A pbxproj keeps every node in a flat `objects` dict under a
24-hex id, and a group's `children` is a list of references to those ids, so an
id names a node wherever it is listed. The JSON document has no such dict: a
node is its own entry in a nested array, and the only ids in the corpus sit on
targets and on the products those targets point at. So a node is addressed by
its **navigator path** — `Sources/App/ContentView.swift`, the display names
from the root — which is the spelling `membership` and `folder` already take,
the one the document itself uses for a target's `product`, and the one a person
would type. Ids are not invented to keep the old argument shape: `fileref list`
and `group list` print the address each format wants back, and every verb takes
it. Every mutation's JSON names the node it touched as `address` too, `group
attach`/`detach` included, whose `group` is the address of the group whose
children changed rather than the spelling `--group` was given in. The `data`
of `group detach 85BB78F9ECC9184F5BA8114B --group Sources/App --json`:

```
{"action": "detach", "address": "85BB78F9ECC9184F5BA8114B",
 "group": "71376D09ABE451C1E73CAAE7", "changed": true}
```

`group add` also returns the group's `navigatorPath`, and its human line
starts the way the group's `group list` row does: the address, the navigator
path, then the directory. An organizational group resolves to its parent's
directory, so the directory alone would not say which group was made.

In either format, a `group add` that finds a sibling group showing the same
name returns that group instead of adding a second one. Only a group counts.
In a `project.pbxproj` a file of that name is no obstacle: Xcode lists the two
side by side, and each keeps its own id. A `project.xcproj` names both by one
navigator path, so there the add is refused with a message naming the node it
found. `fileref add` beside a group with the file's name is refused the same
way.

The navigator path is not the on-disk path. A node stored as
`<PROJECT>/Sources/Deep.swift` but listed at the root appears as `Deep.swift`,
so the listings carry both and `--under` still filters on the disk one.

**What cannot cross says so.** The rule is that a flag with no meaning in the
format in front of it is an error naming what to use instead — never a silent
no-op, and never quietly redefined into the nearest thing:

- **`group attach`/`detach`** exist only because a pbxproj group lists
  references, so an object can exist with no group listing it: `detach`
  unlists one, and `attach` lists one that nothing lists. An object another
  group lists already is refused, naming `group move`. Xcode 27.2 will not
  open a project that lists a node in two groups ("a member of more than one
  group"), and 27.0 keeps only one of the listings. In a tree a node is in
  exactly one place and the operation is a move. On a `project.xcproj` they
  are refused, naming **`group move <node> [--to G]`**, which is new and works
  on both formats. A move takes a node out of every group listing it, so a
  malformed double listing comes out as one.
- **`group remove --orphan-children`** has nothing to orphan on a
  `project.xcproj`: the children are nested inside the group rather than listed
  by it, so deleting it would delete them. Refused, naming `group move` to
  empty the group first. The guard itself is unchanged — a group with children
  is never deleted out from under them.
- **`--source-tree`** takes either vocabulary: `SOURCE_ROOT`,
  `BUILT_PRODUCTS_DIR`, `SDKROOT` and `DEVELOPER_DIR` map onto `<PROJECT>`,
  `<PRODUCTS>`, `<SDK>` and `<DEVELOPER>`, `<group>` and `<absolute>` stay
  themselves, and an anchor with no spelling in the format is an error listing
  the ones there are.

`fileref remove --dangling` does carry, with the consequence stated per format:
a pbxproj is left with build files pointing at nothing, while here the
memberships live on the node and go with it. The guard is the same either way —
a delete that would drop membership asks first. `--dangling` covers membership
only: a file the rest of the document names is refused under it too.

**`--parent` and `--to` default to the navigator root**, which is the
`mainGroup` in one format and the top of `files` in the other. That relaxes
`group add`'s previously required `--parent` rather than adding a second
spelling for the same place. A `project.pbxproj` group also answers to its
navigator path now, alongside the id and the resolved directory it already
took, so the one spelling selects a group in either format — and it is the only
spelling that separates two organizational groups.

**`move` keeps the file, not the spelling.** A `<group>`-relative path names a
different file under a different group, so a move rewrites it: the new group's
directory comes off the front when it prefixes the resolved path, and the node
is anchored at the project root when it does not. Each outcome reports the
resolved path, so the preservation is checkable rather than promised, and a
move and its reverse leave the document byte for byte as it was. A named group
moved into a group whose directory is its own needs no path there and loses
the one it has, so an organizational group moved out and back gets its
original spelling back. A path the first move had to anchor at the project
stays anchored, as any anchored path does.

In a `project.xcproj` a move also changes the node's address, and the document
names nodes by address: a configuration's xcconfig `file` or its `anchor`, a
target's `product`, and the `products-group`, which Xcode reads as `Products`
when the document leaves it out. Moving `Config` under `Sources` left
`"file": "Config/Base.xcconfig"` naming nothing, and Xcode 27.0 and 27.2 then
refused the project with "Invalid reference". A move rewrites every reference
into the moved subtree to the new address, and writes `products-group` when
the products group leaves the root, dropping it again when it comes back. A
reference written as `id:` needs nothing. A move onto a navigator path another
node already has is refused, as an add there is: the two would share one
address, and a reference to either would then name both.

## 9h. v8 — `app screenshot` for native macOS apps

`simulator screenshot` covers simulators; nothing covered a **running macOS
app**, so an agent driving the headless loop (`app run --mac --no-logs`) had
no way to visually verify the UI without leaving the CLI. `app screenshot`
closes that loop, and `app stop` learns macOS so the whole cycle —
launch → capture → stop — stays inside sweetpad:

```
sweetpad app screenshot [--output-file PATH] [--window N] [--pid N] [--clipboard]
sweetpad app stop                      # now also terminates a macOS app
```

**Target resolution** follows the `logs`/`stop` ladder:

1. `--pid N` captures that process's window directly — no project context,
   no bundle resolution (for windows sweetpad didn't launch).
2. Otherwise the **last-launched app** recorded for this project (unless
   explicit targeting flags opt out): a `macos` record captures the app
   window; a `simulator` record delegates to the `simctl io screenshot`
   path (same capture `simulator screenshot` does — so one verb serves
   the agent loop on either destination); a `device` record errors
   (devicectl has no capture).
3. Otherwise the resolved build target: `platform=macOS` destinations
   resolve the built `.app` via the in-process build-settings resolver (no
   build, no xcodebuild spawn), simulators delegate, devices error.

**Window discovery** is the CGWindowList path: the app's pids come from
matching `ps` command paths against the bundle's executable
(`…/Contents/MacOS/<name>` — full-path matching, so same-named binaries
elsewhere never collide), and `CGWindowListCopyWindowInfo` (on-screen,
front-to-back) filtered to those pids at layer 0 with nonzero alpha yields
the capturable windows. The default is the frontmost; `--window N` picks
the Nth (1-based, front-to-back), and an out-of-range index errors listing
what's there. A freshly-`open`ed app gets a short grace poll (~5s) for its
first window instead of a racy instant failure; a pid with no on-screen
window after that errors (minimized windows are off-screen by definition).

**Capture** is `screencapture -o -x -l<windowid>` — the window is captured
wherever it is on screen (no focus steal, no shadow, no sound). Preflight is
`CGPreflightScreenCaptureAccess()`: without the Screen Recording permission
`screencapture` silently produces wallpaper instead of failing, so the
missing permission is a hard, actionable error naming System Settings →
Privacy & Security → Screen Recording (and, on an interactive terminal
only, `CGRequestScreenCaptureAccess()` triggers the one-time OS prompt; a
headless run never pops UI). Window *enumeration* needs no permission —
only capture does.

`--output-file PATH` overrides the destination (default
`./sweetpad-shots/<app>-<epoch>.png`, the `simulator screenshot`
convention); `--clipboard` additionally copies the PNG to the pasteboard.
`--json` emits `{path, pid, windowId, bundleId, windows}`. The interactive
session's `s` key captures macOS targets through the same path (it was
simulator-only).

`app stop` on a macOS target terminates by pid — the recorded last launch's
executable path (fast path, no resolution), else the resolved bundle's —
with SIGTERM, and reports `{action: "terminated", pid}` (the `udid` field
is null for mac).

`app launch --mac` is its symmetric counterpart: it starts an already-built
macOS app and returns, leaving it running. The process is deliberately not
ours — `setsid` gives it its own session so a Ctrl-C in the launching
terminal can't reach it, and stdout/stderr are redirected to
`<state>/logs/<bundle-id>.log` rather than a pipe. The pipe is the reason
the interactive session's `d` (detach) key carries a caveat: a detached
child whose console pipes died with the CLI is killed by its next `print`.
A launch that never had pipes has no such failure mode. Spawning the
executable directly (rather than `open`) is also what lets `--env` reach the
process.

`app run --mac --detach` is the build-first form of the same thing: build,
launch detached, return. On a simulator or device the app already outlives
the CLI, so `--detach` there is `--no-logs`. It is rejected with `--hot`,
which has to stay attached to recompile and inject.

Every launch a run plan drives, on a simulator, a device or the Mac, starts
from what the scheme's Run action launches the app with, the way Xcode
applies it (`Scheme::launch_settings` in sweetpad-lib). The enabled argument
and environment rows are used, with `$(VAR)` expanded against the resolved
build settings of the app the plan launches (`RunPlan::located`, the
locator's pick from the same Run action), or of the target the Run action's
`MacroExpansion` names. Each argument row is then split with shell-style
quoting, and App Language and App Region add `-AppleLanguages`,
`-AppleTextDirection` and `-AppleLocale`. The rules come from `xcodebuild
test` on Xcode 27.0, whose Test action launches with the Run action's rows.
Build settings are resolved only when a row refers to one. The `--arg`s
follow the scheme's arguments, and an `--env` replaces a scheme variable with
the same name. A Swift package's `swift run` takes only the typed ones. The
extension reads the same function through the addon (`schemeLaunchSettings`).
Ahead of the scheme's variables, every app launch carries `NSUnbufferedIO=YES`,
as Xcode's launches do (`xcodebuild test` shows it in the test process). A
macOS app's stdout is a pipe or a file on every launch, and Foundation
block-buffers `print` there, so without it a foreground `run --mac`, a
session and a detached launch show nothing until 4 KB have piled up.

Every macOS launch sweetpad drives (`run --mac` and its session's
relaunches, `--hot`, `app launch --mac`, `app debug --mac`, `app diagnose
--mac`) passes `-ApplePersistenceIgnoreState YES` ahead of the other arguments.
Without it, a relaunch after a crash can open AppKit's "reopen windows?"
alert, a modal loop on the main thread that nobody at the terminal sees: the
app is up and does nothing, and `app sample` reads it as idle. The pair is
settled once on the run plan, so every launch that plan drives carries it.
`--restore-state` leaves it out, as does an `--arg` or a scheme launch
argument that already sets the key; simulators and devices never get it.

AppKit acknowledges the key on stderr at every launch
(`ApplePersistenceIgnoreState: Existing state will not be touched. New state
will be written to …`). When the plan added the pair itself
(`RunPlan::added_ignore_persistence`), that line describes sweetpad's launch
rather than the app, so the run session, plain and `--hot`, leaves it out of
the app's console. A key the caller or the scheme set keeps its line. A
detached launch's stdio goes straight to its captured file, so the line is
written there whoever added the key. The file's run header says who did:
the caller's arguments follow `· args:`, and the pair sweetpad added gets a
clause of its own, `· sweetpad added: -ApplePersistenceIgnoreState YES`.
`app logs` reads the header first and leaves the line out when that clause
is there.

The header is sweetpad's, not the app's. `app logs` shows it as a dim
`── sweetpad launched … ──` separator, leaves it out of the `--json` stream,
and never matches an `--until` against it (it names the app, so an
`--until` for the app's name would match before the app printed anything).
The hot session's injection client logs a line through NSLog when the
session's server closes at quit or detach (`[<InjectionNext: 0x…>
readInt:0x… length:4] error: 0 Operation not supported`: zero bytes read, so
the connection ended between commands). That line is sweetpad's doing too,
and the hot session leaves it out of the app's console.

A macOS app logs to two disjoint places — plain stdout/stderr (`print`,
`NSLog`'s stderr leg, C `printf`) and the unified log (`os_log`/`Logger`) —
so `app logs` on macOS follows *both*: the captured
`<state>/logs/<bundle-id>.log` and `log stream`, interleaved by arrival.
`--source oslog|stdout|both` narrows it (default `both`); `stdout` is
macOS-only, since only a detached launch captures a file. The captured file
is truncated per launch and stamped with a one-line run header, so reading it
from the top shows only the current run — never `print` output a previous run
left behind (the append-mode file used to carry stale lines that read as the
live run). Simulator and device logs stay `os_log`-only. In `--json`/`-o
ndjson`, `os_log` entries pass through as the raw `log stream` objects and
captured lines are tagged `{"source":"stdout",…}`, so the two are
distinguishable on one stream. `--last <dur>` (e.g. `2m`, `90s`, `1h`) swaps
follow for a one-shot backfill — `log show --last` for the `os_log` history
`log stream` can't replay (without the count `log show` ends on, which is no
entry), plus the captured file — for an app that has gone
quiet or already exited; it is refused for a physical device, whose syslog has
no history query. `app status` prints the `detached log` path when the last
launch was macOS, so the file is discoverable without catching the one launch
line that first named it.

`install`/`uninstall` stay simulator/device verbs: a macOS app is built in
place, so there is nothing to install.

The CoreGraphics/CoreFoundation FFI is a small hand-rolled block private to
the `app` command (DOCS §3: no binding-crate dependency for four calls);
everything above it — `ps` parsing, window filtering/pick — is pure and
unit-tested.

*Deliberately not built:* a `--screenshot PATH` auto-capture flag on
`app run` (the agent loop composes explicit commands — `app run --mac
--no-logs`, then `app screenshot`, then `app stop` — and owns its own
timing, per §9g's explicit-primitives philosophy), and device screenshots
(devicectl exposes no capture; the error says so).

## 9i. v8 — `app ui` — reading and driving a macOS app's UI

§9h closed the *observe* half of the agent loop and `app open-url` covers
*stimulate* at arm's length, but "click this, assert that" had no spelling: a
PNG is not something a script can assert on, and a deep link only reaches
states the app chose to expose as URLs. `app ui` adds the missing half
through the Accessibility API, where one interface serves both — the element
tree is the assertion surface and the same elements take the actions:

```
sweetpad app ui tree  [--depth N] [--pid N]           # what the app exposes
sweetpad app ui click <--label TEXT|--role ROLE> [--nth N] [--pid N]
sweetpad app ui type  <TEXT> --label TEXT [--role ROLE] [--nth N] [--pid N]
```

A bare `app ui` runs `ui tree`, the one verb that only observes.

**Target resolution** is §9h's ladder exactly — `--pid`, then the recorded
last launch, then the resolved build target, never a build. It is
**macOS-only**, and the non-mac error says what does work there instead
(`app screenshot` + `app open-url`, or a UI test target through
`sweetpad test`): `simctl` has no tap or type verb at all, so the honest
answer for a simulator is XCUITest, which is a different model — write a
Swift test, build it, run it — not a command an agent issues between edits.
Where §9h picks the frontmost window when an app has several, `ui` *refuses*
a multi-process app: there is no "frontmost" element tree, so it names the
pids and asks for `--pid`.

That scope is a property of the API, not a gap left to fill. Accessibility
is host-side and addresses processes on the Mac by pid; a simulated app runs
inside the simulator's own OS and is not in that namespace. The tempting
workaround does not exist either, and it fails in the way most likely to be
mistaken for progress: `Simulator.app` is itself a macOS app, so
`app ui --pid <Simulator pid>` *succeeds* and prints a 368-element tree —
which is **entirely its own menu bar** (`File`, `Device`, `I/O`). There is
no `AXWindow` in it at all; the device window is not bridged, so nothing
about the simulated app's UI is reachable. Anyone reaching for `--pid` as
an escape hatch gets real-looking output and no way to act on the app, which
is why the destination-level refusal is a hard error rather than a
best-effort attempt.

**The tree** is `AXUIElementCreateApplication(pid)` walked through
`AXChildren`, each node carrying its role, label, identifier, enabled state
and the actions it advertises. The label is `AXTitle`, else
`AXDescription`, else a string-valued `AXValue`. An `AXIdentifier` is
preferred over the label when a developer assigned one — it survives copy
changes, so it is the thing to write in a script — but AppKit hands *every*
view an auto-generated identifier of the form `_NS:945`, an internal serial
number that changes between runs and would otherwise mask every real title
(`AXWindow "_NS:34"` for a window plainly called `uitest.txt`). Those are
dropped at read time and never reach the model.

**Matching** follows §9g's resolver rule — naming nothing, or two things, is
an error rather than a pick. `--label` matches identifier or label,
case-insensitively, with **exact matches tried before substring ones** so
`--label Save` prefers a "Save" button over "Save As…" instead of calling
the pair ambiguous. `--role` accepts `button` or `AXButton`. A genuine tie
lists its candidates and asks for `--nth` (1-based, front-to-back), and the
suggestion to narrow names the axis the caller hasn't already used. An empty
query is refused outright: it would match the application element and press
something arbitrary.

**Acting** is `AXUIElementPerformAction(…, "AXPress")` for `click` and
setting `AXValue` for `type`. Because a snapshot is pure data holding no
live element refs, `act` re-descends by index path and **re-checks the role
on arrival** — if the UI restructured between snapshot and act, that is a
clear "the UI changed under us" error rather than a press landing on
whatever now occupies the slot. `type` is a value assignment, not keystroke
synthesis; the help says so, because an app watching for individual key
events won't see any.

**Permission** is `AXIsProcessTrustedWithOptions`, mirroring §9h's
Screen Recording preflight: a missing Accessibility grant is a hard error
naming System Settings → Privacy & Security → Accessibility, and only an
interactive terminal triggers the one-time OS prompt. The grant attaches to
the *hosting* app (Terminal, iTerm, the editor), not to the sweetpad binary.

**Occlusion does not gate reads**, which is what makes this usable
unattended. §9h's capture path and the AppKit notes behind it fail when a
window is occluded or the display is asleep — the display pipeline is gated,
`cacheDisplay` reads empty backing stores, and lazy `NSTableView` row views
never materialize. The accessibility hierarchy is derived from the view
tree instead, and a tree walked while the app sat fully behind other windows
was byte-identical to the same walk with it frontmost. What an app
*exposes* is still its own choice: an unlabeled SwiftUI view is a bare
`AXGroup` with nothing to match on, and no amount of CLI can invent a label
the app never set.

The `ApplicationServices`/CoreFoundation FFI is the same shape as §9h's — a
small hand-rolled block, no binding crate, no Objective-C runtime, since
`AXUIElement` is plain C — with the tree model, matching and rendering above
it pure and unit-tested.

*Deliberately not built:* coordinate-based clicking via `CGEvent` (brittle,
and it can assert nothing — the tree is the point), keystroke synthesis,
element waiting/polling (`ui tree` is cheap; a caller that needs to wait owns
its own timing, per §9g), and any simulator path short of XCUITest.

## 9j. v8 — `app diagnose` and scriptable `app debug --batch`

`app debug` handed the terminal to lldb and waited at its prompt — no way to
pass commands, no batch mode, so it was unusable from an agent or CI. Chasing a
swallowed Objective-C exception (which imitates a hang, App Nap, and an executor
bug at once — see the wedge-hunt notes below) meant dropping to raw lldb by hand
and resolving the binary out of DerivedData yourself. The consumer of a debugger
on this platform is almost always an *agent* hunting a defect, rarely a human at
a prompt, so the surface leads with a structured preset and keeps the raw
passthrough as the escape hatch.

```
sweetpad app diagnose [--mac|--device] [--arg A] [--env K=V] [--timeout SECS]
                      [-- XCODEBUILD_ARGS]
sweetpad app debug --batch [--cmd LLDB_CMD]… [--on-crash LLDB_CMD]… [--timeout SECS]
                   [-- XCODEBUILD_ARGS]
```

Both build before they hand off to lldb, so both take the `--` tail (§3).

**`app diagnose`** is the agent-facing verb: build, launch under `lldb -b` with a
breakpoint on `objc_exception_throw`, run bounded by `--timeout`, and on the
first stop print a structured report — `pid`, `stopReason`, `signal`,
`exitStatus`, `exception { name, reason }`, `verdict`, `backtrace`,
`lldbStatus`, `chainComplete`, and the full lldb `transcript` — then kill the
app and quit. On a simulator the pid is the one
`simctl` launched; on macOS lldb owns the launch, so it is read from lldb's
`Process <pid> launched|stopped|exited` line, and a run that times out while
lldb's `run` still blocks (so no such line has printed) takes it from the
process the timeout kills. `-o json` is the point of the verb: the freeform lldb text
becomes fields an agent acts on, with the raw transcript alongside for whatever
parsing can't reach. Human mode prints a one-line verdict and the backtrace. lldb
recognizes an ObjC throw natively (`stop reason = hit Objective-C exception`);
`$arg1` at that breakpoint is the `NSException`, read ABI-neutrally so it works on
arm64 and x86_64 sims. The chain prints `script print('@@…@@')` sentinels between
sections and runs under `-Q` (no command echo), so a captured transcript splits
cleanly even though lldb interleaves prompts, app `os_log` lines, and its own
diagnostics.

**A crash's backtrace comes from the `-k` commands.** `lldb -b` runs its `-o`
commands only while the process stops normally. A breakpoint is a normal stop,
so an Objective-C throw carries the chain on through the exception's name and
reason, the backtrace, and the kill. A crash (a Mach exception, a signal, a
Swift runtime failure) ends the chain after `run` or `continue`, and lldb runs
the `-k` commands instead, which are the backtrace and the kill again. A clean
exit carries the chain on with no process to read, so the `po`s, the `bt` and
the kill each run inside a `script` guard that skips them unless the process is
stopped. Unguarded, the first `po` would fail and end the batch with lldb's
status 1. Transcripts
captured for each of these, on macOS and on the iPhone 17 simulator, are the
parser's test input (`fixtures/diagnose`). On a simulator lldb attaches to a
process launched suspended, and the attach stops it with `signal SIGSTOP`
before the `continue`. The parser skips that stop, which it would otherwise
read as the result of every run.

**A crash reads as its signal.** On Apple platforms lldb stops on the Mach
exception, before the kernel turns it into a signal, so a crash's stop reason
is `EXC_BAD_ACCESS (code=1, address=0x10)` rather than `signal SIGSEGV`
(captured on arm64 for a bad write, a write to read-only memory,
`__builtin_trap()` and an undefined instruction). `signal` carries the signal
XNU's `ux_exception` maps the exception to, and `verdict` says what happened
in words with the exception named after it: `crashed with SIGSEGV: a bad
memory access at 0x10 (EXC_BAD_ACCESS)`. The mapping covers the four
exceptions with one settled meaning: `EXC_BAD_ACCESS` is `SIGSEGV` for code 1
(`KERN_INVALID_ADDRESS`) or an Intel general protection fault and `SIGBUS`
for any other code, `EXC_BREAKPOINT` is `SIGTRAP` (a trap instruction),
`EXC_BAD_INSTRUCTION` is `SIGILL`, and `EXC_ARITHMETIC` is `SIGFPE`. Any other
exception (`EXC_GUARD`, `EXC_RESOURCE`, …) leaves `signal` null and the raw
stop reason as the verdict. A Swift runtime failure stops before its trap
instruction, as `Fatal error: <message>` where the runtime reports it
(`fatalError`, a failed `precondition`, an index out of range in a Debug
build) and as `Swift runtime failure: <message>` where an optimized build
traps in place. Both are `SIGTRAP`, the signal the app dies of without a
debugger, and read `crashed with SIGTRAP: Swift fatal error "<message>"`.
`stopReason` is always lldb's text as printed. The human line is `<bundle id>:
<verdict>`.

**`app debug --batch`** is the raw escape hatch: `--cmd` forwards to lldb's
`-o/--one-line` verbatim (sweetpad's own `-o` already selects the output format,
so the flag is `--cmd`, not `-o`), `--on-crash` to `-k`. You write your own
`run`/`continue` and `quit`; the output streams. It rejects `--json` like `app
run` does — a live lldb session has no coherent one-shot envelope; that is
exactly what `diagnose` is for. The two verbs are easy to mix up, and
`diagnose --batch` reads as plausible. clap rejects it as an unknown argument
(exit 2), and its stock tip for a verb with a `--` tail is to pass the flag
after `--`, which would hand it to xcodebuild. So for `--batch`, `--cmd` and
`--on-crash` on `app diagnose`, the tip names the verb that has them instead:
`'--batch' belongs to 'app debug': 'sweetpad app debug --batch --cmd
<LLDB_CMD>' runs your own lldb commands` (`hint_tail_flag`, which reworks that
stock tip for every unknown flag on a verb with a tail; see §2).

**The exit code reflects the launch, not the finding.** `lldb -b` returns `0`
whether the debuggee crashed, threw or exited, so neither verb derives success
from it. `diagnose`'s answer is the report (an agent
reads `stopped`/`stopReason`/`exception`); `--batch`'s answer is the streamed
transcript. The help says so on both.

**`diagnose` still reports how lldb itself ended.** lldb stops a `-b` chain at
the first command that fails and exits 1. A failed attach does this (a pid that
is gone, or one owned by another user), and the chain never reaches the stop it
was waiting for. Two fields say so without reading the `transcript`:
`lldbStatus` is lldb's exit code (null when the timeout killed it), and
`chainComplete` is whether the transcript holds the closing
sentinel, which the chain prints once the backtrace is dumped. A crash, a throw
and a clean exit all read `lldbStatus: 0` and `chainComplete: true`, on the
`-o` chain or the `-k` one. A timeout reads `null` and `false`. A non-zero
status with the chain incomplete and no stop parsed gets its own verdict,
`lldb stopped partway with status 1: attach failed: no such process`, with
lldb's first `error:` line after the colon, instead of `no stop observed`
(`DiagnoseReport::stopped_partway`).

**Timeout is mandatory, not optional.** `lldb -b`'s `run` blocks until the
process stops or exits, so an app that launches and stays up (the common GUI
case) would hang the session forever — the precise anti-pattern for an
unattended agent. `--timeout` (30s for `diagnose`, 300s for `--batch`, `0`
disables) spawns lldb, waits, and on expiry kills the *inferior first* (resolved
by executable path on macOS, by the launched pid on a simulator) then lldb, so a
timed-out `diagnose` reports `timedOut: true` and leaves nothing running. The
kill targets only what this run launched: a diagnose against a DerivedData build
never touches a copy the user opened from `/Applications`.

**Target coverage** mirrors interactive `app debug`. On macOS lldb owns the
launch (`run`, breakpoints armed before the process exists); on a simulator the
app is launched suspended via `simctl --wait-for-debugger` and lldb attaches to
the pid and `continue`s — the one asymmetry, hidden inside `diagnose` and left
verbatim in `--batch`. Physical devices and Swift packages are refused with a
pointer (`lldb -b … -- <binary>` for SPM), matching where the interactive verb
already draws the line.

*Deliberately not built:* parsed backtrace *frames* (SB/Python API — the
transcript plus a best-effort frame-line list is enough for v1), a device path
(needs `debugserver` plumbing, like the interactive verb), and any preset beyond
exception-catching. The stack snapshot for "wedged or merely idle?" inspects a
*running* pid rather than owning a launch, so it sits with `screenshot`/`ui`
under the observe verbs as `app sample` (§9r), not here.

### `app logs --exits`

`diagnose` catches a crash in a launch it owns. It cannot explain a death after
the fact, and the deaths that need explaining often leave no crash report. A UI
test failed with `Failed to application com.hyzyla.reflow is not running`;
DiagnosticReports was empty, and the answer was only in the simulator's unified
log, as launchd's exit line for the app: `exited with exit reason (namespace: 10
code: 0xfbfbfbfb) - OS_REASON_SPRINGBOARD | … explanation:Termination requested
by simulator host`. The host had killed it; nothing had crashed. Finding that
took `simctl spawn <udid> log show`, a hand-picked window, and a grep.

```
sweetpad app logs --exits [--last DUR]
```

It is a preset on `app logs`. The app resolves the way `app logs` resolves it
(the recorded last launch, else the build target), and `--last` sets the
window, 10 minutes by default. launchd writes one line when a job ends, in
three shapes, all captured on Xcode 27:

- `exited due to exit(3)`: a status.
- `exited due to SIGABRT | sent by App[pid]`: a signal and its sender
  (`exc handler[pid]` for a fault).
- `exited with exit reason (namespace: N code: 0x…) - OS_REASON_… | <…
  explanation:…>`: the system ended it and said why.

The job's label and pid are in the entry's `subsystem`, not its message, so the
query matches the bundle id there. The parser then checks the label exactly,
because a predicate `CONTAINS` for `com.app` also matches `com.app.widget`.

**A label only where the meaning is settled.** Every exit carries its raw
namespace, code, reason name and launchd's explanation. A plain-words label is
added only where the meaning is well established: a status (`exited normally`,
`exited with status 3`), a fault or abort signal (`crashed with SIGABRT`, plus
the crash report when the system wrote one, matched by bundle id and pid),
jetsam, `0x8badf00d`, `0xdead10cc`. Everything else gets no label rather than a
guess. For `0xfbfbfbfb`, launchd's own "Termination requested by simulator host"
says more than a label could.

**Simulator and macOS, not devices.** A simulator's log is read through `simctl
spawn`, a Mac app's from the host `log`. On macOS launchd records only the jobs
it started, meaning apps opened through LaunchServices. `app run --mac` and `app
launch --mac` spawn the executable directly, so their processes have no job and
no exit line; `runningboardd` notes only `termination reported by proc_exit`,
with no status. A physical device is refused.

**Two more accounts fill that gap.** Each exit carries a `source`:

- `crashReport`: every `.ips` in `~/Library/Logs/DiagnosticReports` for the
  bundle captured inside the window, whatever started the app. The body names
  the pid, the signal and who sent it (`termination.byProc`/`byPid`), and the
  exception when it says more than `EXC_CRASH` (`EXC_BAD_ACCESS
  KERN_INVALID_ADDRESS at 0x10`, a Swift trap's `EXC_BREAKPOINT`). The
  exception is its own field (`exception` in JSON), not launchd's
  `explanation`, and the human line gives it after a `;`: `crashed with
  SIGSEGV (sent by exc handler[60122]; EXC_BAD_ACCESS …)`. Run together, the
  sender's name and the exception read as one phrase. Simulator
  reports land in the same directory, so the process path picks the device:
  a simulator app runs out of `CoreSimulator/Devices/<udid>/`.
- `sweetpad`: a macOS app sweetpad spawns and waits on (`run --mac` attached,
  its session, `--hot`) has sweetpad as its parent, so its exit status or
  signal is recorded with the pid and start and end times, including the
  SIGKILL sweetpad itself sends on rebuild or quit (`sent by sweetpad`). The
  records live in `<state>/sweetpad/exits.toml`, 20 per project for at most 7
  days. They are not in `state.toml`: a session writes one mid-run, and
  `state.toml` is rewritten whole from each process's startup copy, so the
  next save would drop it.

The same termination seen twice (one pid, within 15s) is listed once: launchd's
line over sweetpad's record over the report's reading, with the report's path,
sender and exception carried onto the entry kept. `app debug` and `app
diagnose` leave no sweetpad record, since lldb is the app's parent there, and a
detached launch (`app launch --mac`, `run --detach`) has no parent left to see
it end. Only a clean exit or an outside kill of a detached launch goes
unrecorded, and the empty macOS result says so. A test failure's
`terminationReason` reads crash reports the same way; a test's app is never
sweetpad's child, so it has no sweetpad record.

**Bounded.** One `log show` over the window, killed after 30s. Over an hour of
history it took 2 to 3s.

*Deliberately not built:* a device path, and following exits live. `app logs`
already follows, and an exit is a question asked after the fact.

## 9k. v8 — bounded follows, listener recovery, honest compile counts

Three papercuts from one family: the CLI already held the answer and had no way
to say it or act on it. Each showed up in an agent session as lost time rather
than as an error, which is why none of them had been filed as a bug.

### `app logs --until` / `--timeout`

`app logs` followed until killed — the human half of the verb. The agent-shaped
operation is "start it, poke it, tell me what it said", and expressing that
against a follow-forever stream costs a background job, two guessed `sleep`s and
a `kill` per cycle, where the guess either wastes time or truncates the answer.

```
sweetpad app logs --until TEXT [--timeout DUR]
sweetpad app logs --timeout DUR
```

`--until` ends the follow on the first line containing TEXT and exits 0; missing
it exits non-zero, so the caller branches on the exit code instead of parsing.
`--timeout` alone bounds a follow and exits 0 — a tail with a deadline asks no
question, so reaching the end is not a failure. Together, the deadline is the
answer's deadline.

**Substring, not a regex.** The stop condition is nearly always a literal marker
line, and `regex` would add four crates to a nine-entry dependency list to serve
it. The help says "plain substring" outright rather than leaving the caller to
discover which metacharacters quietly do nothing.

**The match runs against the rendered line**, not the raw ndjson, so what you
match is what you see. On macOS both sources feed it, since a `--detach`ed app's
marker may arrive on captured stdout rather than through `os_log`; either source
matching ends the follow, by SIGTERM-ing the `log stream` child whose EOF
unblocks the reader, so every exit leaves through one path. A deadline ends the
same child, and stands down when the stream finishes first so it can never signal
a pid it no longer owns. `--last` conflicts with both flags at parse time —
history is already finite.

### `hot status` / `hot reset`

A `--hot` session that dies without unwinding leaves `:8887` bound, and every
later run fails to bind. The error already named the holding pid — `port_holder()`
shelled out to `lsof` for exactly that — but nothing could act on it, so recovery
lived outside the CLI (`lsof`, then `kill`) and the standing workaround became
"always pass `--no-hot`": silently giving up the feature to avoid the papercut.

```
sweetpad hot status
sweetpad hot reset [--force]
```

**`reset` is guarded by ownership rather than by prompting.** The port can
legitimately belong to InjectionNext.app or an unrelated listener, so the holder's
executable is resolved through `ps -o comm=` and only a `sweetpad` process is
ended by default; anything else is named in the refusal and needs `--force`.
**The result reports the port, not the signal** — it polls for the release and
says so when a holder outlives its SIGTERM, because "a signal was sent" is not
the question being asked.

### One `Compiling` line per file

Xcode emits two shapes for the same Swift work: a batch header (`SwiftCompile …
Compiling\ A.swift,\ B.swift <paths>`) and then a line per file. Rendering both
announced most files twice, which reads as duplicated work — and a header took
whichever member `source_name` happened to match first, so a group of twelve was
labelled with one arbitrary file.

A one-file header now defers to the per-file line behind it, and a wider header
renders its count (`Compiling 12 files`). Entries are counted by their separators,
so an escaped space inside a filename stays one entry. A suppressed header becomes
`Other`, which keeps it visible under `-v` and out of ndjson, where it was never a
distinct unit of work.

*Deliberately not built:* `--until` as a regex; a `--signal` sibling on `app run`
for apps that expose Darwin-notification debug hooks (a narrower need than the
stop condition); and dedupe of a wider batch header against the per-file lines
behind it, which would need lookahead over a stream to save a line that is honest
as it stands.

## 9l. v8 — reaching the evidence inside the result bundle

`test` reports a verdict and retains the `.xcresult` that explains it, then only
ever reads two things back out of that bundle: the summary and, for `--failed`,
the failing selectors. Everything else a run recorded — what a test attached, and
what it printed — stays sealed inside a format nothing else can open.

The shape repeats per artifact: the failure message says *what* failed, the
bundle says *why*, and only the first was reachable. Both verbs below read the
retained bundle, so they answer after the fact without re-running anything.

`test run`'s flags are resource-global, so they also parse after `test
attachments` and `test output`. Both verbs redeclare them hidden under the same
ids, as `build diagnostics` does, so their help lists only what they take, and
a run flag given anyway is refused by name instead of dropped, as a usage error
(exit 2, §4). `--only-testing`
and `--result-bundle` are redeclared visible, with help that says what they
pick here: the tests and the bundle to read. The targeting flags past the
container are run flags here too: `--scheme`, `--configuration`, `--sdk`,
and the destination flags `--mac`, `--on` and `--destination`. A project
keeps one retained bundle, whatever the run's scheme, configuration or
destination was, so none of them picks anything. `--workspace` and
`--project` stay, since they pick the project. All but `--mac` are refused
only when typed, since a `SWEETPAD_*` variable can set each of them for every
command. `build diagnostics` refuses these flags through the shared
`HiddenTargetArgs`, since a project also keeps one build record, whatever the
build was for.

### `test attachments`

For a UI test the two halves of a diagnosis live in different places: the failure
message says the query failed, and the screenshot says the sheet was under the
keyboard. The same reach reads as verification on a green run, where the
screenshot is the only evidence the UI actually looks right.

```
sweetpad test attachments [--output-dir DIR] [--only-failures]
                          [--only-testing ID]…
```

**The manifest join is the feature.** `xcresulttool export attachments` writes
files named by UUID, so the export alone is unusable; the names live in a sibling
`manifest.json`, keyed per test. The export is staged, joined, and renamed back
to what each test called the file — `01-document-loaded.png`, not
`26174667-246E-4154-859F-1F7A0CE79766.png`. XCTest's `_<run>_<UUID>` uniquifier
is stripped, and a name it collides with takes a counter rather than overwriting
the earlier file, because attachment names are not unique — that suffix exists
for exactly that reason.

**Staging is not an implementation detail.** A second export into a populated
directory writes `name (1).png` duplicates instead of replacing, so the export
always gets an empty directory of its own and the files are moved out of it. The
default destination is a per-project slot beside the retained bundle, emptied
first so a stale file can never be read as part of this run; a directory the
caller names with `--output-dir` is theirs, and is added to rather than emptied.

**Grouped by test, ordered as the test recorded them.** Attachments land under
one directory per test with the run's own timestamps deciding order within it —
the manifest's order is neither. Note that execution order is XCTest's
alphabetical-by-method, which a suite numbering its screenshots as a narrative
will not match; grouping is what keeps that legible, and a flat chronological
dump is what makes it noise.

**An empty export says which emptiness it is.** A run that attached nothing and a
run whose attachments were discarded look identical on disk, and the discard is
the *default* — `XCTAttachment.lifetime` is `.deleteOnSuccess` unless a test says
`.keepAlways`. That is not guessable from an empty directory, so it is stated,
in the payload as well as on stderr, since a `note` is muted under `-o json`.

**`--only-failures` keeps the failed tests' files.** xcresulttool has a flag of
the same name, which keeps only the files it marks `isAssociatedWithFailure`.
For iOS simulator UI tests, Xcode 27 marks none of them, the crash log and
screen recording of a test whose app crashed included, so the flag exported
nothing from a run with five failures. On macOS it does mark the crash log of a
test whose host crashed, but not what a failed test attached itself. Either
way the mark leaves out files a failed test recorded, so the export is always
whole, and the filter keeps every file
of a test the test tree lists as failed, plus any file that is marked. The tree
is already read to put each test under its target, so the filter costs no extra
call. The same tree words an empty result: the run had no failing tests, or its
failing tests attached nothing, or the tree could not be read and only the mark
was there to go by. The listing's `(failure)` marker, `failure` in JSON, takes
the same two sources: every file of a failed test, and any file that is marked.

**The age of the evidence travels with it.** The export reads the *last* run, so
running it after an edit but before a re-run hands back screenshots of the old
build — the failure mode §9g's stale-project guard exists for, and worse here,
because a screenshot looks authoritative whatever its age. The report carries
when the run was recorded.

*Deliberately not built:* automatic export on a failing `test` run. It would
remove the round-trip an agent pays to discover the verb, and `--only-failures`
makes it nearly free, but it writes files nobody asked for and changes `test`'s
payload; the verb is the honest surface for an explicit request.

### `test output`

The same seam, one artifact over. A test that *measures* rather than asserts —
a benchmark printing `BOOK REFLOW: 300 source pages -> 486 pages in 26.9s`, a
fixture reporting its size — makes its point in a `print`, and pass/fail says
nothing about it. That output reaches the terminal only under `-v`, which also
prints the entire xcodebuild transcript, so it has to be grepped out of the
noise; nothing in `test --help` suggests `-v` is where stdout went.

```
sweetpad test output [--only-testing ID]… [--full]
```

**It is not in the test tree.** `get test-results activities` returns an empty
tree for a test whose only output is a `print` — activities record `XCTContext`
steps, not the process's stdout. The stream lives in the *diagnostics* export,
one file per test process, bracketed by XCTest's own `Test Case '-[Module.Class
method]' started.` / `passed|failed (Ns)` markers. Slicing on those markers is
what turns per-process output back into per-test output.

**Read only the test processes' streams.** The same export also holds the
app-under-test's `os_log` firehose, named `StandardOutputAndStandardError-<bundle
id>.txt` — 27 MB against the test streams' 113 KB in the run this was built
against. The unsuffixed name is the discriminator.

**Sized from what tests actually write.** Measured over one real suite: 36 unit
tests wrote 362 bytes between them and only 3 wrote anything at all, while 9 UI
tests wrote 66 KB — all of it XCUITest's automation trace, the largest single
test 20.6 KB. So each test's output is cut to its last 4 KB, which passes every
unit test through whole and holds a UI trace to its end, where a failure is.
`--full` lifts the cap, and the streams themselves are kept beside the bundle so
the untruncated text stays readable either way.

**Attribution holds while each test process runs one test at a time**, since
it brackets output between one test's markers. A parallel run keeps to that:
each worker is a process of its own and writes a stream of its own. On macOS,
a scheme marked `parallelizable` ran its three test classes on three workers,
and each stream held one class's tests in turn.

**Tests are listed in the order they started.** A stream holds one process's
tests in the order it ran them, and a parallel run has a stream per worker, so
the streams are merged on when each test started. A suite banner (`Test Suite
'AlphaTests' started at 2026-09-27 15:07:26.978.`) dates its suite. A test in
it starts once the tests before it have run for the time their end markers
give (`passed (0.405 seconds)`). The estimate leaves out the time between
tests, such as a class's `setUp`, so it can run a little early. Rounding in the
durations can also carry it past the next banner, so a test is never dated
earlier than the one before it in its stream, and each stream keeps its own
order. The banners give no zone, so they are only compared with each other.
The streams are the only source: the test tree has durations but no start
times, and a unit test's activity log records no start.

**A line belongs to the innermost test running.** XCTest runs a test on the
thread that asks for it. A test that runs another case inside itself
(`InnerTests(selector:).run()`) therefore writes the inner case's markers
between its own, and its own lines resume once the inner case ends. So the
markers are read as a stack. In a run on Xcode 27, the outer test's `outer
after` line, written after the inner case ended, is listed under the outer
test. XCTest's two warnings about the nesting land under whichever test was
running when XCTest wrote them. A test whose end never comes crashed its
process, and XCTest restarts the process. That test's lines run up to the next
start, so the crash's `Fatal error` and XCTest's `Restarting after unexpected
exit` line stay with the test that crashed.

**The warning keys on markers that cross.** Two cases running at once in one
process end out of order: a test ends while a case that started after it is
still open, or ends with nothing open. The lines in between can't be sorted
out, so the warning names the kept streams, which hold the output as it was
written. JSON carries this as `overlapped`, beside `serial`, which comes from
`scheduling.log`'s `Parallelization disabled` line. It keys on the markers
rather than the run's mode, since a parallel run's workers each write their
own stream.

**Output written outside a test case is listed only when nothing was
attributed.** Between tests the stream carries XCTest's own bookkeeping and the
app's `os_log` chatter, which buries the handful of lines a test meant to write.
But a framework whose markers this parser does not recognise — Swift Testing —
attributes *nothing*, and there the same bucket is the only account of what ran.
Dropping it unconditionally would lose the run; showing it unconditionally
would bury the answer. When some test was attributed, the note counts the
lines written while a stream's tests ran, after its first case started and
before its last one ended, and names the kept streams. Those are lines between
two tests, such as a class's teardown. A plain run gets no note: a hosted app's
launch logging comes before the first test, and a stream with no case in it,
such as one holding only xcodebuild's `*** If you believe this error represents
a bug` line, has no tests to be between. What comes after the last test ended, such as
XCTest's own crash after a nested run, is left to the kept streams too. Suite
banners and the `Executed N tests` line under a suite's end are structure and
are never counted.

### One name per test

The failure summary, a `test output` or `test attachments` heading, and the
selectors `--failed` builds all name a test the way `-only-testing` takes it,
`Target/Class/method`, so any one of them can be pasted into a rerun. JSON
carries it as `identifier`, next to the `test` and `target` fields; in `test
attachments` the `test` field keeps the manifest's own `Class/method()`, and
each test's directory is named after the identifier.

JUnit splits the same name at its last `/`: `classname="Target.Class"` and
`name="method"`, or `classname="Target"` for a Swift Testing function outside
any suite. Jenkins reads a classname as `package.Class`, splitting at the last
dot, so the target becomes the package its classes group under; GitLab shows
the classname as the suite; the GitHub reporter actions key a test on the pair.
The report has one `<testsuite>`, named after the scheme, so the target has to
ride in `classname` for two targets' same-named classes to stay apart.

### Every failure a test recorded

The summary gives one failure message per test, its first. A test can record
several: two assertions, or a UI test whose app crashed during a wait, which
fails the wait too (`… crashed`, then `XCTAssertTrue failed`). The rest are in
the test tree, as `Failure Message` nodes under the test case or under its
`Arguments` and `Repetition` runs, so a run with failures reads the tree as
well. A retry records the same message again, and each is kept once.

The first message stays on the `✗` line and each other one gets a line of its
own below it. A message's own continuation lines (Swift Testing prints each
operand of a failed expectation on one: `n → 2`) are indented deeper still, so
where one message ends and the next begins stays visible:

```
  ✗ SweetpadCIAppTests/AppTests/testGreeting: XCTAssertEqual failed: ("hello") is not equal to ("world")
      XCTAssertTrue failed - second failure in the same test
  ✗ SweetpadCIAppTests/GreetingSuite/suiteGreeting(): Expectation failed: n == 1
        n → 2
```

JSON adds `messages`, every message in order, beside `message`, which stays
the first. When the tree can't be read, `messages` holds just that one.

**The target comes from the test tree.** The bundle's own identifiers start at
the class (`AppTests/testGreeting()`), and xcodebuild rejects a selector built
from them: the target "isn't a member of the specified test plan or scheme". The
tree nests each case under its unit or UI test bundle, and it names that bundle
by its target. Neither the case marker's module nor the directory a test's output
lands in will do: they carry the module and product names, and renaming the
product changes both while the target stays put.

**The `()` depends on the framework.** Xcode 27 takes an XCTest method either
way. A Swift Testing test needs it (`Target/Suite/function()`, or
`Target/function()` for a test outside any suite): without it, `-only-testing`
selects nothing and Swift Testing reports a pass. The test's `test://` URL in the
bundle keeps the parentheses only where they belong, so the URL decides. With no
URL, a test gets the XCTest spelling.

**A failure outside any test is not rerun.** When a test process fails outside
any test, XCTest records it as a case of its own, `<App> (<pid>) encountered an
error`, under a `System Failures` suite with no `test://` URL. A macOS host that
crashed at launch got one on Xcode 27, and it was the run's only failure. Given
`-only-testing:SweetpadB6TestMacTests/SweetpadB6TestMac (16050) encountered an
error`, xcodebuild ran no test and printed `** TEST SUCCEEDED **`, so `--failed`
reported a green run of nothing. A case is a test when it has a `test://` URL,
or, in a tree with no URLs, an identifier with no space. `--failed` leaves the
rest out with a note naming them. When nothing else failed it stops with an
error that names the failure and points at a plain `sweetpad test`. The summary
and JUnit report still list the failure, since it is one.

### A JUnit report of every test

`--junit` writes a `<testcase>` for every test the run recorded: passed, failed,
or skipped (`<skipped message="…"/>`), each with its `time` when the tree has
one. GitLab counts the cases and ignores the suite's `tests`/`failures`
attributes, so a report of failures alone showed 5 tests for a run of 6. The
summary lists only failures, so the passed and skipped tests come from the test
tree, which a red run reads anyway. A green run reads it only for `--junit`,
about 50 ms on the fixture. With no tree the report falls back to the failures
under the summary's totals, and `test` warns that it did.

A failure's `message` attribute is the first line of its first message. An XML
parser reads a raw line break inside an attribute as a space, so a Swift Testing
expectation's `n → 2` ran into its headline. The `<failure>` body holds every
message the test recorded (`messages` in JSON), in full, with a blank line
between two. When the failure has an exit behind it (§9m), its `app
terminated: …` line comes last, a blank line after the messages, since a CI
reader has only the report to learn why the app went. The report is written
after the exit lookup for that reason. Text is escaped for XML 1.0, and the
control characters it cannot carry at all, such as an ANSI escape's ESC, become
U+FFFD.

## 9m. v8 — failures that name their own fix

Six more from §9k's family, all found by agents losing time rather than by
anyone filing a bug: the CLI knew what had gone wrong and said something that
did not help, or said nothing at all.

### A blocked build is not a broken one

An SPM build-tool plugin (SwiftLint, SwiftGen, SwiftFormat) has to be approved
before it runs. Xcode asks with a trust prompt; from the CLI the build just
fails, and no output names `-skipPackagePluginValidation`. Nothing about it is
inferable from the code being built, and it hits every such project on its
first CLI build.

It is reported as a **blocker**, distinct from a compile failure: no diagnostic
describes it and no edit fixes it, so the flag *is* the answer. Human mode said
only `xcodebuild exited with a non-zero status`; `-o json` printed the whole
transcript, which for this failure is several KB of package resolution (2.1 KB
down to 288 bytes on the project it was built against).

**The signal is version-dependent, and the obvious match is a false positive.**
Older Xcode says `must be enabled before it can be used` outright. Xcode 26
instead names the step — `Validate plug-in “SwiftLintPlugin” in package
“swiftlint”`, in curly quotes — and prints that same line on *successful* builds
once the plugin is trusted. So it counts only where it appears under `The
following build commands failed:`, which makes the watcher stateful; matching
the line anywhere would tell every healthy build to pass a flag it does not
need. Detection sits on the line stream rather than the transcript, so it works
in the human and ndjson paths that never assemble one. `app run`'s session
spawns xcodebuild itself so Ctrl-C can cancel the build. It feeds its lines
through the same watcher and builds its error with the same function
(`xcodebuild::build_failure`), so a blocked build under `run` or `run --hot`
ends on the same hint as under `build`.

### A path guessed at positionally

`sweetpad app screenshot shot.png` — a command whose whole job is writing one
file reads as taking the destination positionally. clap rejected the bare path
with `unexpected argument 'shot.png' found`, naming no flag, so the guess cost
a `--help` round-trip.

The `-o shot.png` hint already existed for the neighbouring mistake; it now
covers the unknown-argument branch too. **A positional is deliberately not the
fix:** `simulator screenshot` already spends its positional on the target
device, so accepting one on the sibling would make `shot.png` mean a file in
one command and a simulator in the other. Both tips use single quotes, matching
clap's own text — a terminal prints backticks literally.

### An ambiguity reported as a missing argument

`--simulator`'s help says it defaults to the booted one, which holds right up
until two are booted. Past that the command failed with `no simulator
specified and the terminal is not interactive`, which reads as the command
being broken rather than as the choice being genuinely ambiguous — and it
pointed at "the TARGET argument", which `app open-url` does not have. Its only
positional is the URL, so a reader went hunting for an argument that does not
exist.

The count and the candidates are what turn "broken" into "ambiguous", so the
error names them (capped, since the unbooted pool is every installed
simulator). And the way to name a simulator now travels from the caller: a
positional for the `simulator` verbs, `--simulator <name|udid>` for `app
open-url`. One shared resolver cannot know which, and guessing wrong sends the
reader after nothing.

### A destination error whose reason comes after it

A device xcodebuild cannot use fails the build with one line: `Timed out
waiting for all destinations matching the provided destination specifier to
become available`, or `Unable to find a device matching …`. The reason comes
after a blank line, in xcodebuild's own listing of the destinations it
considered. The listing is the only place the locked phone or the missing
Developer Mode shows up (`error:Iphone 13 needs to be unlocked to enable
development services`). The indented lines shown under a failed package
resolve (§9e) never reached it, because the timeout line does not end in a
colon and a blank line separates the two. So every mode reported the timeout
and dropped the cause.

`buildlog::LogParser` holds a destination error until its listing ends (at the
next line that starts in column 0, or at the end of the output) and folds the
listing into the error's message. Human, `-o json` and `-o ndjson` output then
carry the same diagnostic. A full listing is dozens of simulators, so only what bears on the
failure is kept:

- the specifier xcodebuild echoes back as the requested one, and the listing's
  entry for it, from either side;
- every usable ("compatible", older Xcode "available") destination with an
  `error:`;
- the prose between sections.

Everything else is counted: `(26 other destinations omitted)`. The
incompatible side's errors are left out on purpose. Every entry there has one,
and it says the platform does not match the scheme, which is true of all of
them and explains nothing.

The listing stays in the diagnostic. `-o json`'s `error.message` names only the
error's first line, without the colon that introduced the listing, and then the
full log, so the headline stays on one line.

When the requested destination is a physical device (`platform=iOS,id=…`,
from `--on`, `--destination` or the `--` passthrough), the failure ends on
`tip: run 'sweetpad device info <udid>' to see why the device isn't ready`
(`error.tip` in the machine modes). The listing says what xcodebuild saw;
`device info` (§9q) connects and names the first thing to fix. Both destination
errors get it: on Xcode 27 a device id that matches nothing xcodebuild can see
waits about a minute and then reports `Unable to find a device`, not the
timeout.

xcodebuild prints no `** BUILD FAILED **` for either error, so human output
would end on the listing with no verdict under it. The streamed log closes on
`✗ Build failed` anyway (`✗ Tests failed` under `test`), in `build`, `test`
and `app run`'s session alike, and the tip comes after it.

### A test whose app vanished says what ended it

When the app under a UI test dies mid-test, XCTest reports only that it is
gone. Xcode 27 says `<bundle id> crashed` or `Failed to application <bundle id>
is not running`, and `test` passed that on with nothing else. Why it went is in
launchd's exit line (§9j), which `test` reads for exactly these failures and
attaches:

```
  ✗ ExitProbeUITests/ProbeUITests/testAppKilledFromOutside: Failed to application dev.sweetpad.exitprobe.app is not running
      app terminated: Termination requested by simulator host (OS_REASON_SPRINGBOARD 0xfbfbfbfb)
```

Under `-o json` and ndjson the failure gains `terminationReason`, the same
object `app logs --exits` reports per exit: `namespace`, `code`, `reason`,
`label`, `explanation`, `exception`, and the rest. The field is additive, so `schema` stays.

**Which failures, in Xcode's words.** Each wording below was reproduced against
a scratch app on Xcode 27, unless it is marked as read from XCTest's own
strings. `<id> crashed`, `<id> crashed in <symbol>` (strings) and `…
application <id> is not running` name the app. `Test crashed with signal kill.`
(the UI test runner killed) points at the process running the tests. So does
what a unit test's host app gets when it crashes: `Crash: <App> at <frame>.
<library>: <reason>`, or `The test runner crashed while preparing to run tests:
…` when it was the first test to run. A host that calls `exit` gets `The test
runner exited with code 3 before finishing running tests. …`. A Swift Testing
test crashes its host in the same words. From the strings come `Lost connection
to the test runner`, `Lost connection to test process`, `The test runner
crashed before establishing connection: …` and `Test runner crashed.` When the
host crashed on every restart, XCTest writes `Exceeded max restart count of 2.
(Underlying Error: …)`, which counts when its underlying error does. A UI
test's runner is its `.xctrunner` app; a unit test's is the host app, whose
bundle id comes from the in-process build-settings resolver. Every message a
test recorded is checked (§9l), and the first with one of these wordings
decides, since an assertion that failed before the crash is recorded ahead of
it.

**A wording, not a word.** Matching "crashed" anywhere in a message took an
assertion's own text for a crash: `XCTFail("the helper crashed")`, recorded as
`failed - the helper crashed`, or a Swift Testing `Issue recorded: …`. That
test then got whichever exit of the host fell in its window, a later test's
crash or the host's `exited with status 1` at teardown. So each wording is
matched where Xcode writes it: the runner's at the start of the message, an
app's `crashed` right after its bundle id, and `is not running` only after
`application <id>`. Any other failure is left alone, so an ordinary red suite
pays nothing.

**Which exit.** One `log show` covers every app job's exits over the run. Per
failure, the test's activity log gives two times: the test's start and the
moment that message was recorded. A unit test records no start, so its window
opens with the run. The exit is that process's last one between them. On a
simulator the window runs half a second past the failure. launchd can log the
reap just after XCTest notices, but the margin cannot be wider: XCTest restarts
a crashed unit-test host, and the restart exited cleanly 1.5s after the failure
it caused. On a Mac the window closes at the failure. There XCTest records a
failure only after the exit behind it: 0.3s after a host's `exit`, and 2s to
6.3s after a crash, while it waits for the crash report. The host it restarts
to finish the run exited 0.4s to 0.8s after the failure, so with the
simulator's margin two runs out of four gave the test whose host went away the
restarted host's `exited normally`.

**Best effort, bounded.** The query gives up after 15s, and at most 20 failures
are timed (each costs an `xcresulttool` read). A failed query, a device
destination, or a failure with no activity log leaves the field out rather
than guessing.

**One more look.** When a failure that says something vanished finds no exit,
`test` waits 2s and queries once more for the failures still without one.
launchd's line is readable within a second of the exit, so the second query is
not waiting for the log to catch up: in the one run of six that printed no
`app terminated:` line, launchd had logged the crash ten seconds before the
query ran. What the second query recovers is a first one that failed or ran out
of time, and contention can cause that. Four other readers dumping the
simulator's log, as `simctl diagnose` does, stretched the usual 1.5s query to
13s. Only a red run with such a failure and no exit for it pays the 2s and the
extra query. A green run, or a red one with no failure of that kind, never
queries at all.

**No exit found says where to look.** When both queries come back without the
exit (they failed or ran out of time, or the exit landed outside the window),
the failure says only that the app is gone. It then gets a line, and a `note` in
JSON, naming the `app logs --exits` command for the run's destination:

```
      couldn't find launchd's exit record; try 'sweetpad app logs --exits --on F13C004A-…'
```

The command carries `--on <udid>` for a simulator or `--mac`, plus whichever
project and target flags the run was given (`--project`, `--scheme`, …), so it
reads the same log for the same app. A device destination has no exit log to
read and gets no line.

**A hostless bundle crashes `xctest`.** A unit-test bundle with no host app
runs in `xctest`, which xcodebuild starts itself. launchd logs no exit for it,
and there is no app to read one for. XCTest names the process: `Crash: xctest
at static xctest.main()`, also under `Exceeded max restart count`. `test` reads
that wording as `xctest`, not the host app, and does not search launchd's log
for it. The host app fallback would give it a wrong exit: in a macOS run of a
hostless target beside a hosted one, the hostless crash would get the hosted
target's app `exited normally`.

XCTest waits for the crash report and attaches it to the failing test as
`Crash Log <date>.ips`. The same report is in `~/Library/Logs/DiagnosticReports`
as `xctest-<date>.ips`, but its header names no bundle id, and every hostless
crash on the Mac writes one under that name. Only the attachment ties a report
to its test. So `test` exports each such failure's crash logs from the result
bundle (`xcresulttool export attachments --test-id <url> --filter 'Crash
Log*'`, about 40ms on Xcode 27) and reads the latest one that is `xctest`'s.
The report gives the failure its `terminationReason`, with `source:
crashReport` and a `null` `bundleId`. Its backtrace gives `crashedIn`, the same
way a host app's report does. Measured on Xcode 27 with a hostless macOS
bundle, a `fatalError` and a write through a bad pointer:

```
  ✗ B9MiscHostless/CrashTests/testBFatal: Crash: xctest at static xctest.main()
      xctest terminated: crashed with SIGTRAP (sent by exc handler[18465]; EXC_BREAKPOINT)
  ✗ B9MiscHostless/CrashTests/testCBadPointer: Crash: xctest at static xctest.main()
      xctest terminated: crashed with SIGSEGV (sent by exc handler[18468]; EXC_BAD_ACCESS KERN_INVALID_ADDRESS at 0x0000000000000010)
```

`crashReport` names the copy in DiagnosticReports. Both headers carry the same
`incident_id`, and only reports written since the run started are opened. When
that copy is gone, the failure keeps its exit and gets a line, and a `note` in
JSON, naming the `test attachments` command that exports the attached one:

```
      no crash report was found in DiagnosticReports; 'sweetpad test attachments --only-testing B9MiscHostless/CrashTests/testBFatal' exports the copy XCTest attached
```

When no crash log could be read, the line names the same command:

```
      the tests ran in 'xctest', which launchd logs no exit for; 'sweetpad test attachments --only-testing B8TestHostless/CrashTests/testACrash' exports the crash log XCTest attached
```

The command carries `-C`, the project flags and, when the run named its own,
`--result-bundle`, since those pick the bundle. `test attachments` refuses the
target flags. At most 20 failures have their crash logs read.

**A crash without its report says why.** The fault's detail (`EXC_BREAKPOINT`,
the address) comes from the crash report, and a suite that crashes its app all
day stops getting reports. osanalyticshelper counts the reports it saves for an
app (`Saved type '309(…)' report (25 of max 25) at …/SweetpadCIApp-….ips` on
macOS 27) and logs each crash after that as `not saved because the limit of 25
logs has been reached`. launchd's line is then all there is: `crashed with
SIGTRAP (sent by exc handler[…])`, with nothing to say why the rest is missing.
So a crash with no matching report gets a line under it, and the failure gains
a `note` in JSON:

```
      app terminated: crashed with SIGTRAP (sent by exc handler[79175])
      no crash report was found; macOS may have reached its limit of crash reports for this app
```

It says "may" because the limit is only the cause seen so far: the report may
also not be written yet, and nothing in the log says when the count resets.
The limit is not strict either. Another process on the same Mac reached `33 of
max 25`, and each count seen matched the reports of that name on disk, so a
report that goes away may make room for one more. `app logs --exits` gives the
same reason in its note about a crash with no report.

**The test that crashed the host is not always the one that failed.** A unit
test's host runs every test of its target, and XCTest attributes a host crash to
whatever was running when it happened. Reproduced on Xcode 27 with a Swift
Testing suite running in parallel in a macOS host, where `b_crashesHost()` calls
`fatalError`:

- With `a_recordsCrashed()` still sleeping when `b` crashed, both failed with the
  same `Crash: <App> at specialized static Runner._applyScopingTraits(…)`, and
  the summary listed `a` first.
- With `b` scheduling the `fatalError` on a dispatch queue and returning, `b`
  passed and `a`, the test running a second later, took the crash alone.
- With every test finished first, the run passed: XCTest recorded no crash at
  all.

The test tree gives no more than that: each failed case holds the same
`Failure Message`, and its activities hold the crash log's attachment and
nothing about the thread. Only the crash report says whose code it was. Its
crashed thread runs through `ParallelSuite.b_crashesHost()` in the first case
and `closure #1 in ParallelSuite.b_crashesHost()` in the second. So each crash
with a report has its backtrace read for the innermost frame that is one of the
run's tests: a demangled `Type.method(…)`, the last word of the symbol, or an
Objective-C `-[Class method]`, matched against the tree's cases with their
targets and argument lists dropped. The failure gains a line when that test is
another one (`the crash report's backtrace is in <id>, not this test`) and
`crashedIn` in JSON either way. When the backtrace is in no test (the fault is in
a framework thread) or no report was found, a crash that failed two or more
tests says it can't tell them apart, and JSON lists them as `crashCandidates`.
Two failures share a crash when their exits have the same bundle id, pid and
time. The JUnit body carries the same line after `app terminated:`.

### A red suite that looked hung

From Xcode 26 on, a test run that fails starts `simctl diagnose --timeout=600`
before xcodebuild exits: a sysdiagnose-sized collection for the result bundle,
which nobody asked for and which can take the full ten minutes. `test` printed
nothing in that time, so every failing run looked like a hang, and three agents
separately waited it out. `test` passes `-collect-test-diagnostics never` on
Xcode 26 and later. Older Xcodes collect nothing by default and may not know the
flag, so they get none. A `-collect-test-diagnostics` in the passthrough keeps
its own value, so `-- -collect-test-diagnostics on-failure` brings the
collection back.

## 9n. Direction — the run session as a server

`app run`'s only door is a tty. The session owns everything an iterating loop
needs — the settled `RunPlan`, the live process, the hot-reload channel (§9d),
the log stream — and the sole way to ask it for anything is a keystroke it
reads in raw mode ([`rawmode`]). A second window, whether a human's or an
agent's, cannot reach it.

> Status: direction, not a committed version. Nothing below is scheduled.

That gap forces every other client to re-derive the world, and the copies
disagree. A detached `app run` from another window calls `plan` again,
independently: remembered scheme/configuration/destination live in the same
state file that window's own commands write, `refresh_stale_destination` can
recover a vanished simulator pin to a *different* destination mid-session, and
resolution takes structurally different paths interactively (a picker) versus
not (an error, per `--non-interactive`). Worse, the run-shaping options never
persist at all — `hot`, `hot_entitlements`, `launch` args/env and `passthrough`
come from the command line of the session that started it, and
`LastLaunchedApp` records none of them. A session started as `run --hot --env
API=staging` is answered by a detached run that builds a non-hot binary without
the variable. That isn't drift; it's a different build, arrived at silently.

The answer is to stop treating the tty as the interface. The session becomes
the owner and exposes a control channel; the terminal attaches as a client, an
agent attaches as a client, and the extension (§7) could attach as a third. `r`
and an `app session rebuild` from another window resolve to the same command on
the same channel — one produced locally by a keystroke, the other arriving over
a socket. The divergence above then has nothing left to diverge: one
resolution and one app, because there is one owner — structurally, rather than
by keeping copies in sync. This is the shape dev loops with more than one
observer converge on (Flutter's daemon mode, Metro, Vite's HMR clients).

The dispatch half already exists: the session loops match on a `SessionKey`
command enum rather than on raw bytes, so the keystroke reader is one
*producer* of commands, not the control flow itself. What's missing is a second
producer (a socket listener feeding the same channel), a response path (build
results reach the client that asked, not only stdout), and the surface —
`app session <rebuild|relaunch|status|stop>`, under a `session` noun rather
than a bare `app rebuild` that would read as a sibling of `build` and blur that
it addresses a *running* thing. Per §9g's rule that a caller owns its own
timing, the request is non-blocking: `rebuild` returns a job id and `status`
polls, so a long build never freezes an agent's loop.

*Considered and rejected as an interim:* persisting the settled `RunPlan` into
state for a detached run to adopt. It removes the silent-wrong-build hazard
with no IPC at all, but it cannot carry `--hot` (which needs the live process),
it makes one process depend on another's pid-namespaced temp directory for
`hot_entitlements`, and it is discarded wholesale once the channel exists.
Signals and watched control files are cheaper still and strictly worse: they
trigger a rebuild but return nothing, leaving the caller blind to the result
that was the reason it asked.

## 9o. v8 — `app container` — where the app's files live

Seeding a PDF into an app's Documents folder before a UI test needs the data
container's path. `simctl get_app_container` prints it, but only for a UDID
and bundle id the caller resolves by hand, and sweetpad already resolves both
for `app install`, `launch` and `stop`.

```
sweetpad app container [--kind data|app|groups]
```

**Resolution is `stop`'s:** the recorded last launch unless a targeting flag
names another app, else the resolved build target, and never a build. Human
mode prints the path alone on stdout, so `cd "$(sweetpad app container)"`
works, and `--kind groups` prints one `id  path` line per App Group. `-o json`
emits `{path, kind, bundleId, destination}`, with `groups: [{id, path}]` in
place of `path`.

**On a simulator** it is `simctl get_app_container`, after booting the
simulator, because a shut-down one fails every lookup with a CoreSimulator
state error. simctl reports an app that isn't installed as a bare ENOENT (`No
such file or directory`), so that case is reworded to name the app and the
simulator, with the `app install --on <udid>` that fixes it. The suggested
command carries the `--scheme`, `--configuration` and container flags this
lookup was given, since a typed scheme isn't remembered and the command would
otherwise stop at "no scheme specified"; every `app` hint that names a
follow-up command is spelled the same way.

**On macOS** `app` is the built product, `data` is `~/Library/Containers/<bundle
id>/Data`, and each id in `com.apple.security.application-groups` maps to
`~/Library/Group Containers/<id>`. Whether the app is sandboxed comes from the
signed product (`codesign -d --entitlements - --xml`), not from whether the
container directory exists: the directory appears only on first launch and
stays behind after the sandbox is switched off. An app without the sandbox has
no container, and the error says so instead of pointing at Application
Support. App Groups work without the sandbox, so `groups` reads the
entitlement either way.

**A physical device** is refused. Its containers stay on the device, so the
error names `xcrun devicectl device copy to` and `copy from`, with the device
and bundle id filled in.

## 9p. v8 — `test build`: compiling what `test` would run

`build` compiles the scheme's Run targets and nothing else, so it succeeds while
a test target fails to compile. Agents are told to use `build -q` as the cheap
"does it still compile" check, so a break confined to test code passed it
silently. The only other route was the undocumented passthrough `sweetpad build
-- build-for-testing`.

```
sweetpad test build [--watch] [--show-command] [-- XCODEBUILD_ARGS]
```

**A build, not a run.** The verb is `build` with xcodebuild's `build-for-testing`
action in place of `build`: one `BuildPlan`, so the transcript, `-q`, and the `-o
json`/`ndjson` result are the same, and it writes the one build record `build
diagnostics` reads back. The result bundle it asks for is the build's slot, never
the bundle `test run` retains, so compiling the tests cannot erase the failures
that `--failed`, `test output`, and `test attachments` read. It passes
`-resultBundlePath` for `build`'s reason: without it xcodebuild writes no
activity log, and `xcode-build-server` has nothing to read. `productPath` is
`null`, because what a test build writes is the test bundles.

**Targeting is `test run`'s.** The scheme, configuration, and destination come
from the testing context, so it compiles what `test run` would run.

**No `--only-testing`.** Measured on Xcode 27: `build-for-testing
-only-testing:A` still compiles every test target the scheme tests, and a compile
error in a target the filter leaves out still fails the build. A filter would
promise a narrowing that never happens. The run flags (`--only-testing`,
`--skip-testing`, `--failed`, `--result-bundle`, `--junit`, `--retry-flaky`,
`--coverage`) are resource-global, so they parse after `build`; they are refused
by name as a usage error (exit 2), not dropped. The verb redeclares them hidden,
so `test build --help` leaves them out.

`--watch` is `build --watch`'s loop unchanged. A Swift package runs `swift build
--build-tests`.

## 9q. v8 — physical devices: how they connect, and whether they are ready

A physical device in `sweetpad devices` read the same whether it was plugged
in, unlocked and in Developer Mode, or none of those. An agent picked a listed
iPhone and the build spent a minute timing out (§9m has the other half of
that report).

`devices` and `device list` carry what `devicectl list devices` knows about the
link: `connection` (`properties.connection.state`, or `tunnelState` before
jsonVersion 5), `transport` (`wired` / `localNetwork`) and `pairing`
(`paired`). The fields are flat, under the name `device list` already used for
`connection`, and null on simulator and macOS entries. Human output shows only
a hint: `[usb]`, `[wifi]`, `[wifi, not paired]`.

**`connection` is not a readiness signal.** An idle device reads
`disconnected` even when it works, because xcodebuild and devicectl open the
tunnel on demand. Human output never shows it, and nothing refuses a build on
it.

On Xcode 27 the same listing also returns simulators (`reality: "simulated"`,
`transportType: "sameMachine"`). As devices they got a `platform=iOS,id=<sim>`
specifier that cannot build for them, and they made `--on device` ambiguous
whenever the listing held one. They are dropped when the listing is parsed,
since `simctl` already reports them.

The parsing lives in `sweetpad-core`'s `devices` module, and the VS Code
extension reads the same listings through the addon, so the two agree on what
is a device. That module also finds an app's processes on a device for `app
stop` and the extension's debugger. devicectl reports each executable as a
`file://` URL, which spells a space `%20`, so the URL is decoded before the
match. The bundle directory has to match whole: `App.app` never claims
`MyApp.app`'s processes.

### `device info` — asking the device itself

```
sweetpad device info [DEVICE] [--timeout SECONDS]
```

Readiness needs a connection attempt, which the listing never makes.
`device info` runs `devicectl device info details` against one device, and
`device info lockState` once it has connected, then reports pairing,
connection and transport, Developer Mode (`developerModeStatus`, present only
once connected), whether the developer disk image's services are up
(`ddiServicesAvailable`), the lock, boot state, OS version, and devicectl's
own warnings (written only to its human output, read back through
`--log-output`).

It ends with one verdict, `ready` or a `reason` that names the first thing to
fix. The checks go in the order the problems have to be fixed: not paired;
did not answer, or did not connect within the timeout; Developer Mode off; not
booted; locked; development services down. The payload is `ok: true` either
way, and a device that is not ready exits 1, as `doctor` does with a problem.

- **Placement.** `device` is the singular resource noun beside `simulator`,
  and `info` acts on one named device the way the `simulator` verbs act on
  their TARGET. `devices` stays the aggregated view with no actions: `devices
  info X` would be a per-item verb hung off a list. So `device` is visible in
  the help, with `list` beside `info`.
- **The argument** resolves like `--on`, over physical devices only: a
  `context alias` of the project in the current directory, a UDID, an exact
  name, a unique part of a name, or `device`. With none, the only paired
  device. A simulator's name gets an error that says it is a simulator.
- **Bounded.** devicectl gets `--timeout` (default 10, at least 5, which is
  devicectl's floor), and the process is killed 5 seconds past it in case
  devicectl does not honour its own. The lock query gets devicectl's minimum,
  since it runs only against a device that has already connected.
- **Read-only**, apart from what connecting does: the details request may
  try to mount the developer disk image, as xcodebuild does before a build.

## 9r. v8 — `app sample` — wedged or merely idle?

An app that looks alive but has stopped doing work raises one question first:
is the main thread stuck, or waiting for something that never came? Answering
it meant finding the pid by hand, although sweetpad already records it for `app
stop`, and then running `sample <pid>`, whose call graph is the hard part to
read. The two answers lead in opposite directions. A wedged main thread is a
deadlock or a hot loop, and the stack shows where. An idle one is not hung at
all; the bug is a callback, completion handler or queue that never fired.

```
sweetpad app sample [--seconds N] [--output-file PATH] [--pid PID]
```

**Target resolution** is §9h's ladder: `--pid`, then the recorded last launch,
then the resolved build target, and never a build. A simulator app is a process
on this Mac, so it samples the same way a macOS app does; its pid comes from
matching `ps` against the executable in the bundle `simctl get_app_container`
names. A physical device's app is out of reach, and the error says so. An app
with several processes is refused with their pids, as `ui` refuses it.

**Capture** is `/usr/bin/sample <pid> <secs> -file <path>`. `spindump` would
add the kernel's view but needs root. Sampling defaults to 3 seconds and
accepts 1 to 60; `sample` then has 60 more seconds to symbolicate before it is
killed, so the verb always returns. The full report is always written and its
path is part of the result. By default it goes to
`<state>/sweetpad/samples/<app>-<time>.txt` rather than the working directory,
because it is evidence to read back, not a file the project keeps.

**The verdict reads the main thread only**, with rules narrow enough to pin
against real captures. Every sample's stack lands in one of four buckets:

- *run-loop wait*: the top of the stack is `mach_msg` stubs directly under
  `__CFRunLoopServiceMachPort`. AppKit's loop, UIKit's `GSEventRunModal` and a
  modal alert's nested loop all end there;
- *waiting*: the top is in the kernel, and a known wait primitive sits below it
  before any app frame: a mutex, condition variable, rwlock, semaphore,
  `dispatch_sync`, dispatch group or once, `os_unfair_lock`, a sleep, or a
  synchronous IPC reply;
- *running*: the top is in user space;
- *syscall*: any other kernel call (`read`, `open`), which no rule claims.

`idle` needs run-loop waits in at least 90% of the samples. A responsive app
still services the odd timer while it is sampled, and below that line "idle"
would hide real work. `blocked` and `busy` need two thirds, so the named state
outweighs everything else twice over. `blocked` names the wait and the
innermost app frame on its heaviest stack; `busy` lists the app frames the
samples were spent in. Anything else is `unclassified`, reported with its split
rather than a guess. The human line for `idle` says outright that the app is not
hung and to look for what never fired, because an idle main thread in a stopped
app reads as a hang.

**The wait is named by the most specific marker nearest the top.** On macOS 27
a `dispatch_sync` onto a stuck queue ends in `kevent_id` under
`__DISPATCH_WAIT_FOR_QUEUE__`, not in a ulock, and an `os_unfair_lock` ends in
`__ulock_wait2`, which a dispatch group and `pthread_join` also end in. So the
scan walks down from the top of the stack, takes the first specific marker, and
falls back to a generic one (`__ulock_wait*`, a bare `mach_msg`) only when
nothing more specific is there. It stops at the first app frame or callout
boundary, since a marker below that belongs to a different call.

**App frames come from the bundle, not from `sample`'s `+` marker.** `sample`
marks every image outside the host OS with `+`, which in a simulator includes
the whole iOS runtime. The images inside the sampled process's `.app` are the
app's: its executable, `.debug.dylib`, embedded frameworks. `sample` redacts
directory names (`/tmp/*/App.app/…`), so the bundle is matched by name. Entry
points and Swift thunks are skipped when a frame is named, because every
main-thread stack has them.

**A swallowed exception is flagged.** When AppKit catches an Objective-C
exception and keeps running, HIServices records the first one by parking a
thread named `HIE: M_ <hash> <time>` in a function called
`SOME_OTHER_THREAD_SWALLOWED_AT_LEAST_ONE_EXCEPTION`. Either one in the report
raises the flag. The fixture is a real capture of an exception thrown from a
timer callback (one thrown from a dispatch block terminates the app instead).
The flag points at `app diagnose`, which stops at the throw.

`-o json` emits `{pid, bundleId, seconds, reportPath, mainThread: {state,
samples, breakdown, topFrames: [{symbol, image, samples}], wait?}, flags:
[{kind, thread}]}`, with `wait` (`{kind, symbol, caller, samples}`) present when
blocked. As with `app diagnose`, the exit code reports the capture, not the
finding: a clean sample exits 0 whatever state the app was in.

*Deliberately not built:* verdicts on threads other than the main one (the full
report has them, and a background deadlock matters to this question once the
main thread waits on it); `spindump`; physical devices; and telling a nested run
loop from the top-level one, since a modal alert is idle and reads as idle.

## 9s. v8 — the locator sees what the build saw

The `app` verbs find the product through the in-process resolver (§9h), which
has to reach the answer `xcodebuild` reached from the same arguments. Where it
did not, a verb launched a stale bundle, reported the wrong bundle id, or could
not launch at all.

### A product built under a typed `-derivedDataPath`

After `sweetpad build --on mac -- -derivedDataPath build/dd`, nothing could
launch the app: `app launch` refuses the `--` tail (§3), `sweetpad.toml`
refuses `-derivedDataPath` (§6), and the locator looked in the default
DerivedData.

```
sweetpad app launch --mac --derived-data-path build/dd
```

**A flag, not the tail, and not a record.** Three shapes were on the table:

- A `--` tail on `launch` that takes only `-derivedDataPath`. Everywhere else
  the tail means "arguments for xcodebuild", and here it would mean one
  argument that reaches no xcodebuild. `launch` would also lose the
  parse-time refusal that §3 gives every verb that builds nothing.
- Reading the product path the last `build` recorded. That remembers a typed
  one-off, which §5 rules out for a scheme and which holds just as well for a
  location. A later build from Xcode, or of another scheme, would leave the
  record naming the wrong bundle, and nothing would say so.
- A flag, typed per command like the build's own `-derivedDataPath`. This is
  the one built.

**A relative path resolves where xcodebuild ran.** sweetpad runs xcodebuild
from the directory holding the container, so `-- -derivedDataPath build/dd`
typed in a nested source directory writes beside the project, not below the
working directory. The locator joins a relative path onto that same
directory, for the passthrough and the flag alike. The same text then names
the same place on both commands, and `build -o json`'s `productPath` is an
absolute path to a bundle that exists.

**Only `launch` takes it.** The verbs that act on a running app (`stop`,
`logs`, `screenshot`, `sample`, `ui`, `container`) read the recorded last
launch first, and that record carries the path `launch` started. Targeting
flags keep the record when they name the same launch: the record carries the
scheme, configuration and `-destination` it was planned with, so `app ui
click --scheme AppMac --on mac` after `app launch --mac --derived-data-path
dd` finds the app running out of `dd`. Each typed flag has to agree with the
record. `--scheme`, `--configuration` and `--destination` compare as typed,
`--mac`/`--device` compare the record's kind, and `--on` compares `mac` or a
UDID directly and resolves anything else against the simulator list as the
plan would. A flag that names another scheme, configuration or destination
resolves that app instead, and so does a typed `--scheme`, `--configuration`
or `--destination` against a record that lacks the field (an older state
file). `uninstall`
and a simulator `launch` need only the bundle id, which no DerivedData
location changes. A macOS `launch` whose product is missing says it isn't
built, names the `build` that makes it, and names the flag when it wasn't
given.

### A product the locator couldn't find says why

`-o json` on `build` reports `productPath: null` with a `note` naming the
reason when the lookup fails, since `null` alone reads the same as a scheme
with nothing launchable. A Swift package or a test build, which has no `.app`
by design, gets none.

### Command-line settings reach the locator

`xcodebuild` applies a `KEY=VALUE` argument above every project layer, and the
locator ignored them. With `PRODUCT_BUNDLE_IDENTIFIER=com.example.x` in the
tail or in `[xcodebuild] args`, `app install` reported the project's bundle id,
and `app launch` started a different app that happened to be installed under
it. The assignments go into the resolver's command-line layer
(`BuildSettingsOptions::overrides`, applied to every query in order), so the
product path, bundle id and executable are the ones the build wrote.

An assignment is an argument whose text before `=` is an identifier, skipping
the value after a flag that takes one: `-destination 'OS=17.0,platform=iOS
Simulator'` is a specifier, not a setting named `OS`. The flags are a fixed
list; one missing from it reads as a switch, which costs at most one setting
read from its value.

A `TARGET_BUILD_DIR=` override is followed through the same layer.

### Every build-settings caller sees the build's arguments

The locator was the only resolver caller that took the command line, so
`settings show` and the `--hot` recompiler could disagree with the build they
describe. The arguments are read once, as sweetpad-core's `app_locator::CommandLineSettings`
(the `-derivedDataPath`, the `-xcconfig` overlay, the `KEY=VALUE`
assignments), and handed to each caller from the arguments it has:

- the locator, from the plan's passthrough (the file's `[xcodebuild] args`,
  then the typed tail);
- the recompiler, from the `--hot` build's passthrough, with this machine's
  Xcode locations, so a recompiled file's search paths name the products that
  build wrote;
- `settings show`, from the file's `[xcodebuild] args` and a `--` tail of its
  own, merged the way a build merges them, so a one-off `KEY=VALUE` or
  `-xcconfig` can be previewed before a build is spent on it;
- `pbxproj settings set`/`unset`, from the file's `[xcodebuild] args`: the
  effect rows they print after an edit are what a build resolves, and a
  touched key the file also sets, directly or in its `-xcconfig`, is warned
  about, since it outranks the stored layer and the edit changes nothing the
  build sees;
- the BSP server `bsp init` configures, from the file's `[xcodebuild] args`,
  read once at startup, for the editor's compiler arguments and the
  `buildTarget/prepare` build (the settings go before prepare's own
  `CODE_SIGNING_ALLOWED=NO`, which wins). A file `build` would refuse is
  warned about on stderr and left out, so the index keeps working.

A path resolves against the directory `xcodebuild` runs from, as the build
reads it. `xcodebuild` takes one `-xcconfig` and fails on a second, so a
typed one replaces the file's (§6) and the build and the resolver read the
same one. `-xcconfig` sits above the command line: Xcode 27 resolves
`-xcconfig X.xcconfig FOO=cli SWIFT_VERSION=5.9`, with the file holding `FOO =
$(inherited) x` and `SWIFT_VERSION = 6.0`, to `FOO = cli x` and
`SWIFT_VERSION = 6.0`, as its man page says. The resolver places an override
of a key the overlay also sets just below the overlay, and keeps every other
override on top. Against `xcodebuild -showBuildSettings` on the fixture, with
that file and those settings, `settings show` differs in the same keys as
with none: a `/tmp` versus `/private/tmp` spelling, which the resolver
follows (see "The project path is spelled the way xcodebuild spells it"
below). `CODE_SIGN_IDENTITY`
differed too, and the reason was not the fixture's `CODE_SIGNING_ALLOWED =
NO`: Xcode 27 reports `-` for a macOS app with no `DEVELOPMENT_TEAM` and no
authored identity whatever `CODE_SIGNING_ALLOWED` or `CODE_SIGN_STYLE` say,
and `Apple Development` once a team is set. The resolver applies that to
macOS apps, as it already did to macOS test bundles and tools.

With the overlay reaching the locator, the warning that an `-xcconfig` "can
move the build output … the launched bundle may be stale or missing" had
nothing left to cover and is gone. An overlay setting `SYMROOT` is followed
like any xcconfig `SYMROOT` (the `symroot_override_oracle` capture): a `build
-o json -- -xcconfig sym.xcconfig -derivedDataPath dd` names the `.app` under
the file's `SYMROOT`, where the build wrote it, instead of a stale one under
`dd`.

The BSP server the VS Code extension configures reads no `sweetpad.toml`.
The extension's `buildServer.json` runs `sweetpad bsp serve --config
<bsp.json>`, and the container, configuration and DerivedData come from that
`bsp.json`. The extension's own builds take its `sweetpad.build.args` setting,
not the file, and discovery from the working directory could name a container
other than the one `bsp.json` does. So the extension writes that setting into
`bsp.json` as `buildArgs`, and `serve` reads its `KEY=VALUE` settings and last
`-xcconfig` the way the CLI reads `[xcodebuild] args`, through the same
`CommandLineSettings` in sweetpad-core's `app_locator` (the parser lives in
`xcodebuild_args`). A relative `-xcconfig` is read against `workspacePath`,
where the extension's builds run `xcodebuild`, by that folder's physical path,
as `xcodebuild` reads it: through a symlinked folder, `../ci.xcconfig` is the
real folder's sibling. A `-configuration` in `buildArgs` replaces the
extension's own on its builds' command line, so `serve` resolves that
configuration instead of `bsp.json`'s `configuration`, at startup and when the
file changes. The
extension's builds read the setting with a copy of core's `VALUE_FLAGS`
(`XCODEBUILD_VALUE_FLAGS`), and a spec fails when the two differ, so a build
and the index agree on which argument is a flag's value: `-xcconfig -quiet`
names a file called `-quiet` in both. A flag outside the list takes the next
argument only when that is not a flag, a setting or an action, so `-quiet
build` stays a switch and an action. The extension
rewrites `bsp.json` when the setting changes, and the server re-reads it as it
does a new scheme or configuration. A `bsp.json` without `buildArgs`, from an
older extension, has none. The CLI's own server reads `sweetpad.toml` only at
startup, so an edit to it reaches the index when the editor restarts the
server.

A `-derivedDataPath` in `sweetpad.build.args` replaces the extension's own on
its builds' command line, so it has to move the index too. The extension
resolves it, not `serve`: the last one in the setting wins over
`sweetpad.build.derivedDataPath`, a relative one is read against
`workspacePath`, and the result is `bsp.json`'s `derivedDataPath`. The same
resolver answers the extension's app locator and its CLI server, so builds,
launches and the index share one answer. `serve` ignores the flag in
`buildArgs`, which keeps one field in charge of the location. It reads that
field at startup only, since the index store it advertises in `initialize`
lives there, so a change reaches the index when the server restarts. The
extension rewrites `bsp.json` when `sweetpad.build.args` or
`sweetpad.build.derivedDataPath` changes. When the new `derivedDataPath`
differs from the one in the file it replaces, it restarts the Swift language
server, and sourcekit-lsp starts a new server that reads the new location.
`sweetpad.build.autoRestartSwiftLSP` turns that restart off, as it does the
one after each build.

### Settings that move the product are followed (decided)

`SYMROOT=`, `OBJROOT=` and `CONFIGURATION_BUILD_DIR=` were refused before a
build was spent on them, because nobody had checked the resolver's reading of
a command-line one against `xcodebuild`. With the assignments reaching the
resolver, that check was made on Xcode 27: `xcodebuild -showBuildSettings`
given each setting, against `settings show` given the same one in
`[xcodebuild] args`, for the fixture's macOS scheme and its iOS scheme on a
simulator, comparing `SYMROOT`, `OBJROOT`, `BUILD_DIR`,
`CONFIGURATION_BUILD_DIR`, `TARGET_BUILD_DIR`, `BUILT_PRODUCTS_DIR`,
`CODESIGNING_FOLDER_PATH`, `OBJECT_FILE_DIR` and `TARGET_TEMP_DIR`.

- An absolute value, or one anchored by a macro (`$(SRCROOT)/build`), agreed
  on every key for all three settings and for `SYMROOT` and
  `CONFIGURATION_BUILD_DIR` together. `OBJROOT` moves only the intermediates;
  the product stays under DerivedData, as the resolver has it.
- A relative value did not. `xcodebuild` reads a relative `SYMROOT`,
  `OBJROOT` or `CONFIGURATION_BUILD_DIR` against the directory of the project
  that owns the target, wherever it runs from and also through a workspace in
  another directory, while it leaves a relative `TARGET_BUILD_DIR` as typed.
  The resolver kept `relsym`. It reads a relative value against
  `$(PROJECT_DIR)` now, per target and from any layer (below), and every case
  agrees, workspace included.

All three are followed, so the refusal is gone. Real builds agreed with the
check: `build -o json -- SYMROOT=relsym` on macOS names
`<project>/relsym/Debug/SweetpadCIMac.app`, where the build wrote it, and
`app launch --mac` with the setting in `sweetpad.toml` starts that bundle. On
the iPhone 17 simulator `app install` and `app launch` with an absolute
`CONFIGURATION_BUILD_DIR` and `OBJROOT` in the file install and start the
relocated build.

### Build locations are folded where they resolve

`xcodebuild` folds a build location lexically, `..` and `.` folded without
resolving a symlink: `SYMROOT=../x/sym2` is `<parent>/x/sym2` and
`/tmp/../tmp/obj` is `/tmp/obj`. The locator used to fold the product's
directory on its own, which fixed `productPath` but left `settings show`,
the BSP arguments and every other reader of the resolver with the `..`
chain. The folding now happens in the resolver, and the locator's copy is
gone.

Which settings fold was measured on Xcode 27.0 with `-showBuildSettings`,
setting each path setting to `/tmp/../tmp/<KEY>` and to `a/../rel/<KEY>`, on
the command line, in an `-xcconfig` and in the project. The three layers
behave the same.

- `SYMROOT`, `OBJROOT`, `DSTROOT`, `CONFIGURATION_BUILD_DIR`,
  `BUILT_PRODUCTS_DIR`, `CONFIGURATION_TEMP_DIR`, `TARGET_TEMP_DIR`,
  `TEMP_DIR`, `SHARED_PRECOMPS_DIR`, `LOCROOT` and `LOCSYMROOT` fold, and a
  relative value is read against the project's directory first. An
  `-xcconfig` elsewhere reads it against the project too, not against the
  file.
- `TARGET_BUILD_DIR` and `INSTALL_DIR` fold but stay relative.
- Every other path setting keeps its spelling, `BUILD_DIR` included, and so
  does a setting of your own.
- A setting built from a folded one sees the folded value: `BUILD_DIR` from
  `SYMROOT`, `DERIVED_FILE_DIR` from `TARGET_TEMP_DIR`. `TARGET_BUILD_DIR` is
  the exception. A value set for it is reported folded, while
  `CODESIGNING_FOLDER_PATH` and `METAL_LIBRARY_OUTPUT_DIR` keep the spelling
  it was given.

The resolver resolves once, then pins each changed location to its folded
value on top of the stack and resolves again, in the order `xcodebuild`
settles them, so the settings built from it follow. The default layout is
already folded, so a project that moves nothing pays for one resolve. The
same measurements found three recipes the resolver had wrong once a location
moves, and they follow CoreBuildSystem.xcspec now: `TEMP_ROOT` is
`$(OBJROOT)`, `INSTALL_ROOT` is `$(DSTROOT)`, and `TARGET_BUILD_DIR` hangs
off `CONFIGURATION_BUILD_DIR` rather than `BUILT_PRODUCTS_DIR`. The
`build_location_fold_oracle` suite in sweetpad-core quotes the measured
values. One gap is left: with a scheme, `xcodebuild` keeps
`SHARED_PRECOMPS_DIR` under DerivedData's intermediates when `OBJROOT`
moves, where the resolver derives it from `OBJROOT`.

### The project path is spelled the way xcodebuild spells it

`xcodebuild` prints one spelling of the project's directory, however the
project is named. On Xcode 27, with the project at `/private/tmp/…/app`,
`-showBuildSettings` was run with `-project` as `/tmp/…/app/…`, as
`/private/tmp/…/app/…` and through a symlinked `/private/tmp/…/link`, both
absolute and relative from a directory reached the same way. All six printed
`PROJECT_DIR = /tmp/…/app`. With `SYMROOT=build` they printed
`SYMROOT = /tmp/…/app/build`. A symlinked checkout under `/Users` printed its
real directory. That is the standardized path DerivedData is hashed over
(symlinks resolved, a leading `/private` dropped), so the resolver gives
`PROJECT_DIR`, `SRCROOT` and `PROJECT_FILE_PATH` that path. Every location
read against them follows. It used the canonical path, so `settings show`
printed `/private/tmp/…` for a project under `/tmp`.

A `-derivedDataPath` always loses the `/private`, and whether it keeps its
symlinks depends on whether the directory exists. `/private/tmp/…/link/dd`
printed `BUILD_DIR = /tmp/…/link/dd/Build/Products`, symlink kept. Once that
run had created the directory, the same command printed
`/tmp/…/app/dd/Build/Products`. A path that doesn't exist has `..` collapsed
as written: `…/link/../dd` printed `…/dd`. The resolver follows both
(`derived_data_spelling`). A relative one is read against the physical
directory `xcodebuild` runs in. With the project opened through `link` and
`xcodebuild` run there, `dd` printed `/tmp/…/app/dd` and `../dd` printed the
directory beside `app`, before and after either existed. The CLI joins a
relative path onto the standardized project directory for that reason.
Joined onto `link` as spelled, `settings show` printed `…/link/dd`, and put
`../dd` beside `link` instead. A setting given an absolute path keeps its
spelling: `SYMROOT=/private/tmp/…/sym` printed as typed. The BSP server's
source lists stay on the canonical project directory, and it compares
spellings when it matches a file, so the index finds a file either way. The
sweetpad-core test `project_paths_take_the_spelling_xcodebuild_prints`
quotes the measurements.

### A project keys DerivedData by itself

`xcodebuild -project App.xcodeproj build` builds into `App-<hash>`, whose
`info.plist` names the `.xcodeproj` as its `WorkspacePath`, even when an
`.xcworkspace` beside it lists the project (Xcode 27). The resolver guessed
otherwise: with no workspace declared, it looked for one beside or above the
project and keyed DerivedData by it. So `settings show --project` named the
workspace's folder, while `build --project` and the BSP index read the
project's. The guess is gone. The resolver keys DerivedData by the project
unless a workspace is declared, as `--workspace` and a workspace BSP root
declare it, and all three agree with `xcodebuild`. The corpus oracle declares
the workspace its captures were taken through, and its canonical score rose
from 97% to 99%. The per-target and project-defaults suites, captured with
`-project`, rose too.

### The app the Run action launches

A scheme can build more than one app: a helper app, a second app, or a share
extension listed ahead of the app it runs. The locator took the first `.app`
whose `SUPPORTED_PLATFORMS` covers the destination. On Xcode 27, a scheme that
builds `LocMac` and `LocMacHelper` and runs `LocMacHelper` reported
`LocMac.app` as `build -o json`'s `productPath`, and `app launch --mac`
started `LocMac`. The locator now takes the target the scheme's Run action
launches (its `BuildableProductRunnable`, or a watch scheme's
`RemoteRunnable`) when that target builds a `.app` that can run on the
destination. Otherwise the destination's platform picks, as before, so a
watch scheme run on an iPhone simulator still gets the iPhone app. A scheme
with no file (an autocreated one) has no Run action to read and keeps the old
pick.

### The extension locates the app with the same code

The VS Code extension found the app with its own copy of the rules, and the
settings it read ignored `sweetpad.build.args`. The build applies those
arguments, so a `PRODUCT_NAME=`, `PRODUCT_BUNDLE_IDENTIFIER=`, `SYMROOT=` or
`-xcconfig` there built one bundle and launched another, or none. A
`-configuration Release` there built Release and looked for Debug. The
locator now lives in sweetpad-core (`app_locator`). The CLI calls it, and the
addon exposes it as `locateApp`. That call takes the extension's build options
plus the raw build arguments and the directory the build runs `xcodebuild`
in. It reads the arguments the way the extension's build reads them. The
settings and `-xcconfig` go into the resolver, as a CLI passthrough does. A
`-scheme`, `-configuration` or `-derivedDataPath` there replaces the
extension's own, as it does on the build's command line. The extension's
`buildSettings` queries take the same arguments. With a customized
`sweetpad.build.xcodebuildCommand`, the extension asks `xcodebuild
-showBuildSettings` with the build's arguments and picks with `pickApp`,
which applies the same rules to settings resolved outside the addon. The RPC
server's `appPath.find` and `bundleId.get` answer from the locator as well.
They took the first target with a wrapper or a bundle id, which could be a
framework, a test bundle or the watch app.

A macOS command-line tool builds no `.app`. The extension runs one on the Mac,
so the locator returns it, marked as a tool, when a scheme builds no app and
the destination is the Mac. The CLI's `app` verbs install and launch bundles,
so they still refuse it.

### The extension's destination picker filters like the CLI's

The extension split its destination list into supported and other platforms
by the `SUPPORTED_PLATFORMS` it resolved with no destination, and the addon
bound such a resolution to `macosx` and `arm64`. An iOS app that authors only
`SDKROOT = iphoneos`, as Xcode's and XcodeGen's templates do, resolved to
`macosx`. The picker then listed My Mac as the only supported destination and
put every iPhone simulator under "Other". A scheme with more than one target
got no split at all. The picker now reads the CLI's `SupportedPlatforms`,
moved to sweetpad-core and exposed as `supportedPlatforms`. It unions the
scheme's targets' authored `SUPPORTED_PLATFORMS`, or the platforms their
`SDKROOT` implies. The addon no longer defaults a resolution's SDK and arch.
With neither an SDK nor a destination, each target resolves under its own
`SDKROOT`, as `xcodebuild -showBuildSettings` and `settings show` resolve it.

## 9t. v8 — `feedback`: an agent's problem report to the maintainer

The agents that drive sweetpad hit its bugs first: a tip that doesn't work, a
gap that sends them back to raw `xcodebuild`, help that disagrees with what a
command does. Without a way to send them, those findings stay in the
agent's session. `feedback` lets the agent send one to the maintainer, with
the user's approval, after the agent has taken the user's names out of it.

```
sweetpad feedback submit <FILE> --dry-run
sweetpad feedback submit <FILE> --approve <DIGEST>
sweetpad feedback off | on | status
sweetpad help feedback
```

**Two approvals, and no prompt.** The CLI never reads stdin, because an agent
must not block on it; the approvals are the agent asking the user. The agent
asks once per issue whether the user would like to send a report. If so, it
writes the report file and runs `--dry-run`, which prints the exact payload
and its digest and sends nothing. The agent shows that to the user, and only
after the user approves it runs `--approve <digest>`. That rebuilds the
payload from the file and sends only if the digest still matches; otherwise
it exits 1 without sending and asks for a fresh dry run. The refusal does
not print the new digest, so an agent can't send a payload the user never
saw by copying it from the error. `submit` with neither flag is a usage error
pointing at `--dry-run`; clap refuses both at once.

**The agent cleans the report; sweetpad doesn't scrub it.** The agent knows
which names in the text are the user's, and a scrubber in the CLI would only
guess. `help feedback` and `skills/sweetpad/SKILL.md` list what to remove
(project, workspace, scheme, target and package names; bundle and team ids;
home and other absolute paths; device names and UDIDs; user and host names;
emails; private URLs; tokens) and what to keep (versions, the command with
its values replaced by placeholders, the exact error text with names
replaced). The same two say when to offer a report: sweetpad crashed, hung
or reported an internal error; its output contradicted itself or Xcode; a
tip or command it suggested didn't work; a gap forced a fall-back to raw
`xcodebuild`, `simctl` or `devicectl`; help or docs disagreed with behavior.
Not for the user's own problems (compile errors, failing tests, signing
setup, missing runtimes, a mistyped command sweetpad refused clearly, an
environment problem it reported correctly). Offer once per issue, and never
again once declined. Error messages elsewhere don't mention `feedback`.

**The file is a sweetpad-feedback log entry**, so an agent can send one it
already logged:

```
## 2026-09-27T10:00Z · bug · medium
- **Seen:** 1× (2026-09-27T10:00Z)
- **Context:** an iOS app in a workspace; the user asked to run it on a simulator
- **Command:** `sweetpad run --on <simulator> --no-logs`
- **Expected:** the app launches and the command returns
- **Actual:** exit 1: `error: couldn't find the built app for <scheme>`
- **Assumption or gap:** the build succeeded, so the app should be where the build wrote it
- **Fix idea:** look for the app under the build's -derivedDataPath
```

The heading gives the kind (`bug`, `gap`, `unclear`, `skill-wrong`,
`docs-wrong`, `friction`; the log's `correction` and `resolved` describe the
log itself and are refused) and the severity (`low`, `medium`, `high`); its
timestamp may be left out. Context, Command, Expected, Actual and Assumption
or gap are required, Fix idea is optional, and field names match without
case. A value runs to the next field line, so it may wrap or hold a code
block. The heading's timestamp and the `Seen:` line, sub-bullets included,
are read and not sent: they carry dates, and the sub-bullets name the other
projects an issue turned up in. An unknown field, text before the first
field, a repeated field and a second heading are usage errors naming the
line, and a report missing fields names every one. The message the fields
become is capped at the 4096 characters Sentry keeps.

**Where it goes.** The VS Code extension's Sentry project: its DSN from
`sweetpad-vscode/.env.example` is a constant (a DSN key is a public client
key). The send is one `POST` of an envelope to
`https://o325723.ingest.us.sentry.io/api/4507950563328000/envelope/` with
`X-Sentry-Auth` carrying the key, holding one `feedback` item. `minreq` with
`native-tls` (Security.framework) makes the request, with a 10-second
deadline covering connect and read. A failure exits 1 and says what
happened: `couldn't send the report: no answer from <host> within 10
seconds`, `… can't reach <host>: <io error>`, or `the report was not
accepted: <host> answered HTTP <code> <reason>: <body>`.

**Through a proxy.** The send honors `https_proxy` and `HTTPS_PROXY`, read
in that order as curl reads them, unless `no_proxy` or `NO_PROXY` exempts
the host. A `no_proxy` entry is `*` or a host name, and it covers the hosts
under it with or without a leading dot. The proxy is an HTTP one, written
`http://host:port` or `http://user:password@host:port` (port 1080 when
none is given). `minreq`'s `proxy` feature opens a `CONNECT` tunnel through
it, with Basic credentials when the URL has them, and TLS runs inside the
tunnel. The feature adds one crate, `base64` 0.22 with no dependencies, and
about 36 KB to the release binary. Left to itself, `minreq` applies the
lower-case `https_proxy` to any request that names no proxy and reads no
`no_proxy`, so `send` removes `https_proxy`, `http_proxy` and `all_proxy`
from its own environment after choosing, and names the proxy on the request.
A value it can't use (`socks5://`, `https://`, a bad port) refuses the send
without echoing the value, which can hold a password. The proxy's answers
have their own messages: `the proxy at <host:port> (HTTPS_PROXY) wouldn't open
a connection to <host>` for a refusal, `… asks for credentials; put them in
HTTPS_PROXY as 'http://user:password@<host:port>'` for a 407 without them,
and `… refused its credentials` for one with them. `-o json` returns
`{sent: true, eventId, digest}` from the send and `{sent: false, digest,
endpoint, payload, addedAtSend: ["timestamp"], added: {sweetpad, xcode,
macos, arch}, sendCommand}` from the dry run.

The item is an event with the report in `contexts.feedback.message`, tagged
so the CLI's reports are apart from the extension's errors:

```json
{
  "contexts": {
    "feedback": { "message": "bug · medium\n\nContext: …\nCommand: …\nExpected: …\nActual: …\nAssumption or gap: …\nFix idea: …" },
    "os": { "build": "26A428", "name": "macOS", "version": "27.0" }
  },
  "environment": "development",
  "event_id": "7acaa8e48af249a3a7b64d34b945988f",
  "level": "info",
  "platform": "other",
  "release": "sweetpad-cli@0.1.10-dev+c6bdf0a0",
  "sdk": { "name": "sweetpad-cli", "settings": { "infer_ip": "never" }, "version": "0.1.10-dev+c6bdf0a0" },
  "tags": { "arch": "arm64", "kind": "bug", "severity": "medium", "source": "cli", "xcode": "27.0 (27A266a)" }
}
```

`environment` is `development` for a `-dev+<sha>` build and `production` for a
tagged one. The envelope header holds `event_id` and the `sdk` name and
version; the item header holds `type: "feedback"` and the length; the event
gains `timestamp` (Unix seconds) at send time. Xcode's version comes from the
active install's `version.plist`, macOS's from `sysctl`
(`kern.osproductversion`, `kern.osversion`), the arch from the running
binary. Nothing names the Mac, the user or a path.

**No IP address, set on the client.** Relay fills `user.ip_address` from the
connection in two cases: the event sends `{{auto}}` (or
`sdk.settings.infer_ip: "auto"`), or, under the default `legacy` setting, the
platform is `javascript`, `cocoa` or `objc` and the event names no IP
(`normalize_ip_addresses` in `relay-event-normalization/src/event.rs`, per
develop.sentry.dev's SDK `settings.infer_ip`). The payload sends
`platform: "other"`, `infer_ip: "never"`, and no `user` object, so neither
applies. The location is a separate lookup: `normalize_user_geoinfo` fills
`user.geo` (country, region, city) from the connection's address for every
event that has no geo, and it is handed the unfiltered client address, so
neither `infer_ip` nor the project's "Prevent Storing of IP Addresses"
reaches it. No client field stops it reliably: `user.geo` is skipped when
empty, so a placeholder `{}` would vanish at the first Relay that
re-serializes the event. The server-side switch is an Advanced Data Scrubbing
rule removing `$user.geo`. "Prevent Storing of IP Addresses" is still worth
turning on as a second line: besides the known IP fields, it replaces
IP-looking text in every string field, which covers an address an agent
left in pasted output. It does nothing for the location.

**The digest.** SHA-256 of the compact JSON `{"endpoint": <url>, "event":
<the event without event_id or timestamp>}`; `serde_json` sorts keys, so the
bytes are the same on every run. The digest is the hash's first 16 hex
digits. `event_id` is its last 32, with the version and variant digits set
so it reads as the UUID v4 Sentry's feedback spec asks for, and the dry run
shows the event id the send will use. Everything the dry run prints is
covered, the added versions and the endpoint included: an Xcode update
between the two steps is a mismatch, and so is a different endpoint. `timestamp` is added at send time
and is not hashed. The hash is CommonCrypto's `CC_SHA256` from libSystem,
which every macOS process links, so it adds no crate; nothing in the
dependency tree had one.

**The off switch** is `[feedback] enabled = false` in config.toml; unset means
on. `off` writes it, creating the table or the file when missing; `on` sets
an `enabled` key that exists to true and otherwise touches nothing, so it
never creates a config file. Both edit through `toml_edit` (already in the
tree under `toml`), keep every other line and comment, and write a temporary
file beside the config and rename it over, at the target when config.toml is
a symlink. While reports are off, `submit` exits 1 in both modes, and `help
feedback` prints only that the user turned them off, that the agent shouldn't
offer one, and that `feedback on` turns them back on. A config that doesn't
parse also makes `submit` refuse, since it may be the file that turned
reports off.

**Testing.** The hidden `SWEETPAD_FEEDBACK_URL` points the send at another
envelope URL. `tests/feedback.rs` runs every command against a
`std::net::TcpListener` stub on 127.0.0.1 with its own HOME and XDG
directories, and no test reaches Sentry: the dry run's shape, the sent
envelope equal to the dry run's payload plus `timestamp`, no `ip_address` or
`{{auto}}` anywhere, a changed file refused with nothing received, off/on/status
keeping the config's comments, the turned-off help text, malformed reports,
a refused connection, and an HTTP 429. The proxy tests send to
`https://sentry.invalid/…`, which no resolver answers for, with the stub as
the proxy: the `CONNECT sentry.invalid:443` it receives and its Basic
credentials, a 403 and a 407, `no_proxy` sending around a lower-case
`https_proxy` (nothing reaches the stub), and a `socks5://` value refused.

*Deliberately not built:* scrubbing in the CLI; a prompt; reading the report
from stdin; attachments or logs; the VS Code extension; SOCKS proxies and
proxies reached over HTTPS, which `minreq` doesn't support.

## 9u. v8 — debugging from any editor (`dap`)

> Asked for in sweetpad-dev/sweetpad#340. `dap` ships in a CLI release first;
> the extension then switches to it by default (the last part of this section).

Debugging an app goes through one of two doors. The VS Code extension builds
and launches through its own pre-launch task, then hands an attach config to
CodeLLDB (`vadimcn.vscode-lldb`). The CLI has `app debug`, an interactive or
`--batch` lldb session in the terminal. Neither reaches Neovim, Zed, Helix or
Emacs, whose debug UIs all speak the Debug Adapter Protocol and expect an
adapter to get the process running.

Xcode already ships the adapter half: `xcrun lldb-dap`, LLVM's DAP server
linked against Apple's LLDB, so Swift variables and expressions behave as they
do in Xcode with no `lldb.library` setup. What it can't do is build a scheme or
start an app on a simulator or device; it expects a program to launch or a pid
to attach to. That gap is exactly the part sweetpad already owns.

```
sweetpad dap                        # serve one debug session on stdio
sweetpad dap init --editor nvim|zed # write the editor's adapter entry
sweetpad dap doctor                 # can this machine serve one?
```

**`sweetpad dap` is an adapter in front of `lldb-dap`.** The editor starts it
over stdio; it starts lldb-dap as a child at the editor's `initialize` and
forwards traffic both ways. Four requests are intercepted:

- `initialize` is forwarded with `pathFormat` and `adapterID` filled in when
  the editor left them out, since Xcode's lldb-dap refuses an `initialize`
  without `pathFormat` although the protocol makes it optional. The reply's
  capabilities lose `supportsRestartRequest`.
- `launch` is never forwarded as sent. Its fields resolve to the same
  `RunPlan` `run` uses; the adapter builds, installs and starts the app, then
  sends lldb-dap the request that reaches it, under the editor's `seq`, and
  relabels lldb-dap's reply with the editor's command.
- `attach` reaches an app that is already running, with no build: a `pid`
  alone is attached to as a process on this Mac, which covers simulator apps;
  otherwise the plan's app is located and its running process found.
- `disconnect` is forwarded, then the app and its log stream stop, unless the
  editor sent `terminateDebuggee: false`. An editor's `launch` that lldb-dap
  received as an `attach` goes out with `terminateDebuggee: true`, since
  lldb-dap detaches from what it attached to by default. Once lldb-dap
  answers, the session ends.

A `launch` or `attach` that names a `program` is an lldb-dap configuration
(a Swift package's binary, say) and goes through untouched, so an editor that
sends every Swift session through this adapter still debugs those. Everything
else (breakpoints, stepping, `stackTrace`, `variables`, `evaluate`) passes
through. `restart` is refused with the way to restart.

**Each destination reaches lldb-dap its own way:**

| destination | started by | lldb-dap is sent |
|---|---|---|
| simulator | `simctl launch --console-pty --wait-for-debugger` | `attach {pid, program}` |
| macOS, Mac Catalyst | lldb-dap itself | `launch {program, args, env}` |
| device (iOS 17+) | `devicectl … launch --start-stopped` | `attach` with `attachCommands` |
| Designed for iPad | `open` (LaunchServices), after configurationDone | `attach {program, waitFor}` |

The simulator's console child stays up for the session: killing it with a
catchable signal takes the app down too, so a detach ends it with SIGKILL. On
a device the pid comes from `devicectl device info processes`, and lldb
selects `remote-ios`, maps its copy of the app onto the device's with
`SetPlatformFileSpec`, and attaches with `device select` and `device process
attach --pid`, the route the extension's `debugger/provider.ts` takes, run
through `script lldb.debugger.HandleCommand` as the extension runs them. The
attach stops the app with a SIGSTOP that lldb-dap reports only after it has
resumed the app, as a stop nobody asked for, so `postRunCommands` sets SIGSTOP
to pass silently once the attach is done. A device on iOS 16 or older is
refused before the build, since devicectl can't drive it. When this Mac has no
copy of the device's system libraries under `iOS DeviceSupport`, lldb reads
each one out of the device's memory as the app loads it, which over a network
connection took more than five minutes on an iPhone 13, against 22 seconds for
the whole session once the copy existed. The console says so up front: Xcode makes the copy the
first time it runs an app on the device, and devicectl has no command for it. An iOS app on the Mac runs only when LaunchServices starts it, so lldb-dap
waits for its executable by name and the adapter opens the app half a second
after forwarding `configurationDone`, with its stdout and stderr in a file the
adapter follows. That last route is written from the extension's waitFor
pattern and hasn't run on hardware: building for Designed for iPad needs a
registered Mac.

Apple's lldb-dap answers `launch` and `attach` at once, attaching (or setting
up the launch) before it replies, and only then sends `initialized`. It
numbers every message `seq: 0`.

**Sequence numbers are renumbered in one direction.** The adapter injects
messages of its own (build output, app logs), and lldb-dap numbers nothing, so
everything flowing to the editor carries the adapter's `seq`. Toward lldb-dap
the editor's `seq` is kept, and the rewritten `launch` reuses the original's,
so lldb-dap's `request_seq` already matches what the editor asked. Reverse
requests from lldb-dap (`runInTerminal`) are renumbered toward the editor and
the editor's replies mapped back to lldb-dap's numbers.

**The launch config mirrors `run`'s flags:**

```json
{ "type": "sweetpad", "request": "launch", "scheme": "MyApp", "configuration": "Debug",
  "destination": "iPhone 18 Pro", "args": [], "env": {}, "xcodebuildArgs": [],
  "cwd": "${workspaceFolder}", "lldb": { "sourceMap": [] } }
```

`destination` is the `--on` reference, so `booted`, `mac`, a name or a UDID
resolve as they do for `run`; a value with an `=` is a raw `-destination`
specifier, which is how Mac Catalyst (`platform=macOS,variant=Mac Catalyst`)
and Designed for iPad (`platform=macOS,variant=Designed for iPad`) are named.
`env` is an object or a list of `KEY=VALUE`; `xcodebuildArgs` is `run`'s `--`
tail; `cwd` is where the project is found from; `workspace` and `project`
name a container; `lldb` goes to lldb-dap untouched, with the adapter's own
command lists (`initCommands`, `preRunCommands`) ahead of the editor's. A field
of the wrong type fails the launch and names the field. Missing fields fall
back to the configured and remembered scheme and destination. A debug session
has no terminal, so it never prompts: with nothing explicit and nothing
remembered, `launch` fails naming the field to set (listing the schemes for a
scheme) and `sweetpad run` as the way to pick once. The extension shows its own
picker and passes the result.

**The adapter owns its stdio before anything can write to it.** The editor
speaks DAP over fds 0 and 1, and the rest of the CLI writes freely: notes and
warnings to stderr, app logs to stdout, and spawned tools inherit both. So the
protocol moves to close-on-exec copies of fds 0 and 1, fd 0 becomes
`/dev/null` so no child reads the editor's messages, and fds 1 and 2 become
pipes whose lines turn into `output` events: fd 1 as `stdout` (app logs), fd 2
as `console` (the CLI's own lines). The `Output` is rebuilt non-interactive and
colorless, so nothing prompts, animates or colors.

**Build and app output go to the debug console.** The build streams as
`output` events in the short build-log form, errors and warnings carrying
`source`, `line` and `column` so editors link them, inside a cancellable
`progressStart`/`progressEnd` whose updates name the file compiling. A failed
build's launch error repeats the first errors, as `build start` does when its
stderr goes elsewhere. Simulator stdout and stderr arrive through the console
child, and os_log through the log stream `run` follows, at `run`'s default
level. On macOS lldb launched the process and relays its stdout itself, so only
the log stream is added. The lines about the launch rather than the app are
left out as `run` leaves them out: the simulator's boot noise, AppKit's note on
the `-ApplePersistenceIgnoreState` sweetpad adds, and the lines Xcode's
debug-dylib stub writes whenever a debugger starts the app. A `disconnect`,
`terminate` or cancelled progress during the build interrupts the `xcodebuild`
the adapter started, and the steps after it don't start.

**Restart and hot reload wait.** v1 advertises no `restart`, and VS Code and
nvim-dap fall back to disconnect-and-launch, which already rebuilds. A native
restart means a fresh lldb-dap child and replaying every breakpoint request the
editor sent. `--hot` stays out of debug sessions until injection with a
debugger attached is tested.

**The surface follows `bsp`.** `dap init --editor nvim` writes a marked block
into the project's `.nvim.lua` (read with `exrc` on) that registers the
adapter and a launch configuration for Swift, Objective-C, C and C++, and a
rerun replaces the block. `dap init --editor zed` adds a scenario to
`.zed/debug.json` for the Swift extension's adapter, whose binary Zed's
settings can override (`"dap": {"Swift": {"binary": …}}`) but which it starts
with no arguments. The binary run as `sweetpad-dap` therefore serves as `sweetpad
dap` does, the Homebrew formula links that name, and `dap init` prints the
settings line with it. `dap doctor` checks that `xcrun lldb-dap` resolves
(following `DEVELOPER_DIR` and `xcode-select`), that Xcode is 16 or later, and
that lldb-dap answers `initialize`. `SWEETPAD_LLDB_DAP` names another lldb-dap
(a newer LLVM's, a wrapper), and `SWEETPAD_DAP_LOG` names a file that records
the full DAP traffic both ways, as `SWEETPAD_BSP_LOG` does for BSP.

**It resolves its plan the way §9n wants.** The launch half lives in
`app::adapter`, a child of `app`, so it plans, builds, installs and follows
logs through `run`'s own code rather than a copy. When the run session becomes
a server the adapter becomes one of its clients instead of a second owner.

**The extension switches to it by default.** Registering `sweetpad-lldb` with
a `DebugAdapterDescriptorFactory` that returns `DebugAdapterExecutable(<cli>,
["dap"])` keeps users' `launch.json` unchanged and drops the pre-launch task on
that route. Once a CLI release carries `dap`, it is the extension's default
debugger, and CodeLLDB leaves `extensionDependencies` to become optional: the
fallback when the CLI is missing or too old to have `dap`, the route for
devices older than iOS 17, or the route a setting picks explicitly.
`codelldbAttributes` is honoured only on that route. With neither the CLI nor
CodeLLDB installed, starting a debug session explains both options instead of
failing silently.

**After v1:** native restart (rebuild, relaunch, reattach, breakpoints
replayed), debugging tests by attaching to a suspended test runner, hot reload
inside a debug session, device stdout (devicectl's `--console` with
`--start-stopped`), and attaching to the §9n run session as one of its
clients. Helix and Emacs (dape) get `dap init` once it's clear how each
registers a custom adapter and what Helix's client supports.

**Testing.** `tests/dap_proxy.rs` runs the adapter against a fake lldb-dap
that echoes what it receives: the renumbering, the filled-in `initialize`, the
trimmed capabilities, an attach by pid, the reverse-request routing, the
`program` passthrough, and launches that fail before any build. `ci/smoke.sh`
drives one real session per destination kind it has through
`ci/dap-session.py`, a small DAP client: a launch on the simulator and on
macOS, a breakpoint, the `stopped` event and its frame, a disconnect, and the
adapter exiting.

## 10. Testing

The CLI modules carry inline `#[cfg(test)]` units that need no Xcode, so the
tool-spawning code is pinned without a Mac:

- **Arg-vector snapshots** — `BuildPlan`/`TestPlan` produce exact `xcodebuild`
  argument vectors (the main guard against silent flag drift).
- **Parser fixtures** — `simctl list`, `devicectl list`, `xcresulttool`
  summary, and `-showBuildSettings` JSON parsed from captured-shape payloads
  (this caught a missing `rename_all` on the devicectl device struct), and the
  `app diagnose` lldb transcript → outcome parse (§9j) against real `lldb -b -Q`
  output for an ObjC throw, a signal crash, and a clean exit. The `app sample`
  verdict (§9r) runs against trimmed real `sample` reports in
  `fixtures/sample/`: idle on macOS, in a simulator and under a modal alert;
  busy; blocked on a semaphore, mutex, unfair lock, condition and
  `dispatch_sync`; and a swallowed exception.
- **Pure logic** — resolution precedence, config/state TOML round-trips,
  `choose` fallback branches, destination/`udid` parsing, and the session
  key → action mapping (`r` rebuild / `q`·Ctrl-C·EOF quit / else ignore).
- **Inject protocol** (§9d) — the little-endian `int`/`string`/`data` framing
  and the handshake parse (version + platform/arch/projectRoot/tmpPath) round-
  trip against captured byte sequences; the resolver→single-file→dylib argv
  transform is an arg-vector snapshot, like `BuildPlan`. The live-injection
  truth (a save actually swaps in the running sim) lands in the `cli-smoke` job.

The *runtime* truth (does xcodebuild actually build, does the log
predicate/console attach behave) is exercised by the `cli-smoke` macOS job.
Its script, `sweetpad-lib/ci/smoke.sh`, runs on a dev Mac as well. It edits
scratch copies only, never a tracked file. The simulator it boots and erases
is one it creates for the run and deletes on exit, or the one
`SWEETPAD_SMOKE_DEST` names (`platform=iOS Simulator,id=<UDID>`); it never
picks a shared simulator on its own. On exit it also purges the DerivedData
its scratch projects built into. It points `XDG_STATE_HOME`, `XDG_CONFIG_HOME`
and `XDG_CACHE_HOME` at a directory of its own for the run and removes that
too, so it reads none of the user's config and writes none of their state.
Runs from two checkouts can go at once: each builds into its own DerivedData,
the CLI finds a running macOS app by its executable path, and the macOS app
the script scaffolds takes a per-run bundle id. The committed fixture's
`dev.sweetpad.ci.mac` is shared, which is what macOS keys the app's
preferences and `app logs --exits` its exit records on. Two runs from one
checkout share its DerivedData, so they go one at a time.

## 11. Build-log beautifier

`build`/`test` output is beautified natively (no `xcbeautify` dependency):
[`buildlog`] parses each raw `xcodebuild` line into a structured [`buildlog::Event`]
(compile/link/sign/diagnostic/test/result), then renders a concise, colorized
stream. Parsing is decoupled from rendering so the events can also feed CI
summaries or diagnostics later. `-v` passes raw output through; `--json` stays
quiet. `parse_line` is pure and unit-tested without Xcode.

## 12. Open / later

- SPM `app run` runs executable products on the host via `swift run <product>`
  (`--device`/`--mac` don't apply; library packages have nothing to run).
- Whether the extension actually adopts the CLI as its engine.
- (Declined for now: `tools` resource, `config`/`state` subcommands.)
