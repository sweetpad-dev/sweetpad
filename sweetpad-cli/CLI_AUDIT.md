# SweetPad CLI — surface audit & DX explorations

A whole-surface review of the standalone `sweetpad` CLI (the non-`vscode`
half of the binary), done against the implementation on `main` (July 2026).
Method: full read of `src/cli/mod.rs` + every `src/cli/commands/*.rs`, a
recursive `--help` dump of the built binary, live probes of the pure-Rust
paths (scaffold → info/scheme/settings/dependency/bsp/context on a generated
project), and a comparison against `CLI_DESIGN.md`, the VS Code extension's
command/settings surface, and the wider ecosystem (fastlane, tuist, simctl,
xcresultparser, xcodes; DX benchmarks: `flutter run`, `cargo`, `gh`, `fly`).

Every item has a unique number (§N.M). Sections 1–2 are findings (verified
bugs, contract holes); sections 3–9 are proposals, tagged **[bet]**
(surface-shaping, decide before v1 freezes), **[add]** (additive, any time),
or **[small]** (afternoon-sized). Section 10 sketches the resulting surface;
section 11 is the priority rollup.

The DX metrics behind the proposals: **keystrokes to first run**, **time to
feedback**, **guessability without docs**, and **agent-parseability** —
sweetpad explicitly serves AI agents, so the last is unusually load-bearing.

---

## 1. Bugs (verified)

**1.1 SIGPIPE panic on any piped command.** `sweetpad settings show --json |
head` panics (`failed printing to stdout: Broken pipe`, exit 101, backtrace
on stderr). Reproduced live. Any consumer that closes the pipe early
(`head`, `grep -m1`, a pager quit) hits it. Fix: restore `SIGPIPE`'s default
disposition at startup (`libc::signal(SIGPIPE, SIG_DFL)` in
`src/bin/sweetpad.rs`) or route `Output` through a BrokenPipe-tolerant
writer.

**1.2 `derived-data purge --json` deletes without confirmation and without
`--yes`.** The gate is `!yes && ctx.out.is_interactive()`
(`commands/derived_data.rs:153`), and `--json` forces non-interactive — a
scripted `purge --all --json` silently rm-rf's the whole DerivedData store.
Non-interactive destructive actions should *require* `--yes`.

**1.3 `SWEETPAD_WORKSPACE` env silently beats an explicit `--project`
flag.** `resolve::container` checks workspace before project
(`resolve.rs:52-57`) and clap's `env =` folds the env var into the flag
layer, with no `conflicts_with` (`mod.rs:113-121`) — inverting the
documented `flag > env` precedence (CLI_DESIGN §5). Fix: error on both-set
from different sources, or prefer the actual CLI token.

**1.4 A corrupt `state.toml` is silently wiped.** `State::load()` errors are
swallowed with `unwrap_or_default()` (`mod.rs:349`); the next best-effort
`save()` rewrites the whole file from the near-empty in-memory state — every
project's remembered context lost without a warning. Compounding it,
`save()` is a plain non-atomic `fs::write` with no lock
(`state.rs:153-162`), so two concurrent sessions do last-writer-wins and can
produce exactly such a torn file. Fix: temp-file + rename; warn (and back
up) on parse failure instead of defaulting.

**1.5 `app run --mac` / `--device` poison the remembered build context.**
`plan()` calls `resolve::remember` unconditionally
(`commands/app.rs:360-365`), persisting `platform=macOS` or the device
specifier into the *shared* build context — the next plain `build start`
silently targets the Mac/device. More generally `remember` persists
flag/env/config-sourced values too (`resolve.rs:333-340`), so state is not
"the last interactive picks" as `state.rs:3-5` and CLI_DESIGN §5 describe,
and a *failed* build still rewrites the context (remember happens at plan
time). Fix: only remember picker-sourced values; never remember
`--mac`/`--device` destinations.

**1.6 `dependency add` on a `Package.swift` mutates before it can fail.**
`add_to_xcode` has a "fail before mutating anything" non-interactive guard
(`commands/dependency.rs:374-378`); `add_to_package` does not —
non-interactive with no `--product`/`--target` it edits the manifest,
resolves, *then* dies in the product picker (`dependency.rs:433` →
`choose`), leaving a dangling manifest edit and a misclassified
`user_cancel` (exit 6). Fix: mirror the guard before step 1.

**1.7 `derived-data` scoping is broken for SPM containers.** The project
scope is the container path's `file_stem` (`derived_data.rs:234-241`), which
for every `Package.swift` is literally `"Package"` — `path`/`size` never
match the real `<DirName>-<hash>` folder Xcode creates, and `purge` would
match a foreign `Package-<hash>` entry. Fix: use the package directory name
for SPM containers.

**1.8 No signal handling outside the raw-mode session.** No SIGINT/SIGTERM
handler exists. Consequences: Ctrl-C exits 130, never the documented
`user_cancel` 6, and even *handled* aborts exit 0 (`BuildOutcome::Aborted` →
`Ok(Streamed)`, `app.rs:577,602,718,963`); a live spinner line is left on
stderr (Drop skipped); SIGTERM during a session leaves the terminal raw
(no-echo); the `--hot` *initial* build spawns xcodebuild into its own
process group before raw mode is on, so Ctrl-C kills the CLI but the build
keeps running detached (`process.rs:144-160`, `app.rs:714`); `simctl … log
stream` children are only reaped in `LogStream::Drop` (`app.rs:1024-1036`),
so SIGINT leaks them. Fix: one small SIGINT/SIGTERM handler — restore
termios, forward to the build pgid, run the Drop-equivalent reaps, exit 6
(or 130).

**1.9 Two dead context knobs.** `context select sdk` and `context select
target --testing` write state that *nothing reads*: `settings show`
hardcodes `sdk: String::new()` (`commands/settings.rs:115`), `BuildPlan` has
no sdk field, and neither `resolve_testing` (`resolve.rs:149-191`) nor
`test run` consumes `testing.target`. Users can set them, `context show`
displays them, and they do nothing. Fix: plumb them through (add `--sdk`;
make `test run` honor the target) or drop the variables.

**1.10 Misclassified exit codes.** (a) `app run` build failures exit 1, not
3: `build_and_install` never tags `BuildFailure` (`app.rs:409`; root cause:
`BuildPlan::run` returns an unclassified error, `xcodebuild.rs:80-82` —
classify there once). (b) `simulator boot NAME` with no match exits 1, not 4
(`commands/simulator.rs:141-148`), while the identical error via
`select_simulator` is classified (`resolve.rs:275`). (c) Device-resolution
errors (`app.rs:323,330,1918-1927`) are Generic, should be
`target_resolution`.

**1.11 `app logs --json` emits human-rendered log lines on stdout** — the
one command that silently ignores `--json` entirely (`app.rs:1849-1915`).
Either reject it like `app run` does, or emit JSON-lines events (see §7.2 —
the latter is what agents actually want).

**1.12 `--quiet` doesn't quiet the build.** The build spinner gates only on
`is_interactive()` (`buildlog.rs:250-254`), not `!quiet` like `Output::step`
(`output.rs:148`), and the beautified log stream (`out.line`) is never
muted — `sweetpad -q build start` animates and prints everything. `--hot`'s
status lines also print to *stdout* and ignore `--quiet`
(`app.rs:650-663`).

**1.13 Silent config typos, and a doc example that never matches.** No
`deny_unknown_fields` anywhere in `config.rs` — `[default]` instead of
`[defaults]`, or `schme = "App"`, parses cleanly and is ignored (verified
live). The design doc's example key `[projects."/Users/me/code/MyApp"]` (a
*directory*) never matches: the real key is the canonicalized **container**
path (`/…/MyApp.xcodeproj`, `resolve.rs:41-46`) — a user copying the doc
gets a silently dead override. Fix: warn on unknown keys and on
`[projects.*]` tables matching no known key; fix the doc example (or accept
directory keys).

**1.14 Configuration is never validated or prompted.** `build_target`
prompts for scheme and destination but silently defaults configuration to
`"Debug"` (`resolve.rs:312-315`) without consulting `configurations()` — a
project with only `UAT-Debug`/`Prod` invokes xcodebuild with a nonexistent
configuration. The candidates helper exists and is only used by
`context select`.

**1.15 Same-kind container ambiguity is nondeterministic.** Two
`.xcodeproj` in one directory resolve to the first `read_dir` entry
(`resolve.rs:77-87`) — filesystem order, no warning. Error or warn on
ambiguity.

**1.16 Minor.** The `derived-data purge` confirm is the only dialoguer use
without the color-aware theme (`derived_data.rs:162`) — ignores
`--no-color`. State entries for deleted projects and
`destination_recents`/`destination_usage` grow forever (no pruning,
`resolve.rs:390-400`). Canonicalize-failure falls back to a raw relative
path as the state key (cwd-dependent duplicates); case-insensitive APFS can
yield two keys for one project.

## 2. Contract holes (design promises vs code)

**2.1 "`--json` on any command" (§4) has exceptions.** `app run` rejects it
with a *Generic* error, exit 1 (`app.rs:253-258`) — fine to reject, but it
deserves a usage class. `app logs` ignores it (bug 1.11). `completions`
ignores it (harmless; document). Clap usage errors are human text on stderr
under `--json` (documented as clap-owned, but consumers should know). The
streaming commands' real fix is §7.2 (NDJSON).

**2.2 Envelope opacity.** A single global `"schema": 1` is shared by all
commands and errors — a consumer can't distinguish payload shapes, and no
`.md` documents a bump policy. Success JSON is pretty-printed, error JSON
compact — pick one. `ok: true` alongside non-zero exits (test failures exit
3, doctor problems exit 1, `format --check` exit 3) is intentional ("ok" =
command executed) but unstated — document it and ensure each such payload
carries its own status field. See §7.3 (`sweetpad schema`) for the
structural fix.

**2.3 The exit-code taxonomy is invisible.** 1/2/3/4/5/6 is a good taxonomy
no user can discover: not in `--help`, not in CLI_DESIGN.md, not in the docs
site. Ship it via §5.6 (help topics + man + docs reference).

**2.4 Under `--json`, child stderr interleaves.** Child processes always
inherit stderr (`process.rs:18,65,87`), so raw xcodebuild noise mixes with
the one structured error line. Consider capturing child stderr when
`--json`.

**2.5 Undocumented universal flags & env.** `--non-interactive` and
`-q/--quiet` are missing from CLI_DESIGN §8's universal-flag list;
`SWEETPAD_NONINTERACTIVE` from §5's env list; `CLICOLOR_FORCE`/`FORCE_COLOR`
are implemented (`output.rs:33-35`) but undocumented. `SWEETPAD_NONINTERACTIVE`
is checked with `is_some()` — `=0` still triggers it; standardize truthy
parsing (see also §9.12).

**2.6 Strict-mode error text lies in places.** `missing()` always says
"pass --{what} or set it in config" (`resolve.rs:196-201`) — wrong for
simulator/device (no such flag or config key) — and says "stdout is not a
TTY" while the gate is actually stderr-TTY/`--json`/non-interactive.

**2.7 The `vscode` namespace speaks a different dialect.** `sweetpad vscode
--help` prints a JSON *error* envelope (`code: "USAGE"`, no `schema` field)
with exit 0 — different envelope, different code taxonomy, help-as-error.
Align the envelope shape, or at least give `vscode --help` human output.

**2.8 Doc drift.** CLI_DESIGN §9 says SPM schemes come from `xcodebuild
-list -json`; the code reads the manifest via `swift package dump-package`
(`resolve.rs:217-220`). Update whichever is wrong.

## 3. Grammar & naming (surface-shaping)

**3.1 Verb-first for the dev loop, nouns for management. [bet]** The daily
loop is typed hundreds of times; management a few times a week.
Best-in-class CLIs are verb-first exactly where frequency is highest
(`cargo build/run/test`, `flutter run`) and resource-first where inventory
is the point (`gh pr`, `docker container`). Today the loop is `build start`
/ `app run` / `test run` — three tokens for the most common actions, the
flagship hidden under `app`, and the only surface where `start` and `run`
mean the same thing (the audit's verb inconsistency: `build start` vs
`test run`/`format run`/`app run`). Proposal — promote five verbs:

```
sweetpad run          # today: app run   (build + install + launch + logs, keys)
sweetpad build        # today: build start
sweetpad test         # today: test run
sweetpad clean        # NEW: xcodebuild clean; --purge adds derived-data (closes the
                      #      missing standalone-clean parity gap; today only build --clean)
sweetpad fmt          # today: format run  (fmt matches cargo/go; keep `format` as alias)
```

`app` stays as the lifecycle noun (`install/launch/logs/stop/uninstall`).
This kills `start`-vs-`run` by construction and halves tokens on the hot
path; keep `build start` etc. as hidden aliases for one release. Cheaper
fallback: default actions (`sweetpad build` ⇒ `build start`, `sweetpad app`
⇒ `app run`) — same keystroke win, grammar formally unchanged. Related
inconsistency to settle at the same time: `project info` vs
`settings show`/`context show` — pick one display verb.

**3.2 One `devices` view, not three list commands. [bet]**
`destination list`, `simulator list`, `device list` are three spellings of
"what can I run on", each shaped differently; `destination list` also has no
container tier, so it can't mark the project's remembered destination
(unlike `scheme list`, which marks the selection), and none of the three use
the adaptive most-used-first ordering the picker has. Proposal: `sweetpad
devices` — everything runnable (mac + sims + physical), each row with its
ready specifier, most-used-first, remembered one marked. `simulator` keeps
only lifecycle verbs; drop the `destination` and `device` resources (the
latter is already a strict subset). Flutter's `flutter devices` is the
model.

**3.3 Fold the merge trio into one resource. [bet]** `pbxproj resolve` +
`spm resolve` + `merge install/driver` is one feature across three top-level
nouns, with `resolve` colliding against `dependency resolve` — `dep resolve`
and `spm resolve` even touch the same `Package.resolved` for unrelated jobs.
Proposal:

```
sweetpad merge install [--global]
sweetpad merge run [PATHS…] [--force]   # auto-detects pbxproj vs Package.resolved
sweetpad merge driver <KIND> …          # hidden, unchanged
```

Two top-level resources removed, the verb collision gone, and the mental
model matches the feature.

**3.4 `context` → `use` + `status`. [bet]** `context select/show/remove` is
accurate but bureaucratic, and `select` is prompt-only — it strict-errors
off a TTY, so scripts/CI *cannot* seed remembered state at all. Proposal:

```
sweetpad use                                        # interactive: scheme → config → destination
sweetpad use --scheme App --dest "iPhone 16 Pro"    # non-interactive setter (closes the gap)
sweetpad use --testing --config Test
sweetpad use --clear [VAR]
sweetpad status                                     # container, scheme, config, destination,
                                                    # last app — WITH provenance (§4.5)
```

`rustup default`, `kubectl config use-context`, `nvm use` trained the verb.
Minimum viable version if the rename feels aggressive: add `context set
<VAR> <VALUE>` and the provenance column to `context show`.

**3.5 Removal candidates — subtract while it's cheap. [bet]** `device` and
`destination` resources (absorbed by §3.2); `pbxproj`/`spm` (absorbed by
§3.3); `derived-data` absorbed into `clean` (`clean --purge`, `clean
--path`, `clean --size`) or kept with a `dd` alias; `settings` → `project
settings`? (weak opinion — it's the one single-action noun with a real
payload). Net effect of §3.1–3.5: **20 top-level entries → ~13**, every
survivor a daily verb or a real inventory noun.

**3.6 One policy for `--force`/`--yes`/`--all`. [bet]** Today:
`derived-data purge --yes` = skip prompt; `project new --force` = skip
prompt *and* waive the non-empty-dir check; `pbxproj/spm resolve --force` =
redo work git finished. `context remove --all` = the whole context;
`derived-data --all` = widen scope to the global store; `dependency update`
spells "everything" by omitting the positional. Suggested rule: `--yes`
skips confirmation, `--force` overrides a safety check — and document it in
CLI_DESIGN §2.

**3.7 Deletion verbs.** `simulator erase` (domain-faithful, keep),
`context remove`/`dependency remove`, `derived-data purge` — three synonyms.
Fine individually; pick a policy and note it. (Moot for `purge` if §3.5
folds it into `clean`.)

**3.8 `--target` is overloaded four ways.** Build target (`settings`), link
target (`dependency`), testing context variable (`context`), simulator
positionals named `target`. Probably livable; worth a glossary line in the
design doc.

**3.9 Arity mismatch on identical flags.** `dependency add
--product/--target` are repeatable; `dependency remove --product/--target`
are single. Align or document.

**3.10 `--hot-recompiler` should be a `ValueEnum`. [small]** It's a
hand-parsed free string (`app.rs:52-53,94-100`) while every comparable flag
is an enum — typos become runtime errors and completions can't offer values.

**3.11 `context remove` no-arg should be a clap error. [small]** Neither
`VARIABLE` nor `--all` is a runtime error today (`context.rs:243-247`) —
express it as an `ArgGroup` so usage errors are clap's (exit 2, before any
I/O).

**3.12 Aliases. [small]** Only `dep` exists. `sim` (longest
frequently-typed resource) and `dd` (the only hyphenated name) earn one by
the same criterion; `fmt` if §3.1's rename doesn't happen. Error messages
already teach `dep` — keep doing that.

## 4. Frictionless targeting

**4.1 Human destination references — `--on`. [bet, highest leverage]**
`--destination "platform=iOS Simulator,name=iPhone 15"` is xcodebuild's
worst ergonomic export, and today the flag accepts *only* that raw form
(`resolve.rs:302`). Meanwhile "which simulator/device" is spelled three
ways across the surface (positional `[TARGET]` on `simulator` verbs,
`--simulator` on `app open-url`, `--device-id` on `app run`), plus
`--mac`/`--device` mode flags, `simulator appearance` puts the target
positional second while every sibling takes it first, and `boot`'s prompt
bypasses the adaptive most-used-first picker and records no usage
(`simulator.rs:144-149` vs `resolve.rs:367-385`). One flag resolves all of
it:

```
sweetpad run --on "iPhone 16 Pro"   # fuzzy name match over the devices list
sweetpad run --on booted            # the booted sim
sweetpad run --on mac / --on device # platform words (replaces --mac/--device)
sweetpad run --on 1A2B3C…           # UDID
sweetpad run --on ios               # newest iOS sim, most-used-first tiebreak
```

Resolved against the same aggregated device list (§3.2), erroring with a
did-you-mean table on ambiguity; `--destination` stays as the raw escape
hatch. Also removes bug 1.5's sharpest edge (no more mode flags to
mis-remember).

**4.2 Named device aliases. [add]** `sweetpad use --alias work-phone --on
00008120-…`, then `run --on work-phone`. Stored in state; listed in
`devices`. Cheap once §4.1 exists.

**4.3 Walk-up discovery, and `-C`. [small, verified gap]** Discovery is
cwd-only (`resolve.rs::discover`) — `sweetpad build` from `Sources/Feature/`
fails with "no .xcodeproj found". git, cargo, npm, flutter all walk up: walk
parents to the git root (or `/`), stopping at the first container. Pair
with a global `-C <dir>` (chdir like `git -C`) so CI never needs `cd`.

**4.4 A committed, project-local config. [bet]**
`~/.config/sweetpad/config.toml` keyed by absolute path is personal and
doesn't travel: a teammate cloning the repo gets nothing, and the abs-path
key breaks per checkout (bug 1.13). Allow an *optional, hand-authored,
committed* `sweetpad.toml` at the repo root:

```toml
scheme = "MyApp"
configuration = "Debug"
[testing]
configuration = "Test"
[format]
tool = "swiftlint"
```

Precedence slot between user-config and remembered state. This is how teams
standardize (mise, fastlane, swiftlint trained the pattern), it doesn't
violate "no files *written* to the project root" (sweetpad still never
writes it), and it gives `format`/`new`/`hot` their missing config home —
today the format tool, `project new` defaults (org bundle-id prefix,
platform), and the hot-reload recompiler/dylib defaults (design §9d's
declared "remaining nicety") have flags but no config anywhere.

**4.5 Resolution provenance everywhere. [small]** Precedence bugs (1.3) are
invisible because resolution is silent. In `status` (§3.4) and under `-v` on
any build-ish command, print *where* each value came from:

```
scheme         MyApp        (remembered — picked 2d ago)
configuration  Debug        (default — project has: Debug, Release, UAT)
destination    iPhone 16    (sweetpad.toml)
```

`git config --show-origin` / `go env` model; turns "why is it building the
wrong thing" from a bug report into a glance.

## 5. Help & everyday UX

**5.1 Group the help output. [small, big payoff]** Targeting and global
flags interleave arbitrarily today (`dependency add --help` lists `--from`,
`--workspace`, `--exact`, `--project`, …). `help_heading` on the tier
structs ("Target selection") and `GlobalArgs` ("Global"), plus
`display_order`, fixes every command at once: action flags → targeting →
global.

**5.2 Stop tier flags leaking onto actions that ignore them. [small]**
`project new` advertises `--workspace`/`--project` (it *creates* a project);
`app open-url` advertises `--scheme`/`--configuration`/`--destination` (it
resolves only a simulator); `scheme list` advertises `--scheme`. Meanwhile
`app logs`/`app stop` consume the full tier and may prompt for a scheme and
*persist it* just to tail/kill an app. The design's own rule ("a resource
that doesn't consume a tier never advertises its flags", §8) is violated one
level down; per-action flattening fixes both directions.

**5.3 `settings show --key X` should print the bare value. [small]** Today
it prints `# target: Demo` + `PRODUCT_NAME = Demo`, so `$(…)` needs sed.
With `--key`, print the raw value (or add `--raw`).

**5.4 Doc-comment nits.** `--deployment-target` help says "iOS deployment
target (default: 17.0)" but the default is platform-dependent (macOS 14.0).
Overlong first doc lines become the one-line list help (`context select`,
`dependency update`). Cryptic requirement-flag help (`--exact` = "`exact:
\"x.y.z\"`."). "Omitted → …" arrow telegraphese in `dependency`. `app
open-url` "drives … in" reads broken. The three `derived-data` `--all`
mentions are worded three different ways. `bsp init --output` says
"project's parent" (it's the *container's* parent — for SPM the package dir
itself). `app run` promises "press `r`" unconditionally (TTY-only).
`context::Variable` values are undocumented in help (`target` especially).

**5.5 Bare `sweetpad` = status, not help. [small]** In a project directory,
bare `sweetpad` should print the `status` view (container, context,
doctor-lite one-liner, "run `sweetpad run` to start") instead of the clap
help wall; outside a project, keep help. (`fly`, `railway`, `git status`
muscle memory.)

**5.6 Help topics, man pages, and a docs reference. [small]** `sweetpad
help destinations`, `help config` (precedence, file locations, the key
format from bug 1.13!), `help environment` (the full `SWEETPAD_*` set,
gh-style), `help exit-codes`, `help hot-reload` — the design doc's best
sections shipped into the binary; kills findings 2.3 and 2.5. Add
`clap_mangen` at release time, and a generated CLI reference page on the
docs site (today `agent-cli.md` covers the RPC half and only name-drops the
standalone CLI).

**5.7 First-run hint. [small]** On the very first invocation (no state
file), one stderr line after the output: `tip: sweetpad init sets up
completions, lsp, and merge drivers — run it once per repo`. Once ever,
suppressible.

## 6. The flagship session (`sweetpad run`)

**6.1 Adopt the flutter-run keymap. [add]** The session already owns raw
mode with `r`/`q`; flutter's keymap is the category standard:

```
r  hot reload (when --hot; today reload is save-triggered only)
R  full rebuild + relaunch (today's `r`)
s  screenshot → ./sweetpad-shots/…png
o  bring simulator to foreground
c  clear the screen
v  toggle verbose/raw log passthrough
d  detach (leave the app running — today quitting kills the app)
q  quit (terminate app)
h  list keys
```

`d`etach matters more than it looks: today's session conflates "stop
watching" with "stop the app".

**6.2 A one-line session header. [small]** `MyApp · Debug · iPhone 16 Pro
(booted) · hot reload on · build 4.2s` — the session always answers "what am
I running, where, how fast". A build-time trend (`4.2s ▼`) is a free
dopamine loop from data already in hand.

**6.3 `--hot` becomes the default posture, eventually. [bet]** Once the
injection client is proven across Xcode versions, the best DX is flutter's:
hot reload *is* the dev loop. Path: `--hot` flag → `[run] hot = true` config
default (§4.4 gives it a home; design §9d already calls the config default
the "remaining nicety") → default-on for simulator debug builds with
`--no-hot` opt-out. Don't rush it; do sequence it.

## 7. Output & agents

**7.1 `--output`/`-o` instead of boolean `--json`. [bet]** While the
surface is fluid, switch the axis to an enum: `-o human` (default) /
`-o json` (envelope, today's `--json`) / `-o ndjson` (streaming events) /
`-o quiet`. `--json` stays as an alias forever. This creates the slot §7.2
needs without a second migration.

**7.2 NDJSON event streams for the long-running verbs. [bet,
agent-defining]** `build`/`test`/`run`/`app logs` are *streams*, which is
why they're the `--json` holes today (2.1, 1.11). The `buildlog::Event`
parser already produces structured events — emit them:

```
sweetpad build -o ndjson
{"event":"task","kind":"compile","file":"Foo.swift"}
{"event":"diagnostic","severity":"error","file":"…","line":12,"message":"…"}
{"event":"result","ok":false,"errors":3,"duration_ms":48210}
```

Same for test events (case started/passed/failed) and log lines. This is
the feature that makes sweetpad the default tool AI agents reach for — an
agent watches a build live instead of parsing a beautified transcript. The
parse/render decoupling in CLI_DESIGN §11 was built for exactly this.

**7.3 `sweetpad schema`. [small]** Print the JSON Schema for any command's
`-o json` payload (`sweetpad schema build`, `schema --list`), generated from
the serde types with `schemars`. The structural fix for 2.2's opaque
`schema: 1` — gives agents/tooling a contract to generate types from.

**7.4 Last-build diagnostics as a queryable artifact. [add]** Persist the
last build's structured events per project (state dir), then `sweetpad
build diagnostics [-o json]` — errors/warnings from the last build without
rebuilding. Mirrors the RPC server's most-used method
(`build.diagnostics`); agents stop re-running builds to re-read errors.

**7.5 CI is a first-class, detected mode. [small]** Auto-enable
non-interactive when `CI=1` (every modern CLI does); add `--gh-annotations`
(emit `::error file=…::…` from diagnostic events) and `test --junit PATH`.
One env check + two renderers over existing events buys "works perfectly in
Actions out of the box".

**7.6 `--show-command` / `--dry-run`. [small]** Print the exact
`xcodebuild`/`simctl` invocation (and env) that would run, then exit.
Teaches users what the tool abstracts, de-mystifies bug reports, lets agents
plan — and pairs perfectly with the "xcodebuild for humans" positioning:
humans graduate.

## 8. Parity gaps (extension → CLI)

**8.1 Device parity for the app lifecycle. [add]** `app
install/launch/logs/stop` are hard-coded simulator-only
(`app.rs:1809-1813`); the devicectl + pymobiledevice3 plumbing exists but
only inside `app run --device`. Also missing: `app uninstall` (sim and
device — the `vscode` namespace has `simulator.uninstall`), device tunnel
management (`pymobiledevice3 remote tunneld` — the extension autostarts
it), and a configurable pymobiledevice3 path/args.

**8.2 Launch args & env. [add]** The extension (and the `vscode`
namespace's `simulator.launchApp --args-json/--env-json
--wait-for-debugger`) can pass launch arguments/environment; `app
run`/`app launch` cannot. Repeatable `--arg`/`--env` plus
`--wait-for-debugger` unlock real debugging flows.

**8.3 Debugger. [add]** The extension ships an LLDB bridge
(`sweetpad-lldb`); the CLI has nothing — even an `app debug` that launches
wait-for-debugger and attaches `lldb -p` closes most of it.

**8.4 Build knobs. [add]** No CLI/config equivalents of `build.args`,
`build.env`, `build.derivedDataPath`, `build.arch`/Rosetta,
`allowProvisioningUpdates`, or an xcodebuild-path override. `--`
passthrough (`sweetpad build -- EXTRA_XCODE_ARGS…`) plus a `[build] args`
key in §4.4's config covers the long tail cheaply.

**8.5 Testing. [add]** No `build-for-testing`/`test-without-building`
split. The `.xcresult` is written to a temp dir and **deleted** after
summarizing (`test.rs:113-139`) — keep it (or `--result-bundle PATH`);
failure attachments/logs are otherwise unrecoverable, and retention enables
`test --failed` (rerun the last run's failures — cargo-nextest/jest's
most-loved flag) and §9.2's xcresult tooling. No `test list` to enumerate
what `--only-testing` accepts.

**8.6 Tuist/XcodeGen regeneration. [add]** `tuist generate` / `xcodegen
generate` with optional watch. The design's "no XcodeGen" decision covers
*scaffolding*, not regenerating an existing project the user already has.

**8.7 Medium.** BSP `doctor`/log access (the extension has both; the CLI
tells users to delete a stale `buildServer.json` by hand, `bsp.rs:84-90`);
log-stream shaping (custom predicate / subsystem allow-deny lists vs the
fixed `processImagePath CONTAINS` predicate, `app.rs:1549` — see §9.6);
simulator video streaming (`serve-sim`).

**8.8 Declared out of scope — don't re-litigate.** `tools` (Homebrew
installs) and `config`/`state` subcommands (CLI_DESIGN §12). Noted so this
list doesn't resurrect settled decisions.

## 9. Ecosystem features & small delights

**9.1 `archive` / IPA export. [add]** `xcodebuild archive` +
`-exportArchive` with a generated ExportOptions.plist and signing-identity/
profile discovery (fastlane gym/sigh territory). The single biggest missing
chunk of "xcodebuild for humans"; nothing in the design doc yet.

**9.2 Test hardening. [add]** `--retry-flaky` (`-retry-tests-on-failure
-test-iterations`), `--coverage` (via `xccov --json`), an `xcresult`
resource (summary/browse/attachment export — pairs with 8.5's retention).
(`--junit` lives in §7.5.)

**9.3 Richer simctl surface. [add, cheap + high-delight]** `simulator
create/delete/clone` (lifecycle is incomplete without them — `erase` exists
but not `create`), `push` (APNs payload), `privacy grant/revoke`,
`status-bar override` (clean marketing screenshots — pairs with
`screenshot`), `record` (video), `location set`, `media add`, and
`boot --wait` (bootstatus). The plumbing already exists.

**9.4 Toolchain selection. [add]** `--xcode <version>` / `DEVELOPER_DIR`
pinning (per-project in §4.4's config), maybe `xcode list/select`
(xcodes-style). `doctor` can only observe today.

**9.5 Watch mode. [add]** `build --watch` / `test --watch` — the debounced
watcher already exists (`cli/inject/watcher.rs`); hot reload covers only the
sim-run case.

**9.6 `app logs` filters. [add]** `--subsystem/--category/--predicate/
--level` — both `log stream` and pymobiledevice3 support them natively;
closes 8.7's shaping gap from the flag side.

**9.7 `sweetpad open [xcode|sim|dd|config]`. [small]** Open the container
in Xcode, Simulator.app, the DerivedData folder, or the config file.
(Subsumes the extension's `build.openXcode`.)

**9.8 Screenshot niceties. [small]** `simulator screenshot --clipboard`;
default output into `./sweetpad-shots/` named by device + time (shared with
§6.1's `s` key).

**9.9 Did-you-mean for values, not just subcommands. [small]** `--scheme
MyAp` → `error: unknown scheme "MyAp" — did you mean "MyApp"? (schemes:
MyApp, MyAppTests)`. The resolver knows the candidates; today they die
inside xcodebuild's error. Same for configurations (fixes the sharp edge of
bug 1.14).

**9.10 Build-time history. [small]** Append per-build duration to state;
`sweetpad stats` sparkline; `status`/session header show the trend
(cold/warm annotated via derived-data presence). Local-only.

**9.11 Trash, don't delete. [small]** `derived-data purge` moves to
`~/.Trash` when interactive; rm only with `--yes`/non-interactive.
Forgiveness > confirmation (and softens bug 1.2's blast radius).

**9.12 Housekeeping. [small]** Standardize truthy parsing for all
`SWEETPAD_*` env vars (see 2.5) and document the set in `help environment`;
`--version --json`; `sweetpad self-update` (or brew-aware upgrade hint).

## 10. A north-star surface sketch

What the tree could look like with the §3 bets taken (strawman):

```
sweetpad                        # status in a project; help outside one          (§5.5)
sweetpad init                   # onboard a repo: bsp + merge drivers + sweetpad.toml + context
sweetpad run [--on X] [--hot]   # the flagship session (flutter keymap)          (§3.1, §6)
sweetpad build [--clean] [--watch]
sweetpad test [--failed] [--junit P]
sweetpad clean [--purge]        # xcodebuild clean; --purge = derived data       (§3.1, §3.5)
sweetpad fmt [--check]
sweetpad devices                # everything runnable, specifier-ready           (§3.2)
sweetpad simulator <boot|shutdown|erase|create|delete|clone|screenshot|appearance|open|push|…>
sweetpad app <install|uninstall|launch|logs|stop|open-url|debug>                 (§8.1–8.3)
sweetpad use / sweetpad status  # set / show the target, with provenance         (§3.4, §4.5)
sweetpad project <info|new|settings|open>
sweetpad dependency <list|add|remove|update|resolve>          # alias: dep
sweetpad merge <install|run>    # semantic conflict resolution                   (§3.3)
sweetpad doctor [--fix]
sweetpad completions | schema | help <topic>
```

(`sweetpad init` is the onboarding story in one command: pick
scheme/destination seeding state, `bsp init`, offer `merge install`, offer a
starter `sweetpad.toml`, finish with doctor-lite. "Clone → `sweetpad init` →
`sweetpad run`" becomes the README. `doctor --fix` runs the safe remedies —
brew installs, `-runFirstLaunch` — with per-item confirmation. Dynamic shell
completions — scheme names, simulator names, `--only-testing` identifiers
via clap_complete's dynamic completer — are the quiet best-in-class layer on
top; `gh` sets the bar and nobody in the Xcode space has it.)

## 11. Priorities

**Fix now (correctness, small diffs):** 1.1 SIGPIPE · 1.2
purge-without-consent · 1.3 env-beats-flag · 1.4 state wipe + atomic writes
· 1.5 remember-poisoning · 1.6 dep-add ordering · 1.7 SPM derived-data stem
· 1.10 exit codes · 1.11 `app logs --json`.

**One afternoon of polish, outsized UX:** 5.1 help grouping · 5.2 per-action
tiers · 3.4-minimum `context set` · 1.12 `--quiet` · 3.10 ValueEnum · 3.12
aliases · 5.6 help topics/man/docs (kills 2.3, 2.5) · 1.13 config warnings +
doc fix · 5.3 `--key` raw · 4.3 walk-up.

**Grammar decisions (one-time, before the Homebrew binary calcifies):** 3.1
verb-first (or default actions) · 3.2 devices · 3.3 merge consolidation ·
3.4 use/status · 3.6 `--yes`/`--force` policy · 4.1 `--on` · 7.1 `-o`.

**Feature investments (rough leverage order):** 8.2 launch args/env + 8.1
device lifecycle/uninstall → 8.5 xcresult retention + 7.5 CI mode → 8.4
build passthrough + 3.1's `clean` → 9.1 archive → 9.3 simctl niceties → 9.5
watch → 8.3 debugger.

**If you only take five:** 4.1 `--on` human destinations (the category's
worst papercut, owned outright) · 3.1 verb-first loop (`sweetpad run` is the
brand) · 7.1 + 7.2 `-o` + NDJSON events (the agent-native Xcode CLI, which
nothing else is) · §10's `init` + 4.4 `sweetpad.toml` (the team onboarding
story) · 6.1 + 6.2 keymap + status header (the daily feel of the product).
