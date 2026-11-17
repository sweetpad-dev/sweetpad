#!/usr/bin/env python3
"""Check the Apple account material for App Store Connect delivery tests.

Reads ~/.sweetpad-e2e (or --dir) and reports every gap in one pass:
certificates, provisioning profiles, the App Store Connect API key, the
Mac's keychain, and the Linux VM's reach to Apple's services. It reads and
never writes: the only App Store Connect calls are GETs.

Layout, with e2e.env holding KEY=VALUE lines:

  e2e.env        TEAM_ID BUNDLE_ID ASC_APP_ID DEVICE_UDID P12_PASSWORD
                 SWEETPAD_ASC_KEY_ID SWEETPAD_ASC_ISSUER_ID SWEETPAD_ASC_KEY_PATH
                 (P12_PASSWORD_<FILE> overrides one file, e.g.
                 P12_PASSWORD_DEVELOPER_ID_INSTALLER)
  keys/AuthKey_<KEYID>.p8
  certs/{development,distribution,mac-installer,
         developer-id-application,developer-id-installer}.p12
  profiles/ios-{development,appstore,adhoc}-{app,widget}.mobileprovision
           mac-{appstore,developerid}-{app,widget}.provisionprofile

The widget's bundle id is BUNDLE_ID.Widget; the App Group is group.BUNDLE_ID.
Secrets never reach the output: passwords go to openssl through its
environment and the token is not printed.
"""
import argparse
import base64
import datetime
import hashlib
import json
import os
from pathlib import Path
import plistlib
import re
import shutil
import subprocess
import time
import urllib.error
import urllib.parse
import urllib.request

APPLE_OID = re.compile(r"1\.2\.840\.113635\.100\.6\.1\.(\d+)")
# Apple's certificate policy extensions, 1.2.840.113635.100.6.1.<n>.
KINDS = {"2": "development", "12": "development", "4": "ios-distribution",
         "7": "mac-app-distribution", "8": "mac-installer",
         "13": "developer-id-application", "14": "developer-id-installer"}
CERTS = {"development": "development", "distribution": "ios-distribution",
         "mac-installer": "mac-installer",
         "developer-id-application": "developer-id-application",
         "developer-id-installer": "developer-id-installer"}
# profile file stem -> (kind, signing certificate file, platform family)
PROFILES = {"ios-development": ("development", "development", "ios"),
            "ios-appstore": ("app-store", "distribution", "ios"),
            "ios-adhoc": ("ad-hoc", "distribution", "ios"),
            "mac-appstore": ("mac-app-store", "distribution", "mac"),
            "mac-developerid": ("developer-id", "developer-id-application", "mac")}
ENDPOINTS = ["https://api.appstoreconnect.apple.com/v1/apps",
             "http://timestamp.apple.com/ts01",
             "https://api.apple-cloudkit.com/",
             "https://s3.us-west-2.amazonaws.com/"]
REQUIRED = ["TEAM_ID", "BUNDLE_ID", "ASC_APP_ID", "DEVICE_UDID", "P12_PASSWORD",
            "SWEETPAD_ASC_KEY_ID", "SWEETPAD_ASC_ISSUER_ID", "SWEETPAD_ASC_KEY_PATH"]


def utc_now():
    return datetime.datetime.now(datetime.timezone.utc).replace(tzinfo=None)


class Report:
    def __init__(self):
        self.rows = []

    def add(self, ok, area, message):
        self.rows.append({"ok": ok, "area": area, "message": message})

    def failures(self):
        return [row for row in self.rows if row["ok"] is False]


def load_env(path):
    values = {}
    for line in path.read_text().splitlines():
        line = line.strip()
        if not line or line.startswith("#") or "=" not in line:
            continue
        key, value = line.split("=", 1)
        key = key.strip().removeprefix("export ").strip()
        value = value.strip()
        if len(value) >= 2 and value[0] == value[-1] and value[0] in "'\"":
            value = value[1:-1]
        values[key] = os.path.expandvars(os.path.expanduser(value))
    return values


def openssl(args, stdin=None, env=None):
    return subprocess.run(["openssl", *args], input=stdin, capture_output=True,
                          env=env, timeout=60)


def p12_certificates(path, password):
    """The client certificates of a .p12 as (DER, text, has key), or an error."""
    env = dict(os.environ, SWEETPAD_CHECK_PASS=password)
    for legacy in ([], ["-legacy"]):
        pem = openssl(["pkcs12", "-in", str(path), "-passin", "env:SWEETPAD_CHECK_PASS",
                       "-nokeys", "-clcerts", *legacy], env=env)
        if pem.returncode == 0:
            keys = openssl(["pkcs12", "-in", str(path), "-passin", "env:SWEETPAD_CHECK_PASS",
                            "-nocerts", "-nodes", *legacy], env=env)
            has_key = keys.returncode == 0 and b"PRIVATE KEY" in keys.stdout
            blocks = re.findall(rb"-----BEGIN CERTIFICATE-----.+?-----END CERTIFICATE-----",
                                pem.stdout, re.S)
            out = []
            for block in blocks:
                der = openssl(["x509", "-outform", "der"], stdin=block).stdout
                text = openssl(["x509", "-noout", "-text", "-subject", "-enddate",
                                "-nameopt", "RFC2253"], stdin=block).stdout.decode(errors="replace")
                out.append((der, text, has_key))
            return out, None
    return None, "can't open it with its password (wrong password or unsupported format)"


def describe(der, text):
    kinds = sorted({KINDS[n] for n in APPLE_OID.findall(text) if n in KINDS})
    subject = re.search(r"^subject=(.*)$", text, re.M)
    subject = subject.group(1) if subject else ""
    team = re.search(r"(?:^|,)OU=([^,]+)", subject)
    name = re.search(r"(?:^|,)CN=([^,]+)", subject)
    end = re.search(r"^notAfter=(.*)$", text, re.M)
    expires = None
    if end:
        expires = datetime.datetime.strptime(end.group(1).strip(), "%b %d %H:%M:%S %Y %Z")
    return {"sha1": hashlib.sha1(der).hexdigest().upper(), "kinds": kinds,
            "team": team.group(1) if team else None, "name": name.group(1) if name else subject,
            "expires": expires}


def check_certificates(root, env, report):
    found = {}
    for stem, kind in CERTS.items():
        path = root / "certs" / (stem + ".p12")
        if not path.is_file():
            report.add(False, "certificates", f"missing {path}")
            continue
        password = env.get("P12_PASSWORD_" + stem.upper().replace("-", "_"), env.get("P12_PASSWORD", ""))
        certs, error = p12_certificates(path, password)
        if error:
            report.add(False, "certificates", f"{path.name}: {error}")
            continue
        described = [describe(der, text) | {"key": key} for der, text, key in certs]
        match = [c for c in described if kind in c["kinds"]]
        if not match:
            seen = ", ".join(c["name"] for c in described) or "no certificate"
            report.add(False, "certificates", f"{path.name}: no {kind} certificate (has {seen})")
            continue
        cert = match[0]
        problems = []
        if not cert["key"]:
            problems.append("no private key")
        if cert["team"] != env.get("TEAM_ID"):
            problems.append(f"team {cert['team']}, expected {env.get('TEAM_ID')}")
        now = utc_now()
        if cert["expires"] and cert["expires"] < now:
            problems.append(f"expired {cert['expires']:%Y-%m-%d}")
        elif cert["expires"] and cert["expires"] < now + datetime.timedelta(days=30):
            report.add(None, "certificates", f"{path.name}: expires {cert['expires']:%Y-%m-%d}")
        if problems:
            report.add(False, "certificates", f"{path.name} ({cert['name']}): " + "; ".join(problems))
        else:
            report.add(True, "certificates", f"{path.name}: {cert['name']}")
        found[stem] = cert
    return found


def profile_plist(path):
    data = path.read_bytes()
    start, end = data.find(b"<?xml"), data.find(b"</plist>")
    if start < 0 or end < 0:
        raise ValueError("no property list inside")
    return plistlib.loads(data[start:end + len(b"</plist>")])


def profile_kind(plist):
    platforms = plist.get("Platform", [])
    entitlements = plist.get("Entitlements", {})
    certs = [describe(der, openssl(["x509", "-inform", "der", "-noout", "-text", "-subject",
                                    "-enddate", "-nameopt", "RFC2253"], stdin=der)
                      .stdout.decode(errors="replace"))
             for der in plist.get("DeveloperCertificates", [])]
    kinds = {k for c in certs for k in c["kinds"]}
    devices = plist.get("ProvisionedDevices")
    if "OSX" in platforms:
        if "developer-id-application" in kinds:
            return "developer-id", certs
        if devices or kinds == {"development"}:
            return "mac-development", certs
        return "mac-app-store", certs
    if entitlements.get("get-task-allow") or kinds == {"development"}:
        return "development", certs
    if plist.get("ProvisionsAllDevices"):
        return "enterprise", certs
    return ("ad-hoc" if devices else "app-store"), certs


def check_profiles(root, env, certs, report):
    team, bundle = env.get("TEAM_ID"), env.get("BUNDLE_ID")
    uuids = []
    for stem, (kind, cert_stem, family) in PROFILES.items():
        for role in ("app", "widget"):
            ext = ".mobileprovision" if family == "ios" else ".provisionprofile"
            path = root / "profiles" / f"{stem}-{role}{ext}"
            if not path.is_file():
                report.add(False, "profiles", f"missing {path}")
                continue
            try:
                plist = profile_plist(path)
            except (ValueError, plistlib.InvalidFileException) as error:
                report.add(False, "profiles", f"{path.name}: {error}")
                continue
            uuids.append((path.name, plist.get("UUID")))
            actual, profile_certs = profile_kind(plist)
            entitlements = plist.get("Entitlements", {})
            target = bundle + (".Widget" if role == "widget" else "")
            app_id = entitlements.get("application-identifier") or entitlements.get(
                "com.apple.application-identifier")
            problems = []
            if actual != kind:
                problems.append(f"is a {actual} profile, expected {kind}")
            if plist.get("TeamIdentifier", [None])[0] != team:
                problems.append(f"team {plist.get('TeamIdentifier')}, expected {team}")
            if app_id != f"{team}.{target}":
                problems.append(f"app id {app_id}, expected {team}.{target} (explicit, not wildcard)")
            if plist.get("ExpirationDate") and plist["ExpirationDate"] < utc_now():
                problems.append("expired")
            want = certs.get(cert_stem)
            if want and want["sha1"] not in {c["sha1"] for c in profile_certs}:
                problems.append(f"doesn't include the certificate in {cert_stem}.p12")
            groups = entitlements.get("com.apple.security.application-groups", [])
            if env.get("APP_GROUP") not in groups:
                problems.append(f"no App Group {env.get('APP_GROUP')}")
            if role == "app":
                aps = entitlements.get("aps-environment") or entitlements.get(
                    "com.apple.developer.aps-environment")
                expected = "development" if kind == "development" else "production"
                if aps != expected:
                    problems.append(f"push environment {aps!r}, expected {expected!r}")
            if kind in ("development", "ad-hoc") and env.get("DEVICE_UDID") not in plist.get(
                    "ProvisionedDevices", []):
                problems.append("doesn't list DEVICE_UDID")
            if problems:
                report.add(False, "profiles", f"{path.name}: " + "; ".join(problems))
            else:
                report.add(True, "profiles", f"{path.name}: {kind} for {target}")
    return uuids


def b64url(data):
    return base64.urlsafe_b64encode(data).rstrip(b"=").decode()


def der_signature_to_raw(der):
    # SEQUENCE { INTEGER r, INTEGER s } -> r || s, 32 bytes each.
    def integer(data, at):
        assert data[at] == 0x02
        length = data[at + 1]
        value = data[at + 2:at + 2 + length].lstrip(b"\0")
        return value.rjust(32, b"\0"), at + 2 + length
    at = 2 if der[1] < 0x80 else 3
    r, at = integer(der, at)
    s, _ = integer(der, at)
    return r + s


def token(env):
    header = {"alg": "ES256", "kid": env["SWEETPAD_ASC_KEY_ID"], "typ": "JWT"}
    now = int(time.time())
    payload = {"iss": env["SWEETPAD_ASC_ISSUER_ID"], "iat": now, "exp": now + 600,
               "aud": "appstoreconnect-v1"}
    signing_input = (b64url(json.dumps(header).encode()) + "." +
                     b64url(json.dumps(payload).encode())).encode()
    signed = openssl(["dgst", "-sha256", "-sign", env["SWEETPAD_ASC_KEY_PATH"]], stdin=signing_input)
    if signed.returncode != 0:
        raise ValueError("openssl can't sign with the .p8 key")
    return signing_input.decode() + "." + b64url(der_signature_to_raw(signed.stdout))


def asc_get(jwt, path):
    request = urllib.request.Request("https://api.appstoreconnect.apple.com" + path,
                                     headers={"Authorization": "Bearer " + jwt})
    with urllib.request.urlopen(request, timeout=30) as response:
        return json.load(response)


def check_api_key(env, report):
    key = Path(env.get("SWEETPAD_ASC_KEY_PATH", ""))
    if not key.is_file():
        report.add(False, "api key", f"missing {key}")
        return
    if key.name != f"AuthKey_{env.get('SWEETPAD_ASC_KEY_ID')}.p8":
        report.add(None, "api key", f"{key.name} isn't named AuthKey_<KEY_ID>.p8")
    try:
        jwt = token(env)
        query = urllib.parse.quote(env["BUNDLE_ID"])
        apps = asc_get(jwt, f"/v1/apps?filter[bundleId]={query}&fields[apps]=bundleId,name")
    except urllib.error.HTTPError as error:
        report.add(False, "api key", f"App Store Connect refused the key: HTTP {error.code}")
        return
    except (urllib.error.URLError, ValueError, OSError) as error:
        report.add(False, "api key", f"can't call App Store Connect: {error}")
        return
    exact = [a for a in apps.get("data", []) if a["attributes"].get("bundleId") == env["BUNDLE_ID"]]
    if not exact:
        report.add(False, "api key", f"no app with bundle id {env['BUNDLE_ID']} is visible to this key")
        return
    if exact[0]["id"] != env.get("ASC_APP_ID"):
        report.add(False, "api key", f"app id is {exact[0]['id']}, e2e.env says {env.get('ASC_APP_ID')}")
        return
    report.add(True, "api key", f"team key sees {exact[0]['attributes'].get('name')} ({exact[0]['id']})")
    try:
        versions = asc_get(jwt, f"/v1/apps/{exact[0]['id']}/appStoreVersions"
                                "?fields[appStoreVersions]=platform&limit=50")
        platforms = {v["attributes"].get("platform") for v in versions.get("data", [])}
        missing = {"IOS", "MAC_OS"} - platforms
        if missing:
            report.add(None, "api key", "app record has no version for " + ", ".join(sorted(missing)))
        else:
            report.add(True, "api key", "app record has iOS and macOS")
    except (urllib.error.URLError, OSError) as error:
        report.add(None, "api key", f"couldn't list the app's platforms: {error}")


def check_keychain(env, uuids, report):
    if shutil.which("security") is None:
        report.add(None, "mac", "not on a Mac; keychain checks skipped")
        return
    listing = subprocess.run(["security", "find-identity", "-v"], capture_output=True,
                             text=True).stdout
    team = env.get("TEAM_ID")
    for label, names in [] if not team else [("Apple Distribution", ["Apple Distribution"]),
                         ("Mac Installer Distribution", ["3rd Party Mac Developer Installer",
                                                         "Mac Installer Distribution"])]:
        if any(re.search(re.escape(n) + r".*\(" + re.escape(team or "") + r"\)", listing)
               for n in names):
            report.add(True, "mac", f"login keychain has {label} for {team}")
        else:
            report.add(False, "mac", f"login keychain has no {label} identity for {team}")
    folders = [Path.home() / "Library/MobileDevice/Provisioning Profiles",
               Path.home() / "Library/Developer/Xcode/UserData/Provisioning Profiles"]
    installed = set()
    for folder in folders:
        for path in folder.glob("*provision*") if folder.is_dir() else []:
            try:
                installed.add(profile_plist(path).get("UUID"))
            except (ValueError, plistlib.InvalidFileException, OSError):
                pass
    absent = [name for name, uuid in uuids if uuid not in installed]
    if absent:
        report.add(False, "mac", "Xcode can't see " + ", ".join(absent))
    elif uuids:
        report.add(True, "mac", "Xcode sees all profiles")
    free = shutil.disk_usage(Path.home()).free / 2**30
    if free < 40:
        report.add(False, "mac", f"{free:.0f} GB free; the VM, SDK and builds need about 40 GB")
    else:
        report.add(True, "mac", f"{free:.0f} GB free")


def reach(prefix):
    results = {}
    for url in ENDPOINTS:
        command = [*prefix, "curl", "-sS", "-o", "/dev/null", "-m", "15", "-w", "%{http_code}", url]
        run = subprocess.run(command, capture_output=True, text=True)
        results[url] = run.stdout.strip() if run.returncode == 0 else None
    return results


def check_network(prefix, area, report):
    for url, code in reach(prefix).items():
        host = url.split("/")[2]
        if code and code != "000":
            report.add(True, area, f"reaches {host} (HTTP {code})")
        else:
            report.add(False, area, f"can't reach {host}")


def check_vm(root, env, vm, report):
    listed = subprocess.run(["orb", "list", "-q"], capture_output=True, text=True)
    if listed.returncode != 0 or vm not in listed.stdout.split():
        report.add(False, "vm", f"OrbStack machine {vm} doesn't exist; create it with "
                               f"'orb create ubuntu:noble {vm}' and run 'sweetpad setup' in it")
        return
    prefix = ["orb", "-m", vm]
    readable = subprocess.run([*prefix, "test", "-r", str(root / "e2e.env")])
    report.add(readable.returncode == 0, "vm", f"{'can' if readable.returncode == 0 else 'can' + chr(39) + 't'} read {root}")
    check_network(prefix, "vm", report)
    doctor = subprocess.run([*prefix, "bash", "-lc", "sweetpad doctor --json --non-interactive"],
                            capture_output=True, text=True, timeout=300)
    if doctor.returncode == 0:
        report.add(True, "vm", "sweetpad doctor passes")
    else:
        report.add(False, "vm", "sweetpad doctor fails or sweetpad isn't installed in the VM")
    devices = subprocess.run([*prefix, "bash", "-lc", "sweetpad device list --json --non-interactive"],
                             capture_output=True, text=True, timeout=120)
    if env.get("DEVICE_UDID") and env["DEVICE_UDID"] in devices.stdout:
        report.add(True, "vm", "sees the iPhone")
    else:
        report.add(False, "vm", "doesn't see DEVICE_UDID (pair it over Wi-Fi or pass USB through)")


def main():
    parser = argparse.ArgumentParser(description=__doc__,
                                     formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--dir", type=Path, default=Path.home() / ".sweetpad-e2e")
    parser.add_argument("--vm", default="sweetpad-linux")
    parser.add_argument("--skip-vm", action="store_true")
    parser.add_argument("--json", action="store_true")
    args = parser.parse_args()
    report = Report()
    root = args.dir.expanduser()
    env_file = root / "e2e.env"
    env = {}
    if not env_file.is_file():
        report.add(False, "setup", f"missing {env_file}")
    else:
        env = load_env(env_file)
        missing = [key for key in REQUIRED if not env.get(key)]
        if missing:
            report.add(False, "setup", "e2e.env lacks " + ", ".join(missing))
        if root.stat().st_mode & 0o077:
            report.add(None, "setup", f"{root} is readable by others; use mode 0700")
    if env.get("BUNDLE_ID"):
        env.setdefault("APP_GROUP", "group." + env["BUNDLE_ID"])
    certs = check_certificates(root, env, report) if env else {}
    uuids = check_profiles(root, env, certs, report) if env.get("BUNDLE_ID") else []
    if all(env.get(k) for k in ("SWEETPAD_ASC_KEY_ID", "SWEETPAD_ASC_ISSUER_ID", "BUNDLE_ID")):
        check_api_key(env, report)
    check_keychain(env, uuids, report)
    check_network([], "mac", report)
    if not args.skip_vm:
        check_vm(root, env, args.vm, report)
    if args.json:
        print(json.dumps({"passed": not report.failures(), "checks": report.rows}, indent=2))
    else:
        marks = {True: "ok  ", False: "FAIL", None: "warn"}
        for row in report.rows:
            print(f"{marks[row['ok']]}  {row['area']:<12} {row['message']}")
        failures = len(report.failures())
        print(f"\n{failures} problem{'s' if failures != 1 else ''}" if failures else "\nAll checks pass")
    return 1 if report.failures() else 0


if __name__ == "__main__":
    raise SystemExit(main())
