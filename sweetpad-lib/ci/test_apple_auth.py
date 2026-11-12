#!/usr/bin/env python3
"""Authentication tests use synthetic accounts and never contact Apple."""
import base64
from email.message import Message
import hashlib
import gzip
import http.cookiejar
import importlib.util
import json
import io
import os
from pathlib import Path
import stat
import select
import signal
import tempfile
import time
import pty
import termios
import unittest
from unittest.mock import patch
import urllib.request

spec = importlib.util.spec_from_file_location("apple_auth", Path(__file__).with_name("apple-auth.py"))
auth = importlib.util.module_from_spec(spec)
spec.loader.exec_module(auth)


def headers(**items):
    result = Message()
    for key, value in items.items():
        result[key.replace("_", "-")] = value
    return result


def cookie(domain=".apple.com", domain_specified=True):
    return http.cookiejar.Cookie(0, "synthetic", "session-secret", None, False,
                                 domain, domain_specified, domain.startswith("."),
                                 "/", True, True, None, True, None, None, {})


class FakeClient:
    def __init__(self, responses):
        self.responses, self.requests = list(responses), []
        self.cookies = auth.HTTPClient().cookies

    def request(self, url, **kwargs):
        self.requests.append((url, kwargs))
        response = self.responses.pop(0)
        if isinstance(response, Exception):
            raise response
        return response

    def open(self, url, **kwargs):
        self.requests.append((url, kwargs))
        response = io.BytesIO(b"")
        response.code = 200
        return response


class AppleAuthTests(unittest.TestCase):
    def test_gzip_catalog_responses_are_decoded_with_a_limit(self):
        response = io.BytesIO(gzip.compress(b'{"resultCode":0,"downloads":[]}'))
        response.headers = {"Content-Encoding": "gzip"}
        self.assertEqual(json.loads(auth.response_bytes(response))["downloads"], [])
        response = io.BytesIO(gzip.compress(b"a" * 1000))
        response.headers = {"Content-Encoding": "gzip"}
        with self.assertRaisesRegex(auth.AppleError, "size limit"):
            auth.response_bytes(response, 100)

    def test_public_service_key_lookup_does_not_use_authenticated_cookies_or_follow_logout(self):
        client = FakeClient([(302, headers(Location="https://idmsa.apple.com/appleauth/auth/signin?widgetKey=public-key"), {})])
        session = auth.Session(Path("unused"))
        session.client.cookies.set_cookie(cookie())
        login = auth.AppleLogin(session)
        with patch.object(auth, "HTTPClient", return_value=client):
            self.assertEqual(login.service_key(), "public-key")
        self.assertEqual(len(list(client.cookies)), 0)
        self.assertEqual(client.requests[0][1], {"method": "HEAD", "follow": False})

    def test_terminal_password_input_never_echoes_the_secret(self):
        pid, terminal = pty.fork()
        if pid == 0:
            try:
                value = auth.terminal_secret("Password: ")
                os.write(1, b"accepted\n" if value == "synthetic-tty-password" else b"wrong\n")
            finally:
                os._exit(0)
        transcript = b""
        deadline = time.monotonic() + 10
        sent = False
        try:
            while time.monotonic() < deadline:
                ready, _, _ = select.select([terminal], [], [], 0.1)
                if ready:
                    try:
                        data = os.read(terminal, 4096)
                    except OSError:
                        break
                    if not data:
                        break
                    transcript += data
                if b"Password: " in transcript and not sent:
                    self.assertFalse(termios.tcgetattr(terminal)[3] & termios.ECHO)
                    os.write(terminal, b"synthetic-tty-password\n")
                    sent = True
                if b"accepted" in transcript:
                    break
            self.assertIn(b"accepted", transcript)
            self.assertNotIn(b"synthetic-tty-password", transcript)
        finally:
            os.close(terminal)
            try:
                os.kill(pid, signal.SIGTERM)
            except ProcessLookupError:
                pass
            os.waitpid(pid, 0)

    def test_private_session_round_trip_includes_session_only_cookies_not_passwords(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "private/sweetpad/state/session.json"
            session = auth.Session(path)
            session.apple_id = "synthetic@example.com"
            session.client.cookies.set_cookie(cookie())
            previous = os.umask(0)
            try:
                session.save()
            finally:
                os.umask(previous)
            self.assertEqual(stat.S_IMODE(path.stat().st_mode), 0o600)
            self.assertEqual(stat.S_IMODE(path.parent.stat().st_mode), 0o700)
            self.assertEqual(stat.S_IMODE(path.parent.parent.stat().st_mode), 0o700)
            self.assertEqual(stat.S_IMODE(path.parent.parent.parent.stat().st_mode), 0o700)
            stored = json.loads(path.read_text())
            self.assertEqual(set(stored), {"version", "appleId", "cookies"})
            loaded = auth.Session(path)
            self.assertTrue(loaded.load(required=True))
            self.assertEqual(next(iter(loaded.client.cookies)).value, "session-secret")
            self.assertEqual(loaded.apple_id, session.apple_id)

    def test_session_rejects_unsafe_files_symlinks_and_non_apple_cookie_domains(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "session.json"
            session = auth.Session(path)
            session.apple_id = "synthetic@example.com"
            session.save()
            path.chmod(0o644)
            with self.assertRaisesRegex(auth.AppleError, "mode 600"):
                session.load()
            path.chmod(0o600)
            alias = path.with_name("link")
            alias.symlink_to(path)
            with self.assertRaises(OSError):
                auth.Session(alias).load()
            data = json.loads(path.read_text())
            data["cookies"] = [{"name": "x", "value": "secret", "domain": "apple.com.evil.example",
                                "domainSpecified": True, "path": "/", "expires": None}]
            path.write_text(json.dumps(data))
            with self.assertRaisesRegex(auth.AppleError, "Invalid cookie"):
                session.load()

    def test_session_host_only_cookies_and_secure_transport_are_preserved(self):
        jar = auth.HTTPClient().cookies
        jar.set_cookie(cookie("idmsa.apple.com", False))
        for url, expected in [("https://idmsa.apple.com/", True),
                              ("https://child.idmsa.apple.com/", False),
                              ("http://idmsa.apple.com/", False),
                              ("https://example.com/", False)]:
            with self.subTest(url=url):
                request = urllib.request.Request(url)
                jar.add_cookie_header(request)
                self.assertEqual(request.has_header("Cookie"), expected)

    def test_apple_redirects_reject_other_hosts_and_http(self):
        for url in ["https://apple.com.evil.example/", "http://idmsa.apple.com/",
                    "https://user@apple.com/", "https://apple.com:444/", "file:///tmp/cookies"]:
            self.assertFalse(auth.apple_url(url))
        redirect = auth.Redirects(auth.apple_url, True)
        with self.assertRaises(auth.AppleError):
            redirect.redirect_request(None, None, 302, "", {}, "https://example.com/")

    def test_srp_rejects_bad_public_keys_rounds_and_protocols(self):
        self.assertEqual(auth.N.bit_length(), 2048)
        valid = {"salt": base64.b64encode(b"salt").decode(), "b": "Ag==", "iteration": 1000, "protocol": "s2k"}
        for changes in [{"b": "AA=="}, {"b": base64.b64encode(auth.integer_bytes(auth.N)).decode()},
                        {"iteration": 0}, {"iteration": 10_000_001}, {"protocol": "unknown"}, {"b": "!"}]:
            with self.subTest(changes=changes), self.assertRaises(auth.AppleError):
                auth.SRP(private=123).proofs("synthetic@example.com", "password", dict(valid, **changes))

    def test_srp_matches_independent_server_calculation_for_both_apple_password_modes(self):
        # Server S=(A*v^u)^b independently checks the client's subtraction/exponent path.
        username, password, salt = "synthetic@example.com", "not-a-real-password", b"fixture-salt"
        private_b = int("E487CB59D31AC550471E81F00F6928E01DDA08E974A004F49E61F5D105284D20", 16)
        for protocol in ("s2k", "s2k_fo"):
            with self.subTest(protocol=protocol):
                h = lambda x: hashlib.sha256(x).digest()
                pad = lambda value: value.to_bytes(256, "big")
                digest = h(password.encode())
                if protocol == "s2k_fo":
                    digest = digest.hex().encode()
                derived = hashlib.pbkdf2_hmac("sha256", digest, salt, 1000, 32)
                x = int.from_bytes(h(salt + h(b":" + derived)), "big")
                v = pow(2, x, auth.N)
                k = int.from_bytes(h(pad(auth.N) + pad(2)), "big")
                b_public = (k * v + pow(2, private_b, auth.N)) % auth.N
                client = auth.SRP(private=int("60975527035CF2AD1989806F0407210BC81EDC04E2762A56AFD529DDDA2D4393", 16))
                challenge = {"salt": base64.b64encode(salt).decode(),
                             "b": base64.b64encode(auth.integer_bytes(b_public)).decode(),
                             "iteration": 1000, "protocol": protocol}
                m1, m2 = client.proofs(username, password, challenge)
                public_a = int.from_bytes(client.public, "big")
                u = int.from_bytes(h(pad(public_a) + pad(b_public)), "big")
                shared = pow(public_a * pow(v, u, auth.N), private_b, auth.N)
                key = h(auth.integer_bytes(shared))
                xor = bytes(a ^ b for a, b in zip(h(pad(auth.N)), h(pad(2))))
                expected_m1 = h(xor + h(username.encode()) + salt + client.public + auth.integer_bytes(b_public) + key)
                self.assertEqual(base64.b64decode(m1), expected_m1)
                self.assertEqual(base64.b64decode(m2), h(client.public + expected_m1 + key))

    def test_hashcash_uses_apple_decimal_counter_and_leading_zero_bits(self):
        proof = auth.hashcash(8, "fixture-resource", "261112230000")
        self.assertTrue(proof.startswith("1:8:261112230000:fixture-resource::"))
        self.assertLess(int.from_bytes(hashlib.sha1(proof.encode()).digest(), "big"), 1 << 152)
        with self.assertRaises(auth.AppleError):
            auth.hashcash(30, "resource")

    def test_login_completes_srp_device_code_and_web_session_handoff(self):
        with tempfile.TemporaryDirectory() as directory:
            client = FakeClient([
                (200, headers(), {"federated": False}),
                (200, headers(X_Apple_HC_Bits="0", X_Apple_HC_Challenge="fixture"), {}),
                (200, headers(), {"salt": "c2FsdA==", "b": "Ag==", "iteration": 1, "protocol": "s2k", "c": "challenge"}),
                (409, headers(scnt="first", X_Apple_ID_Session_Id="session"), {}),
                (200, headers(scnt="rotated"), {"trustedPhoneNumbers": [], "securityCode": {"length": 6}}),
                (204, headers(), {}), (204, headers(), {}),
                (200, headers(), {"user": {"fullName": "Synthetic User"}}),
                (200, headers(), {"resultCode": 0, "downloads": []}),
            ])
            session = auth.Session(Path(directory) / "session.json", client)
            prompts = iter(["synthetic-password", "123456"])
            login = auth.AppleLogin(session, prompt=lambda _: next(prompts))
            with patch.object(login, "service_key", return_value="public-key"):
                login.login("synthetic@example.com")
            verification = client.requests[5]
            self.assertTrue(verification[0].endswith("/verify/trusteddevice/securitycode"))
            self.assertEqual(verification[1]["data"], {"securityCode": {"code": "123456"}})
            self.assertNotIn("synthetic-password", session.path.read_text())
            self.assertEqual(len(client.responses), 0)

    def test_second_factor_handles_automatic_sms_and_refuses_security_keys(self):
        for options, expected in [({"noTrustedDevices": True, "trustedPhoneNumbers": [{"id": 9}]}, "phone"),
                                  ({"trustedPhoneNumbers": []}, "trusteddevice")]:
            client = FakeClient([(200, headers(), options), (204, headers(), {}), (204, headers(), {})])
            login = auth.AppleLogin(auth.Session(Path("unused"), client), lambda _: "123456")
            login.headers = {"scnt": "s", "X-Apple-ID-Session-Id": "id"}
            login.second_factor()
            self.assertTrue(client.requests[1][0].endswith(f"/verify/{expected}/securitycode"))
        client = FakeClient([(200, headers(), {"fsaChallenge": {"challenge": "x"}})])
        login = auth.AppleLogin(auth.Session(Path("unused"), client), lambda _: self.fail("must not prompt"))
        login.headers = {"scnt": "s", "X-Apple-ID-Session-Id": "id"}
        with self.assertRaisesRegex(auth.AppleError, "Security-key"):
            login.second_factor()

    def test_failed_login_does_not_overwrite_saved_session(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "session.json"
            path.write_text("existing-session")
            client = FakeClient([(200, headers(), {"federated": True})])
            login = auth.AppleLogin(auth.Session(path, client), lambda _: self.fail("must not prompt"))
            with patch.object(login, "service_key", return_value="key"), self.assertRaisesRegex(auth.AppleError, "Federated"):
                login.login("synthetic@example.com")
            self.assertEqual(path.read_text(), "existing-session")

    def test_expired_session_diagnostic_does_not_include_response_secrets(self):
        session = auth.Session(Path("unused"), FakeClient([(401, headers(), {"secret": "never-log-this"})]))
        with self.assertRaisesRegex(auth.AppleError, "session is unavailable") as raised:
            session.catalog()
        self.assertNotIn("never-log-this", str(raised.exception))


if __name__ == "__main__":
    unittest.main()
