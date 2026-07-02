# SweetPad CLI — surface audit

A whole-surface audit of the standalone `sweetpad` CLI (the non-`vscode` half of
the binary), done against the implementation on `main` (July 2026). Method:
full read of `src/cli/mod.rs` + every `src/cli/commands/*.rs`, a recursive
`--help` dump of the built binary, live probes of the pure-Rust paths
(scaffold → info/scheme/settings/dependency/bsp/context on a generated
project), and a comparison against `CLI_DESIGN.md`, the VS Code extension's
command/settings surface, and the wider ecosystem (fastlane, tuist, simctl,
xcresultparser, xcodes).

Sections: **1. Bugs** (verified, severity-ordered) · **2. Contract holes**
(the design doc's promises vs the code) · **3. Grammar & naming consistency** ·
**4. Help/UX papercuts** · **5. Parity gaps** (extension vs CLI) ·
**6. Ideas** (ecosystem) · **7. Suggested priorities**.

---

## 1. Bugs (verified)

1. **SIGPIPE panic on any piped command.** `sweetpad settings show --json | head`
   panics (`failed printing to stdout: Broken pipe`, exit 101, full backtrace on
   stderr). Reproduced on a scaffolded project. Any consumer that closes the
   pipe early (`head`, `grep -m1`, a pager quit) hits it. Fix: restore
   `SIGPIPE`'s default disposition at startup (`libc::signal(SIGPIPE, SIG_DFL)`
   in `src/bin/sweetpad.rs`) or route `Output` writes through a
   BrokenPipe-tolerant writer.

2. **`derived-data purge --json` deletes without confirmation and without
   `--yes`.** The gate is `!yes && ctx.out.is_interactive()`
   (`commands/derived_data.rs:153`), and `--json` forces non-interactive — so a
   scripted `purge --all --json` silently rm-rf's the whole DerivedData store.
   Non-interactive destructive actions should *require* `--yes`, not assume it.

3. **`SWEETPAD_WORKSPACE` env silently beats an explicit `--project` flag.**
   `resolve::container` checks workspace before project (`resolve.rs:52-57`) and
   clap's `env =` folds the env var into the flag layer, with no
   `conflicts_with` between the two (`mod.rs:113-121`). This inverts the
   documented `flag > env` precedence (CLI_DESIGN §5). Fix: error on both-set
   from different sources, or prefer whichever was an actual CLI token.

4. **A corrupt `state.toml` is silently wiped.** `State::load()` errors are
   swallowed with `unwrap_or_default()` (`mod.rs:349`); the next best-effort
   `save()` then rewrites the whole file from the near-empty in-memory state —
   every project's remembered context is lost without a warning. Compounding
   it, `save()` is a plain non-atomic `fs::write` with no lock (`state.rs:153-162`),
   so two concurrent sessions do last-writer-wins and can produce exactly such a
   torn file. Fix: temp-file + rename, warn (and back up) on parse failure
   instead of defaulting.

5. **`app run --mac` / `--device` poison the remembered build context.**
   `plan()` calls `resolve::remember` unconditionally (`commands/app.rs:360-365`),
   persisting `platform=macOS` or the device specifier into the *shared* build
   context — the next plain `build start` silently targets the Mac/device. More
   generally `remember` persists flag/env/config-sourced values too
   (`resolve.rs:333-340`), so state is not "the last interactive picks" as
   `state.rs:3-5` and CLI_DESIGN §5 describe, and a *failed* build still
   rewrites the context (remember happens at plan time). Fix: only remember
   values that came from a picker (or at least never remember `--mac`/`--device`
   destinations).

6. **`dependency add` on a `Package.swift` mutates before it can fail.**
   `add_to_xcode` has a "fail before mutating anything" non-interactive guard
   (`commands/dependency.rs:374-378`); `add_to_package` does not — non-interactive
   with no `--product`/`--target` it edits the manifest, resolves, *then* dies
   in the product picker (`dependency.rs:433` → `choose`), leaving a dangling
   manifest edit and a misclassified `user_cancel` (exit 6). Fix: mirror the
   guard before step 1.

7. **`derived-data` scoping is broken for SPM containers.** The project scope
   is the container path's `file_stem` (`derived_data.rs:234-241`), which for
   every `Package.swift` is literally `"Package"` — `path`/`size` never match
   the real `<DirName>-<hash>` folder Xcode creates, and `purge` would match a
   foreign `Package-<hash>` entry. Fix: use the package directory name for SPM
   containers.

8. **No signal handling outside the raw-mode session.** There is no
   SIGINT/SIGTERM handler anywhere. Consequences: Ctrl-C exits 130 (never the
   documented `user_cancel` 6), and even *handled* aborts exit 0
   (`BuildOutcome::Aborted` → `Ok(Streamed)`, `app.rs:577,602,718,963`); a live
   spinner line is left on stderr (Drop skipped); SIGTERM during a session
   leaves the terminal in raw no-echo state; the `--hot` *initial* build spawns
   xcodebuild into its own process group before raw mode is on, so Ctrl-C kills
   the CLI but the build keeps running detached (`process.rs:144-160`,
   `app.rs:714`); and `simctl … log stream` children are only reaped in
   `LogStream::Drop` (`app.rs:1024-1036`), so SIGINT leaks them. Fix: one small
   SIGINT/SIGTERM handler — restore termios, forward to the build pgid, run the
   Drop-equivalent reaps, exit 6 (or 130).

9. **Two dead context knobs.** `context select sdk` and
   `context select target --testing` write state that *nothing reads*:
   `settings show` hardcodes `sdk: String::new()` (`commands/settings.rs:115`),
   `BuildPlan` has no sdk field, and neither `resolve_testing`
   (`resolve.rs:149-191`) nor `test run` consumes `testing.target`. Users can
   set them, `context show` displays them, and they do nothing. Fix: plumb them
   through (add `--sdk`; make `test run` honor the target) or drop the
   variables.

10. **Misclassified exit codes.**
    - `app run` build failures exit 1, not 3: `build_and_install` never tags
      `BuildFailure` (`app.rs:409`; root cause: `BuildPlan::run` returns an
      unclassified error, `xcodebuild.rs:80-82` — classify it there once).
    - `simulator boot NAME` with no match exits 1, not 4
      (`commands/simulator.rs:141-148`), while the identical error via
      `select_simulator` is classified (`resolve.rs:275`).
    - Device resolution errors ("no device matching …", `app.rs:323,330,1918-1927`)
      are Generic, should be `target_resolution`.

11. **`app logs --json` emits human-rendered log lines on stdout** — the one
    command that silently ignores `--json` entirely (`app.rs:1849-1915`).
    Either reject it like `app run` does, or emit JSON-lines events (the
    latter is genuinely useful for agents).

12. **`--quiet` doesn't quiet the build.** The build spinner gates only on
    `is_interactive()` (`buildlog.rs:250-254`), not `!quiet` like `Output::step`
    (`output.rs:148`), and the beautified log stream (`out.line`) is never
    muted — `sweetpad -q build start` animates and prints everything. Also
    `--hot`'s status lines print to *stdout* and ignore `--quiet`
    (`app.rs:650-663`).

13. **Silent config typos.** No `deny_unknown_fields` anywhere in `config.rs` —
    `[default]` instead of `[defaults]`, or `schme = "App"`, parses cleanly and
    is ignored (verified live). Worse, the design doc's example key
    `[projects."/Users/me/code/MyApp"]` (a *directory*) never matches: the real
    key is the canonicalized **container** path (`/…/MyApp.xcodeproj`,
    `resolve.rs:41-46`) — a user copying the doc gets a silently dead override.
    Fix: warn on unknown keys and on `[projects.*]` tables that match no known
    key; fix the doc example (or accept directory keys).

14. **Configuration is never validated or prompted.** `build_target` prompts
    for scheme and destination but silently defaults configuration to `"Debug"`
    (`resolve.rs:312-315`) without consulting `configurations()` — a project
    with only `UAT-Debug`/`Prod` invokes xcodebuild with a nonexistent
    configuration. The candidates helper exists and is only used by
    `context select`.

15. **Same-kind container ambiguity is nondeterministic.** Two `.xcodeproj` in
    one directory resolve to the first `read_dir` entry (`resolve.rs:77-87`) —
    filesystem order, no warning. Error or warn on ambiguity.

16. Minor: the `derived-data purge` confirm prompt is the only dialoguer use
    without the color-aware theme (`derived_data.rs:162`) — ignores
    `--no-color`; state entries for deleted projects and
    `destination_recents`/`destination_usage` grow forever (no pruning,
    `resolve.rs:390-400`); canonicalize-failure falls back to a raw relative
    path as the state key (cwd-dependent duplicates); case-insensitive APFS
    can yield two keys for one project.

## 2. Contract holes (design promises vs code)

- **"`--json` on any command"** (§4): `app run` rejects it with a *Generic*
  error, exit 1 (`app.rs:253-258`) — fine to reject, but it deserves a usage
  class; `app logs` ignores it (bug 11); `completions` ignores it (harmless,
  worth documenting); clap usage errors are human text on stderr under
  `--json` (documented as clap-owned, but consumers should know).
- **Envelope**: single global `"schema": 1` shared by all commands and errors —
  a consumer can't distinguish payload shapes, and no `.md` documents the bump
  policy. Consider `schema: "<command>/1"` or a `command` field, and one doc
  paragraph on stability. Success JSON is pretty-printed, error JSON compact —
  pick one. `ok: true` alongside non-zero exits (test failures exit 3, doctor
  problems exit 1, `format --check` exit 3) is intentional ("ok" = command
  executed) but unstated — document it, and make sure each such payload carries
  its own status field.
- **Exit-code taxonomy is invisible.** 1/2/3/4/5/6 is a good taxonomy that no
  user can discover: it's not in `--help`, not in CLI_DESIGN.md, not in the
  docs site. Add an `EXIT CODES` section to the top-level long help and docs.
- **Under `--json`, child stderr interleaves.** Child processes always inherit
  stderr (`process.rs:18,65,87`), so raw xcodebuild noise mixes with the one
  structured error line. Consider capturing child stderr when `--json`.
- **`--non-interactive` and `-q/--quiet` are missing from CLI_DESIGN §8's
  universal-flag list**, and `SWEETPAD_NONINTERACTIVE` from §5's env list;
  `CLICOLOR_FORCE`/`FORCE_COLOR` are implemented (`output.rs:33-35`) but
  undocumented. Also `SWEETPAD_NONINTERACTIVE` is checked with `is_some()` —
  `SWEETPAD_NONINTERACTIVE=0` still triggers it.
- **Strict-mode error text lies in places**: `missing()` always says "pass
  --{what} or set it in config" (`resolve.rs:196-201`) — wrong for
  simulator/device (no such flag or config key), and says "stdout is not a TTY"
  while the gate is actually stderr-TTY/`--json`/non-interactive.
- **The `vscode` namespace speaks a different dialect.** `sweetpad vscode --help`
  prints a JSON *error* envelope (`code: "USAGE"`, no `schema` field) with exit
  0 — different envelope, different code taxonomy, and help-as-error. Worth
  aligning the envelope shape or at least giving `vscode --help` human output
  like the rest of the binary.
- **Doc drift**: §9 says SPM schemes come from `xcodebuild -list -json`; the
  code reads the manifest via `swift package dump-package`
  (`resolve.rs:217-220`). Update whichever is wrong.

## 3. Grammar & naming consistency

- **Primary verbs disagree**: `build start` vs `test run` / `format run` /
  `app run` (`start` implies async semantics the blocking compile doesn't
  have). Align on `run` — or better, make the bare resource work for
  single-action resources (below).
- **Single-action resources tax the grammar**: `scheme list`,
  `destination list`, `device list`, `build start`, `test run`, `format run`,
  `bsp init`, `pbxproj resolve`, `spm resolve`, `merge install` — half the
  surface is `noun verb` where only one verb exists, while `doctor` is a bare
  verb. Consider default actions (`sweetpad build` ⇒ `build start`,
  `sweetpad test` ⇒ `test run`) and/or a top-level `sweetpad run` alias for
  `app run` — the most-typed command deserves the shortest spelling
  (cf. `cargo run`, `flutter run`).
- **`resolve` collides**: `dependency resolve` (SPM resolution) vs
  `pbxproj resolve`/`spm resolve` (git-conflict merge). `dep resolve` and
  `spm resolve` even touch the same file for different jobs. Rename the merge
  actions (e.g. `pbxproj merge` / `spm merge`, matching the `merge` resource
  they share plumbing with).
- **Three deletion verbs**: `simulator erase` (domain-faithful, keep),
  `context remove` / `dependency remove`, `derived-data purge`. Fine
  individually; pick a policy and note it in CLI_DESIGN §2.
- **`--force`/`--yes`/`--all` semantics diverge**:
  `derived-data purge --yes` = skip prompt; `project new --force` = skip prompt
  *and* waive the non-empty-dir check; `pbxproj/spm resolve --force` = redo
  work git finished. `context remove --all` = whole context;
  `derived-data --all` = widen scope to the global store; `dependency update`
  spells "everything" by omitting the positional. Suggested rule: `--yes` skips
  confirmation, `--force` overrides a safety check, and document it.
- **"Which simulator" is spelled three ways**: positional `[TARGET]` on all
  `simulator` actions, `--simulator NAME` on `app open-url`, `--device-id` on
  `app run`. Pick one addressing convention. Also
  `simulator appearance MODE [TARGET]` puts the target second while every
  sibling takes it first; and `boot` *prompts* when the positional is omitted
  while shutdown/erase/screenshot/appearance default to the booted sim — two
  omission behaviors, undocumented as a rule. `boot`'s prompt also bypasses the
  adaptive most-used-first picker and doesn't record usage
  (`simulator.rs:144-149` vs `resolve.rs:367-385`).
- **`--target` is overloaded four ways**: build target (`settings`), link
  target (`dependency`), testing context variable (`context`), simulator
  positional name. Probably livable, but worth a glossary line in the design
  doc.
- **Arity mismatch on the same flags**: `dependency add --product/--target` are
  repeatable; `dependency remove --product/--target` are single.
- **`--hot-recompiler` is a hand-parsed free string** (`app.rs:52-53,94-100`)
  while every comparable flag is a `ValueEnum` — typos become runtime errors
  and completions can't offer values.
- **`context remove` with neither `VARIABLE` nor `--all`** is a runtime error
  (`context.rs:243-247`) — express it as a clap `ArgGroup` so usage errors are
  clap's (exit 2, before any I/O).
- **Aliases**: only `dep` exists. `sim` (longest frequently-typed resource) and
  `dd` (the only hyphenated name) earn one by the same criterion.

## 4. Help & UX papercuts

- **Flag ordering in help is shuffled.** Targeting and global flags interleave
  arbitrarily (`dependency add --help` lists `--from`, `--workspace`,
  `--exact`, `--project`, …). One-line fix with big payoff: `help_heading` on
  the tier structs ("Target selection") and `GlobalArgs` ("Global"), plus
  `display_order`, so every command's help groups action flags → targeting →
  global.
- **Tier flags leak onto actions that ignore them**: `project new` advertises
  `--workspace`/`--project` (it creates a project); `app open-url` advertises
  `--scheme`/`--configuration`/`--destination` (it resolves only a simulator);
  `scheme list` advertises `--scheme`. Meanwhile `app logs`/`app stop` *do*
  consume the full tier — they may prompt for a scheme and persist it just to
  tail/kill an app. The design's own rule ("a resource that doesn't consume a
  tier never advertises its flags", §8) is violated one level down. Per-action
  flattening fixes both.
- **`settings show --key X` output isn't script-friendly**: prints
  `# target: Demo` + `PRODUCT_NAME = Demo`, so `$(…)` needs sed. With `--key`,
  print the raw value (or add `--raw`).
- **`project new --deployment-target` help says "iOS deployment target
  (default: 17.0)"** — the default is platform-dependent (macOS 14.0).
- Doc-comment nits (from the full sweep): overlong first lines becoming list
  help (`context select`, `dependency update`); cryptic requirement-flag help
  (`--exact` = "`exact: \"x.y.z\"`."); "Omitted → …" arrow telegraphese in
  `dependency`; `app open-url` "drives … in" reads broken; the three
  `derived-data` `--all` mentions are worded three different ways;
  `bsp init --output` says "project's parent" (container's parent — for SPM
  the package dir itself); `app run` promises "press `r`" unconditionally
  (TTY-only); `context::Variable` values are undocumented in help (`target`
  especially).
- **Missing non-interactive context setter**: `context select` is prompt-only
  and strict-errors off a TTY, so scripts/CI *cannot* seed remembered state at
  all. Add `context set <VAR> <VALUE>` (and let `select` keep the picker).
- **No man pages** (`clap_mangen` is one dependency away; completions already
  exist) and **no CLI reference page** on the docs site — `agent-cli.md` covers
  the RPC half and only name-drops the standalone CLI. A generated reference
  (help-dump → markdown) would close it cheaply.

## 5. Parity gaps (extension → CLI)

High value:

- **Device parity for the app lifecycle.** `app install/launch/logs/stop` are
  hard-coded simulator-only (`app.rs:1809-1813`); the full devicectl +
  pymobiledevice3 plumbing exists but only inside `app run --device`. Also
  missing: `app uninstall` (both sim and device; the vscode namespace has
  `simulator.uninstall`), device tunnel management
  (`pymobiledevice3 remote tunneld` — extension autostarts it), and
  configurable pymobiledevice3 path/args.
- **Launch args & env.** The extension (and the `vscode` namespace's
  `simulator.launchApp --args-json/--env-json --wait-for-debugger`) can pass
  launch arguments/environment; `app run`/`app launch` cannot. `--arg`/`--env`
  (repeatable) plus `--wait-for-debugger` would unlock real debugging flows.
- **Debugger.** The extension ships an LLDB bridge (`sweetpad-lldb`); the CLI
  has nothing — even a `app debug` that launches wait-for-debugger and attaches
  `lldb -p` would close most of it.
- **Build knobs.** No CLI/config equivalents of `build.args`, `build.env`,
  `build.derivedDataPath`, `build.arch`/Rosetta, `allowProvisioningUpdates`,
  or xcodebuild-path override. `--` passthrough (`sweetpad build start --
  EXTRA_XCODE_ARGS…`) plus a `[build] args = […]` config key covers the long
  tail cheaply.
- **Testing.** No `build-for-testing`/`test-without-building` split; the
  `.xcresult` is written to a temp dir and **deleted** after summarizing
  (`test.rs:113-139`) — keep it (or `--result-bundle PATH`), since failures'
  attachments/logs are otherwise unrecoverable. No `test list` to enumerate
  what `--only-testing` accepts.
- **Standalone `build clean`** (bare `xcodebuild clean`; `--clean` only exists
  fused to a build today).
- **Tuist/XcodeGen regeneration** (`tuist generate`, `xcodegen generate` with
  optional watch). The design's "no XcodeGen" decision covers *scaffolding*,
  not regenerating an existing project the user already has.
- Medium: BSP `doctor`/log access (extension has both), log-stream shaping
  (custom predicate / subsystem allow-deny lists vs the fixed
  `processImagePath CONTAINS` predicate, `app.rs:1549`), config default for
  `--hot`'s recompiler + dylib path (design §9d itself calls this the remaining
  nicety), `project open` (open container in Xcode — trivial, handy),
  simulator video streaming.
- Declared out of scope (don't re-litigate, just noting): `tools`
  (Homebrew installs), `config`/`state` subcommands (§12).

## 6. Ideas (ecosystem)

- **`archive` / IPA export** — `xcodebuild archive` + `-exportArchive` with a
  generated ExportOptions.plist and signing discovery. The single biggest
  missing chunk of "xcodebuild for humans"; nothing in the design doc yet.
- **Test hardening**: `--retry-flaky` (`-retry-tests-on-failure
  -test-iterations`), `--coverage` (via `xccov --json`), `--junit` export, an
  `xcresult` resource (summary/browse/attachments — pairs with keeping the
  bundle).
- **CI helpers**: GitHub Actions annotations (`::error file=…`) emitted
  straight from `buildlog::Event` (§11 explicitly anticipated events feeding CI
  summaries), per-target build-timing summary.
- **Richer simctl surface** (cheap, high-delight — plumbing already exists):
  `simulator create/delete/clone`, `push` (APNs payload), `privacy
  grant/revoke`, `status-bar override` (clean screenshots — pairs with
  `screenshot`), `record` (video), `location set`, `media add`, and
  `simulator boot --wait` (bootstatus).
- **Toolchain selection**: `--xcode <version>`/`DEVELOPER_DIR` pinning
  (per-project in config), maybe `xcode list/select` (xcodes-style); `doctor`
  can only observe today.
- **Watch mode**: `build start --watch` / `test run --watch` — the debounced
  watcher already exists (`cli/inject/watcher.rs`); hot reload covers only the
  sim-run case.
- **`app logs` filters**: `--subsystem/--category/--predicate/--level` (both
  `log stream` and pymobiledevice3 support them natively).
- Smaller: `app run --no-build`, `settings diff <config-a> <config-b>`,
  `sweetpad self-update`, `--version --json`, a global `-C <dir>` (chdir like
  `git -C`) so CI never needs `cd`, opt-in build-time history
  (`sweetpad stats`).

## 7. Suggested priorities

**Fix now (correctness, small diffs):** SIGPIPE (1) · purge-without-consent (2)
· env-beats-flag (3) · state wipe + atomic writes (4) · remember-poisoning (5)
· dep-add ordering (6) · SPM derived-data stem (7) · exit-code
misclassifications (10) · `app logs --json` (11).

**One afternoon of polish, outsized UX:** help grouping via `help_heading` ·
per-action tier flattening · `context set` · `--quiet` actually quiet ·
`ValueEnum` for `--hot-recompiler` · `sim`/`dd` aliases · exit-code section in
help/docs · config unknown-key warning + fixed doc example · man pages + docs
CLI reference.

**Grammar decisions (one-time, before v1 freezes):** `build start` → `run` (or
default actions + top-level `run`) · rename `pbxproj/spm resolve` →
`… merge` · `--yes`/`--force` policy · one simulator-addressing convention.

**Next feature investments (in rough order of leverage):** launch args/env +
`app uninstall` + device lifecycle parity → `test` xcresult retention +
`--junit`/coverage → `build` passthrough args + `build clean` → `archive` →
simctl niceties → watch mode → debugger attach.
