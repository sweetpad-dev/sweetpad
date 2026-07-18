# CLI audit — round 3, workspace pass (July 2026)

A fresh adversarial pass over the whole workspace (CLI, core, lib), run
against `main` at c155570 (0.2.7). Method: thirteen independent review passes
partitioned by module group, each finding verified against the source (and
where possible reproduced with a compiled repro) before fixing. **Every
verified finding is fixed** (commits `addcb1e`, `a12631c`) except the "known
open" list below.

## Worst findings (all fixed)

- **`stream_lines` hung every streamed build/test at completion.** The
  `Command` kept the merged pipe's two write ends after spawn (spawn only
  borrows fds > 2), so the reader never saw EOF once xcodebuild exited.
  Reproduced with a compiled repro; regression introduced by round 2's 4.7
  fix. Also: the hand-rolled pipe lacked `FD_CLOEXEC`, so a child spawned
  concurrently on another thread (BgBoot, watcher compiles) could hold the
  pipe open with the same hang. (process.rs)
- **The BSP server died on a percent-encoded URI panic.** `path_from_uri`
  sliced the `&str` by byte index; a client URI with unencoded non-ASCII
  after `%` (several BSP clients don't percent-encode) panicked the request
  loop — in the extension, inside the extension host. Reproduced.
  (bsp/mod.rs)
- **xcconfig `#include` flattening was exponential.** The chain-only cycle
  guard admitted 2^n re-inlining of diamond graphs — ~22 two-include files
  took 14.6 s and 2M assignments; ~30 is an OOM. Reproduced. Now
  include-once per flatten (matching Xcode's "already included" skip) plus a
  depth cap. (resolver.rs)
- **pbxproj merge silently corrupted duplicate-bearing token lists**
  (`OTHER_LDFLAGS = ("-framework", A, "-framework", B)` lost the second
  `-framework`) while reporting a clean merge; such arrays now conflict
  instead. (pbxproj_merge.rs)
- **A stale `--testing` pick failed every `sweetpad test` forever** —
  `recover_stale` never looked at (or cleared) the testing-state layer.
  (resolve.rs)
- **`dep update` destroyed `Package.resolved` on a failed resolve** (deleted
  before resolving, no restore), and the interactive `Package.swift` add
  wrote a `--package` value (manifest name) that broke the next resolve for
  packages whose identity differs (firebase-ios-sdk). (dependency.rs)

The remaining ~35 fixes span: signal-registry pid-recycle windows
(`check_exit`, `simctl record`), inject-server robustness (bounded protocol
strings, stop-aware waits, straggling-payload desync, transient accept
failures, shell-aware transcript tokenizing, path-boundary file matching),
picker twins acting on the wrong simulator, `boot --wait` reporting failed
boots as success, `core.quotePath` hiding conflicted non-ASCII paths, ndjson
result events on pre-dispatch errors, empty `SWEETPAD_*` vars poisoning
resolution, OpenStep octal/surrogate escapes, writer comments containing
`*/`, cleared-setting flags (`-swift-version ""`, bare `-O`), quote-aware
preprocessor-define splitting, JSON-RPC header caps, and scratch/clone dir
leaks. See the two commit messages for the per-fix rationale.

## Verified but deliberately not fixed (known open)

- **xcconfig `serialize` corrupts files where a multi-line `/* … */` comment
  ends on an entry line** (the raw-capture replay leaves an unterminated
  comment that swallows the rest of the file on the next parse). `serialize`
  has no callers outside the lib's own tests — no CLI write path reaches it —
  so the fix (capturing raw spans from comment-stripped source) is deferred
  until a write path exists.
- **`<hex>` data literals round-trip as bare strings** in the pbxproj writer
  (no `Value::Data` representation). Real Xcode projects don't carry them;
  parse-only paths are unaffected.
- **`spm unlink` with a target filter removes a product dependency shared by
  several targets from all of them** (shared product-dependency objects are
  a Tuist/hand-edit shape; single-target in practice).
- **Inject-server verdicts are attributed by global counters**, so a verdict
  arriving after its 15 s timeout is credited to the next load. Fixing this
  needs a sequence tag the InjectionNext wire protocol doesn't carry.
- **`file_cache` stamps are `(len, mtime)`**, so a same-length rewrite within
  the filesystem's mtime granularity can serve a stale parse to the
  long-lived addon. Needs a content hash or a don't-cache-recent rule.

---

# CLI audit — round 3, CLI pass (July 2026)

A fresh stability/correctness audit of the whole `sweetpad` CLI, run against
`main` at c409c45 (all workspace tests green, clippy clean). Method: seven
independent adversarial review passes — signals & process lifecycle,
state/config/resolution, output & exit-code contracts, external-tool
invocation & parsing, CLI surface wiring, a crash/panic sweep with deep reads
of the mutation-heavy modules (scaffold/merge/dependency/project), and a
hunk-by-hunk hostile re-review of the round-2 fix commit itself — each pass
reproducing its findings against the built binary in isolated temp dirs
(redirected `HOME`/`XDG_*`, stub `xcodebuild`/`simctl`/`swift`/`brew` on a
prepended `PATH`). Headline findings were then re-verified independently
during synthesis. Every finding carries how it was verified:

- **[repro]** — reproduced with the built binary in isolated temp dirs.
- **[code]** — proven from the source; a reproduction would need nothing the
  code doesn't already show.
- **[plausible]** — the code path is real but confirming end-to-end needs
  hardware/state this audit didn't have.

Numbers in section order, worst first. Each item ends with a proposed fix.
Round-2 items and their documented accepted limitations (3.9 spawn→register
gap, 3.12 Mac detach warning, the reparented `log` leak) are not re-reported;
several findings below are regressions the round-2 fixes introduced and are
marked as such.

> **Status (July 2026): every item below is implemented and marked [DONE].**
> Six shipped with a different mechanism than the sketch proposed: **1.2**
> renames the four per-command flags to `--output-file` (clap can't shadow a global
> long under a different id); **3.1** additionally reassembles the serve
> flags from the parsed targeting (the resource-global ContainerArgs swallow
> `--project` before the trailing args see it — caught by the new doctor
> probe itself); **3.13** warns on the silent debug rewrite while `context
> set` keeps rejecting non-Debug/Release pins; **4.8** escalates on the
> *second* signal rather than exempting SIGTERM (a single SIGTERM must keep
> finalizing a recording per round-2 3.3); **4.9** ships as the honest doc
> fix (the Rust runtime erases the inherited disposition before main can see
> it); **4.6** joins the background boot on the cancel paths rather than
> registering it (registration would need the boot to hold a `Child`).

## 1. Hangs & crashes

**[DONE] 1.1 Every streaming xcodebuild command deadlocks forever after the child
exits — build, test, clean, archive, dep resolve, in human and ndjson modes.
[repro]** (regression of round-2 4.7, the stderr-merge fix)
`merged_output_pipe` (process.rs:144-169) hand-rolls `pipe(2)` + `dup(2)` and
returns the two write ends as `Stdio::from_raw_fd` values; `stream_lines`
(process.rs:122-139) stores them into the `Command` via
`cmd.stdout(out).stderr(err)`. std keeps those parent-side fds alive *inside
the `Command`* until it drops — but `read_lines_lossy(reader, …)` runs first,
so after the child exits the parent still holds two open write ends of its own
merged pipe and `read_until` never sees EOF. Five passes reproduced it
independently: stub `xcodebuild` printing `** BUILD SUCCEEDED **` + exit 0 →
`sweetpad build` renders "✓ Build succeeded (0.0s)" and never exits (killed by
timeout, exit 124/142); `ps` shows the child as a zombie; `sample` shows the
main thread in `read_lines_lossy → read`; `lsof` shows the parent holding fd 3
(read end) *and* fds 4/5 (both write ends). Under `-o ndjson` the events are
emitted but the terminal `{"event":"result"}` line never comes — a machine
consumer starves. Blast radius: every consumer of `buildlog::run` /
`run_collecting` / `run_ndjson` — `build` (human/quiet/ndjson), `test`
(human/ndjson), `clean` (xcodebuild branch), `archive` (the export step never
runs), `dependency add/update/resolve` beautified paths, and `app run`'s
non-interactive deploy build. Unaffected: `--json` (`run_captured`), `-v`
(inherited stdio), and the interactive session build (`spawn_piped_group`
builds its `Command` in an inner scope that drops before the caller reads —
the exact pattern `stream_lines` needs). The pre-round-2 code used
`Stdio::piped()`, which std manages correctly.
*Fix:* `drop(cmd)` immediately after `cmd.spawn()` in `stream_lines`, before
any read (Child is owned, not borrowed); set `FD_CLOEXEC` on the raw fds for
defense in depth; add a stub-child integration test that runs a fake
xcodebuild to completion in human and ndjson modes.

**[DONE] 1.2 `--output` on `archive`, `simulator record`, `simulator screenshot`, and
`bsp init` panics the CLI — those flags are unusable. [repro]**
The global `-o/--output <mode>` (mod.rs:95-96) and the four per-command
`--output <path>` args (archive.rs:46-49, simulator.rs:99-101 and :121-124,
bsp.rs:18-20) share the clap id `output`. When only the subcommand's flag is
typed, `GlobalArgs`' typed access hits the `PathBuf` value: `sweetpad
simulator record --output /tmp/x.mp4` → `thread 'main' panicked … Mismatch
between definition and access of 'output'. Could not downcast to
OutputMode, need to downcast to PathBuf` — exit 101, raw panic on stderr, no
envelope, in every output mode. Typing both (`--output /tmp/x.mp4 -o json`)
is a clap usage error, so the file-output flags can't be combined with the
machine modes either.
*Fix:* give the per-command flags a distinct clap id (keep the long name via
`long = "output"` on a differently-named field), and add
`Cli::command().debug_assert()` as a unit test — clap's debug assertions
catch id collisions.

## 2. Data loss & state corruption

**[DONE] 2.1 `dep update` destroys `Package.resolved` before running a resolve that
can fail. [repro]**
`update_resolve` (dependency.rs:775-794) deletes the lockfile (or the one
pin, dependency.rs:763-767) *first* so the resolve re-pins; a failed resolve
(offline, bad credentials, broken project) leaves the deletion permanent —
the team's known-good pins are gone and the next successful resolve may pin
different versions. Reproduced: seeded `Package.resolved` + failing
xcodebuild stub → `dep update` errors and the swiftpm directory is empty.
The requirement-change path also never rolls back the already-written
requirement edit when the resolve fails.
*Fix:* snapshot the lockfile and restore it when `resolve_packages` errors
(or resolve into a scratch first and swap on success).

**[DONE] 2.2 A signal during `dep add`'s post-mutation steps keeps the mutation with
no rollback — and 1.1 makes that window the default outcome. [repro]**
The pbxproj is written at step 1; the round-2 1.5 rollback runs only in the
closure's `Err` arm (dependency.rs:392-443). SIGINT/SIGTERM during steps 2-4
(resolve/discovery — seconds-to-minutes of network work) `_exit`s with the
mutation kept. Compounding: because of 1.1 every human-mode `dep add`
*hangs* in the resolve step, so the user's inevitable Ctrl-C lands exactly in
this window. Reproduced: `dep add ../LocalPkg …` SIGTERMed during resolve →
pbxproj left mutated, differs from pristine; the same add via `--json` (no
hang) errors and rolls back byte-identically.
*Fix:* persist the pristine text to a sibling backup file before step 1 and
self-heal on the next run (the mechanism round-2 1.7 shipped), removing it on
success.

**[DONE] 2.3 `project.pbxproj` and `Package.resolved` are rewritten with non-atomic
in-place `fs::write`. [code]**
`write_pbxproj` (dependency.rs:1093-1102) and `remove_pin`
(dependency.rs:655-676) truncate-then-write; ENOSPC/crash/kill mid-write
leaves a truncated project file. The `remove`/`update` paths take no pristine
snapshot at all, so a partial write there is unrecoverable corruption — while
`state.rs` gives a far less precious file pid-tmp + rename.
*Fix:* write to a same-directory temp file and `rename` over the target, as
`State::save` already does.

**[DONE] 2.4 A summary read failure turns a passing test run into "xcodebuild test
failed before any test ran" (exit 3) and destroys the fresh result bundle.
[repro]** (regression of round-2 2.3; undermines 1.4's promotion)
test.rs:278-303: `test_summary(&run_bundle).ok()` swallows the real error, so
`ran_tests = summary.is_some_and(…)` is false whenever `xcresulttool` fails
(Xcode format drift, transient xcrun error) even when `outcome.passed`. The
`!ran_tests` path then `remove_dir_all(&run_bundle)` — evidence gone, the
stale retained bundle survives, and a following `test --failed` reruns the
*previous* run's failures. Reproduced with stubs: green suite + failing
xcresulttool → exit 3, `build_failure` envelope, new bundle deleted. The same
unconditional delete also destroys an explicitly requested `--result-bundle
PATH` scratch on the early-failure path, where pre-round-2 the xcresult (with
the build log) survived for CI upload.
*Fix:* take the "failed before any test ran" branch only when
`!outcome.passed`; surface the actual `test_summary` error instead of
`.ok()`; keep/promote the bundle whenever the run produced one, always when
`--result-bundle` was explicit.

**[DONE] 2.5 `context set`/`remove`/`alias` exit 0 and print "(remembered)" while
persisting nothing when the state file is unreadable. [repro]** (sharp edge
of round-2 1.1's read-only mode)
`State::save` returns `Ok(())` under `read_only` (state.rs:213-220), so the
context commands' `ctx.state.save().map_err(…)?` propagation is dead code.
Reproduced: `chmod 000 state.toml` → `context set sdk iphoneos` exits 0 and
reports success; the file is untouched. Correct for best-effort `remember`,
wrong for commands whose whole purpose is the write.
*Fix:* make `save()` report the skip (error or enum); keep `remember`'s
swallow, let explicit context mutations fail with exit non-zero.

**[DONE] 2.6 `--show-command` dry runs still mutate persistent state on two paths.
[repro]** (residue of round-2 1.2; introduced/missed by 2.13's fix)
(a) `pick_destination` (resolve.rs:971-997) calls `track_destination` +
`state.save()` unconditionally — the `track` gate added in round 2 covers
only the `--on` branch. Under a PTY, `build --show-command` with no
remembered destination prompts (a dry run opening an interactive picker) and
then writes `destination_recents`/`destination_usage`. (b) `recover_stale`
(resolve.rs:827-870) clears a stale remembered scheme/configuration and saves
mid-dry-run — reproduced: state `scheme = "Ghost"` → `build --show-command`
rewrites state.toml. Both contradict the code's own "a dry run prints and
exits before any state is persisted".
*Fix:* thread `track` into `pick_destination` (skip track+save when false)
and into `settle_*`/`recover_stale` (return the plain validation error when
previewing instead of clear+save).

**[DONE] 2.7 The pick settled after a stale-scheme recovery is not remembered — the
user re-picks every run. [repro]**
`remember` gates on pre-settlement `resolved.scheme.is_none()`
(resolve.rs:911-930), but after `recover_stale` the field still holds the
stale `Some("Ghost")`, so the freshly settled scheme is never written.
Reproduced: stale remembered scheme → warn, auto-pick, build OK — state
afterwards has no scheme; the next build resolves/prompts again
(contradicting 2.13's "drop to the picker (updating state)").
*Fix:* have `settle_scheme`/`settle_configuration` report recovery so callers
treat the field as picker-sourced (clear it on the `Resolved` before
`remember`).

**[DONE] 2.8 hot-selfcheck deletes its backup even when the restore copy failed.
[code]** (weakens round-2 1.7's own mechanism)
app.rs:1168-1170: `let _ = std::fs::copy(&backup, file);` then
unconditionally `remove_file(&backup)` — a failed copy (EACCES/ENOSPC) still
destroys the only pristine copy, leaving the fixture nonce-corrupted with no
future self-heal.
*Fix:* remove the backup only if the copy succeeded.

**[DONE] 2.9 `project new --force`/`--current-dir` silently replaces an existing
`.gitignore` (and any same-named file) wholesale. [repro]**
`--force` waives only the "directory not empty" gate; `write_files`
(project.rs:301-315) then overwrites collisions. Reproduced: a dir with a
custom `.gitignore` → `project new Demo --current-dir --force` → exit 0, the
user's rules are gone.
*Fix:* append missing lines to an existing `.gitignore` (like
`merge.rs::ensure_lines`) and skip+note other collisions.

## 3. Wrong results

**[DONE] 3.1 `bsp init` writes a `buildServer.json` whose `argv` the CLI cannot
serve — the BSP feature is dead on arrival, and `bsp doctor` green-lights it.
[repro]**
`write_config`'s `server_argv` is `[current_exe, "bsp", --project, …]`
(sweetpad-core bsp/mod.rs:50-54), designed for the `bsp-server` binary; from
the sweetpad CLI, `sweetpad bsp --project X` is a clap usage error
(mod.rs:499-504 requires an `init|doctor` subcommand). sourcekit-lsp's launch
of the written argv exits 2; autocomplete silently never works. `bsp doctor`
(bsp.rs:113-132) validates field presence and that `argv[0]` exists on disk —
never that the argv starts a server — so the broken config passes 7/7 checks,
exit 0. The Swift-package arm (bsp.rs:67-70) reports the one state its own
doc comment calls "the one real hazard" (a stale `buildServer.json` breaking
sourcekit-lsp's native SwiftPM support) as a note with exit 0, worded as an
init message.
*Fix:* add a hidden serve mode (`sweetpad bsp serve` or bare-flags) that runs
`sweetpad_core::bsp::run` and write that spelling into `argv`; make doctor
probe-launch the argv and fail the stale-package-file state with exit 1.

**[DONE] 3.2 Empty-but-set `SWEETPAD_*` env vars are "set" for real commands but
"unset" for the bare status view. [repro]**
clap's `env = …` attrs (mod.rs:155-203) fold a present-but-empty var into
`Some("")`, while `Targeting::from_env` (mod.rs:317-334) filters empties — so
the two disagree and `""` flows into resolution. Reproduced:
`SWEETPAD_SCHEME="" sweetpad build --show-command` → exit 4 `unknown scheme
""`; `SWEETPAD_ON="" sweetpad build` → `--on "" is ambiguous`;
`SWEETPAD_WORKSPACE=` → clap exit 2 "a value is required";
`SWEETPAD_DESTINATION=""` would pass `-destination ''` to xcodebuild. A
placeholder `export SWEETPAD_SCHEME=` in CI/.envrc breaks every build while
bare `sweetpad` shows the remembered context as if nothing were wrong.
*Fix:* treat empty env-sourced values as unset post-parse (one shared helper
in the `From<…> for Targeting` impls), mirroring `from_env`.

**[DONE] 3.3 Bare `sweetpad app` drops the entire `SWEETPAD_*` env layer. [repro]**
`Action::default_run()` (mod.rs:756-759, app.rs:229-231) builds
`RunArgs::default()`, so clap's env attrs never run and targeting is
all-`None`. Reproduced: `SWEETPAD_SCHEME=Bogus sweetpad app run` → exit 4;
`SWEETPAD_SCHEME=Bogus sweetpad app` → env silently ignored, builds another
scheme.
*Fix:* seed the default action's targeting from `Targeting::from_env()` (as
the bare-`sweetpad` gate does), or flatten `RunArgs` at the App resource.

**[DONE] 3.4 Env-sourced `SWEETPAD_ON` breaks flag-beats-env for the mode flags, and
both-env `ON`+`DESTINATION` hard-errors against the documented contract.
[repro]**
(a) `RunArgs`' `--mac`/`--device`/`--device-id` declare `conflicts_with =
"on"` (app.rs:37-45), so `SWEETPAD_ON=mac sweetpad run --device` is a clap
usage error blaming a flag the user never typed. (b) With both
`SWEETPAD_ON` and `SWEETPAD_DESTINATION` exported, `build_target`
(resolve.rs:737-741) errors "--on and --destination are mutually exclusive"
(generic exit 1) while `help environment` (help_topics.rs:69-70) and
`status`'s annotation both promise "overrides SWEETPAD_DESTINATION"; the
round-2 unit test codifies the behavior the docs contradict. (c) The app
stage commands (`install`/`launch`/`uninstall`/`stop`) declare no conflict at
all and consult `on` first (app.rs:152-161, :558), so `app install --device
--on 'iPhone 15'` silently installs to the simulator — three policies across
three surfaces.
*Fix:* drop the clap conflicts and disambiguate post-parse at one chokepoint
(typed mode flag beats env `--on`; both-typed errors; both-env resolves
on-wins per the docs), applied to RunArgs and StageTargetArgs alike; classify
the residual error TargetResolution and name the env vars in it.

**[DONE] 3.5 `archive` resolves its relative output paths against two different
directories — the export step reads a plist the CLI wrote elsewhere. [repro]**
The default `--output build` is relative: `create_dir_all` and the generated
`ExportOptions.plist` land in the CLI's cwd (archive.rs:117-155, 180,
193-197) while `-archivePath`/`-exportPath`/`-exportOptionsPlist` are
resolved by xcodebuild against the container's parent (`working_dir`,
xcodebuild.rs:146). `discover_walk_up` explicitly supports running from
subdirectories: `cd Demo/Demo && sweetpad archive` → the plist is written
under `Demo/Demo/build/` while xcodebuild looks at the root → export fails
"cannot read ExportOptions.plist" or silently uses a stale plist at the root.
(Divergence proven via stub cwd log; the export step itself is currently
behind 1.1's hang.)
*Fix:* absolutize `out_dir` once (`std::path::absolute`) and use it for both
argv and filesystem writes.

**[DONE] 3.6 A relative `--result-bundle` from a subdirectory makes a green test run
exit 3. [code]**
Same cwd divergence as 3.5: xcodebuild writes
`<container-parent>/x.new.xcresult`, the CLI's `exists()`/`rename` look in
its own cwd (test.rs:212-213, 239, 268-311) → `summary` is `None` →
`ran_tests` false → BuildFailure despite passing tests.
*Fix:* absolutize `final_bundle` before deriving `run_bundle`.

**[DONE] 3.7 `app run`'s app-location step ignores the build's passthrough args —
installs a stale or missing binary. [code]**
The build honors everything after `--` (`-derivedDataPath`, `SYMROOT=…`,
`CONFIGURATION_BUILD_DIR=…`; app.rs:429-440), but `RunPlan::app_bundle`
(app.rs:446-485) computes `TARGET_BUILD_DIR` with `derived_data_path: None`
and no passthrough — despite its doc comment claiming "the same
TARGET_BUILD_DIR the build produced". `sweetpad run -- -derivedDataPath
/tmp/dd`: the app is built into `/tmp/dd/…` and, if a previous plain build
left an old `.app` in the default DerivedData, **the stale binary is silently
installed and launched** — a vicious debugging trap; otherwise the install
fails without mentioning the mismatch.
*Fix:* thread `-derivedDataPath` from the passthrough into
`BuildSettingsOptions.derived_data_path`, and refuse the other
product-relocating overrides loudly.

**[DONE] 3.8 The hot-reload buildlog recompiler splits shell-escaped transcript args
on whitespace — broken argv for any path containing a space. [code]**
`buildlog_tokens` (inject/recompiler.rs:336) tokenizes with
`split_whitespace().map(unescape)`, so the transcript's escaping for spaced
paths (`/Users/me/My\ Project/Foo.swift`) splits into two corrupt tokens
before `unescape` can see the `\ `. Reachable via `--hot-recompiler buildlog`
and as the Resolver mode's fallback (recompiler.rs:144). `is_primary_line`
still matches the fragment, so the corrupted command is selected: every
recompile in a project under a spaced path (iCloud "Mobile Documents",
"My Project") fails — or compiles the wrong path.
*Fix:* tokenize with a shell-style lexer that treats `\<char>` (and quotes)
as escapes during splitting, instead of split-then-unescape.

**[DONE] 3.9 No staleness recovery for the remembered destination — a deleted
simulator UDID is a self-perpetuating build failure. [code]**
Xcode/runtime updates routinely delete simulators; the remembered
`platform=iOS Simulator,id=<gone>` is then used verbatim (resolve.rs:753-756)
and every plain build fails with xcodebuild's raw "Unable to find a device
matching…" — no provenance hint, no self-clear; the exact failure class
round-2 2.13 fixed for schemes, and `recover_stale` (resolve.rs:827-870)
handles scheme/configuration only.
*Fix:* when the remembered destination carries `id=`, check it against the
`simctl list` already fetched in `build_target` and clear/re-pick like a
stale scheme.

**[DONE] 3.10 `app_bundle`'s platform filter can eliminate every candidate and
hard-error where first-pick used to launch. [plausible]** (fallout of
round-2 2.7)
xcodebuild.rs:560-590: targets that declare `SUPPORTED_PLATFORMS` without the
destination token never become the fallback; if all candidates declare
platforms and none matches (Mac Catalyst under `platform=macOS` → token
`macosx` vs declared `iphoneos iphonesimulator`), the result is "could not
find a launchable .app".
*Fix:* fall back to the first `.app` candidate when the filter rejects
everything.

**[DONE] 3.11 `archive` and `clean` validate the scheme directly, bypassing the
stale-remembered-value recovery build/test get. [code]**
archive.rs:110-112 and clean.rs's scheme branch call `validate_choice`
themselves, so a stale remembered scheme hard-errors with no provenance and
no self-heal — while clean's *configuration* does recover via
`settle_configuration` (inconsistent within one command).
*Fix:* route archive/clean scheme settling through `settle_scheme`.

**[DONE] 3.12 `--on` + `--destination` (both typed) behaves three different ways
across commands. [repro]**
build/test error (resolve.rs:738); `archive` silently prefers
`--destination` and never validates the `--on` word (archive.rs:241-247 —
`archive --on watchos --destination platform=macOS` archives macOS);
`settings show` prefers `--on` (settings.rs:117-128).
*Fix:* enforce one rule at one chokepoint (post-`From<BuildTargetArgs>`),
consumed by all three.

**[DONE] 3.13 Swift packages: `context set configuration UAT` is rejected while
`build --configuration UAT` silently builds debug. [repro]**
context.rs:253-256 validates against `["Debug","Release"]` and exits 4;
`settle_configuration` (resolve.rs:793-799) skips validation for packages and
swiftpm silently maps any unknown name to `debug` — same input, opposite
outcomes, and the silent rewrite loses the user's intent.
*Fix:* align both — accept-with-warning in both places, or validate both
against Debug/Release.

**[DONE] 3.14 Config lint misses `[projects."…/App.xcodeproj/"]` (trailing slash) —
the key silently never matches. [repro]**
`lint_project_key` (config.rs:185-213) compares `Path`s, and `Path` equality
ignores trailing slashes/`.`/`//`; `for_project` (config.rs:100-119) looks up
by raw *string*. A trailing-slash key draws no warning and its
`configuration = …` never applies — exactly the silently-dead-key case the
lint exists to catch.
*Fix:* compare strings (`canonical.to_string_lossy() != key`) in
`lint_project_key`.

**[DONE] 3.15 `context set`/`alias` against a nonexistent container: exit 0, and the
entry is silently pruned by its own save (or kept forever as garbage).
[repro]**
Explicit `--project`/`--workspace` paths are never existence-checked
(resolve.rs:76-96). With the parent dir present, `pruned_view`
(state.rs:239-263) drops the just-written entry during its own save — exit 0,
"(remembered)", state.toml unchanged. With the parent missing, the junk
entry persists forever under the unmounted-volume rule.
*Fix:* error (TargetResolution) when an explicitly flagged container path
doesn't exist, at least for state-mutating commands.

**[DONE] 3.16 Deleted cwd + relative `--project` mints a relative state key —
cross-project state cross-talk. [code]**
`absolutize` falls back to `current_dir().unwrap_or_default()` = `""`
(resolve.rs:54-71), producing a key like `"App.xcodeproj"`; every later save
prunes or keeps it depending on the *pruning* process's cwd
(state.rs:294-296).
*Fix:* skip state persistence for the key when both canonicalize and
`current_dir` fail, instead of using a relative one.

**[DONE] 3.17 `build diagnostics` accepts and silently ignores every build flag.
[repro]**
StartArgs' flags are resource-global (build.rs:15-55); the Diagnostics arm
ignores them all — `build diagnostics --show-command` prints the last build's
diagnostics with no preview, exit 0; `--clean`, `--watch`, and `--`
passthrough are likewise swallowed.
*Fix:* error on StartArgs flags when the Diagnostics action is chosen.

**[DONE] 3.18 `-V`/`--version` under machine output swallows subcommand usage
errors. [repro]** (residue of round-2 4.5)
The version fast path (mod.rs:600-619) matches the token anywhere before
`--`: `sweetpad build --json -V` → version envelope, exit 0, never builds —
while human `sweetpad build -V` is clap exit 2.
*Fix:* fast-path only when the version flag is the first/only non-output
token, or parse with clap first and special-case DisplayVersion.

**[DONE] 3.19 `format`'s default paths claim "the project directory" but use the CLI
cwd. [code]**
format.rs:23-24, 96-101 default to `PathBuf::from(".")`. From `Demo/Sources`,
`format --check` lints only that subtree while the help and container
discovery (walk-up) say the project — CI from a subdir passes on unformatted
files elsewhere.
*Fix:* default to the resolved container's parent (already computed for the
tool default), or fix the help string.

**[DONE] 3.20 `--on mac` requires a working `simctl` it never needs. [code]**
resolve.rs:743 and settings.rs:119 call `simctl::list()?` before
`resolve_on`, which returns `Mac` without touching the list — on a
CLT-only/broken-simctl host, `build --on mac` fails with a tool error though
no simulator is involved.
*Fix:* resolve the `mac`/alias fast path before listing (make `sims` lazy).

**[DONE] 3.21 `simctl` state-matching is wrong in both directions: real
terminate/shutdown failures report success; a concurrently-booting simulator
is a hard error. [plausible]**
`terminate`/`shutdown` accept any stderr containing `"Unable to terminate"` /
`"Unable to shutdown"` (simctl.rs:364-369, :388) — the prefix of *every*
failure message, not just the already-stopped cases — so `simulator shutdown`
against a device mid-transition reports success and the next `erase` fails
confusingly. Meanwhile `boot` accepts only `"current state: Booted"`
(simctl.rs:224), so a device in state `Booting` (Simulator.app just opened
it, or a second `sweetpad run` racing the first) fails the whole run even
though the sim is up seconds later.
*Fix:* narrow the terminate/shutdown matches to the specific
already-in-desired-state messages; treat `"current state: Booting"` as
success followed by `boot_wait`.

**[DONE] 3.22 SPM `build --clean --show-command` preview omits the `swift package
clean` step the real run executes. [repro]**
build.rs:166-173 prints only `swift build --configuration debug`; archive
previews multi-step, this path doesn't.
*Fix:* include the clean invocation in the preview payload.

## 4. Stability — signals, terminal, processes

**[DONE] 4.1 `check_exit` reaps the session's console child but leaves its pid
registered in CHILD_PIDS — a later signal SIGTERMs a recycled pid. [code]**
`try_wait().ok().flatten()` returning `Some` (app.rs:2052-2065) *is* the
reap, but the slot in the signal handler's registry is never cleared —
violating the registry's own invariant (signals.rs:126-128: deregister
*before* the reap, "the handler must never signal a stranger"). The session
stays open (by design) after an app crash; hours of rebuild cycles churn the
pid space; a SIGTERM/SIGPIPE then sweeps CHILD_PIDS and kills an unrelated
process. Unlike the accepted 3.9 microsecond gap, this window is open-ended.
*Fix:* clear the slot in `check_exit` when `try_wait` reports exit
(`signals::unregister_child(running.reap_slot.take())`).

**[DONE] 4.2 Session console/log reader threads die on the first invalid-UTF-8 line
— which can SIGPIPE-kill the user's own macOS app mid-session. [code]**
(the exact bug class round-2 2.4 fixed in `stream_lines`; these four readers
were missed)
`render_logs`/`render_console`/`render_log_stderr`/`render_device_logs`
(app.rs:2171-2172, 2202-2203, 2230-2231, 2265-2266) use `BufReader::lines()`
with `let Ok(line) = line else { break }` — one non-UTF-8 line ends the
thread and drops the pipe read end. On the Mac target the streamed child *is*
the app (app.rs:1496-1521): its next `print` raises SIGPIPE → **the session
kills the app it launched**, then reports "✗ … exited". On simulator/device
targets the console child dies the same way → false "exited" alert and all
further console output lost.
*Fix:* replace the four `lines()` loops with `process::read_lines_lossy`.

**[DONE] 4.3 A direct `kill -TERM`/`-HUP` during a plain `build`/`test` orphans
xcodebuild — the streamed child is registered nowhere. [repro]**
The `stream_lines` child is in neither BUILD_PGID nor CHILD_PIDS
(process.rs:122-139), so the handler exits without touching it. Terminal
Ctrl-C still works (kernel signals the foreground group); directly-addressed
signals (`timeout(1)`, CI cancellation, `kill`) leave the stub running,
reparented to pid 1. Honest bound: the orphan dies of SIGPIPE at its next
write to the dead pipe — so it persists through silent stretches, which for
real xcodebuild means long compile/link phases or an entire `-- -quiet`
build.
*Fix:* register the `stream_lines` child in CHILD_PIDS after spawn,
deregister before `wait()` (the LogStream discipline).

**[DONE] 4.4 The session-build watcher can SIGINT a recycled process group after the
build was reaped. [code]**
The watcher holds the raw pid and calls `kill(-pid, SIGINT)`
(app.rs:1621-1640) without consulting BUILD_PGID or `done`; the main thread's
reap ordering (app.rs:1669-1672) protects only the signal handler. A Ctrl-C
keystroke landing in the poll→kill gap at the exact moment the build finishes
targets a freeable pgid.
*Fix:* set `done` and join the watcher *before* `child.wait()`, or gate the
watcher's kill on `BUILD_PGID.load() == pid`.

**[DONE] 4.5 `simctl::record` clears forward mode after the reap, violating the
documented invariant. [code]**
simctl.rs:556-558 runs `set_forward_child` → `child.wait()` →
`clear_forward_child`, against signals.rs:135-136 ("cleared *before* the
child is reaped"). A second Ctrl-C in the wait-return→clear gap makes the
handler SIGINT a recycled pid and return as if the recording were gracefully
stopped. (`stream_logs` gets this right by clearing at EOF, while the child
is still an unreapable zombie.)
*Fix:* observe the exit without reaping first (`waitid(…, WNOWAIT)`), then
clear, then reap — or document it beside the accepted spawn→register gap.

**[DONE] 4.6 Ctrl-C during the initial session/hot build leaks the background
`simctl boot` child — the simulator boots after the user cancelled. [code]**
BgBoot (app.rs:655-681) runs `simctl::boot` via a blocking `output()` on a
thread; the child is registered nowhere, and the `BuildOutcome::Aborted`
paths (app.rs:869-874, 1046-1053) return without `boot.wait()`.
*Fix:* register the boot child in CHILD_PIDS (spawn via `Child` rather than
`output()`), or at minimum `wait()` on the Aborted paths.

**[DONE] 4.7 `q` during an in-flight hot-reload injection stalls session teardown
until the recompile finishes. [code]**
`Watcher::drop` joins the poll thread (inject/watcher.rs:78-83), which may be
inside a multi-second synchronous `swiftc` capture; the stop flag is only
polled between scans. The session appears hung; Ctrl-C is just a byte in raw
mode and also maps to quit — the only escape is an external SIGTERM.
*Fix:* dispatch `on_change` on a worker so the watcher thread stays joinable,
or check the stop flag inside `inject` and abandon the in-flight compile.

**[DONE] 4.8 Forward-only signal mode never escalates — a wedged child makes the CLI
unkillable by INT/TERM/HUP. [plausible]**
While FORWARD_PID is set, every SIGINT/SIGTERM/SIGHUP re-forwards SIGINT and
returns (signals.rs:211-218); a `recordVideo`/log child that ignores SIGINT
leaves only SIGKILL.
*Fix:* if FORWARDED is already true (or on SIGTERM), fall through to the
normal exit path.

**[DONE] 4.9 The "honors inherited SIG_IGN" claim for SIGPIPE is unreachable — main
resets it to SIG_DFL first. [code]**
bin/sweetpad.rs:27-29 unconditionally sets SIGPIPE to SIG_DFL (to undo Rust's
runtime SIG_IGN) before `signals::install`, so `install_unless_ignored` can
never observe a genuinely inherited ignore: `sh -c 'trap "" PIPE; sweetpad
simulator list | head -1'` dies 141 despite the parent's shield.
*Fix:* distinguishing an inherited SIG_IGN from Rust's own is not worth the
contortion — fix the signals.rs module doc to state SIGPIPE is always reset.

**[DONE] 4.10 `read_lines_lossy` buffers a newline-less stream without bound.
[code]**
process.rs:176-192: `read_until(b'\n')` grows the buffer indefinitely; a
run-script phase emitting gigabytes with no newline balloons the CLI's RSS
until OOM.
*Fix:* flush oversized chunks as synthetic lines (e.g. at 1 MiB) inside
`read_lines_lossy`.

**[DONE] 4.11 `dep add` remote discovery leaks a full package-checkout tree in
TMPDIR per invocation. [code]**
`clone_dir` = `$TMPDIR/sweetpad-spm-<pid>` (dependency.rs:1147-1149, used at
586-593) is never removed; repeated adds accumulate whole dependency graphs.
*Fix:* `remove_dir_all` after discovery, or reuse one stable cache dir.

## 5. Machine-output contract

**[DONE] 5.1 Four call sites still run child tools with inherited stdout under
`--json`/`-o ndjson`, corrupting the machine stream. [repro]**
One pattern, four leaks — the round-2 4.3 sweep missed them: `clean`'s
SwiftPackage branch (`process::stream`, clean.rs:58-62 — the xcodebuild
branch right below gates correctly); `swiftpm::add_dependency` /
`add_target_dependency` (swiftpm.rs:229-262, via dependency.rs:481/509 —
Swift 6 prints progress lines); `self-update`'s `brew upgrade`
(self_update.rs:33 — brew always writes `==> …` to stdout, so a brewed
`self-update --json` is guaranteed corruption); `simulator record`'s
`recordVideo` spawns with inherited stdio (simctl.rs:539-546). All
reproduced with stubs: child chatter lands above/inside the envelope — the
one-JSON-document contract breaks.
*Fix:* one sweep: capture/quiet child stdout under `is_json() ||
is_ndjson()` at all four sites (recordVideo: stdout to null, keep the
signal-forwarding group), and extend the envelope integration test to
Swift-package containers.

**[DONE] 5.2 Two early-exit error paths break the ndjson terminal-result contract.
[repro]**
The `-C` chdir failure (mod.rs:634-641) and the `--gh-annotations` rejection
(mod.rs:649-655) call `out.error(…)` and return, bypassing `render_result`'s
Err arm — under `-o ndjson`, stdout is empty and no
`{"event":"result","ok":false,…}` line is emitted, while CLI_DESIGN promises
exactly one terminal line.
*Fix:* route both through `render_result(&ctx, Err(e))`.

**[DONE] 5.3 `status --json` bakes a human annotation into the machine value.
[repro]**
status.rs:153-159: `context.on.value == "mac (overrides destination)"` —
consumers must strip prose to get the reference.
*Fix:* keep `value` bare; move the note to the human renderer or a separate
field.

**[DONE] 5.4 ANSI color leaks into a piped stderr — color is decided from stdout
terminality only. [repro]**
output.rs:48 keys the single color bit to `stdout().is_terminal()`, but
`warn`/`error`/`note`/`alert` write to stderr: `sweetpad scheme list
2>err.log` from a terminal captures raw `\033[31m` escapes.
*Fix:* compute a second `color_stderr` from `stderr().is_terminal()` (same
NO_COLOR/force overrides) for the stderr emitters.

## 6. Quoting & small correctness

**[DONE] 6.1 `simulator screenshot --clipboard` interpolates the output path
unescaped into AppleScript. [repro]**
simulator.rs:461-469: a `"` or `\` in `--output` breaks the osascript source
(and is injection-shaped, though it's the user's own flag): `--output
'shot".png'` → osascript syntax error, clipboard copy fails opaquely.
*Fix:* escape `\` and `"` before embedding, or pass the path via `on run
argv`.

**[DONE] 6.2 `merge install` embeds the executable path in a shell-evaluated driver
command with only double quotes. [code]**
merge.rs:446: git runs `merge.<name>.driver` through `sh -c`; `$`, backtick,
`\`, `"` in `current_exe()`'s path expand or break — every pbxproj merge then
silently falls back to a failed driver invocation.
*Fix:* single-quote with `'\''` escaping (xcodebuild.rs already has the
helper).

**[DONE] 6.3 `vscode` client: flag values beginning with `-` are misparsed as a
boolean toggle plus a rejected positional. [code]**
vscode_cli.rs:136: `--limit -5` makes `--limit` boolean true and `-5` a
usage error; only the `--limit=-5` form works, undocumented.
*Fix:* mention the `=` form in the usage error (or accept a lone `-`-leading
next token when it parses as JSON).

**[DONE] 6.4 `help environment` omits `DEVELOPER_DIR`. [code]**
help_topics.rs:59-87 documents "every … control" but not the live env knob on
`--developer-dir` (mod.rs:89) that redirects every spawned tool.
*Fix:* add it (with the config `developer_dir` interplay) and pin it in the
env-list test.

## 7. Verified sound

Deliberately probed, no defect found:

- **Signal handler**: async-signal-safe throughout (`write`, `tcsetattr`,
  `kill`, `_exit`, sigprocmask family + atomics; no allocation/locks/stdio);
  RAW_ACTIVE Release/Acquire publication, CHILD_PIDS CAS + `swap(0)` sweep
  (no double-kill), FORWARDED-before-FORWARD_PID ordering, TSTP/CONT
  block/raise/re-arm dance, EINTR paths (`poll_key` → Idle, EOF → Closed —
  round-2 3.4 holds), closed-std-fds startup (std reopens /dev/null),
  `with_sigint_ignored` around lldb, no self/pgid-0 kill path.
- **Round-2 fix claims re-verified against the shipped code** (the diff
  pass): 1.1, 1.3, 1.5–1.7, 2.1, 2.2, 2.4–2.6, 2.8–2.13, 3.1–3.12, 4.1–4.9,
  5.1–5.15 all do what CLI_AUDIT round 2 claims, modulo the regressions
  reported above (2.4→ours 2.4; 2.13→ours 2.6/2.7; 1.1→ours 2.5; 1.7→ours
  2.8; 4.7→ours 1.1).
- **State/resolution**: atomic pid-tmp+rename saves, quarantine `.corrupt.N`
  backups, true-LRU recents, unmounted-volume pruning rule, case-insensitive
  APFS canonical keys (no fragmentation), `/tmp`→`/private/tmp` consistency,
  precedence flag > config > sweetpad.toml > state across all fields,
  remember gates at all three call sites (`--on`/`--mac`/`--device`/SPM
  never remembered), `flag_typed`/`disambiguate_*` truth tables and `--` stop,
  `resolve_on` exact-beats-substring, alias chains, reserved names.
- **Output/streaming**: NDJSON framing survives invalid UTF-8, embedded ANSI,
  control chars, 200 KB lines (one valid compact object per line,
  python-validated); `run_captured` drains both pipes (no 10 MB stderr
  deadlock); exit taxonomy on the main paths (build_failure 3,
  target_resolution 4, UserCancel 6, `data_with_exit` payloads);
  verbose/quiet can't change machine stdout; spinners inert off-TTY and
  under machine modes; prompt gating via `is_interactive()`; env truthiness
  (`NO_COLOR` non-empty, `SWEETPAD_NONINTERACTIVE`/`CI` truthy,
  `CLICOLOR_FORCE`/`FORCE_COLOR`) matches docs.
- **Parsers**: buildlog diagnostics folding/escaping (round-2 2.10/2.11),
  simctl/devicectl JSON tolerant of unknown states/missing keys, oslog
  ndjson renderer defaults + byte-safe ANSI strip, pymobiledevice3 banner
  handling, swiftpm dump-package preamble skip and `swift --version` parse,
  xcresulttool argv construction.
- **Mutation safety elsewhere**: merge driver end-to-end (conflict report,
  clean semantic merge, unparseable → Skipped, no write on failure,
  idempotent); scaffold purity + name validation; `dep` happy paths
  (snapshot-rollback on all *returned* errors, fail-before-mutate guards,
  `relative_path` arithmetic bounded); simulator delete/erase consent
  ladder; dd purge scoping.
- **Panic sweep came up empty** outside 1.2: no non-test `unwrap`/`expect`
  on runtime data, no reachable `unreachable!`, no byte-slicing of multibyte
  input (all `find()`-offset or `.get()`-guarded), saturating arithmetic,
  lossy UTF-8 fallbacks, no RefCell, bounded locks.
- **`spawn_piped_group` does not share 1.1's fd retention** (its `Command`
  drops before the caller reads) — the interactive session build is safe.

## 8. Suggested priorities

1. **1.1** (universal streaming hang) and **1.2** (`--output` panic) — both
   trivial fixes, both block basic use; 1.1 also unblocks verifying 3.5's
   export step and defuses 2.2's Ctrl-C trap.
2. The data-loss cluster: **2.1**, **2.2**, **2.3**, **2.4** (one test.rs
   region), **2.8**.
3. The state-contract cluster: **2.5**, **2.6**, **2.7**, **3.15**, **3.16**.
4. The env-layer cluster: **3.2**, **3.3**, **3.4** (one disambiguation
   chokepoint).
5. The signal-registry cluster: **4.1**, **4.3**, **4.5**, **4.6** (one
   discipline, four sites) plus **4.2** (one helper swap, four sites).
6. The machine-output sweep: **5.1**, **5.2** (one pattern each).
7. Feature-restoring fixes as they're touched: **3.1** (BSP), **3.5**/**3.6**
   (cwd absolutizing), **3.7**, **3.8** (hot reload), the rest of §3, §4,
   §6.
