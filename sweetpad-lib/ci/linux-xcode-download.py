#!/usr/bin/env python3
"""Authenticate with Apple and download private Xcode archives on Linux or macOS."""

import argparse
import contextlib
import fcntl
import hashlib
import http.client
from html.parser import HTMLParser
import importlib.util
import json
import os
from pathlib import Path
import re
import shutil
import stat
import sys
import time
import urllib.parse


spec = importlib.util.spec_from_file_location("sweetpad_apple_auth", Path(__file__).with_name("apple-auth.py"))
auth = importlib.util.module_from_spec(spec)
spec.loader.exec_module(auth)
AppleError = auth.AppleError


def normalize_version(value):
    value = re.sub(r"^xcode[\s_-]+", "", value.strip(), flags=re.I)
    value = re.sub(r"[\s_-]+", "", value.lower()).replace("releasecandidate", "rc")
    match = re.fullmatch(r"(\d+(?:\.\d+){0,2})((?:beta|rc)\d*)?", value)
    if not match:
        raise AppleError("Use an explicit Xcode version, such as 27.2 or '27.3 beta'.")
    numbers = [int(part) for part in match[1].split(".")]
    while len(numbers) > 1 and numbers[-1] == 0:
        numbers.pop()
    suffix = match[2] or ""
    if suffix in ("beta1", "rc1"):
        suffix = suffix[:-1]
    return ".".join(map(str, numbers)) + suffix


def archive_path(value):
    # Catalog values are paths, never arbitrary URLs or filesystem names.
    if not isinstance(value, str):
        raise AppleError("Apple returned an unsupported Xcode archive path.")
    path = urllib.parse.unquote(value)
    if (not path.startswith("/Developer_Tools/") or "\\" in path
            or any(piece in (".", "..") for piece in path.split("/"))
            or "?" in path or "#" in path or any(ord(c) < 32 for c in path)
            or Path(path).suffix.lower() not in (".xip", ".aea")):
        raise AppleError("Apple returned an unsupported Xcode archive path.")
    return path


class PrereleaseLinks(HTMLParser):
    def __init__(self):
        super().__init__()
        self.downloads = []

    def handle_starttag(self, tag, attrs):
        attributes = dict(attrs)
        if tag != "a" or "direct-download" not in attributes.get("class", "").split():
            return
        url = urllib.parse.urljoin("https://developer.apple.com/download/", attributes.get("href", ""))
        if not auth.apple_url(url):
            return
        parsed = urllib.parse.urlsplit(url)
        path = urllib.parse.parse_qs(parsed.query).get("path", [parsed.path])[0]
        filename = Path(path).name
        if not re.fullmatch(r"Xcode_\d[\w. -]*\.(aea|xip)", filename, flags=re.I):
            return
        path = archive_path(path)
        self.downloads.append({"name": Path(filename).stem.replace("_", " "),
                               "files": [{"remotePath": path}]})


def select_release(downloads, version):
    selected = normalize_version(version)
    choices = {}
    for item in downloads:
        if not isinstance(item, dict) or not isinstance(item.get("name"), str):
            continue
        if not re.match(r"^Xcode\s+\d", item["name"], re.I):
            continue
        try:
            matches = normalize_version(item["name"]) == selected
        except AppleError:
            continue
        if not matches:
            continue
        for entry in item.get("files", []):
            if not isinstance(entry, dict):
                continue
            raw = entry.get("remotePath", "")
            if Path(raw).suffix.lower() not in (".xip", ".aea"):
                continue
            path = archive_path("/" + raw.lstrip("/"))
            size = entry.get("fileSize")
            if size is not None:
                try:
                    size = int(size)
                    if size <= 0:
                        raise ValueError()
                except (TypeError, ValueError):
                    raise AppleError("Apple returned an invalid archive size.") from None
            choices[path] = {"name": item["name"], "path": path, "size": size}
    if not choices:
        raise AppleError(f"Xcode {version} is absent from Apple's downloads. Use its exact release or beta version.")
    # The two formats can describe the same release; prefer the smaller modern archive.
    modern = [item for path, item in choices.items() if path.lower().endswith(".aea")]
    choices = modern or list(choices.values())
    if len(choices) != 1:
        raise AppleError("Several Apple downloads match this version. Use a more specific beta or release-candidate version.")
    return choices[0]


def digest_file(path):
    digest = hashlib.sha256()
    with path.open("rb") as source:
        for block in iter(lambda: source.read(1024 * 1024), b""):
            digest.update(block)
    return digest.hexdigest()


def regular_owned(path):
    info = path.lstat()
    if (not stat.S_ISREG(info.st_mode) or info.st_uid != os.geteuid()
            or info.st_mode & 0o077):
        raise AppleError("Download cache files must be owned regular files with mode 600.")
    return info


@contextlib.contextmanager
def download_lock(path):
    descriptor = os.open(path, os.O_CREAT | os.O_RDWR | os.O_NOFOLLOW | os.O_NONBLOCK, 0o600)
    with os.fdopen(descriptor, "a") as output:
        regular_owned(path)
        try:
            fcntl.flock(output, fcntl.LOCK_EX | fcntl.LOCK_NB)
        except BlockingIOError:
            raise AppleError("Another process is downloading this Xcode archive.") from None
        yield


def default_destination(filename):
    cache = Path(os.environ.get("XDG_CACHE_HOME", Path.home() / ".cache"))
    if not cache.is_absolute():
        cache = Path.home() / ".cache"
    return cache / "sweetpad/downloads" / filename


def download(session, release, destination, persist_session=False):
    auth.private_directory(destination.parent)
    path = release["path"]
    url = "https://download.developer.apple.com" + urllib.parse.quote(path, safe="/")
    receipt = destination.with_name(destination.name + ".sweetpad-download.json")
    partial = destination.with_name(destination.name + ".part")
    state_path = partial.with_name(partial.name + ".json")
    with download_lock(destination.with_name(destination.name + ".lock")):
        if destination.exists() or destination.is_symlink():
            info = regular_owned(destination)
            try:
                previous = json.loads(auth.private_read(receipt))
            except (FileNotFoundError, ValueError):
                raise AppleError("An unverified archive exists at the destination. Choose another --output-file.") from None
            digest = digest_file(destination)
            if (not isinstance(previous, dict) or previous.get("sourcePath") != path or previous.get("sha256") != digest
                    or previous.get("archiveBytes") != info.st_size):
                raise AppleError("Cached Xcode archive differs from its receipt. Choose another --output-file.")
            return dict(previous, downloaded=True, reused=True, archive=str(destination))

        authorization = "https://developer.apple.com/services-account/download?" + urllib.parse.urlencode({"path": path})
        with session.client.open(authorization, headers={"Accept": "*/*"}, follow=False) as response:
            if response.code in (401, 403):
                raise AppleError("Apple download authorization expired. " + auth.LOGIN_HINT)
            if response.code not in (200, 302, 303, 307):
                raise AppleError(f"Apple download authorization failed (HTTP {response.code}).")
            if response.code != 200:
                redirect = urllib.parse.urljoin(authorization, response.headers.get("Location", ""))
                parsed = urllib.parse.urlsplit(redirect)
                if (not auth.apple_url(redirect) or parsed.hostname != "download.developer.apple.com"
                        or urllib.parse.unquote(parsed.path) != path):
                    raise AppleError("Apple did not authorize the selected Xcode archive. " + auth.LOGIN_HINT)
                url = redirect
        if persist_session:
            session.save()

        offset, state = 0, {}
        if partial.exists() or partial.is_symlink():
            info = regular_owned(partial)
            try:
                state = json.loads(auth.private_read(state_path))
            except (FileNotFoundError, ValueError):
                raise AppleError("Incomplete download metadata is missing. Remove the .part file before retrying.") from None
            if not isinstance(state, dict) or state.get("sourcePath") != path:
                raise AppleError("This partial download belongs to another Xcode archive.")
            offset = info.st_size
            if offset == state.get("expectedTotal", release["size"]):
                # Recover interruption after EOF but before publication, without
                # asking the server for an unsatisfiable range at end-of-file.
                offset = max(0, offset - 1024 * 1024)
        headers = {"Accept": "*/*", "Accept-Encoding": "identity"}
        if offset and state.get("validator"):
            headers.update(Range=f"bytes={offset}-", **{"If-Range": state["validator"]})
        else:
            offset = 0
        with session.client.open(url, headers=headers) as response:
            if response.code in (401, 403):
                raise AppleError("Apple denied the archive download. " + auth.LOGIN_HINT)
            if response.code not in (200, 206):
                raise AppleError(f"Apple archive download failed (HTTP {response.code}). Retry the same command to resume.")
            if "text/" in response.headers.get("Content-Type", "").lower():
                raise AppleError("Apple returned a sign-in page instead of an archive. " + auth.LOGIN_HINT)
            length = response.headers.get("Content-Length")
            try:
                length = int(length) if length is not None else None
                if length is not None and length < 0:
                    raise ValueError()
            except ValueError:
                raise AppleError("Apple returned an invalid download length.") from None
            if response.code == 206:
                match = re.fullmatch(r"bytes (\d+)-(\d+)/(\d+)", response.headers.get("Content-Range", ""))
                if (not offset or not match or int(match[1]) != offset
                        or int(match[2]) != int(match[3]) - 1
                        or (length is not None and length != int(match[3]) - offset)):
                    raise AppleError("Apple returned an invalid download range. Retry without the partial file.")
                total = int(match[3])
                validator_header = "ETag" if state.get("validator", "").startswith('"') else "Last-Modified"
                current_validator = response.headers.get(validator_header)
                if current_validator and current_validator != state.get("validator"):
                    raise AppleError("Apple archive changed during download. Remove the partial file before retrying.")
            else:
                offset, total = 0, length
            if release["size"] is not None and total is not None and release["size"] != total:
                raise AppleError("Archive length differs from Apple's catalog. Retry after the catalog updates.")
            total = total if total is not None else release["size"]
            if total is not None and shutil.disk_usage(destination.parent).free < total - offset + 64 * 1024 * 1024:
                raise AppleError("Not enough disk space for the Xcode archive. Choose another --output-file.")
            validator = response.headers.get("ETag")
            if validator and validator.startswith("W/"):
                validator = None
            validator = validator or response.headers.get("Last-Modified")
            auth.private_write(state_path, {"sourcePath": path, "validator": validator, "expectedTotal": total})
            descriptor = os.open(partial, os.O_CREAT | os.O_RDWR | os.O_NOFOLLOW | os.O_NONBLOCK, 0o600)
            with os.fdopen(descriptor, "r+b") as output:
                os.fchmod(output.fileno(), 0o600)
                if not stat.S_ISREG(os.fstat(output.fileno()).st_mode):
                    raise AppleError("Partial download must be a regular file.")
                output.seek(offset)
                output.truncate()
                count, started = offset, time.monotonic()
                print(f"Downloading {release['name']} ({offset} bytes already present)", file=sys.stderr)
                try:
                    while True:
                        chunk = response.read(1024 * 1024)
                        if not chunk:
                            break
                        count += len(chunk)
                        if total is not None and count > total:
                            raise AppleError("Apple returned more archive data than expected.")
                        output.write(chunk)
                        if time.monotonic() - started >= 10:
                            print(f"Downloaded {count / 1024**3:.2f} GiB", file=sys.stderr)
                            started = time.monotonic()
                except (OSError, TimeoutError, http.client.HTTPException):
                    raise AppleError("Download interrupted. Retry the same command to resume.") from None
                output.flush()
                os.fsync(output.fileno())
            if total is not None and count != total:
                raise AppleError("Download is incomplete. Retry the same command to resume.")
            with partial.open("rb") as source:
                magic = source.read(4)
            expected = b"AEA1" if path.lower().endswith(".aea") else b"xar!"
            if magic != expected:
                raise AppleError("Downloaded data is not the expected Xcode archive format.")
            digest = digest_file(partial)
            result = {"downloaded": True, "reused": False, "archive": str(destination),
                      "xcodeVersion": release["name"].removeprefix("Xcode "),
                      "sourcePath": path, "archiveBytes": count, "sha256": digest,
                      "nextStep": "Import this archive with sweetpad sdk import, or use sweetpad setup --xcode VERSION."}
            auth.private_write(receipt, result)
            os.replace(partial, destination)
            state_path.unlink(missing_ok=True)
            return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("action", choices=("login", "logout", "download"))
    parser.add_argument("--apple-id")
    parser.add_argument("--session-file")
    parser.add_argument("--xcode")
    parser.add_argument("--output-file", type=Path)
    parser.add_argument("--non-interactive", action="store_true")
    args = parser.parse_args()
    session = auth.Session(auth.session_path(args.session_file))
    try:
        if args.action == "logout":
            if session.path.exists() or session.path.is_symlink():
                auth.private_read(session.path)
                session.path.unlink()
            result = {"loggedOut": True, "sessionFile": str(session.path)}
        elif args.action == "login":
            if not args.apple_id or "@" not in args.apple_id or any(c.isspace() for c in args.apple_id):
                raise AppleError("Supply --apple-id EMAIL.")
            reused = False
            if session.load() and session.apple_id.casefold() == args.apple_id.casefold():
                try:
                    session.catalog()
                    reused = True
                except AppleError:
                    pass
            if not reused:
                if args.non_interactive:
                    raise AppleError("No valid session is available in non-interactive mode. " + auth.LOGIN_HINT)
                # A failed account switch leaves the previously saved session unchanged.
                session.client.cookies.clear()
                auth.AppleLogin(session).login(args.apple_id)
            result = {"authenticated": True, "reused": reused, "appleId": session.apple_id,
                      "sessionFile": str(session.path)}
        else:
            normalize_version(args.xcode or "")
            session.load(required=True)
            releases = session.catalog()
            try:
                release = select_release(releases, args.xcode)
            except AppleError:
                # Apple can publish a new beta on /download before listDownloads includes it.
                with session.client.open("https://developer.apple.com/download/", headers={"Accept": "text/html"}) as response:
                    if response.code != 200:
                        raise AppleError("Cannot read Apple's prerelease download catalog.")
                    raw = auth.response_bytes(response, 8 * 1024 * 1024)
                    links = PrereleaseLinks()
                    links.feed(raw.decode("utf-8"))
                release = select_release(releases + links.downloads, args.xcode)
            destination = (args.output_file or default_destination(Path(release["path"]).name)).expanduser().absolute()
            result = download(session, release, destination,
                              persist_session=not (args.session_file or os.environ.get("SWEETPAD_APPLE_SESSION_FILE")))
        print(json.dumps(result))
        return 0
    except (AppleError, OSError, ValueError, UnicodeError):
        error = sys.exception()
        # Filesystem and parser errors can include input data. Emit only controlled errors.
        message = str(error) if isinstance(error, AppleError) else "Cannot read or write Apple session/download files. Check paths and private permissions."
        print(json.dumps({"error": message}))
        return 1
    except KeyboardInterrupt:
        print(json.dumps({"error": "Cancelled. Retry the download command to resume."}))
        return 130


if __name__ == "__main__":
    sys.exit(main())
