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
"""

import os
import plistlib
import shutil
import subprocess
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
        finally:
            run("security", "delete-keychain", keychain)


if __name__ == "__main__":
    main()
