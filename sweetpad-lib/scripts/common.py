"""Shared helpers for the corpus-capture scripts.

This module is intentionally dependency-free (stdlib only) so the scripts can
run with whatever Python is on PATH. It centralizes:

  - Repo / fixture / corpus path resolution.
  - The 5-project corpus definition and Xcode-version discovery.
  - Destination slugging.
  - `with_xcode(version)` context manager (saves & restores `xcode-select -p`).
  - The env-var allowlist used by the toolchain shim.
  - Subprocess helpers with consistent logging.

Nothing here performs network or sudo operations at import time.
"""

from __future__ import annotations

import contextlib
import dataclasses
import json
import os
import re
import shlex
import shutil
import subprocess
import sys
from pathlib import Path
from typing import Iterable, Iterator, Optional


# --- Paths -----------------------------------------------------------------

REPO_ROOT = Path(__file__).resolve().parent.parent
SCRIPTS_DIR = REPO_ROOT / "scripts"
CORPUS_DIR = REPO_ROOT / "corpus"
FIXTURES_DIR = REPO_ROOT / "fixtures"
XCSPEC_CACHE_DIR = REPO_ROOT / "xcspec-cache"
TOOLSHIM_DIR = SCRIPTS_DIR / "toolshim"
MANIFEST_PATH = CORPUS_DIR / "manifest.json"


# --- Corpus definition -----------------------------------------------------

@dataclasses.dataclass(frozen=True)
class CorpusProject:
    slug: str
    repo: str  # https URL
    pin: str  # "latest-release" | "default-branch"
    notes: str = ""


CORPUS: list[CorpusProject] = [
    CorpusProject(
        slug="ice-cubes",
        repo="https://github.com/Dimillian/IceCubesApp.git",
        pin="latest-release",
        notes="SwiftPM resolves on first build",
    ),
    CorpusProject(
        slug="alamofire",
        repo="https://github.com/Alamofire/Alamofire.git",
        pin="latest-release",
    ),
    CorpusProject(
        slug="netnewswire",
        repo="https://github.com/Ranchero-Software/NetNewsWire.git",
        pin="default-branch",
    ),
    CorpusProject(
        slug="tuist-fixtures",
        repo="https://github.com/tuist/tuist.git",
        pin="latest-release",
        notes="Subset of fixtures/, requires `tuist install && tuist generate`",
    ),
    CorpusProject(
        slug="kingfisher",
        repo="https://github.com/onevcat/Kingfisher.git",
        pin="latest-release",
    ),
]


def project_by_slug(slug: str) -> CorpusProject:
    for p in CORPUS:
        if p.slug == slug:
            return p
    raise KeyError(f"unknown corpus project: {slug}")


# --- Xcode version discovery / switching -----------------------------------

@dataclasses.dataclass(frozen=True)
class XcodeInstall:
    slot: str  # "current" | "prev-major" | "prev-major-2"
    version: str  # e.g. "26.0.1"
    app_path: Path  # /Applications/Xcode-26.0.1.app
    developer_dir: Path  # .../Contents/Developer


_XCODE_APP_RE = re.compile(r"^Xcode-([0-9]+(?:\.[0-9]+)*)\.app$")


def discover_installed_xcodes(apps_dir: Path = Path("/Applications")) -> list[XcodeInstall]:
    """Find /Applications/Xcode-X.Y(.Z).app installs and order them newest first."""
    found: list[tuple[tuple[int, ...], str, Path]] = []
    for entry in apps_dir.iterdir():
        m = _XCODE_APP_RE.match(entry.name)
        if not m:
            continue
        ver = m.group(1)
        try:
            sortable = tuple(int(x) for x in ver.split("."))
        except ValueError:
            continue
        found.append((sortable, ver, entry))
    found.sort(reverse=True)

    # Bucket by major and keep latest minor per major
    by_major: dict[int, tuple[tuple[int, ...], str, Path]] = {}
    for sortable, ver, app in found:
        major = sortable[0]
        if major not in by_major:
            by_major[major] = (sortable, ver, app)

    majors_desc = sorted(by_major.keys(), reverse=True)
    slots = ["current", "prev-major", "prev-major-2"]
    out: list[XcodeInstall] = []
    for slot, major in zip(slots, majors_desc):
        _, ver, app = by_major[major]
        out.append(XcodeInstall(
            slot=slot,
            version=ver,
            app_path=app,
            developer_dir=app / "Contents" / "Developer",
        ))
    return out


def xcode_select_current() -> Path:
    """Return the current `xcode-select -p` developer dir."""
    out = subprocess.run(
        ["xcode-select", "-p"], check=True, capture_output=True, text=True
    ).stdout.strip()
    return Path(out)


def xcode_select_set(developer_dir: Path) -> None:
    """`sudo xcode-select -s <dir>`. Caller is responsible for sudo credentials."""
    subprocess.run(
        ["sudo", "xcode-select", "-s", str(developer_dir)],
        check=True,
    )


@contextlib.contextmanager
def with_xcode(xcode: XcodeInstall) -> Iterator[XcodeInstall]:
    """Select `xcode` for child xcodebuild/xcrun calls via `DEVELOPER_DIR`.

    Sets the `DEVELOPER_DIR` environment variable rather than running
    `sudo xcode-select -s`, so switching Xcode needs **no sudo** and never
    mutates the machine's global active Xcode. xcodebuild / xcrun / swift /
    simctl all honor `DEVELOPER_DIR` ahead of `xcode-select`, and it is
    inherited by every subprocess spawned inside the block — including the
    capture scripts the multi-version orchestrator drives. Restores the prior
    value (or unsets it) on exit.
    """
    target = str(xcode.developer_dir)
    previous = os.environ.get("DEVELOPER_DIR")
    if previous == target:
        yield xcode
        return
    log(f"setting DEVELOPER_DIR -> {target}")
    os.environ["DEVELOPER_DIR"] = target
    try:
        yield xcode
    finally:
        if previous is None:
            os.environ.pop("DEVELOPER_DIR", None)
            log("unset DEVELOPER_DIR")
        else:
            os.environ["DEVELOPER_DIR"] = previous
            log(f"restored DEVELOPER_DIR -> {previous}")


# --- Destination slugging --------------------------------------------------

_SLUG_RE = re.compile(r"[^A-Za-z0-9._-]+")


def slug(text: str) -> str:
    """Filesystem-safe slug. Collapses runs of non-[A-Za-z0-9._-] to '-'."""
    return _SLUG_RE.sub("-", text).strip("-")


def destination_slug(dest: str) -> str:
    """Convert an xcodebuild `-destination` value into a filesystem-safe slug.

    Example:
      "platform=iOS Simulator,name=iPhone 15,OS=latest"
      -> "platform-iOS-Simulator_name-iPhone-15_OS-latest"
    """
    parts = [p.strip() for p in dest.split(",") if p.strip()]
    return "_".join(slug(p) for p in parts)


# --- Toolshim env allowlist ------------------------------------------------

# Exact-match env vars the shim should record. Anything matching the
# SHIM_ENV_PREFIXES regex is also kept.
SHIM_ENV_EXACT: frozenset[str] = frozenset({
    "SDKROOT",
    "BUILT_PRODUCTS_DIR",
    "CONFIGURATION",
    "CONFIGURATION_BUILD_DIR",
    "DERIVED_FILE_DIR",
    "PROJECT_DIR",
    "PROJECT_NAME",
    "SRCROOT",
    "TARGET_NAME",
    "EFFECTIVE_PLATFORM_NAME",
    "ARCHS",
    "CURRENT_ARCH",
    "SWIFT_VERSION",
})

SHIM_ENV_PREFIX_PATTERNS: tuple[str, ...] = (
    "OTHER_",
    "GCC_",
    "SWIFT_",
    "WARNING_",
    "CLANG_",
    "LD_",
)

# Suffix-style match: any var that ends with _SEARCH_PATHS.
SHIM_ENV_SUFFIX_PATTERNS: tuple[str, ...] = (
    "_SEARCH_PATHS",
)


def shim_env_allowlist(env: dict[str, str]) -> dict[str, str]:
    out: dict[str, str] = {}
    for k, v in env.items():
        if k in SHIM_ENV_EXACT:
            out[k] = v
            continue
        if any(k.startswith(p) for p in SHIM_ENV_PREFIX_PATTERNS):
            out[k] = v
            continue
        if any(k.endswith(s) for s in SHIM_ENV_SUFFIX_PATTERNS):
            out[k] = v
    return out


# --- Subprocess helpers ----------------------------------------------------

def log(msg: str) -> None:
    print(f"[sweetpad] {msg}", file=sys.stderr, flush=True)


def run(
    cmd: list[str],
    *,
    cwd: Optional[Path] = None,
    env: Optional[dict[str, str]] = None,
    check: bool = True,
    capture: bool = False,
    quiet: bool = False,
    timeout: Optional[float] = None,
) -> subprocess.CompletedProcess[str]:
    """Run a command with consistent logging.

    If `capture=False`, stdout/stderr stream to the terminal.
    If `capture=True`, both are captured and returned as text.
    """
    if not quiet:
        cwd_str = f" (cwd={cwd})" if cwd else ""
        log("$ " + " ".join(shlex.quote(c) for c in cmd) + cwd_str)
    return subprocess.run(
        cmd,
        cwd=str(cwd) if cwd else None,
        env=env,
        check=check,
        capture_output=capture,
        text=True,
        timeout=timeout,
    )


def run_capture(
    cmd: list[str],
    *,
    cwd: Optional[Path] = None,
    env: Optional[dict[str, str]] = None,
    check: bool = True,
    quiet: bool = True,
) -> str:
    """Convenience: run and return stdout (stripped)."""
    cp = run(cmd, cwd=cwd, env=env, check=check, capture=True, quiet=quiet)
    return cp.stdout.rstrip()


# --- Fixture-path helpers --------------------------------------------------

def fixture_dir(slug_: str, xcode_version: str) -> Path:
    return FIXTURES_DIR / slug_ / f"xcode-{xcode_version}"


def metadata_dir(slug_: str, xcode_version: str) -> Path:
    return fixture_dir(slug_, xcode_version) / "metadata"


def raw_dir(slug_: str, xcode_version: str) -> Path:
    return fixture_dir(slug_, xcode_version) / "raw"


def build_dir(slug_: str, xcode_version: str) -> Path:
    return fixture_dir(slug_, xcode_version) / "build"


def errors_dir(slug_: str, xcode_version: str) -> Path:
    return fixture_dir(slug_, xcode_version) / "errors"


def write_error(slug_: str, xcode_version: str, step: str, message: str) -> None:
    d = errors_dir(slug_, xcode_version)
    d.mkdir(parents=True, exist_ok=True)
    (d / f"{step}.txt").write_text(message + "\n")
    log(f"WROTE error: {d / (step + '.txt')}")


def ensure_dir(p: Path) -> Path:
    p.mkdir(parents=True, exist_ok=True)
    return p


# --- Manifest helpers ------------------------------------------------------

def load_manifest() -> dict:
    if not MANIFEST_PATH.exists():
        return {"projects": {}}
    with MANIFEST_PATH.open() as f:
        return json.load(f)


def save_manifest(data: dict) -> None:
    ensure_dir(CORPUS_DIR)
    with MANIFEST_PATH.open("w") as f:
        json.dump(data, f, indent=2, sort_keys=True)
        f.write("\n")


# --- Misc ------------------------------------------------------------------

def brew_path() -> Optional[Path]:
    for p in (Path("/opt/homebrew/bin/brew"), Path("/usr/local/bin/brew")):
        if p.exists() and os.access(p, os.X_OK):
            return p
    return None


def which(tool: str, *, path: Optional[str] = None) -> Optional[Path]:
    p = shutil.which(tool, path=path)
    return Path(p) if p else None


def host_macos_version() -> str:
    return run_capture(["sw_vers", "-productVersion"])


def xcodebuild_version() -> str:
    """Returns the full multi-line `xcodebuild -version` output."""
    return run_capture(["xcodebuild", "-version"])


# --- Iteration -------------------------------------------------------------

def selected_projects(only: Optional[str] = None) -> list[CorpusProject]:
    if only:
        return [project_by_slug(only)]
    return list(CORPUS)


def selected_xcodes(installs: list[XcodeInstall], only: Optional[str] = None) -> list[XcodeInstall]:
    if only is None:
        return installs
    out = [x for x in installs if x.version == only or x.slot == only]
    if not out:
        raise SystemExit(
            f"--xcode {only!r} matches no installed Xcode "
            f"(have: {[x.version for x in installs]})"
        )
    return out
