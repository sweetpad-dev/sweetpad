#!/usr/bin/env python3
"""Apple Developer terminal authentication, without Apple platform frameworks.

Protocol references: XcodesOrg/XcodesLoginKit (Client, URLRequest+Apple,
AppleServiceKeyResolver) and XcodesOrg/swift-srp. Passwords are never stored.
"""

import base64
import datetime
import getpass
import gzip
import hashlib
import http.client
import http.cookiejar
import json
import os
from pathlib import Path
import secrets
import stat
import sys
import tempfile
import time
import urllib.error
import urllib.parse
import urllib.request
import warnings


AUTH = "https://idmsa.apple.com/appleauth/auth"
CATALOG = "https://developer.apple.com/services-account/QH65B2/downloadws/listDownloads.action"
LOGIN_HINT = "Run sweetpad auth login --apple-id EMAIL, or supply --session-file for CI."


class AppleError(Exception):
    """A diagnostic that contains no credentials or response bodies."""


def response_bytes(response, limit=64 * 1024 * 1024):
    encoding = response.headers.get("Content-Encoding", "identity").lower()
    if encoding not in ("identity", "gzip"):
        raise AppleError("Apple returned an unsupported response encoding.")
    source = gzip.GzipFile(fileobj=response) if encoding == "gzip" else response
    try:
        data = source.read(limit + 1)
    except (OSError, EOFError, http.client.HTTPException):
        raise AppleError("Apple service response was interrupted or invalid. Retry the command.") from None
    if len(data) > limit:
        raise AppleError("Apple service response exceeds the size limit.")
    return data


def apple_url(url):
    parsed = urllib.parse.urlsplit(url)
    return (parsed.scheme == "https" and parsed.port in (None, 443)
            and not parsed.username and not parsed.password
            and (parsed.hostname == "apple.com" or (parsed.hostname or "").endswith(".apple.com")))


class Redirects(urllib.request.HTTPRedirectHandler):
    def __init__(self, allowed, follow):
        self.allowed, self.follow = allowed, follow

    def redirect_request(self, request, response, code, message, headers, url):
        if not self.allowed(url):
            raise AppleError("Apple returned an unsupported redirect destination.")
        if not self.follow:
            return None
        return super().redirect_request(request, response, code, message, headers, url)


class HTTPClient:
    def __init__(self, cookies=None, allowed=apple_url):
        self.cookies = cookies if cookies is not None else http.cookiejar.CookieJar(
            policy=http.cookiejar.DefaultCookiePolicy(
                strict_ns_domain=http.cookiejar.DefaultCookiePolicy.DomainStrictNonDomain))
        self.allowed = allowed

    def open(self, url, method="GET", data=None, headers=None, follow=True):
        if not self.allowed(url):
            raise AppleError("Only HTTPS Apple service URLs are supported.")
        request_headers = {"User-Agent": "Sweetpad/1 (Linux; Apple Developer downloads)",
                           "Accept": "application/json", "Accept-Encoding": "gzip"}
        request_headers.update(headers or {})
        if data is not None:
            data = json.dumps(data).encode()
            request_headers["Content-Type"] = "application/json"
        opener = urllib.request.build_opener(
            urllib.request.HTTPCookieProcessor(self.cookies), Redirects(self.allowed, follow))
        request = urllib.request.Request(url, data=data, headers=request_headers, method=method)
        try:
            return opener.open(request, timeout=60)
        except urllib.error.HTTPError as error:
            return error
        except (urllib.error.URLError, TimeoutError, OSError):
            raise AppleError("Cannot connect to Apple. Check the network and certificate store.") from None

    def request(self, url, **kwargs):
        with self.open(url, **kwargs) as response:
            raw = response_bytes(response)
            try:
                data = json.loads(raw) if raw else {}
            except (ValueError, UnicodeError):
                data = None
            return response.code, response.headers, data


def private_directory(path):
    missing, candidate = [], path
    while not candidate.exists() and not candidate.is_symlink():
        missing.append(candidate)
        candidate = candidate.parent
    for directory in reversed(missing):
        try:
            directory.mkdir(mode=0o700)
        except FileExistsError:
            pass
        info = directory.lstat()
        if (not stat.S_ISDIR(info.st_mode) or info.st_uid != os.geteuid()
                or info.st_mode & 0o022):
            raise AppleError("The session/cache directory must be owned and not writable by other users.")
    info = path.lstat()
    if (not stat.S_ISDIR(info.st_mode) or info.st_uid != os.geteuid()
            or info.st_mode & 0o022):
        raise AppleError("The session/cache directory must be owned and not writable by other users.")


def private_read(path):
    descriptor = os.open(path, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK)
    with os.fdopen(descriptor, "rb") as source:
        info = os.fstat(source.fileno())
        if (not stat.S_ISREG(info.st_mode) or info.st_uid != os.geteuid()
                or info.st_mode & 0o077 or info.st_size > 1024 * 1024):
            raise AppleError("The session file must be an owned regular file with mode 600.")
        return source.read(1024 * 1024 + 1)


def private_write(path, data):
    private_directory(path.parent)
    if path.is_symlink():
        raise AppleError("The session file must not be a symbolic link.")
    descriptor, temporary = tempfile.mkstemp(prefix=".apple-session-", dir=path.parent)
    try:
        with os.fdopen(descriptor, "w") as output:
            os.fchmod(output.fileno(), 0o600)
            json.dump(data, output)
            output.write("\n")
            output.flush()
            os.fsync(output.fileno())
        os.replace(temporary, path)
    finally:
        Path(temporary).unlink(missing_ok=True)


def session_path(explicit=None):
    selected = explicit or os.environ.get("SWEETPAD_APPLE_SESSION_FILE")
    if selected:
        return Path(selected).expanduser().absolute()
    root = Path(os.environ.get("XDG_STATE_HOME", Path.home() / ".local/state"))
    if not root.is_absolute():
        root = Path.home() / ".local/state"
    return root / "sweetpad/apple-session.json"


class Session:
    def __init__(self, path, client=None):
        self.path = path
        self.client = client if client is not None else HTTPClient()
        self.apple_id = None

    def load(self, required=False):
        try:
            data = json.loads(private_read(self.path))
        except FileNotFoundError:
            if required:
                raise AppleError("No Apple session file exists. " + LOGIN_HINT) from None
            return False
        except (ValueError, UnicodeError):
            raise AppleError("The Apple session file is not valid Sweetpad session JSON.") from None
        if (not isinstance(data, dict) or data.get("version") != 1
                or not isinstance(data.get("appleId"), str)
                or not isinstance(data.get("cookies"), list)):
            raise AppleError("Unsupported Apple session file. Create it with sweetpad auth login.")
        cookies = []
        try:
            for item in data["cookies"]:
                if (not isinstance(item, dict) or not isinstance(item.get("domain"), str)
                        or not isinstance(item.get("name"), str)
                        or not isinstance(item.get("value"), str)
                        or not isinstance(item.get("domainSpecified"), bool)
                        or not isinstance(item.get("path"), str) or not item["path"].startswith("/")
                        or (item.get("expires") is not None and not isinstance(item["expires"], int))
                        or any(c in item["name"] + item["value"] for c in "\r\n")):
                    raise ValueError()
                domain = item["domain"]
                if not apple_url("https://" + domain.lstrip(".") + "/"):
                    raise ValueError()
                cookies.append(http.cookiejar.Cookie(
                    version=0, name=item["name"], value=item["value"], port=None,
                    port_specified=False, domain=domain,
                    domain_specified=item["domainSpecified"],
                    domain_initial_dot=domain.startswith("."), path=item["path"],
                    path_specified=True, secure=True, expires=item["expires"],
                    discard=item["expires"] is None, comment=None, comment_url=None,
                    rest={"HttpOnly": None}))
        except (KeyError, TypeError, ValueError):
            raise AppleError("Invalid cookie data in the Apple session file.") from None
        self.apple_id = data["appleId"]
        for cookie in cookies:
            self.client.cookies.set_cookie(cookie)
        return True

    def save(self):
        cookies = [dict(name=c.name, value=c.value, domain=c.domain,
                        domainSpecified=c.domain_specified, path=c.path, expires=c.expires)
                   for c in self.client.cookies
                   if apple_url("https://" + c.domain.lstrip(".") + "/") and not c.is_expired()]
        private_write(self.path, {"version": 1, "appleId": self.apple_id, "cookies": cookies})

    def catalog(self):
        code, _, data = self.client.request(CATALOG, method="POST")
        if code in (401, 403) or (code == 200 and not isinstance(data, dict)):
            raise AppleError(f"Apple Developer download session is unavailable (HTTP {code}, JSON response: {isinstance(data, dict)}). " + LOGIN_HINT)
        if code != 200 or not isinstance(data, dict):
            raise AppleError(f"Apple download catalog failed (HTTP {code}).")
        if data.get("resultCode", 0) != 0 or not isinstance(data.get("downloads"), list):
            raise AppleError("Apple rejected download access. Renew login and accept Developer agreements in a browser.")
        return data["downloads"]


# RFC 5054 2048-bit group used by Apple's s2k/s2k_fo protocol.
N = int("""AC6BDB41324A9A9BF166DE5E1389582FAF72B6651987EE07FC3192943DB56050A
37329CBB4A099ED8193E0757767A13DD52312AB4B03310DCD7F48A9DA04FD50E8083969
EDB767B0CF6095179A163AB3661A05FBD5FAAAE82918A9962F0B93B855F97993EC975EE
AA80D740ADBF4FF747359D041D5C33EA71D281E446B14773BCA97B43A23FB801676BD207
A436C6481F1D2B9078717461A5B9D32E688F87748544523B524B0D57D5EA77A2775D2EC
FA032CFBDBF52FB3786160279004E57AE6AF874E7303CE53299CCC041C7BC308D82A5698
F3A8D0C38271AE35F8E9DBFBB694B5C803D89F7AE435DE236D525F54759B65E372FCD68
EF20FA7111F9E4AFF73""".replace("\n", ""), 16)


def integer_bytes(value):
    return value.to_bytes(max(1, (value.bit_length() + 7) // 8), "big")


def sha(data):
    return hashlib.sha256(data).digest()


class SRP:
    def __init__(self, private=None):
        self.private = private if private is not None else secrets.randbits(256) | 1
        self.public = integer_bytes(pow(2, self.private, N))

    def proofs(self, account, password, challenge):
        try:
            salt = base64.b64decode(challenge["salt"], validate=True)
            raw_b = base64.b64decode(challenge["b"], validate=True)
            rounds, protocol = challenge["iteration"], challenge["protocol"]
            if (not isinstance(rounds, int) or not 1 <= rounds <= 10_000_000
                    or not 1 <= len(salt) <= 1024 or not 1 <= len(raw_b) <= 256
                    or protocol not in ("s2k", "s2k_fo")):
                raise ValueError()
            b = int.from_bytes(raw_b, "big")
            if not 0 < b < N:
                raise ValueError()
        except (KeyError, TypeError, ValueError):
            raise AppleError("Apple returned invalid SRP parameters.") from None
        password_hash = sha(password.encode())
        if protocol == "s2k_fo":
            password_hash = password_hash.hex().encode()
        derived = hashlib.pbkdf2_hmac("sha256", password_hash, salt, rounds, 32)
        x = int.from_bytes(sha(salt + sha(b":" + derived)), "big")
        pad = lambda value: value.rjust(256, b"\0")
        k = int.from_bytes(sha(integer_bytes(N) + pad(b"\2")), "big")
        u = int.from_bytes(sha(pad(self.public) + pad(raw_b)), "big")
        if u == 0:
            raise AppleError("Apple returned invalid SRP parameters.")
        secret = pow((b - k * pow(2, x, N)) % N, self.private + u * x, N)
        if not secret:
            raise AppleError("Apple returned invalid SRP parameters.")
        shared = sha(integer_bytes(secret))
        xor = bytes(a ^ b for a, b in zip(sha(integer_bytes(N)), sha(pad(b"\2"))))
        m1 = sha(xor + sha(account.encode()) + salt + self.public + integer_bytes(b) + shared)
        m2 = sha(self.public + m1 + shared)
        return base64.b64encode(m1).decode(), base64.b64encode(m2).decode()


def hashcash(bits, challenge, date=None):
    try:
        bits = int(bits)
        if not 0 <= bits <= 24 or not challenge or len(challenge) > 1024:
            raise ValueError()
    except (TypeError, ValueError):
        raise AppleError("Apple returned an unsupported hashcash challenge.") from None
    stamp = date or datetime.datetime.now(datetime.timezone.utc).strftime("%y%m%d%H%M%S")
    prefix = f"1:{bits}:{stamp}:{challenge}::"
    counter, started = 0, time.monotonic()
    while time.monotonic() - started < 60:
        result = prefix + str(counter)
        if int.from_bytes(hashlib.sha1(result.encode()).digest(), "big") < (1 << (160 - bits)):
            return result
        counter += 1
        if counter % 10000 == 0:
            time.sleep(0.01)
    raise AppleError("Apple hashcash challenge timed out. Retry login later.")


def terminal_secret(prompt):
    # Never let getpass fall back to echoed stdin in a non-terminal context.
    try:
        with open("/dev/tty", "w") as terminal:
            if not terminal.isatty():
                raise OSError()
            with warnings.catch_warnings():
                warnings.simplefilter("error", getpass.GetPassWarning)
                return getpass.getpass(prompt, stream=terminal)
    except (OSError, EOFError, getpass.GetPassWarning):
        raise AppleError("Authentication needs an interactive terminal. " + LOGIN_HINT) from None


class AppleLogin:
    def __init__(self, session, prompt=terminal_secret):
        self.session, self.client, self.prompt = session, session.client, prompt
        self.headers = {}

    def request(self, path, **kwargs):
        code, headers, data = self.client.request(AUTH + path, headers=self.headers, **kwargs)
        for key in ("scnt", "X-Apple-ID-Session-Id"):
            if headers.get(key):
                self.headers[key] = headers[key]
        return code, headers, data

    def service_key(self):
        # Isolate this HEAD request: following its redirect would log out the user.
        client = HTTPClient()
        code, headers, _ = client.request("https://appstoreconnect.apple.com/logout", method="HEAD", follow=False)
        if code in (301, 302, 303, 307, 308):
            location = headers.get("Location", "")
            if apple_url(location):
                key = urllib.parse.parse_qs(urllib.parse.urlsplit(location).query).get("widgetKey", [""])[0]
                if key:
                    return key
        code, _, data = client.request("https://appstoreconnect.apple.com/olympus/v1/app/config?hostname=itunesconnect.apple.com")
        if code == 200 and isinstance(data, dict) and data.get("authServiceKey"):
            return data["authServiceKey"]
        raise AppleError("Cannot resolve Apple's public sign-in key. Retry login later.")

    def login(self, account):
        key = self.service_key()
        self.headers = {"X-Apple-Widget-Key": key, "X-Requested-With": "XMLHttpRequest"}
        code, _, data = self.request("/federate", method="POST", data={"accountName": account, "rememberMe": False})
        if code != 200 or not isinstance(data, dict):
            raise AppleError(f"Apple account lookup failed (HTTP {code}).")
        if data.get("federated"):
            raise AppleError("Federated Apple IDs are not supported by terminal login.")
        password = self.prompt("Apple ID password: ")
        if not password:
            raise AppleError("Apple ID password is empty.")
        code, headers, _ = self.request("/signin?" + urllib.parse.urlencode({"widgetKey": key}))
        if code != 200:
            raise AppleError(f"Apple sign-in challenge failed (HTTP {code}).")
        proof = hashcash(headers.get("X-Apple-HC-Bits"), headers.get("X-Apple-HC-Challenge"))
        srp = SRP()
        code, _, challenge = self.request("/signin/init", method="POST", data={
            "accountName": account, "a": base64.b64encode(srp.public).decode(), "protocols": ["s2k", "s2k_fo"]})
        if code != 200 or not isinstance(challenge, dict) or not isinstance(challenge.get("c"), str):
            raise AppleError(f"Apple SRP initialization failed (HTTP {code}).")
        m1, m2 = srp.proofs(account, password, challenge)
        del password
        self.headers["X-Apple-HC"] = proof
        code, _, _ = self.request("/signin/complete?isRememberMeEnabled=false", method="POST", data={
            "accountName": account, "c": challenge["c"], "m1": m1, "m2": m2, "rememberMe": False})
        self.headers.pop("X-Apple-HC", None)
        if code == 409:
            self.second_factor()
        elif code == 401:
            raise AppleError("Apple rejected the account or password. Check the credentials before retrying.")
        elif code in (403, 423):
            raise AppleError("Apple blocked sign-in. Review the account at account.apple.com before retrying.")
        elif code == 412:
            raise AppleError("Accept Apple's privacy and Developer agreements in a browser, then retry login.")
        elif code != 200:
            raise AppleError(f"Apple sign-in failed (HTTP {code}).")
        self.session.apple_id = account
        # The reference client validates Olympus after SRP/trust. That request
        # completes Apple's web-session handoff before Developer services use it.
        code, _, data = self.client.request("https://appstoreconnect.apple.com/olympus/v1/session")
        if code != 200 or not isinstance(data, dict) or not isinstance(data.get("user"), dict):
            raise AppleError(f"Apple could not establish the web session after sign-in (HTTP {code}).")
        # Preserve the authenticated session even if Developer access requires
        # a separate agreement or a temporary catalog outage.
        self.session.save()
        with self.client.open("https://developer.apple.com/download/", headers={"Accept": "text/html"}) as response:
            if response.code != 200:
                raise AppleError(f"Apple Developer session initialization failed (HTTP {response.code}).")
        self.session.catalog()
        self.session.save()

    def second_factor(self):
        if not self.headers.get("scnt") or not self.headers.get("X-Apple-ID-Session-Id"):
            raise AppleError("Apple omitted the two-factor session headers.")
        code, _, options = self.request("")
        if code != 200 or not isinstance(options, dict):
            raise AppleError("Cannot read Apple's two-factor options.")
        if options.get("fsaChallenge"):
            raise AppleError("Security-key authentication is not supported by terminal login.")
        if options.get("trustedDevices") is not None:
            raise AppleError("Legacy two-step authentication is unsupported. Enable Apple two-factor authentication.")
        info = options.get("securityCode") or {}
        if any(info.get(key) for key in ("securityCodeLocked", "securityCodeCooldown", "tooManyCodesSent", "tooManyCodesValidated")):
            raise AppleError("Apple temporarily locked verification codes. Retry later.")
        phones = options.get("trustedPhoneNumbers") or []
        phone = None
        if options.get("noTrustedDevices"):
            if not phones:
                raise AppleError("Apple returned no trusted device or phone number.")
            if len(phones) == 1:
                phone = phones[0]  # Apple already sends this SMS automatically.
            else:
                for index, item in enumerate(phones, 1):
                    print(f"{index}: {item.get('numberWithDialCode', 'trusted phone')}", file=sys.stderr)
                choice = self.prompt("SMS phone number (list number): ")
                if not choice.isdigit() or not 1 <= int(choice) <= len(phones):
                    raise AppleError("Invalid trusted phone selection.")
                phone = phones[int(choice) - 1]
                code, _, _ = self.request("/verify/phone", method="PUT", data={"phoneNumber": {"id": phone["id"]}, "mode": "sms"})
                if code not in (200, 204):
                    raise AppleError("Apple could not send the SMS code.")
        value = self.prompt("Apple two-factor code: ").replace(" ", "").replace("-", "")
        length = info.get("length", 6)
        if not value.isascii() or not value.isdigit() or len(value) != length:
            raise AppleError("Invalid verification code format.")
        body = {"securityCode": {"code": value}}
        channel = "trusteddevice"
        if phone is not None:
            channel = "phone"
            body.update(phoneNumber={"id": phone["id"]}, mode="sms")
        code, _, _ = self.request(f"/verify/{channel}/securitycode", method="POST", data=body)
        if code not in (200, 204):
            raise AppleError("Apple rejected the verification code. Retry login with a fresh code.")
        code, _, _ = self.request("/2sv/trust")
        if code not in (200, 204):
            raise AppleError("Apple could not trust the authenticated session.")
