# CLI audit — round 2 (July 2026)

A fresh bugs/correctness/stability audit of the whole `sweetpad` CLI, run
against `main` at 003863c (all workspace tests green, clippy clean). Method:
four independent adversarial review passes (command wiring/UX, signals &
process lifecycle, state & resolution, output & streaming contracts) plus a
line-by-line pass over the highest-risk modules. Every finding carries how it
was verified:

- **[repro]** — reproduced with the built binary in isolated temp dirs
  (fake tools / scaffolded projects / redirected `HOME`/`XDG_*`).
- **[code]** — proven from the source; a reproduction would need nothing the
  code doesn't already show.
- **[plausible]** — the code path is real but confirming end-to-end needs
  hardware/GUI this audit didn't have (a physical device, a watch project).

Numbers in section order, worst first. Each item ends with a proposed fix.

> **Status (July 2026): every item below is implemented and marked [DONE].**
> Five shipped with a different mechanism than the sketch proposed:
> **1.7** restores via a sibling backup file (self-healing on the next run)
> instead of teaching the signal handler file I/O; **3.9**'s spawn→register
> gap is accepted and documented (the reorderings shipped); **3.12** ships an
> honest warning on Mac detach plus corrected comments (a full fix needs
> file-backed stdio at spawn time); **4.4** rejects `--gh-annotations` under
> the machine modes rather than emitting into their stdout; **5.13** keeps
> `clean --purge` unprompted and documents the flag-is-consent rule in its
> help.

## 1. Data loss & state corruption

**[DONE] 1.1 A state read *error* wipes every project's remembered context. [repro]**
`State::load` (state.rs:154) treats any read failure — not just NotFound — as
"no file" and returns defaults with no warning; quarantine only covers parse
errors. A momentarily unreadable `state.toml` (mode 000, EIO, ACL) + any
state-writing command → `save()` rename-replaces the file with the near-empty
in-memory state. Reproduced: seeded state, `chmod 000`, ran `context alias`
(exit 0, no warning), `chmod 644` — the other project's entry is gone. This is
exactly the failure `load_or_quarantine` documents as unacceptable, and
`Config::load` (config.rs:79) already distinguishes NotFound.
*Fix:* match `ErrorKind::NotFound` → defaults; any other error → warn + mark
the state read-only for this process (skip saves), mirroring the quarantine
rationale.

**[DONE] 1.2 `--on` poisons the remembered destination; `--show-command` writes state
at all. [repro]** `build.rs:188` calls `remember(…, true)` unconditionally and
`remember_testing` (resolve.rs) has no destination gate, so `sweetpad build
--on mac` persists `destination = "platform=macOS"` and every later plain
`build` silently targets macOS; `--on <device>` persists a specifier that fails
once unplugged. This contradicts the picker-sourced-only rule stated at the
`--on` branch in `resolve::build_target` ("it is never written as the
remembered destination itself") and in `help topics` ("never remembered").
`app run` gates correctly (app.rs:631 passes simulator-only) but its `--on`
sim runs still persist. Worse, both `build --show-command` and `test
--show-command` run `remember`/`remember_testing` (and the `--on` path bumps
usage/recents and saves) *before* the dry-run early-return — a "print only"
command mutates persistent state. All reproduced.
*Fix:* pass `destination: ctx.targeting.on.is_none()` from build/test/app, add
the same gate to `remember_testing`, and move `remember*` below the
`show_command` check in build.rs/test.rs.

**[DONE] 1.3 Destination recents evict first-seen, not least-recently-used. [repro]**
`track_destination` (resolve.rs:868) appends only on first pick and never
re-orders; `pruned_view` (state.rs:221) drains from the front and then prunes
`destination_usage` to the surviving ids. A daily driver picked 100 times is
the oldest entry, so 12 one-off sims evict it and destroy its usage count on
the next save. Reproduced: `DAILY-DRIVER` (usage=100) + 12 one-offs → one save
→ gone.
*Fix:* on re-pick, move the entry to the back (true LRU), or prune by lowest
usage instead of age.

**[DONE] 1.4 A failed `test --failed` rerun destroys the failure set it rents.
[code]** test.rs:253 deletes the retained bundle unconditionally before the
run; if the rerun dies before producing a bundle (compile error, boot failure,
Ctrl-C) the previous failures are gone and the next `test --failed` errors
"no failures to rerun".
*Fix:* run into a sibling temp slot and swap it over the retained path only
once the new bundle exists.

**[DONE] 1.5 A cancelled `dependency add` leaves a dangling package reference.
[code]** dependency.rs writes the pbxproj (step 1) before the product/target
prompts (step 2) — deliberately, since resolution needs the reference on disk
— but an Esc/Ctrl-C at the picker returns exit 6 with the mutation kept: the
exact dangling-ref state the non-interactive guard above it exists to prevent.
*Fix:* snapshot the original pbxproj text and restore it on any error/cancel
after the step-1 write.

**[DONE] 1.6 Quarantine clobbers the previous corruption backup. [repro]**
`load_or_quarantine` renames onto the fixed `state.toml.corrupt`; a second
corruption silently destroys the first backup — the one holding the real
context.
*Fix:* suffix the backup with a timestamp/counter when the path exists.

**[DONE] 1.7 A signal during `--hot-selfcheck` leaves the fixture source rewritten.
[code]** The self-check writes the nonce into the file, waits up to ~200s, and
restores only on the normal path (app.rs:1148–1159); SIGINT/SIGTERM in that
window `_exit`s with the file corrupted. CI-fixture-only today.
*Fix:* restore via a scopeguard plus registering the path for the signal
handler to note (or write the nonce to a copy).

## 2. Wrong results

**[DONE] 2.1 `format --check` passes on files that need formatting. [repro]**
format.rs invokes `swift-format lint` without `--strict`, which exits 0 on
violations, so `--check` reports `passed: true`/exit 0 while printing the
violations — betraying its own doc ("non-zero exit if changes are needed") and
CI users.
*Fix:* add `--strict` for swift-format (swiftlint already exits non-zero via
`--strict`? verify per tool) and cover with a fixture test.

**[DONE] 2.2 On Swift packages, `--show-command` executes the real build/tests.
[repro]** The SPM early-returns in build.rs and test.rs sit above the
`show_command` checks, so the documented dry run compiles the package / runs
`swift test`. The same early-returns silently drop `--` passthrough for both,
and `--coverage`/`--retry-flaky` for test (guard rejects only
failed/result-bundle/junit).
*Fix:* hoist the `show_command` check above the SPM branch (emitting the
`swift build`/`swift test` argv as the preview); pass passthrough through
swiftpm; wire `--coverage` to `swift test --enable-code-coverage` or reject it.

**[DONE] 2.3 A test run that fails before any test ran reports nonsense. [repro]**
Two shapes. (a) No bundle produced (`test -- -zzBogusFlag`): the human path
has no bundle-exists guard (TestPlan::run guards only json/ndjson), so it
falls into `test_summary` → exit **1** with "reading the test results: xcrun
xcresulttool … exited with 64", while the identical `--json` run exits 3 with
the real error. (b) Bundle produced but zero tests (scheme not testable, build
failed — modern xcodebuild writes an xcresult anyway): the report renders a
vacuous "0 passed, 0 failed, 0 skipped (0 total)" (exit 3) and the `--json`
payload carries `failures: []` with **no failure reason at all**.
*Fix:* guard the human path like json; when `!passed && total == 0`, replace
the summary with a BuildFailure error carrying the transcript tail / result
string.

**[DONE] 2.4 One non-UTF-8 output line makes a *successful* build report as failed.
[repro]** `stream_lines` (process.rs:139) breaks on the first `Err` from
`lines()`; the `for`-loop temporary owns the pipe, so the break closes it and
a child with >64KB still buffered dies of SIGPIPE → exit non-zero → "build
failed" for an exit-0 build; with less pending output the rest of the
transcript (including later diagnostics) is silently lost from rendering,
`build diagnostics`, and ndjson events. Same inlined pattern in app.rs
`build()`.
*Fix:* read with `read_until(b'\n')` + `String::from_utf8_lossy` so bad bytes
degrade instead of ending the stream.

**[DONE] 2.5 `archive` ignores `--destination`/`--on`/`--sdk` and archives
`generic/platform=iOS` always; the Release default is defeated by remembered
state. [repro]** archive.rs hardcodes the destination and never reads
`resolved.sdk`/`targeting.on` (flags accepted via the flattened tier). A macOS
or watchOS app cannot be archived correctly. And because
`resolved.configuration` folds in remembered state, one prior plain `build`
(which remembers Debug) makes every later `archive` silently build **Debug**
— reproduced via `archive --show-command` → `-configuration Debug`.
*Fix:* derive `generic/platform=<X>` from `--on`/`--destination`/the scheme's
platform; plumb sdk; for configuration, skip the remembered layer (flag >
config > sweetpad.toml > "Release") or warn loudly when archiving a
remembered Debug.

**[DONE] 2.6 `status` ignores the flag/env layer it claims to display. [repro]**
`Status { target: ContainerArgs }` accepts no scheme/config/destination flags
and never sees `SWEETPAD_SCHEME` etc., so with `SWEETPAD_SCHEME=Bogus` status
shows the remembered scheme while `build` fails on `Bogus` (exit 4). The
`"flag/env"` provenance label in status.rs is unreachable dead code.
*Fix:* flatten `BuildTargetArgs` into Status (and the bare-`sweetpad` gate) so
the shown context is the one build will use.

**[DONE] 2.7 `app_bundle` picks the first `.app` target — wrong app for
watch-companion schemes. [plausible]** xcodebuild.rs:502 returns the first
target with a `.app` wrapper + bundle id; in an iOS+watchOS scheme the watch
app builds first (dependency order), so `app run` would install the watch app
onto the iPhone simulator.
*Fix:* filter candidates by the destination's platform
(SUPPORTED_PLATFORMS/SDKROOT) before first-pick.

**[DONE] 2.8 `test --sdk` is accepted and silently dropped. [repro]** `TestPlan` has
no `sdk` field; `resolve_testing` computes the sdk and test.rs never passes
it. Verified via `--show-command`: build emits `-sdk`, test doesn't.
*Fix:* add `sdk` to TestPlan and plumb it like BuildPlan.

**[DONE] 2.9 An exact device-name match loses to a substring simulator match.
[code]** In `resolve_on` (resolve.rs:606) a device is consulted only when *no*
simulator matched, so device "Speedy" loses to simulator "Speedy Clone" —
contradicting the rustdoc's "preferring exact name" and silently installing to
the wrong hardware.
*Fix:* rank exact name matches (sim or device) above substring matches before
applying the sims-first policy.

**[DONE] 2.10 Tool-prefixed errors are recorded with `location: "clang"`. [repro]**
`parse_diagnostic` treats everything before `": error: "` as a location, so
`clang: error: linker command failed…` / `xcodebuild: error: …` produce
`location:"clang"` in ndjson events, the `build diagnostics` artifact, and
`--gh-annotations` (`::error file=clang::…`).
*Fix:* shape-check the location (path-like: contains `/` or parses as
`file[:line[:col]]`), else emit `location: null`.

**[DONE] 2.11 GitHub annotation *property* values are unescaped. [repro]** Workflow
commands require `%`→%25, `:`→%3A, `,`→%2C in property values; `location_props`
(buildlog.rs:338) interpolates the raw path, so `/Users/x/My Project,v2/…`
splits the property list and anchors the annotation to the wrong file (message
escaping is already correct).
*Fix:* escape property values per the spec.

**[DONE] 2.12 The documented `name=` destination form breaks `run`/`app`. [repro]**
The config examples (and `--destination`'s own help) use
`platform=iOS Simulator,name=iPhone 15`; `build` passes it through fine, but
`app.rs::udid()` demands `id=` → `sweetpad run` exits 4 with "app commands
need a destination with an id=". Reproduced from both flag and config.
*Fix:* when the specifier has `name=` but no `id=`, resolve the UDID via
`simctl list` (name + newest-OS match) instead of erroring.

**[DONE] 2.13 A stale remembered scheme is a self-perpetuating hard error. [repro]**
Rename a scheme and every plain `build` fails `unknown scheme "App"` with no
hint that the value is remembered or how to clear it — and since flags never
overwrite state, `build --scheme App2` works once while the next plain build
fails again; on a TTY it errors rather than re-prompting.
*Fix:* when a *remembered* value fails validation, say so ("remembered scheme
… no longer exists; run `context remove scheme`"), and interactively drop to
the picker (updating state) instead of erroring.

## 3. Stability — signals, terminal, processes

**[DONE] 3.1 `sweetpad run | head` dies of SIGPIPE with the terminal left raw.
[code]** `is_interactive()` checks stderr only, so with stdout piped the
raw-mode session still starts; session output is `println!`-ed from detached
threads; when the pipe reader exits, SIGPIPE (restored to SIG_DFL in main)
kills the CLI with no `Drop` — termios stays no-echo/no-ISIG (`stty sane` to
recover) and the console-pty child, log-stream child, and reparented `log`
all leak.
*Fix:* install a SIGPIPE handler that performs the same termios restore +
child sweep before `_exit(141)` (the async-signal-safe handler already exists
— register it for SIGPIPE while a session is active), or ignore SIGPIPE for
the session and end it on EPIPE write errors.

**[DONE] 3.2 `app debug`: Ctrl-C — lldb's own break gesture — kills sweetpad under
lldb. [code]** lldb runs as an inherited-stdio child in our process group;
terminal SIGINT reaches both, lldb handles it but our handler `_exit(130)`s →
the shell prompt returns while orphaned lldb still owns the TTY.
*Fix:* set SIGINT to SIG_IGN (or a no-op disposition) around the interactive
`lldb` wait, the way `system(3)`/git-around-pagers do.

**[DONE] 3.3 `simulator record` can't succeed: Ctrl-C always exits 130 before the
payload renders. [code]** Ctrl-C is the documented stop; the handler `_exit`s
immediately, so the `SimScreenshot { path }` payload/envelope is unreachable,
exit is always 130, and the CLI's death races `recordVideo`'s mp4
finalization (scripted consumers can read a truncated file). A SIGTERM instead
orphans `recordVideo` entirely (not in CHILD_PIDS) — it keeps recording and
growing the file until the simulator shuts down.
*Fix:* a graceful-record mode: register the child, have the handler forward
SIGINT to it and *return* (skip `_exit`) when a flag is set, then wait for the
child, render the payload, exit 0. Register it so SIGTERM stops it too.

**[DONE] 3.4 A stdin at EOF makes the build watcher busy-spin a core. [code]**
`poll_key` maps a 0-byte read to `Idle`, but after the `poll(2)` gate a zero
read *is* EOF (`/dev/null`, closed pipe → always readable) — so `build()`'s
Ctrl-C watcher thread (spawned unconditionally, app.rs:1544) spins at 100%
CPU for the whole build on any non-TTY stdin, e.g. the CI hot-selfcheck.
*Fix:* map post-poll `read == 0` to `Closed` and break the watcher on
`Closed` (a TTY in the session's raw mode never returns 0 after a successful
poll; Ctrl-D arrives as 0x04).

**[DONE] 3.5 Every Ctrl-C'd `app logs` leaks a `log` process inside the simulator.
[code]** The session path reaps its reparented `log stream` via a predicate
marker + `pkill` on Drop; `stream_logs` (app.rs:2628) passes `marker: None`
and has no Drop path, so each `app logs` + Ctrl-C leaves one `log` streaming
in the sim forever, accumulating across invocations.
*Fix:* route `app logs` through `LogStream` (marker + Drop), and have the
signal path rely on the same registration.

**[DONE] 3.6 The session's console children are never registered for
signal-cleanup. [code]** CHILD_PIDS' own doc says "log streams, device
consoles", but `start_app` registers neither the `simctl launch --console-pty`
child nor the devicectl console; a SIGTERM mid-session leaves the console
child (and the app) running.
*Fix:* `register_child` them alongside the log streams.

**[DONE] 3.7 No SIGHUP handler — closing the terminal detaches the build instead of
stopping it. [code]** Only SIGINT/SIGTERM are handled; on terminal close the
CLI dies by default while the session build's *own process group*
(spawn_piped_group) never gets the signal → xcodebuild keeps building
headless, holding the DerivedData lock.
*Fix:* register the same handler for SIGHUP.

**[DONE] 3.8 `kill -TSTP` wedges the shell on a raw terminal; Ctrl-Z is silently
dead in-session. [code]** Raw mode clears ISIG so keyboard Ctrl-Z is just an
ignored byte (job control unavailable — surprising but safe); an externally
delivered SIGTSTP stops the process with the terminal still raw, and there's
no SIGCONT hook to re-assert modes after `fg`.
*Fix:* handle TSTP/CONT (restore termios, raise default stop, re-enable raw on
continue), or at minimum map 0x1A to a "job control is disabled here" note.

**[DONE] 3.9 Narrow signal-ordering windows (grouped). [code]** (a) RawMode enable
runs `tcsetattr` before `set_raw`, and Drop runs `clear_raw` before
`tcsetattr` — a signal in either gap skips the restore; restore-then-clear /
set-then-flip makes both idempotent. (b) children are registered after spawn
and deregistered *after* `wait()` (LogStream::drop, build's pgid clear), so a
signal in the gap can SIGTERM a freed, recycled pid — deregister before
reaping. All windows are microseconds wide; fix by reordering.

**[DONE] 3.10 The handler overrides an inherited SIG_IGN. [code]** POSIX shells set
SIGINT to ignored in non-job-control background children; `signals::install`
unconditionally replaces it, so a backgrounded sweetpad dies with the
foreground job's Ctrl-C.
*Fix:* check the previous disposition and keep SIG_IGN (per-signal), the
classic `if (signal(SIGINT, SIG_IGN) != SIG_IGN) signal(SIGINT, handler)`.

**[DONE] 3.11 A read error is classified `Closed`, which quits the session and
terminates the app. [code, latent]** Any `read` failure (e.g. a future EINTR
path) maps to `Input::Closed` → session break → `terminate_app`. Currently
unreachable (SA_RESTART + `_exit`ing handlers) but unguarded.
*Fix:* retry on EINTR explicitly; only 0-after-poll and real errors are
`Closed`.

**[DONE] 3.12 Mac-target "detach" likely kills the app on its next print.
[plausible]** The macOS app's stdout/stderr are pipes into the CLI; after
detach the CLI exits, the read ends close, and the app's next
`print`/stderr write raises SIGPIPE (default: terminate). The in-code comment
("stdout may stall once the pipe fills") describes a different, gentler
failure than the actual one.
*Fix:* for Mac runs, spawn with stdio to a file/null when a detach is
possible, or hand the pipes to a lingering drain process; at minimum fix the
comment and warn on `d`.

## 4. Machine-output contract

**[DONE] 4.1 Bare `sweetpad --json` outside a project prints the human help wall on
stdout, exit 0. [repro]** The `resource == None` fallback ignores json/ndjson;
inside a project the same invocation correctly emits the status envelope.
*Fix:* under json/ndjson emit the container-resolution error envelope (exit 4)
— or a `{"help": …}` payload — never bare text on stdout.

**[DONE] 4.2 `build --watch --json` hangs forever with zero bytes of output.
[repro]** The watch loop discards each iteration's payload and never renders;
`app run` rejects json for exactly this reason, watch paths don't. A `--json`
consumer blocks indefinitely; failing iterations would emit repeated error
envelopes on stderr mid-"stream".
*Fix:* reject `--watch` + json (mirroring `app run`), or emit one ndjson
result event per iteration under `-o ndjson`.

**[DONE] 4.3 The ndjson stream contract holds only on migrated happy paths. [repro]**
(a) Self-emitting/Streamed commands never produce the terminal
`{"event":"result"}` line — `dep resolve -o ndjson` exits 0 with **empty
stdout** (its helpers gate on `is_json()` only). (b) No error path emits a
result event (the stream just stops; the envelope goes to stderr). (c) The
`is_json()`-only gates also leak raw child stdout into the event stream:
`dep resolve/update -o ndjson` print `Fetching https://…` lines, `format
--tool swiftlint -o ndjson` interleaves violation lines.
*Fix:* audit every `is_json()` gate to `is_json() || is_ndjson()`; make
`render_result`'s `Err` arm emit `{"event":"result","ok":false,…}` under
ndjson before the stderr envelope; convert the dep report helpers to payloads.

**[DONE] 4.4 `--gh-annotations` is a silent no-op under `--json`/`-o ndjson`.
[repro]** Annotations are generated only in `BuildProgress::line` (the human
path); run_captured/run_ndjson never consult the flag — precisely the CI
combination the flag exists for.
*Fix:* emit annotations (stdout workflow commands are line-oriented and
GitHub-side tolerant) from the captured/ndjson paths too, or reject the
combination loudly.

**[DONE] 4.5 The `--version --json` fast path is greedy and one-eyed. [repro]**
It matches the literal tokens anywhere in argv — including after `--` — so
`sweetpad build --json -- -V` prints the version envelope, exits 0, and never
builds; meanwhile `-o json --version` gets plain text (the path knows only
`--json`), and `--version --json -o human` emits JSON although `-o` is
documented to win.
*Fix:* stop scanning at `--`, and honor `-o`/`--output` in the check.

**[DONE] 4.6 Declining a confirmation exits 0; `help exit-codes` promises 6.
[repro]** `dd purge` / `simulator delete` answered "n" return a normal payload
("aborted") with exit 0 — indistinguishable from success for scripts; only
Esc/Ctrl-C maps to UserCancel.
*Fix:* map a declined prompt to exit 6 (UserCancel) with the "aborted" note,
or amend the help topic — pick one and test it.

**[DONE] 4.7 xcodebuild's stderr bypasses the parser. [code]** `stream_lines` pipes
stdout only (stderr inherited), so invocation-level errors ("xcodebuild:
error: Scheme X is not currently configured…") reach the terminal raw but are
invisible to ndjson events, `StreamStats` counts, and the `build diagnostics`
artifact (the `--json` path is fine — run_captured merges both).
*Fix:* capture stderr through the same parser (spawn_piped_both + merge).

**[DONE] 4.8 An env-sourced `SWEETPAD_DESTINATION` makes a typed `--on` a hard clap
error. [repro]** `conflicts_with = "destination"` fires on env-sourced values,
so `.envrc`-exported destination + `--on mac` → usage error (exit 2) instead
of the documented flag-beats-env. The workspace/project pair got the bespoke
`disambiguate_container` fix; this pair didn't.
*Fix:* drop the clap conflict and resolve the pair post-parse like the
container flags (typed flag wins; both-typed errors).

**[DONE] 4.9 `flag_typed` matches tokens after `--`. [repro]** The argv scan that
implements typed-beats-env for `--workspace`/`--project` sees passthrough
tokens, so `… -- --project` flips container disambiguation.
*Fix:* stop the scan at the first bare `--`.

## 5. UX & polish

**[DONE] 5.1 A malformed user config bricks every command — including the ones that
fix it. [code]** `Config::load` failure is fatal in `run()`, so `sweetpad
help config`, `open config`, and `doctor` all die with the same parse error.
The committed `sweetpad.toml` already warns-and-continues by design.
*Fix:* warn + continue with defaults (the lint already surfaces the message),
or at minimum exempt help/open/doctor.

**[DONE] 5.2 Commands accept targeting flags they ignore. [repro]** `clean` flattens
`BuildTargetArgs` but reads only scheme/configuration (`--destination`,
`--on`, `--sdk` do nothing — a garbage `--on` isn't even validated);
`settings show` honors destination/sdk but silently ignores `--on`.
*Fix:* narrow clean's tier to scheme+configuration; teach settings `--on` (or
reject it).

**[DONE] 5.3 Strict-mode hints name flags the command doesn't have. [repro]**
`resolve::missing()` says "pass --configuration" even from `context select`,
which has no such flag.
*Fix:* thread the invoking command's spelling into the hint (or say
`context set configuration VALUE` for picker-less contexts).

**[DONE] 5.4 Help-topic drift. [code]** `help environment` omits `SWEETPAD_ON` (a
live env var that changes every build); `help config`'s precedence chain
omits the `sweetpad.toml` layer the resolver honors and status/context
display.
*Fix:* update both topics; add a test pinning the env list to the clap
declarations.

**[DONE] 5.5 `context alias` can shadow `--on` keywords. [repro]** Aliases are
substituted before the `mac`/`booted`/platform checks, so `context alias mac
<ref>` silently redefines `--on mac` for that project.
*Fix:* reject reserved names (mac/macos/booted/device/platform words) at
alias-creation time.

**[DONE] 5.6 Bare `sweetpad` warns about ambiguity twice. [repro]** The dispatcher
probes `resolve::container` (which warns) and then `status::run` resolves
again (warns again).
*Fix:* pass the first resolution into status, or silence the probe.

**[DONE] 5.7 `self-update` misdetects Homebrew installs launched via symlink.
[code]** Detection substring-matches `current_exe()` (`/Cellar/`,
`/homebrew/`), but Intel Homebrew's `/usr/local/bin/sweetpad` symlink matches
neither and `current_exe` doesn't resolve symlinks → brewed installs get the
"not installed via Homebrew" advice.
*Fix:* `fs::canonicalize(current_exe())` before matching.

**[DONE] 5.8 Every missing tool is blamed on Xcode. [code]** `spawn_error` appends
"(Xcode command-line tools are required)" to any NotFound — including `brew`,
`lldb`, `pymobiledevice3`.
*Fix:* make the hint per-program (or drop it for non-xcrun tools).

**[DONE] 5.9 `app logs`/`app stop` ignore explicit targeting when a last-launch is
recorded. [code]** Both serve the recorded app first, so `app logs --scheme
Other` silently streams the previously launched app.
*Fix:* skip the last-launched fast path when any scheme/destination flag was
typed.

**[DONE] 5.10 Mac-only projects can't pick their destination interactively. [code]**
`pick_destination` offers simulators only; a macOS-app project must know to
pass `--on mac`/`--destination platform=macOS` (the `devices` command lists
the Mac, the picker doesn't).
*Fix:* include the Mac row in the picker when the project's platforms allow
it.

**[DONE] 5.11 Artifact slots hash with `DefaultHasher`. [code]** SipHash keys are
stable today but the algorithm is explicitly unspecified across Rust
releases; a toolchain bump can orphan every `results/<stem>-<hash16>` slot
(retained xcresults, build-diagnostics artifacts silently start fresh).
*Fix:* hand-roll FNV-1a (a few lines) for a stable, dependency-free hash.

**[DONE] 5.12 `--on` costs three `simctl list` spawns per run. [code]**
`resolve_on` lists; `build_target`'s tracking branch lists again;
`app run`'s summary (`sim_name`) lists a third time — each ~100–300ms.
*Fix:* resolve once and thread the list/name through.

**[DONE] 5.13 `clean --purge` deletes DerivedData with no confirmation while
`derived-data purge` gates. [code]** Defensible (`--purge` is an explicit
flag) but inconsistent with its sibling and undocumented.
*Fix:* pick one behavior and document it in both places.

**[DONE] 5.14 `archive --show-command` previews only the archive step. [code]** The
help says "invocation(s)" but the export invocation (and generated
ExportOptions.plist) is never shown; `--no-export --export-options P`
silently ignores P (no conflict declared).
*Fix:* preview both commands; declare the conflict.

**[DONE] 5.15 The ndjson contract isn't in CLI_DESIGN. [doc]** The
one-result-line/stream shape exists only in the `-o` flag's help and the
OutputMode rustdoc (which overpromises vs. 4.3's reality); CLI_DESIGN §4
mentions ndjson once in passing.
*Fix:* document the stream contract (events on stdout, one terminal result
line, errors as a stderr envelope) beside the JSON envelope section.

## 6. Verified sound

Checked deliberately and found correct — recorded so the next audit doesn't
re-litigate them:

- The `--json` error envelope stays one physical stderr line even with a
  25-line captured tail containing `\r`, `%`, commas, and multibyte UTF-8
  (serde escapes them); `tail_lines` can't panic on char boundaries.
- `NO_COLOR=""` does **not** disable color; `NO_COLOR=1` beats
  `CLICOLOR_FORCE=1`; `-q` beats `-v`; `-o json -q` still emits the envelope.
- SIGPIPE is SIG_DFL for plain list commands (`| head` exits 141, no panic,
  no busy loop) — the session case is 3.1.
- The signal handler body is async-signal-safe (write/tcsetattr/kill/_exit +
  atomics only); the leaked `RAW_TERMIOS` box can't be use-after-freed; no
  `kill(0)`/self-pgid path exists; nothing writes to child stdin (the inject
  server's TcpStream sets SO_NOSIGPIPE).
- Atomic state saves: same-directory pid-suffixed tmp + rename; concurrent
  saves are last-writer-wins with complete files (documented); pruning keeps
  unmounted-volume entries; `Container::key()`'s canonicalize-with-
  absolutize-fallback prevents cwd-dependent duplicate keys.
- Precedence flag > env (container pair) > user config > sweetpad.toml >
  remembered holds behaviorally in `resolve`/`resolve_testing`/`context show`;
  `--project=path` equals-form is detected by `flag_typed`.
- `project_artifact` slots are distinct for same-named projects; `--failed`
  reads selectors before clearing the bundle (within one invocation — see 1.4
  for the cross-invocation hole).
- JUnit escaping covers `& < > "` in attributes; the merge driver survives
  spaced paths (git shell-quotes `%P`); `merge run` can't double-process a
  path; alias arity is guarded (`required_unless_present`/`conflicts_with`).
- vscode JSON-RPC frames carry `schema:1`, cap at 16MB, and error cleanly on
  EOF/timeouts.

## 7. Suggested priorities

1. The state-integrity cluster: **1.1, 1.2, 1.3** (quiet, compounding data
   loss) plus the one-line reorders in **1.6**.
2. The CI liars: **2.1** (format --check), **2.2** (SPM --show-command),
   **2.3** (test misreports), **4.6** (decline exits 0) — anything that lets
   a red state read green.
3. The terminal/process safety set: **3.1–3.5** share one fix surface
   (signals.rs + session teardown) and are best landed together.
4. The ndjson/json sweep: **4.1–4.4** are mostly mechanical
   (`is_json()||is_ndjson()` + one render_result arm).
5. Everything in §5 is independent and small; **5.11** before the next Rust
   toolchain bump.
