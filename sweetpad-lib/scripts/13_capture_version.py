#!/usr/bin/env python3
"""Multi-version capture orchestrator.

Captures a second (or Nth) Xcode version alongside the existing corpus inside a
strict acquire -> capture -> validate -> teardown envelope, so the resolver is
validated against more than one Xcode without ever holding two transient Xcodes
on disk at once. See the "Multi-version capture" section of PLAN.md for the
design; this script only *drives* the existing numbered steps (02/03/04/07-12),
it does not re-implement them.

Per version the loop is:

  preflight disk -> `xcodes install <ver>` -> discover the real installed
  version -> switch xcode-select to it -> provision the iOS sim runtime ->
  snapshot xcspecs (04) -> per smoke-subset project: capture metadata+raw (02),
  smoke build (03), reclaim its `.derived` -> settings steps (07-12) ->
  `cargo test` (version-aware oracle) -> tear the Xcode.app + runtime back down.

The corpus clones under `corpus/<slug>/` are shared across versions (cloned once
via 01, never re-cloned). Kept permanently per version: `fixtures/<slug>/
xcode-<ver>/` and `xcspec-cache/xcode-<ver>/`. Peak disk is one Xcode + one
runtime + fixtures, flat across N versions.

This loop is **sudo-free for an Xcode equal-or-older than the system one**:
`xcodes install --no-superuser` skips the privileged license/first-launch steps
(the system Xcode's license is system-wide and covers equal-or-older versions);
Xcode is selected per-step via `DEVELOPER_DIR` instead of `sudo xcode-select`;
runtimes download via `xcodebuild -downloadPlatform` (sudo-free); transient Xcodes
install into a project-local gitignored `.xcodes/` folder (`--xcodes-dir`) and are
removed with a plain `rm -rf`. The one-time interactive step is the `xcodes`
Apple-ID sign-in (a single 2FA code), after which installs run unattended.

**Caveat — capturing an Xcode *newer* than the system one** (e.g. 26.5 when the
system Xcode is 26.0.1): the skipped license/first-launch steps are NOT covered by
the system license, so `xcodebuild` refuses with a license error, then a
`CoreSimulator`/`IDESimulatorFoundation` plugin-load error, until **two one-time
`sudo` commands** are run by hand (these write to root-owned `/Library`):
  sudo DEVELOPER_DIR=<app>/Contents/Developer xcodebuild -license accept
  sudo DEVELOPER_DIR=<app>/Contents/Developer xcodebuild -runFirstLaunch
After those, the rest (platform downloads, captures, teardown) is sudo-free.
Run `--check-auth` first to surface the sign-in.

Flags:
  --versions V [V ...]  Xcode versions to capture (e.g. 16.4.0). The token is
                        passed to `xcodes install`; the *actually installed*
                        version (from the resulting Xcode-<ver>.app) is then
                        rediscovered and used for all fixture paths.
  --subset a,b,c        corpus slugs to capture (default: the smoke subset
                        alamofire,kingfisher,ice-cubes). tuist-fixtures is
                        best-effort; netnewswire is intentionally excluded.
  --keep                skip teardown (leave the Xcode.app + runtime installed) —
                        use on the first wet run to inspect before trusting it.
  --force               recapture even if the version's fixtures already exist.
  --no-runtime          skip sim-runtime provisioning (settings-only escape
                        hatch; smoke builds needing a simulator will then fail).
  --xcodes-dir DIR      install transient Xcodes here (default: <repo>/.xcodes,
                        gitignored). One `rm -rf` reclaims them; a system
                        /Applications Xcode is reused and never deleted.
  --dry-run             print every step without installing, switching,
                        building, deleting, or running the numbered scripts.
  --check-auth          preflight only: report xcodes / sudo / runtime tooling
                        and the one-time sign-in steps, then exit.

Exit codes: 0 success, 1 a version failed or a preflight blocked the run.
"""

from __future__ import annotations

import argparse
import os
import shlex
import shutil
import subprocess
import sys
import threading
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import common  # noqa: E402


# Smoke subset (see PLAN.md): fast framework, framework+SwiftPM, app+SPM.
SMOKE_SUBSET: list[str] = ["alamofire", "kingfisher", "ice-cubes"]
# Slugs that may fail to generate against a new Xcode — never block the run.
BEST_EFFORT: frozenset[str] = frozenset({"tuist-fixtures"})
# Base slug the synthetic-override step (07) layers KEY=VALUE captures onto.
SYNTHETIC_OVERRIDE_BASE = "alamofire"

# Disk budget before acquiring a version: ~50 GB Xcode + ~10 GB runtime + a
# build/headroom margin. Preflight aborts below this so a run never wedges /.
DISK_BUDGET_GB = 70

# Simulator runtime platform provisioned for smoke builds (16.x -> iOS 18.x).
RUNTIME_PLATFORM = "iOS"

# Where transient Xcodes are installed: a project-local, gitignored folder so a
# single `rm -rf .xcodes` reclaims every Xcode this tool installed. Because it's
# a user-owned directory and we install with `--no-superuser`, neither install
# nor teardown needs sudo. Override with --xcodes-dir. (A system Xcode the user
# already has in /Applications is still reused and never deleted.)
XCODES_DIR_DEFAULT = common.REPO_ROOT / ".xcodes"

DERIVED_DIR_NAME = ".derived"  # 03's per-fixture -derivedDataPath


# --- low-level command runner (dry-run aware) ------------------------------

def run_cmd(
    cmd: list[str],
    *,
    dry: bool,
    allow_fail: bool = False,
    cwd: Path | None = None,
) -> int:
    """Log and run `cmd`; in dry-run just log it and return 0.

    Non-zero exit raises SystemExit unless `allow_fail` (best-effort steps).
    """
    prefix = "DRY-RUN $ " if dry else "$ "
    cwd_str = f"  (cwd={cwd})" if cwd else ""
    common.log(prefix + " ".join(shlex.quote(c) for c in cmd) + cwd_str)
    if dry:
        return 0
    cp = subprocess.run(cmd, cwd=str(cwd) if cwd else None)
    if cp.returncode != 0 and not allow_fail:
        raise SystemExit(f"command failed ({cp.returncode}): {' '.join(cmd)}")
    return cp.returncode


def run_script(name: str, *script_args: str, dry: bool, allow_fail: bool = False) -> int:
    """Invoke one of the numbered capture scripts via the current interpreter."""
    return run_cmd(
        [sys.executable, str(common.SCRIPTS_DIR / name), *script_args],
        dry=dry,
        allow_fail=allow_fail,
    )


# --- disk / tooling preflight ----------------------------------------------

def free_gb(path: Path = common.REPO_ROOT) -> float:
    st = os.statvfs(path)
    return st.f_bavail * st.f_frsize / (1024**3)


def preflight_disk(need_gb: int, *, dry: bool) -> None:
    have = free_gb()
    common.log(f"free disk: {have:.1f} GB (need ~{need_gb} GB to acquire a version)")
    if have < need_gb:
        msg = f"insufficient disk: {have:.1f} GB free < {need_gb} GB budget"
        if dry:
            common.log(f"DRY-RUN: {msg} (would abort)")
        else:
            raise SystemExit(msg)


def sudo_ok() -> bool:
    """True if sudo currently runs without a password prompt."""
    return subprocess.run(["sudo", "-n", "true"], capture_output=True).returncode == 0


def check_auth(*, want_install: bool, want_runtime: bool) -> int:
    """`--check-auth` body: report the acquire prerequisites, fix-it hints, 0/1.

    The Apple-ID session that `xcodes` caches has no cheap read-only probe (no
    `xcodes whoami`), so we report what is verifiable — the binary, sudo, the
    runtime tooling — and print the one-time `xcodes signin` / `sudo -v` steps
    rather than falsely asserting the session is live.
    """
    issues: list[str] = []
    print("=== capture-version preflight ===")

    xcodes = common.which("xcodes")
    if xcodes:
        ver = common.run_capture(["xcodes", "version"], check=False) or "(unknown)"
        print(f"  xcodes:   {xcodes} ({ver})")
    else:
        print("  xcodes:   NOT FOUND")
        if want_install:
            issues.append("xcodes not on PATH — `brew install xcodes`")

    username = os.environ.get("XCODES_USERNAME")
    if username:
        print(f"  apple id: $XCODES_USERNAME={username} (xcodes will use it)")
    else:
        print("  apple id: unknown — xcodes caches the session after a one-time")
        print("            `xcodes signin` (one 2FA code); the first `xcodes")
        print("            install` will otherwise prompt interactively.")

    if sudo_ok():
        print("  sudo:     active (no password prompt)")
    else:
        print("  sudo:     not primed — run `sudo -v` before an unattended run")

    if want_runtime:
        for tool in ("xcrun", "xcodebuild"):
            p = common.which(tool)
            print(f"  {tool+':':9} {p or 'NOT FOUND'}")
            if not p:
                issues.append(f"{tool} not found (install the Xcode command line tools)")

    print("")
    sys.stdout.flush()
    if issues:
        print("=== preflight: NOT READY ===", file=sys.stderr)
        for i, m in enumerate(issues, 1):
            print(f"  [{i}] {m}", file=sys.stderr)
        return 1
    print("=== preflight: ready to acquire ===")
    print("  one-time, if not already done:")
    print("    xcodes signin        # caches the Apple-ID session")
    print("    sudo -v              # primes sudo for the xcode-select switches")
    return 0


# --- sudo keepalive --------------------------------------------------------

def start_sudo_keepalive(*, dry: bool) -> threading.Event | None:
    """Keep an already-primed sudo timestamp warm for the run's duration.

    The capture itself is sudo-free (Xcode is selected via DEVELOPER_DIR), so
    sudo is only needed for teardown's `rm -rf`. We never *prompt* here — if
    sudo isn't already primed we run without it and let teardown skip.
    Note: macOS sudo uses per-tty timestamps, so a `sudo -v` the user ran in
    their own terminal does NOT carry into this process; only a same-context
    prime counts.
    """
    if dry or not sudo_ok():
        if not dry:
            common.log("sudo not primed — running sudo-free (teardown will be skipped)")
        return None
    stop = threading.Event()

    def _loop() -> None:
        while not stop.wait(60):
            subprocess.run(["sudo", "-n", "-v"], capture_output=True)

    threading.Thread(target=_loop, daemon=True).start()
    return stop


# --- acquire / resolve / teardown ------------------------------------------

def installed_versions(xcodes_dir: Path) -> set[str]:
    out = {x.version for x in common.discover_installed_xcodes()}
    if xcodes_dir.is_dir():
        out |= {x.version for x in common.discover_installed_xcodes(xcodes_dir)}
    return out


def resolve_install(
    token: str, before: set[str], xcodes_dir: Path, *, dry: bool
) -> common.XcodeInstall | None:
    """Find the Xcode install that `xcodes install <token>` produced.

    Search the project-local `xcodes_dir` first, then a system /Applications
    install (which is reused but never deleted). Prefer an exact version match,
    then the sole new install since `before`, then a prefix match (`16.4` ->
    `16.4.0`). In dry-run, synthesize a placeholder under `xcodes_dir` so the
    downstream steps can still be printed.
    """
    installs: list[common.XcodeInstall] = []
    if xcodes_dir.is_dir():
        installs += common.discover_installed_xcodes(xcodes_dir)
    installs += common.discover_installed_xcodes()
    for x in installs:
        if x.version == token:
            return x
    delta = [x for x in installs if x.version not in before]
    if len(delta) == 1:
        return delta[0]
    for x in installs:
        if x.version.startswith(token):
            return x
    if dry:
        app = xcodes_dir / f"Xcode-{token}.app"
        return common.XcodeInstall(
            slot="target",
            version=token,
            app_path=app,
            developer_dir=app / "Contents" / "Developer",
        )
    return None


def install_xcode(token: str, xcodes_dir: Path, *, dry: bool) -> None:
    # `--no-superuser` skips the privileged license/first-launch steps, so the
    # install needs NO sudo — only the cached Apple-ID session (one-time
    # `xcodes signin`). On a machine already bootstrapped by a system Xcode the
    # license acceptance is system-wide and covers equal-or-older versions, and
    # read-only `-showBuildSettings` + actual builds then work without the
    # skipped steps (verified on a no-superuser Xcode 15.4 / 16.4 install).
    # `--directory` drops the app in the project-local folder so one `rm -rf`
    # reclaims it; `--experimental-unxip` is faster + lower peak disk;
    # `--empty-trash` reclaims the `.xip` immediately.
    if not dry:
        xcodes_dir.mkdir(parents=True, exist_ok=True)
    run_cmd(
        ["xcodes", "install", token, "--no-superuser", "--experimental-unxip",
         "--empty-trash", "--directory", str(xcodes_dir)],
        dry=dry,
    )


def teardown_xcode(
    install: common.XcodeInstall, baseline_dev: Path, xcodes_dir: Path, *, dry: bool
) -> None:
    """Remove a transient Xcode this tool installed (under `xcodes_dir`). Never
    the baseline, and never a system /Applications Xcode the user manages. The
    app is user-owned (installed with `--no-superuser` into a user-writable
    folder), so a plain `rm -rf` reclaims it without sudo."""
    if install.developer_dir.resolve() == baseline_dev.resolve():
        common.log(f"REFUSING to delete baseline Xcode at {install.app_path}")
        return
    try:
        under_dir = xcodes_dir.resolve() in install.app_path.resolve().parents
    except OSError:
        under_dir = False
    if not under_dir:
        common.log(
            f"{install.app_path} is a system install (outside {xcodes_dir}) — leaving it"
        )
        return
    if not dry and not install.app_path.exists():
        common.log(f"already gone: {install.app_path}")
        return
    run_cmd(["rm", "-rf", str(install.app_path)], dry=dry)


# --- simulator runtime provisioning ----------------------------------------

def simctl_runtime_ids() -> set[str]:
    """Identifiers of currently-installed simulator runtimes (read-only)."""
    import json

    out = common.run_capture(["xcrun", "simctl", "runtime", "list", "-j"], check=False)
    if not out:
        return set()
    try:
        data = json.loads(out)
    except json.JSONDecodeError:
        return set()
    return set(data.keys())


def provision_runtime(*, dry: bool) -> set[str]:
    """Download the iOS sim runtime for the active Xcode; return the new ids.

    The before/after diff lets teardown delete exactly what this run added,
    leaving any runtime the user already had in place.
    """
    before = simctl_runtime_ids()
    run_cmd(["xcodebuild", "-downloadPlatform", RUNTIME_PLATFORM], dry=dry, allow_fail=True)
    after = simctl_runtime_ids()
    new = after - before
    common.log(f"runtimes added this run: {sorted(new) or '(none new / already present)'}")
    return new


def teardown_runtime(ids: set[str], *, dry: bool) -> None:
    for rid in sorted(ids):
        run_cmd(["xcrun", "simctl", "runtime", "delete", rid], dry=dry, allow_fail=True)


# --- per-version capture body ----------------------------------------------

def fixtures_complete(version: str, subset: list[str]) -> bool:
    """True if every subset slug already has captured metadata for `version`."""
    for slug in subset:
        md = common.metadata_dir(slug, version)
        if not md.is_dir() or not any(md.rglob("*.json")):
            return False
    return True


def ensure_corpus_clone(slug: str, *, dry: bool) -> None:
    """Clone the corpus project once (shared across versions); skip if present."""
    if (common.CORPUS_DIR / slug).is_dir():
        common.log(f"corpus/{slug} present — reusing (shared across versions)")
        return
    run_script("01_clone_corpus.py", "--project", slug, dry=dry, allow_fail=slug in BEST_EFFORT)


def purge_derived(slug: str, version: str, *, dry: bool) -> None:
    """Reclaim a project's build output (`.derived`) between projects."""
    d = common.fixture_dir(slug, version) / DERIVED_DIR_NAME
    if dry:
        common.log(f"DRY-RUN: would rm -rf {d}")
        return
    if d.exists():
        common.log(f"reclaiming {d}")
        shutil.rmtree(d, ignore_errors=True)


def capture_projects(
    version: str, subset: list[str], *, no_runtime: bool, force: bool, dry: bool
) -> None:
    force_args = ["--force"] if force else []
    for slug in subset:
        best_effort = slug in BEST_EFFORT
        ensure_corpus_clone(slug, dry=dry)
        # 02 populates raw/ + metadata (no-dest, per-target, project-defaults,
        # scheme) — the `-showBuildSettings` oracle inputs the resolver tests
        # score. 03 runs the smoke build (compiled artifacts + toolshim per-file
        # settings); it needs a simulator runtime and its outputs are NOT oracle
        # inputs, so a settings-only (`--no-runtime`) run skips it.
        run_script("02_capture_metadata.py", "--xcode", version, "--project", slug,
                   *force_args, dry=dry, allow_fail=best_effort)
        if no_runtime:
            common.log(f"--no-runtime: skipping smoke build (03) for {slug}")
        else:
            run_script("03_run_builds.py", "--xcode", version, "--project", slug,
                       *force_args, dry=dry, allow_fail=best_effort)
        purge_derived(slug, version, dry=dry)


def capture_settings_steps(version: str, subset: list[str], *, force: bool, dry: bool) -> None:
    """Steps 07-12: cheap, no smoke builds. Per-project ones loop the subset."""
    force_args = ["--force"] if force else []
    # 08 global defaults, 11 synthetic xcconfigs: version-global, no corpus project.
    run_script("08_global_defaults.py", "--xcode", version, *force_args, dry=dry, allow_fail=True)
    run_script("11_synthetic_xcconfigs.py", "--xcode", version, *force_args,
               dry=dry, allow_fail=True)
    # 09 per-target/project-defaults, 10 real-xcconfig, 12 PIF dumps: per project.
    for slug in subset:
        run_script("09_per_project_settings.py", "--xcode", version, "--project", slug,
                   *force_args, dry=dry, allow_fail=True)
        run_script("10_xcconfig_resolution.py", "--xcode", version, "--project", slug,
                   *force_args, dry=dry, allow_fail=True)
        run_script("12_pif_dumps.py", "--xcode", version, "--slug", slug,
                   *force_args, dry=dry, allow_fail=True)
    # 07 synthetic overrides layer onto alamofire's scheme; only if it's captured.
    if SYNTHETIC_OVERRIDE_BASE in subset:
        run_script("07_synthetic_overrides.py", "--base", SYNTHETIC_OVERRIDE_BASE,
                   "--xcode", version, *force_args, dry=dry, allow_fail=True)


def validate(version: str, *, dry: bool) -> None:
    """Run the version-aware oracle tests (they score every captured version)."""
    run_cmd(
        ["cargo", "test", "--test", "corpus_oracle", "--test", "per_target_oracle",
         "--test", "project_defaults_oracle", "--test", "synthetic_override_oracle",
         "--test", "xcconfig_resolution_oracle"],
        dry=dry, cwd=common.REPO_ROOT, allow_fail=True,
    )


def capture_version(
    token: str,
    *,
    subset: list[str],
    baseline_dev: Path,
    xcodes_dir: Path,
    keep: bool,
    force: bool,
    no_runtime: bool,
    min_disk_gb: int,
    dry: bool,
) -> bool:
    """Acquire -> capture -> validate -> teardown for one version. True on success."""
    common.log(f"==== version {token} ====")
    if fixtures_complete(token, subset) and not force:
        common.log(f"fixtures for {token} already complete — skipping (use --force)")
        return True

    preflight_disk(min_disk_gb, dry=dry)

    before = installed_versions(xcodes_dir)
    if token not in before:
        install_xcode(token, xcodes_dir, dry=dry)
    else:
        common.log(f"Xcode {token} already installed — reusing")

    install = resolve_install(token, before, xcodes_dir, dry=dry)
    if install is None:
        common.log(f"could not locate the installed Xcode for {token} after install")
        return False
    version = install.version  # canonical: the real app's version
    if version != token:
        common.log(f"note: requested {token}, installed/resolved as {version} — using {version}")

    runtime_ids: set[str] = set()
    ok = True
    with common.with_xcode(install) if not dry else _DryXcodeSwitch(install):
        if not no_runtime:
            runtime_ids = provision_runtime(dry=dry)
        else:
            common.log("--no-runtime: skipping sim runtime (smoke builds may fail)")
        run_script("04_snapshot_xcspecs.py", "--xcode", version,
                   *(["--force"] if force else []), dry=dry, allow_fail=True)
        capture_projects(version, subset, no_runtime=no_runtime, force=force, dry=dry)
        capture_settings_steps(version, subset, force=force, dry=dry)

    validate(version, dry=dry)

    if keep:
        common.log(f"--keep: leaving Xcode {version} and its runtime installed")
    else:
        teardown_runtime(runtime_ids, dry=dry)
        teardown_xcode(install, baseline_dev, xcodes_dir, dry=dry)
    return ok


class _DryXcodeSwitch:
    """Dry-run stand-in for `with_xcode` that only logs the switch/restore."""

    def __init__(self, install: common.XcodeInstall) -> None:
        self.install = install

    def __enter__(self) -> common.XcodeInstall:
        common.log(f"DRY-RUN: would switch xcode-select -> {self.install.developer_dir}")
        return self.install

    def __exit__(self, *exc: object) -> None:
        common.log("DRY-RUN: would restore xcode-select -> (baseline)")


# --- main ------------------------------------------------------------------

def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__,
                                 formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--versions", nargs="+", metavar="VER",
                    help="Xcode versions to capture (e.g. 16.4.0)")
    ap.add_argument("--subset", default=",".join(SMOKE_SUBSET),
                    help=f"comma-separated corpus slugs (default: {','.join(SMOKE_SUBSET)})")
    ap.add_argument("--keep", action="store_true",
                    help="skip teardown (leave Xcode.app + runtime installed)")
    ap.add_argument("--force", action="store_true",
                    help="recapture even if the version's fixtures already exist")
    ap.add_argument("--no-runtime", action="store_true",
                    help="skip sim-runtime provisioning (settings-only)")
    ap.add_argument("--min-disk-gb", type=int, default=DISK_BUDGET_GB,
                    help=f"free-disk floor before acquiring a version "
                         f"(default: {DISK_BUDGET_GB})")
    ap.add_argument("--xcodes-dir", default=str(XCODES_DIR_DEFAULT),
                    help=f"folder to install transient Xcodes into (default: "
                         f"{XCODES_DIR_DEFAULT}; gitignored — one `rm -rf` reclaims them all). "
                         f"A system /Applications Xcode is still reused and never deleted")
    ap.add_argument("--dry-run", action="store_true",
                    help="print every step without side effects")
    ap.add_argument("--check-auth", action="store_true",
                    help="preflight xcodes/sudo/runtime tooling and exit")
    args = ap.parse_args()

    if args.check_auth:
        return check_auth(want_install=True, want_runtime=not args.no_runtime)

    if not args.versions:
        ap.error("--versions is required (or use --check-auth)")

    subset = [s.strip() for s in args.subset.split(",") if s.strip()]
    unknown = [s for s in subset if s not in {p.slug for p in common.CORPUS}]
    if unknown:
        ap.error(f"unknown subset slug(s): {unknown}")

    dry = args.dry_run
    common.log(f"versions={args.versions} subset={subset} "
               f"keep={args.keep} force={args.force} no_runtime={args.no_runtime} dry={dry}")

    baseline_dev = common.xcode_select_current()
    common.log(f"baseline xcode-select: {baseline_dev}")

    if not dry:
        rc = check_auth(want_install=True, want_runtime=not args.no_runtime)
        if rc != 0:
            return rc

    stop = start_sudo_keepalive(dry=dry)
    failures: list[str] = []
    try:
        for token in args.versions:
            try:
                if not capture_version(
                    token,
                    subset=subset,
                    baseline_dev=baseline_dev,
                    xcodes_dir=Path(args.xcodes_dir),
                    keep=args.keep,
                    force=args.force,
                    no_runtime=args.no_runtime,
                    min_disk_gb=args.min_disk_gb,
                    dry=dry,
                ):
                    failures.append(token)
            except SystemExit:
                raise
            except Exception as e:  # one bad version must not strand the rest
                common.log(f"version {token} FAILED: {e}")
                failures.append(token)
    finally:
        if stop is not None:
            stop.set()

    if failures:
        common.log(f"versions with failures: {failures}")
        return 1
    common.log("all versions captured")
    return 0


if __name__ == "__main__":
    sys.exit(main())
