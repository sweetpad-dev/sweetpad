# SweetPad CLI — blank-slate DX explorations

Companion to `CLI_AUDIT.md`. That file is "what's inconsistent or broken";
this one is "what would make this the best developer experience in its
category, while the surface can still change dramatically." Everything here
is an option, not a commitment — items are tagged **[bet]** (surface-shaping,
decide before v1 freezes), **[add]** (additive, any time), **[small]**
(afternoon-sized delight).

The benchmark set: `flutter run` (the gold-standard interactive dev loop),
`cargo` (verb-first loop + guessable defaults), `gh` (resource grammar +
dynamic completions + `--json` fields), `fly`/`railway` (status-first,
fuzzy references), `rustc`/`cargo nextest` (error presentation, `--failed`).
The metrics that matter: **keystrokes to first run**, **time to feedback**,
**guessability without docs**, and **agent-parseability** — sweetpad
explicitly serves AI agents, so the last one is unusually load-bearing here.

---

## 1. The grammar itself

### 1.1 Verb-first for the dev loop, nouns for management **[bet]**

The daily loop is typed hundreds of times; management commands a few times a
week. Best-in-class CLIs are verb-first exactly where frequency is highest
(`cargo build/run/test`, `flutter run`, `go test`) and resource-first where
inventory is the point (`gh pr`, `docker container`). Today the loop is
`sweetpad build start`, `sweetpad app run`, `sweetpad test run` — three
tokens for the most common actions, with the flagship hidden under `app`.

Proposal — promote five verbs to the top level and keep nouns for the rest:

```
sweetpad run          # today: app run   (build + install + launch + logs, keys)
sweetpad build        # today: build start
sweetpad test         # today: test run
sweetpad clean        # NEW: xcodebuild clean; --purge adds derived-data
sweetpad fmt          # today: format run   (or keep `format`; fmt matches cargo/go)
```

`app` stays as the lifecycle noun (`install/launch/logs/stop/uninstall`),
`build`/`test` lose their single-action wrapper. This kills the
`start`-vs-`run` inconsistency by construction, halves the tokens on the hot
path, and matches what fingers already know from cargo/flutter. (Clap
supports both worlds coexisting during a deprecation window: keep
`build start` as a hidden alias for one release.)

Cheaper fallback if verb-first feels wrong: make the action optional —
`sweetpad build` ⇒ `build start`, `sweetpad test` ⇒ `test run`,
`sweetpad app` ⇒ `app run`. Same keystroke win, grammar formally unchanged.

### 1.2 One `devices` view, not three list commands **[bet]**

`destination list`, `simulator list`, `device list` are three spellings of
"what can I run on", each with a different shape. Flutter's single
`flutter devices` is the model. Proposal:

- `sweetpad devices` — everything runnable (mac + booted/available sims +
  physical), each row with its ready specifier, most-used-first (the adaptive
  ordering already exists for the picker; today the *list* commands don't use
  it).
- `simulator` keeps only lifecycle verbs (boot/shutdown/erase/create/delete/
  clone/screenshot/…). Its `list` becomes an alias or a filter
  (`devices --sim`).
- Drop the `destination` and `device` resources entirely (two fewer top-level
  nouns; `device list` is already a strict subset).

### 1.3 Fold the merge trio into one resource **[bet]**

`pbxproj resolve` + `spm resolve` + `merge install/driver` are one feature
spread over three top-level nouns, with `resolve` colliding against
`dependency resolve` (same `Package.resolved`, unrelated job). Proposal:

```
sweetpad merge install [--global]
sweetpad merge run [PATHS…] [--force]     # auto-detects pbxproj vs Package.resolved
sweetpad merge driver <KIND> …            # hidden, unchanged
```

Two top-level resources removed, the verb collision gone, and the mental
model matches the feature ("sweetpad's semantic merge").

### 1.4 `context` → `use` + `status` **[bet]**

`context select/show/remove` is accurate but bureaucratic. The operations
users actually think are "use this from now on" and "what am I pointed at":

```
sweetpad use                          # interactive: scheme → config → destination
sweetpad use --scheme App --dest "iPhone 16 Pro"   # non-interactive setter (closes the audit's `context set` gap)
sweetpad use --testing --config Test
sweetpad status                       # container, scheme, config, destination, last app,
                                      # WITH provenance: where each value came from
sweetpad use --clear [VAR]
```

`rustup default`, `kubectl config use-context`, `nvm use` all trained this
verb. `status` doubles as the "bare invocation" answer (§3.1). If renaming
feels too aggressive, at minimum add the non-interactive setter and the
provenance column to `context show`.

### 1.5 Removal candidates — subtract while it's cheap **[bet]**

- `device` resource (absorbed by `devices`, §1.2).
- `destination` resource (same).
- `pbxproj`/`spm` resources (absorbed by `merge`, §1.3).
- `settings` as a top-level noun → `project settings` or keep; it's the one
  single-action noun with a real payload. (Weak opinion.)
- `derived-data` → absorbed into `clean` (`sweetpad clean --purge`,
  `clean --path`, `clean --size`)? Or keep for discoverability but alias `dd`.
- `build start --clean` → subsumed by `sweetpad clean && sweetpad build`
  or keep as `build --clean`.

Net effect of §1 taken together: **20 top-level entries → ~13**, every
survivor either a daily verb or a real inventory noun.

## 2. Frictionless targeting (the #1 papercut in the category)

### 2.1 Human destination references **[bet, high leverage]**

`--destination "platform=iOS Simulator,name=iPhone 15"` is xcodebuild's
worst ergonomic export, and today the flag accepts *only* that raw form
(`resolve.rs:302`). Nobody should ever type it. Accept, in order of
specificity:

```
sweetpad run --on "iPhone 16 Pro"     # fuzzy name match over the devices list
sweetpad run --on booted              # the booted sim
sweetpad run --on mac / --on device   # platform words (replaces --mac/--device)
sweetpad run --on 1A2B3C…             # UDID
sweetpad run --on ios                 # newest iOS sim, most-used-first tiebreak
```

One flag (`--on`, with `--destination` kept as the raw escape hatch),
resolved against the same aggregated device list, erroring with a did-you-
mean table on ambiguity. This also unifies the three current addressing
conventions (positional TARGET / `--simulator` / `--device-id`) — they all
become `--on` (or the shared positional on `simulator` verbs). This is the
single biggest "for humans" delta available.

### 2.2 Named device aliases **[add]**

`sweetpad use --alias work-phone --on 00008120-…` then
`sweetpad run --on work-phone`. Stored in state; listed in `devices`.
(fly/ssh-config model. Cheap once 2.1 exists.)

### 2.3 Walk-up discovery **[small, verified gap]**

Discovery is cwd-only (`resolve.rs::discover`) — `sweetpad build` from
`Sources/Feature/` fails with "no .xcodeproj found". git, cargo, npm, flutter
all walk up. Walk parent directories to the git root (or `/`), stopping at
the first container. Zero-surface change, removes a daily "cd .." tax.

### 2.4 A committed, project-local config **[bet]**

`~/.config/sweetpad/config.toml` keyed by absolute path is personal and
doesn't travel: a teammate cloning the repo gets nothing, and the abs-path
key breaks on every checkout location (see audit §1.13). Allow an *optional,
hand-authored, committed* `sweetpad.toml` (or `.sweetpad.toml`) at the repo
root:

```toml
scheme = "MyApp"
configuration = "Debug"
[testing]
configuration = "Test"
[format]
tool = "swiftlint"
```

Precedence slot: between user-config and remembered state. This is how teams
standardize (mise/.mise.toml, fastlane, swiftlint all trained the pattern).
It doesn't violate "no files *written* to the project root" — sweetpad still
never writes it; users author it. It also gives `format`/`new`/`hot` their
missing config home, colocated with the project.

### 2.5 Resolution provenance everywhere **[small]**

Precedence bugs (audit §1.3) are invisible because resolution is silent. In
`status` (and `-v` on any build-ish command), print *where* each value came
from:

```
scheme         MyApp        (remembered — picked 2d ago)
configuration  Debug        (default — project has: Debug, Release, UAT)
destination    iPhone 16    (sweetpad.toml)
```

`go env`, `pip config debug`, `git config --show-origin` all do this; it
turns "why is it building for the wrong thing" from a bug report into a
glance.

## 3. The flagship session (`sweetpad run`)

### 3.1 Bare `sweetpad` = status, not help **[small]**

In a project directory, bare `sweetpad` printing the `status` view (container,
context, doctor-lite one-liner, "run `sweetpad run` to start") is a far
better front door than the clap help wall. Outside a project, keep help.
(`fly`, `railway`, `git status` muscle memory.)

### 3.2 Adopt the flutter-run keymap **[add]**

The session already owns raw mode with `r`/`q`. Flutter's keymap is the
category standard and every mobile dev already knows it:

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

### 3.3 A one-line session header **[small]**

`MyApp · Debug · iPhone 16 Pro (booted) · hot reload on · build 4.2s` —
persistent first line so the session always answers "what am I running,
where, how fast". Build-time trend (`4.2s ▼`) is a free dopamine loop from
data already in hand.

### 3.4 `--hot` becomes the default posture, eventually **[bet]**

Once the injection client is proven across Xcode versions, the best DX is
flutter's: hot reload *is* the dev loop, not a flag. Path: `--hot` →
`[run] hot = true` config default (§9d's declared nicety) → default-on for
simulator debug builds with `--no-hot` opt-out. Don't rush it; do sequence it.

## 4. Output & agents (the second audience)

### 4.1 `--output`/`-o` instead of boolean `--json` **[bet]**

While the surface is fluid, switch the axis from a bool to an enum:
`-o human` (default) / `-o json` (envelope, today's `--json`) / `-o ndjson`
(streaming events) / `-o quiet`. Boolean `--json` can stay as an alias
forever. This creates the slot NDJSON needs (4.2) without a second migration.

### 4.2 NDJSON event streams for the long-running verbs **[bet, agent-defining]**

`build`/`test`/`run`/`app logs` are *streams*, which is why they're the
`--json` holes today (audit §2). The `buildlog::Event` parser already
produces structured events — emit them:

```
sweetpad build -o ndjson
{"event":"task","kind":"compile","file":"Foo.swift"}
{"event":"diagnostic","severity":"error","file":"…","line":12,"message":"…"}
{"event":"result","ok":false,"errors":3,"duration_ms":48210}
```

Same for test events (case started/passed/failed) and log lines. This is the
feature that makes sweetpad the default tool AI agents reach for — an agent
can watch a build live instead of parsing a beautified transcript. The
parse/render decoupling in §11 of CLI_DESIGN was built for exactly this.

### 4.3 `sweetpad schema` **[small]**

Print the JSON Schema for any command's `-o json` payload
(`sweetpad schema build`, `schema --list`). Solves the "single global
`schema: 1`" opacity (audit §2) and gives agents/tooling a contract to
generate types from. Generate from the serde types with `schemars`.

### 4.4 Last-build diagnostics as a queryable artifact **[add]**

Persist the last build's structured events per project (state dir), then:
`sweetpad build diagnostics [-o json]` — errors/warnings from the last build
without rebuilding. This mirrors the RPC server's most-used method
(`build.diagnostics`) and is the cheap version of "agents don't re-run
builds to re-read errors".

### 4.5 CI is a first-class mode, detected **[small]**

Auto-enable non-interactive when `CI=1` (every modern CLI does); add
`--gh-annotations` (emit `::error file=…::…` from diagnostic events) and
`test --junit PATH`. One env check + two renderers over existing events buys
"works perfectly in Actions out of the box".

### 4.6 `--show-command` / `--dry-run` **[small]**

Print the exact `xcodebuild`/`simctl` invocation (and env) that would run,
then exit. Teaches users what the tool abstracts, de-mystifies bug reports,
and lets agents plan. Pairs beautifully with the "xcodebuild for humans"
positioning: humans graduate.

## 5. Onboarding & lifecycle

### 5.1 `sweetpad init` **[add]**

One command to sweetpad-ify an existing repo, interactive with flags to skip:
pick scheme/destination (seeding state), write `buildServer.json`
(`bsp init`), offer `merge install`, offer a starter `sweetpad.toml` (§2.4),
finish with doctor-lite. "Clone → `sweetpad init` → `sweetpad run`" is the
whole onboarding story, and it's the story the README/docs lead with.

### 5.2 `doctor --fix` **[add]**

Doctor already knows the remedy strings; `--fix` runs the safe ones
(brew installs, `xcodebuild -runFirstLaunch`) with per-item confirmation.
Flutter proved `doctor` is only half the feature.

### 5.3 Dynamic shell completions **[add, quietly best-in-class]**

Static completions exist; the delight tier is completing *values*: scheme
names, simulator names, `--only-testing` identifiers, config names —
clap_complete's dynamic completer wired to the (fast, native) resolver.
`gh` sets the bar here; almost nobody in the Xcode space has this.

### 5.4 Help topics + man pages **[small]**

`sweetpad help destinations`, `help config` (precedence, file locations,
key format!), `help exit-codes`, `help hot-reload` — the design doc's best
sections, shipped into the binary. Plus `clap_mangen` at release time. The
audit's "exit codes are invisible" and "config key format trap" findings both
die here.

### 5.5 First-run hint, not silence **[small]**

On the very first invocation (no state file), after the command's output,
one stderr line: `tip: sweetpad init sets up completions, lsp, and merge
drivers — run it once per repo`. One line, once ever, suppressible.

## 6. Small delights (grab bag)

- **`sweetpad open [xcode|sim|dd|config]`** — open the container in Xcode,
  Simulator.app, the DerivedData folder, or the config file. **[small]**
- **`test --failed`** — rerun only the last run's failures (needs xcresult
  retention from the audit; state already has the per-project slot pattern).
  cargo-nextest/jest's most-loved flag. **[add]**
- **`simulator screenshot --clipboard`**, and screenshots default into
  `./sweetpad-shots/` with the device+time name. **[small]**
- **Aliases**: `sim`, `dd`, `dest` (if it survives), plus `fmt`. **[small]**
- **`settings show --key X` prints the bare value** (scriptability; audit
  §4). **[small]**
- **Fuzzy "did you mean" over scheme/config values**, not just subcommands:
  `--scheme MyAp` → `error: unknown scheme "MyAp" — did you mean "MyApp"?
  (schemes: MyApp, MyAppTests)`. The resolver knows the candidates; today
  they die inside xcodebuild's error. **[small]**
- **Build-time history**: append per-build duration to state;
  `sweetpad stats` sparkline + `status` shows the trend. Local-only. **[add]**
- **Trash, don't delete**: `derived-data purge` moves to the Trash
  (`~/.Trash`) when interactive, rm only with `--yes`/non-interactive.
  Forgiveness > confirmation. **[small]**
- **`--time` on build/test** (or always-on in the result line): cold/warm
  annotation using derived-data presence. **[small]**
- **Respect `SWEETPAD_` prefix everywhere**: audit found `NONINTERACTIVE`
  is `is_some()`-parsed; standardize truthy parsing and document the full
  env set in `help environment` (gh-style). **[small]**

## 7. A north-star surface sketch

What the tree could look like with the §1 bets taken (strawman, 13 entries):

```
sweetpad                        # status in a project; help outside one
sweetpad init                   # onboard a repo (bsp + merge drivers + sweetpad.toml + context)
sweetpad run [--on X] [--hot]   # the flagship session (flutter keymap)
sweetpad build [--clean]        # compile        (-o ndjson for agents)
sweetpad test [--failed] [--junit P]
sweetpad clean [--purge]        # xcodebuild clean; --purge = derived data
sweetpad fmt [--check]
sweetpad devices                # everything runnable, specifier-ready, most-used first
sweetpad simulator <boot|shutdown|erase|create|delete|clone|screenshot|appearance|open|push|…>
sweetpad app <install|uninstall|launch|logs|stop|open-url>   # lifecycle noun
sweetpad use / sweetpad status  # set / show the remembered target (with provenance)
sweetpad project <info|new|settings|open>
sweetpad dependency <list|add|remove|update|resolve>         # alias: dep
sweetpad merge <install|run>    # semantic conflict resolution (pbxproj + Package.resolved)
sweetpad doctor [--fix]
sweetpad completions | schema | help <topic>
```

(17 lines including the utility tail; every daily action ≤ 2 tokens.)

## 8. If you only take five

1. **`--on` human destinations** (§2.1) — the category's worst papercut,
   owned outright.
2. **Verb-first loop or default actions** (§1.1) — `sweetpad run` is the
   brand.
3. **NDJSON events + `-o`** (§4.1–4.2) — makes sweetpad the agent-native
   Xcode CLI, which nothing else is.
4. **`sweetpad init` + committed `sweetpad.toml`** (§5.1, §2.4) — the team
   onboarding story.
5. **Flutter keymap + status header in the session** (§3.2–3.3) — the daily
   feel of the product.
