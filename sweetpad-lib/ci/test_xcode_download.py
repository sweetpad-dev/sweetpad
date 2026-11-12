#!/usr/bin/env python3
"""Download integration tests use a local server and tiny synthetic archives."""
import contextlib
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import importlib.util
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import threading
import unittest
from unittest.mock import patch

spec = importlib.util.spec_from_file_location("xcode_download", Path(__file__).with_name("linux-xcode-download.py"))
download = importlib.util.module_from_spec(spec)
spec.loader.exec_module(download)
auth = download.auth


@contextlib.contextmanager
def server(payload, *, truncate=False, ignore_range=False, text=False, bad_range=False, redirect=False):
    seen = []

    class Handler(BaseHTTPRequestHandler):
        def log_message(self, *_):
            pass

        def do_GET(self):
            seen.append(dict(self.headers))
            if self.path.startswith("/services-account/download"):
                self.send_response(200)
                self.end_headers()
                return
            if redirect:
                self.send_response(302)
                self.send_header("Location", "https://example.com/stolen")
                self.end_headers()
                return
            requested = self.headers.get("Range")
            offset = int(requested.removeprefix("bytes=").removesuffix("-")) if requested and not ignore_range else 0
            self.send_response(206 if offset else 200)
            self.send_header("Content-Length", str(len(payload) - offset))
            self.send_header("Content-Type", "text/html" if text else "application/octet-stream")
            self.send_header("ETag", '"synthetic-version-1"')
            if offset:
                self.send_header("Content-Range", f"bytes {0 if bad_range else offset}-{len(payload)-1}/{len(payload)}")
            self.end_headers()
            self.wfile.write(payload[offset:len(payload)//2] if truncate else payload[offset:])

    httpd = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
    worker = threading.Thread(target=httpd.serve_forever, daemon=True)
    worker.start()
    url = f"http://127.0.0.1:{httpd.server_port}"

    class Client(auth.HTTPClient):
        def __init__(self):
            super().__init__(allowed=lambda target: target.startswith(url + "/"))

        def open(self, target, **kwargs):
            parsed = download.urllib.parse.urlsplit(target)
            return super().open(url + parsed.path + ("?" + parsed.query if parsed.query else ""), **kwargs)

    try:
        yield Client(), seen
    finally:
        httpd.shutdown()
        worker.join()
        httpd.server_close()


class DownloadTests(unittest.TestCase):
    payload = b"AEA1" + bytes(range(256)) * 16
    release = {"name": "Xcode 27.3 beta", "path": "/Developer_Tools/Xcode_27.3_beta/Xcode_27.3_beta.aea", "size": len(payload)}

    def test_version_selection_does_not_silently_substitute_beta_for_release(self):
        catalog = [{"name": "Xcode 27.3 beta", "files": [{"remotePath": self.release["path"], "fileSize": "4100"}]}]
        self.assertEqual(download.select_release(catalog, "27.3-beta1")["path"], self.release["path"])
        with self.assertRaisesRegex(auth.AppleError, "absent"):
            download.select_release(catalog, "27.3")
        self.assertEqual(download.normalize_version("27.2.0"), "27.2")

    def test_prerelease_links_accept_apple_xcode_archives_only(self):
        links = download.PrereleaseLinks()
        links.feed('<a class="direct-download" href="/services-account/download?path=/Developer_Tools/Xcode_27.3_beta/Xcode_27.3_beta.aea">Xcode</a>'
                   '<a class="direct-download" href="https://example.com/Developer_Tools/Xcode_27.3_beta/x.aea">bad</a>')
        self.assertEqual(len(links.downloads), 1)
        self.assertEqual(download.select_release(links.downloads, "27.3 beta")["path"], self.release["path"])

    def test_catalog_paths_cannot_escape_apple_or_the_download_directory(self):
        for path in ["https://example.com/file.aea", "/Developer_Tools/../file.aea", "/Developer_Tools/%2e%2e/file.aea",
                     "/Developer_Tools/file.aea?secret=token", "/Developer_Tools/file.zip"]:
            with self.subTest(path=path), self.assertRaises(auth.AppleError):
                download.archive_path(path)

    def test_complete_download_receipt_and_cached_file_are_verified(self):
        with tempfile.TemporaryDirectory() as directory, server(self.payload) as (client, seen):
            path = Path(directory) / "Xcode.aea"
            session = auth.Session(Path(directory) / "session.json", client)
            first = download.download(session, self.release, path)
            self.assertFalse(first["reused"])
            self.assertEqual(path.read_bytes(), self.payload)
            count = len(seen)
            reused = download.download(session, self.release, path)
            self.assertTrue(reused["reused"])
            self.assertEqual(len(seen), count)
            path.write_bytes(b"AEA1corrupt")
            with self.assertRaisesRegex(auth.AppleError, "differs"):
                download.download(session, self.release, path)

    def test_interrupted_download_resumes_with_validated_content_range(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "Xcode.aea"
            with server(self.payload, truncate=True) as (client, _):
                with self.assertRaisesRegex(auth.AppleError, "incomplete|interrupted"):
                    download.download(auth.Session(Path(directory) / "session", client), self.release, path)
            self.assertFalse(path.exists())
            partial = path.with_name(path.name + ".part")
            offset = partial.stat().st_size
            self.assertGreater(offset, 0)
            with server(self.payload) as (client, seen):
                result = download.download(auth.Session(Path(directory) / "session", client), self.release, path)
            self.assertEqual(path.read_bytes(), self.payload)
            self.assertEqual(seen[-1]["Range"], f"bytes={offset}-")
            self.assertEqual(seen[-1]["If-Range"], '"synthetic-version-1"')
            self.assertEqual(result["archiveBytes"], len(self.payload))

    def test_server_ignoring_range_restarts_without_appending_duplicate_data(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "Xcode.aea"
            partial = path.with_name(path.name + ".part")
            partial.write_bytes(self.payload[:100])
            partial.chmod(0o600)
            auth.private_write(partial.with_name(partial.name + ".json"), {"sourcePath": self.release["path"], "validator": '"old"'})
            with server(self.payload, ignore_range=True) as (client, _):
                download.download(auth.Session(Path(directory) / "session", client), self.release, path)
            self.assertEqual(path.read_bytes(), self.payload)

    def test_recovery_after_eof_does_not_request_an_unsatisfiable_range(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "Xcode.aea"
            partial = path.with_name(path.name + ".part")
            partial.write_bytes(self.payload)
            partial.chmod(0o600)
            auth.private_write(partial.with_name(partial.name + ".json"), {
                "sourcePath": self.release["path"], "validator": '"synthetic-version-1"', "expectedTotal": len(self.payload)})
            with server(self.payload) as (client, seen):
                download.download(auth.Session(Path(directory) / "session", client), self.release, path)
            self.assertNotIn("Range", seen[-1])
            self.assertEqual(path.read_bytes(), self.payload)

    def test_xip_download_and_read_only_ci_session(self):
        payload = b"xar!" + self.payload[4:]
        release = dict(self.release, path="/Developer_Tools/Xcode_27.2/Xcode_27.2.xip", name="Xcode 27.2")
        with tempfile.TemporaryDirectory() as directory, server(payload) as (client, _):
            session = auth.Session(Path(directory) / "session.json", client)
            with patch.object(session, "save", side_effect=AssertionError("CI session must stay read-only")):
                result = download.download(session, release, Path(directory) / "Xcode.xip")
            self.assertEqual(result["xcodeVersion"], "27.2")

    def test_disk_and_catalog_size_errors_stop_before_writing_archive_data(self):
        with tempfile.TemporaryDirectory() as directory, server(self.payload) as (client, _):
            path = Path(directory) / "Xcode.aea"
            session = auth.Session(Path(directory) / "session", client)
            with self.assertRaisesRegex(auth.AppleError, "length differs"):
                download.download(session, dict(self.release, size=len(self.payload) + 1), path)
            with patch.object(download.shutil, "disk_usage") as usage:
                usage.return_value.free = 1
                with self.assertRaisesRegex(auth.AppleError, "disk space"):
                    download.download(session, self.release, path)
            self.assertFalse(path.with_name(path.name + ".part").exists())

    def test_rejects_html_wrong_magic_size_mismatch_and_external_redirect(self):
        for arguments, payload in [({"text": True}, self.payload), ({}, b"HTML" + self.payload[4:]),
                                   ({"redirect": True}, self.payload)]:
            with self.subTest(arguments=arguments, payload=payload[:4]), tempfile.TemporaryDirectory() as directory:
                path = Path(directory) / "Xcode.aea"
                with server(payload, **arguments) as (client, _), self.assertRaises(auth.AppleError):
                    download.download(auth.Session(Path(directory) / "session", client), self.release, path)
                self.assertFalse(path.exists())

    def test_rejects_invalid_range_and_preserves_original_partial_file(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "Xcode.aea"
            partial = path.with_name(path.name + ".part")
            partial.write_bytes(self.payload[:100])
            partial.chmod(0o600)
            auth.private_write(partial.with_name(partial.name + ".json"), {"sourcePath": self.release["path"], "validator": '"old"'})
            with server(self.payload, bad_range=True) as (client, _), self.assertRaisesRegex(auth.AppleError, "range"):
                download.download(auth.Session(Path(directory) / "session", client), self.release, path)
            self.assertEqual(partial.read_bytes(), self.payload[:100])

    def test_symlinked_destination_is_never_overwritten(self):
        with tempfile.TemporaryDirectory() as directory:
            original = Path(directory) / "original"
            original.write_text("preserve")
            path = Path(directory) / "Xcode.aea"
            path.symlink_to(original)
            with server(self.payload) as (client, _), self.assertRaises(auth.AppleError):
                download.download(auth.Session(Path(directory) / "session", client), self.release, path)
            self.assertEqual(original.read_text(), "preserve")

    def test_non_interactive_cli_returns_json_without_prompting_or_network(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "session.json"
            result = subprocess.run([sys.executable, str(Path(__file__).with_name("linux-xcode-download.py")),
                                     "login", "--apple-id", "synthetic@example.com", "--session-file", str(path),
                                     "--non-interactive"], capture_output=True, text=True, timeout=10)
            self.assertNotEqual(result.returncode, 0)
            self.assertIn("non-interactive", json.loads(result.stdout)["error"])
            self.assertFalse(path.exists())
            logout = subprocess.run([sys.executable, str(Path(__file__).with_name("linux-xcode-download.py")),
                                     "logout", "--session-file", str(path)], capture_output=True, text=True, timeout=10)
            self.assertTrue(json.loads(logout.stdout)["loggedOut"])


if __name__ == "__main__":
    unittest.main()
