#!/usr/bin/env python3
"""Regenerate the codesign stand-in's golden files with Apple's codesign.

Needs Xcode and OpenSSL 3. The identity is a self-signed test certificate,
named like an Apple Development one, made once and kept in identity.p12
(password "sweetpad-test"), with identity-aes.p12 holding the same identity
the way OpenSSL 3 exports it. A temporary keychain holds it while codesign
signs; nothing touches the login keychain.

input/ has what Swift Build would hand codesign: a linker-signed
executable and library, an executable lld linked with no signature (LldTool,
linked in the Linux VM and kept as it is), and an app bundle whose nested
extension and debug library are already signed, so both signers see the
same files. expected/ has what codesign makes of each. profiles/ has a provisioning profile for
the identity, signed by it, for the provisioning stand-in's tests.

identities.p12 holds one identity of each other kind, self-signed but with Apple's policy
extensions, exported together from a keychain as Keychain Access exports a selection.
profiles-kinds/ has a profile of each kind for them. `regen.py kinds` remakes only those.
"""

import os
import plistlib
import shutil
import subprocess
import sys
import tempfile

HERE = os.path.dirname(os.path.abspath(__file__))
PASSWORD = "sweetpad-test"
SUBJECT = "/CN=Apple Development: Sweetpad Fixture (TESTTEAM01)/OU=TESTTEAM01/O=Sweetpad Fixture/C=US"


def run(*cmd, **kwargs):
    subprocess.run(cmd, check=True, **kwargs)


def make_identity(work):
    p12 = os.path.join(HERE, "identity.p12")
    key, cert = os.path.join(work, "key.pem"), os.path.join(work, "cert.pem")
    if not os.path.exists(p12):
        run("openssl", "req", "-x509", "-newkey", "rsa:2048", "-nodes", "-keyout", key,
            "-out", cert, "-days", "36500", "-subj", SUBJECT,
            "-addext", "keyUsage=critical,digitalSignature",
            "-addext", "extendedKeyUsage=critical,codeSigning")
        # Keychain Access's format: Triple DES and a SHA-1 MAC.
        run("openssl", "pkcs12", "-export", "-inkey", key, "-in", cert, "-out", p12,
            "-passout", f"pass:{PASSWORD}", "-name", "Sweetpad Fixture",
            "-certpbe", "PBE-SHA1-3DES", "-keypbe", "PBE-SHA1-3DES", "-macalg", "sha1")
        # OpenSSL 3's: PBES2 with AES-256 and a SHA-256 MAC.
        run("openssl", "pkcs12", "-export", "-inkey", key, "-in", cert,
            "-out", os.path.join(HERE, "identity-aes.p12"),
            "-passout", f"pass:{PASSWORD}", "-name", "Sweetpad Fixture")
    run("openssl", "pkcs12", "-in", p12, "-passin", f"pass:{PASSWORD}", "-nokeys",
        "-out", cert)
    run("openssl", "pkcs12", "-in", p12, "-passin", f"pass:{PASSWORD}", "-nocerts", "-nodes",
        "-out", key)
    der = subprocess.run(["openssl", "x509", "-in", cert, "-outform", "DER"],
                         check=True, capture_output=True).stdout
    sha1 = subprocess.run(["shasum"], input=der, check=True, capture_output=True).stdout
    return p12, key, cert, der, sha1.split()[0].decode().upper()


def make_keychain(work, p12):
    keychain = os.path.join(work, "fixture.keychain-db")
    run("security", "create-keychain", "-p", "fixture", keychain)
    run("security", "unlock-keychain", "-p", "fixture", keychain)
    run("security", "import", p12, "-k", keychain, "-P", PASSWORD, "-T", "/usr/bin/codesign",
        stdout=subprocess.DEVNULL)
    run("security", "set-key-partition-list", "-S", "apple-tool:,apple:,codesign:", "-s",
        "-k", "fixture", keychain, stdout=subprocess.DEVNULL)
    return keychain


def compile_c(work, source, output, *flags):
    path = os.path.join(work, os.path.basename(output) + ".c")
    with open(path, "w") as f:
        f.write(source)
    run("xcrun", "-sdk", "iphoneos", "clang", "-arch", "arm64", "-mios-version-min=17.0",
        "-o", output, path, *flags)


def write_plist(path, value, fmt=plistlib.FMT_BINARY):
    with open(path, "wb") as f:
        plistlib.dump(value, f, fmt=fmt)


def make_inputs(work, sign):
    root = os.path.join(HERE, "input")
    # ld64 always leaves room for a signature; lld leaves none, and ends the
    # file off a 16-byte boundary. macOS has no lld to link it again.
    with open(os.path.join(root, "LldTool"), "rb") as f:
        lld_tool = f.read()
    shutil.rmtree(root, ignore_errors=True)
    os.makedirs(root)
    with open(os.path.join(root, "LldTool"), "wb") as f:
        f.write(lld_tool)
    os.chmod(os.path.join(root, "LldTool"), 0o755)
    make_resource_input(root)
    compile_c(work, "int main(void) { return 0; }\n", os.path.join(root, "Tool"))
    compile_c(work, "int library(void) { return 7; }\n", os.path.join(root, "Library.dylib"),
              "-dynamiclib", "-install_name", "@rpath/Library.dylib")

    app = os.path.join(root, "App.app")
    os.makedirs(os.path.join(app, "en.lproj"))
    os.makedirs(os.path.join(app, "fr.lproj"))
    os.makedirs(os.path.join(app, "Base.lproj"))
    extension = os.path.join(app, "PlugIns", "Extension.appex")
    os.makedirs(extension)
    compile_c(work, "int main(void) { return 1; }\n", os.path.join(app, "App"))
    compile_c(work, "int debug_entry(void) { return 2; }\n", os.path.join(app, "App.debug.dylib"),
              "-dynamiclib", "-install_name", "@rpath/App.debug.dylib")
    write_plist(os.path.join(app, "Info.plist"), {
        "CFBundleExecutable": "App",
        "CFBundleIdentifier": "dev.sweetpad.fixture",
        "CFBundlePackageType": "APPL",
        "CFBundleVersion": "1",
    })
    with open(os.path.join(app, "PkgInfo"), "w") as f:
        f.write("APPL????")
    with open(os.path.join(app, "dot.png"), "wb") as f:
        f.write(bytes(range(64)))
    with open(os.path.join(app, "en.lproj", "Localizable.strings"), "w") as f:
        f.write('"hello" = "Hello";\n')
    with open(os.path.join(app, "fr.lproj", "locversion.plist"), "w") as f:
        f.write("omitted\n")
    with open(os.path.join(app, "Base.lproj", "Main.strings"), "w") as f:
        f.write('"title" = "Main";\n')
    compile_c(work, "int main(void) { return 3; }\n", os.path.join(extension, "Extension"))
    write_plist(os.path.join(extension, "Info.plist"), {
        "CFBundleExecutable": "Extension",
        "CFBundleIdentifier": "dev.sweetpad.fixture.extension",
        "CFBundlePackageType": "XPC!",
    })
    entitlements = os.path.join(root, "app.xcent")
    write_plist(entitlements, {
        "application-identifier": "TESTTEAM01.dev.sweetpad.fixture",
        "com.apple.developer.team-identifier": "TESTTEAM01",
        "get-task-allow": True,
        "keychain-access-groups": ["TESTTEAM01.dev.sweetpad.fixture"],
        "com.apple.developer.icloud-container-environment": "Development",
        "fixture.count": 3,
    }, plistlib.FMT_XML)
    # Nested code is signed before the bundle around it, by its own task.
    sign(os.path.join(app, "App.debug.dylib"))
    sign(extension, "--entitlements", entitlements)


def make_resource_input(root):
    bundle = os.path.join(root, "Resources.bundle")
    os.makedirs(bundle, exist_ok=True)
    write_plist(os.path.join(bundle, "Info.plist"), {
        "CFBundleIdentifier": "dev.sweetpad.resources",
        "CFBundlePackageType": "BNDL",
        "CFBundleVersion": "1",
        # Longer than a Mach-O code page: detached signatures still hash once.
        "FixturePadding": "resource-only signing " * 900,
    })
    with open(os.path.join(bundle, "message.txt"), "w") as output:
        output.write("resource-v1\n")


def make_expected(sign):
    root = os.path.join(HERE, "expected")
    shutil.rmtree(root, ignore_errors=True)
    shutil.copytree(os.path.join(HERE, "input"), root, symlinks=True)
    os.remove(os.path.join(root, "app.xcent"))
    entitlements = os.path.join(HERE, "input", "app.xcent")
    sign(os.path.join(root, "Tool"))
    sign(os.path.join(root, "Library.dylib"))
    sign(os.path.join(root, "LldTool"))
    sign(os.path.join(root, "App.app"), "--entitlements", entitlements)
    sign(os.path.join(root, "Resources.bundle"), "--entitlements", entitlements)
    resource_adhoc = os.path.join(root, "Resources.adhoc.bundle")
    shutil.copytree(os.path.join(HERE, "input", "Resources.bundle"), resource_adhoc)
    run("codesign", "--force", "--sign", "-", "--timestamp=none",
        "--entitlements", entitlements, "--generate-entitlement-der", resource_adhoc)
    adhoc = os.path.join(root, "Library.adhoc.dylib")
    shutil.copy(os.path.join(HERE, "input", "Library.dylib"), adhoc)
    run("codesign", "--force", "--sign", "-", "--timestamp=none", "--generate-entitlement-der",
        adhoc, stderr=subprocess.DEVNULL)


def make_profile(work, key, cert, der):
    root = os.path.join(HERE, "profiles")
    shutil.rmtree(root, ignore_errors=True)
    os.makedirs(root)
    profile = {
        "AppIDName": "Sweetpad Fixture",
        "CreationDate": __import__("datetime").datetime(2026, 1, 1),
        "DeveloperCertificates": [der],
        "Entitlements": {
            "application-identifier": "TESTTEAM01.*",
            "com.apple.developer.team-identifier": "TESTTEAM01",
            "get-task-allow": True,
            "keychain-access-groups": ["TESTTEAM01.*"],
        },
        "ExpirationDate": __import__("datetime").datetime(2126, 1, 1),
        "Name": "Sweetpad Fixture Wildcard",
        "Platform": ["iOS", "xrOS", "visionOS"],
        "ProvisionedDevices": ["00008110-000000000000001E"],
        "TeamIdentifier": ["TESTTEAM01"],
        "TeamName": "Sweetpad Fixture",
        "UUID": "0F1C7A55-5EE7-4AD0-8D35-5CA1E0F1C7A5",
        "Version": 1,
    }
    content = os.path.join(work, "profile.plist")
    write_plist(content, profile, plistlib.FMT_XML)
    run("openssl", "cms", "-sign", "-binary", "-nodetach", "-outform", "DER", "-in", content,
        "-signer", cert, "-inkey", key,
        "-out", os.path.join(root, "0F1C7A55-5EE7-4AD0-8D35-5CA1E0F1C7A5.mobileprovision"))


# Apple's certificate policy extensions, 1.2.840.113635.100.6.1.<n>, of each kind identity.
KIND_IDENTITIES = [
    ("Apple Distribution: Sweetpad Fixture (TESTTEAM01)", "TESTTEAM01", [4, 7]),
    ("Developer ID Application: Sweetpad Fixture (TESTTEAM01)", "TESTTEAM01", [13]),
    ("3rd Party Mac Developer Installer: Sweetpad Fixture (TESTTEAM01)", "TESTTEAM01", [8]),
    ("Apple Development: Other Fixture (OTHERTEAM2)", "OTHERTEAM2", [2, 12]),
]
DEVICE = "00008110-000000000000001E"


def make_kind_identities(work):
    p12 = os.path.join(HERE, "identities.p12")
    if not os.path.exists(p12):
        keychain = os.path.join(work, "kinds.keychain-db")
        run("security", "create-keychain", "-p", "fixture", keychain)
        try:
            run("security", "unlock-keychain", "-p", "fixture", keychain)
            for i, (name, team, arcs) in enumerate(KIND_IDENTITIES):
                key, cert = os.path.join(work, f"kind{i}.key"), os.path.join(work, f"kind{i}.pem")
                extensions = []
                for arc in arcs:
                    extensions += ["-addext", f"1.2.840.113635.100.6.1.{arc}=critical,DER:0500"]
                run("openssl", "req", "-x509", "-newkey", "rsa:2048", "-nodes", "-keyout", key,
                    "-out", cert, "-days", "36500",
                    "-subj", f"/CN={name}/OU={team}/O=Sweetpad Fixture/C=US",
                    "-addext", "keyUsage=critical,digitalSignature", *extensions)
                single = os.path.join(work, f"kind{i}.p12")
                run("openssl", "pkcs12", "-export", "-inkey", key, "-in", cert, "-out", single,
                    "-passout", f"pass:{PASSWORD}", "-certpbe", "PBE-SHA1-3DES",
                    "-keypbe", "PBE-SHA1-3DES", "-macalg", "sha1")
                run("security", "import", single, "-k", keychain, "-P", PASSWORD, "-A",
                    stdout=subprocess.DEVNULL)
            run("security", "export", "-k", keychain, "-t", "identities", "-f", "pkcs12",
                "-P", PASSWORD, "-o", p12)
        finally:
            run("security", "delete-keychain", keychain)
    pem = subprocess.run(["openssl", "pkcs12", "-in", p12, "-passin", f"pass:{PASSWORD}",
                          "-nokeys", "-legacy"], check=True, capture_output=True).stdout.decode()
    certificates = {}
    for block in pem.split("-----BEGIN CERTIFICATE-----")[1:]:
        block = "-----BEGIN CERTIFICATE-----" + block.split("-----END CERTIFICATE-----")[0] \
            + "-----END CERTIFICATE-----\n"
        der = subprocess.run(["openssl", "x509", "-outform", "DER"], input=block.encode(),
                             check=True, capture_output=True).stdout
        subject = subprocess.run(["openssl", "x509", "-noout", "-subject", "-nameopt", "multiline"],
                                 input=block.encode(), check=True,
                                 capture_output=True).stdout.decode()
        name = next(line.split("=", 1)[1].strip() for line in subject.splitlines()
                    if "commonName" in line)
        certificates[name] = der
    return certificates


def make_kind_profiles(work, key, cert, development_der, certificates):
    """A profile of each kind, named after it, each for TESTTEAM01's dev.sweetpad.kinds.*: an
    app and its extensions."""
    root = os.path.join(HERE, "profiles-kinds")
    shutil.rmtree(root, ignore_errors=True)
    os.makedirs(root)
    distribution = certificates[KIND_IDENTITIES[0][0]]
    developer_id = certificates[KIND_IDENTITIES[1][0]]
    ios = ["iOS", "xrOS", "visionOS"]
    kinds = [
        # name, platforms, certificate, devices, all devices, get-task-allow, aps-environment
        ("development", ios, development_der, [DEVICE], False, True, "development"),
        ("app-store", ios, distribution, None, False, False, "production"),
        ("ad-hoc", ios, distribution, [DEVICE], False, False, "production"),
        ("enterprise", ios, distribution, None, True, False, "production"),
        ("mac-development", ["OSX"], development_der, ["00006000-000A1B2C3D4E5F60"], False,
         True, "development"),
        ("mac-app-store", ["OSX"], distribution, None, False, False, "production"),
        ("developer-id", ["OSX"], developer_id, None, False, False, "production"),
    ]
    for i, (name, platforms, der, devices, all_devices, debuggable, aps) in enumerate(kinds):
        mac = platforms == ["OSX"]
        app = "com.apple.application-identifier" if mac else "application-identifier"
        entitlements = {
            app: "TESTTEAM01.dev.sweetpad.kinds.*",
            "com.apple.developer.team-identifier": "TESTTEAM01",
            "com.apple.developer.aps-environment" if mac else "aps-environment": aps,
            "com.apple.security.application-groups": ["group.dev.sweetpad.kinds"],
            "com.apple.developer.icloud-container-environment": ["Development", "Production"],
            "keychain-access-groups": ["TESTTEAM01.*"],
        }
        if not mac or debuggable:
            entitlements["com.apple.security.get-task-allow" if mac else "get-task-allow"] = \
                debuggable
        if name == "app-store":
            entitlements["beta-reports-active"] = True
        uuid = f"0F1C7A55-5EE7-4AD0-8D35-5CA1E0F1C7{i:02X}"
        profile = {
            "AppIDName": "Sweetpad Kinds",
            "CreationDate": __import__("datetime").datetime(2026, 1, 1),
            "DeveloperCertificates": [der],
            "Entitlements": entitlements,
            "ExpirationDate": __import__("datetime").datetime(2126, 1, 1),
            "Name": f"Sweetpad Kinds {name}",
            "Platform": platforms,
            "TeamIdentifier": ["TESTTEAM01"],
            "TeamName": "Sweetpad Fixture",
            "UUID": uuid,
            "Version": 1,
        }
        if devices:
            profile["ProvisionedDevices"] = devices
        if all_devices:
            profile["ProvisionsAllDevices"] = True
        content = os.path.join(work, f"{name}.plist")
        write_plist(content, profile, plistlib.FMT_XML)
        extension = "provisionprofile" if mac else "mobileprovision"
        run("openssl", "cms", "-sign", "-binary", "-nodetach", "-outform", "DER", "-in", content,
            "-signer", cert, "-inkey", key, "-out", os.path.join(root, f"{name}.{extension}"))


def make_kinds(work):
    _, key, cert, der, _ = make_identity(work)
    make_kind_profiles(work, key, cert, der, make_kind_identities(work))


def main():
    with tempfile.TemporaryDirectory() as work:
        p12, key, cert, der, sha1 = make_identity(work)
        keychain = make_keychain(work, p12)
        try:
            def sign(path, *flags):
                run("codesign", "--keychain", keychain, "--force", "--sign", sha1, *flags,
                    "--timestamp=none", "--generate-entitlement-der", path,
                    stderr=subprocess.DEVNULL)
            make_inputs(work, sign)
            make_expected(sign)
            make_profile(work, key, cert, der)
            make_kind_profiles(work, key, cert, der, make_kind_identities(work))
        finally:
            run("security", "delete-keychain", keychain)


if __name__ == "__main__":
    if sys.argv[1:] == ["kinds"]:
        with tempfile.TemporaryDirectory() as scratch:
            make_kinds(scratch)
    else:
        main()
