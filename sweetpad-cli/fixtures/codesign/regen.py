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
orphan.p12 holds an identity issued by a made-up authority whose certificate it leaves out.

input/Mac.app is a macOS app as Swift Build hands it to codesign, its code linker-signed: a
universal executable, a helper in MacOS/, a versioned framework, a dylib, an XPC service and a
resource bundle. expected/Mac.app is codesign's signature of it inside out, as Swift Build
asks for one, with the hardened runtime; expected/Mac.adhoc.app the same ad hoc. Assets.bundle
is the resource bundle signed alone. `regen.py mac` remakes only those.

Watch, Phone15, Phone10 and TV are linker-signed executables for the platforms whose code
directories differ from the Mac's: a watchOS 9 arm64_32 and arm64 universal file, iOS 15 and
tvOS 17 hashed in 4 KiB pages, and iOS 10 with a SHA-1 code directory beside the SHA-256 one.
Info.dylib carries an Info.plist in __TEXT,__info_plist, which names its identifier.
Legacy.app is an iOS app around an iOS 10 executable, whose resource seal codesign writes
with SHA-1 hashes beside the SHA-256 ones. expected/ has each signed ad hoc, and Phone10 also
with the identity. `regen.py platforms` remakes only those.
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


MAC_ENTITLEMENTS = {
    "com.apple.application-identifier": "TESTTEAM01.dev.sweetpad.macfixture",
    "com.apple.developer.team-identifier": "TESTTEAM01",
    "com.apple.security.app-sandbox": True,
    "com.apple.security.application-groups": ["TESTTEAM01.dev.sweetpad.macfixture"],
    "com.apple.security.get-task-allow": True,
}


def compile_mac(work, source, output, archs, *flags):
    path = os.path.join(work, os.path.basename(output) + ".c")
    with open(path, "w") as f:
        f.write(source)
    arch_flags = [flag for arch in archs for flag in ("-arch", arch)]
    run("xcrun", "-sdk", "macosx", "clang", *arch_flags, "-mmacos-version-min=13.0",
        "-Wl,-dead_strip", "-o", output, path, *flags)


def make_mac_inputs(work):
    root = os.path.join(HERE, "input")
    for name in ["Mac.app", "Assets.bundle", "mac.xcent"]:
        path = os.path.join(root, name)
        shutil.rmtree(path, ignore_errors=True) if os.path.isdir(path) else (
            os.path.exists(path) and os.remove(path))
    universal = ["arm64", "x86_64"]
    contents = os.path.join(root, "Mac.app", "Contents")
    resources = os.path.join(contents, "Resources")
    for folder in ["MacOS", "Resources/en.lproj", "Resources/fr.lproj", "Resources/Base.lproj"]:
        os.makedirs(os.path.join(contents, folder))
    write_plist(os.path.join(contents, "Info.plist"), {
        "CFBundleExecutable": "Mac",
        "CFBundleIdentifier": "dev.sweetpad.macfixture",
        "CFBundlePackageType": "APPL",
        "CFBundleVersion": "1",
        "LSMinimumSystemVersion": "13.0",
    })
    with open(os.path.join(contents, "PkgInfo"), "w") as f:
        f.write("APPL????")
    compile_mac(work, "int main(void) { return 0; }\n", os.path.join(contents, "MacOS", "Mac"),
                universal)
    compile_mac(work, "int main(void) { return 1; }\n", os.path.join(contents, "MacOS", "Helper"),
                ["arm64"])
    with open(os.path.join(resources, "dot.txt"), "w") as f:
        f.write("dot\n")
    os.symlink("dot.txt", os.path.join(resources, "link.txt"))
    with open(os.path.join(resources, "en.lproj", "Localizable.strings"), "w") as f:
        f.write('"hello" = "Hello";\n')
    with open(os.path.join(resources, "fr.lproj", "locversion.plist"), "w") as f:
        f.write("omitted\n")
    with open(os.path.join(resources, "Base.lproj", "Main.strings"), "w") as f:
        f.write('"title" = "Main";\n')
    for assets in [os.path.join(resources, "Assets.bundle"), os.path.join(root, "Assets.bundle")]:
        os.makedirs(os.path.join(assets, "Contents", "Resources"))
        write_plist(os.path.join(assets, "Contents", "Info.plist"), {
            "CFBundleIdentifier": "dev.sweetpad.assets",
            "CFBundlePackageType": "BNDL",
            "CFBundleVersion": "1",
        })
        with open(os.path.join(assets, "Contents", "Resources", "asset.txt"), "w") as f:
            f.write("asset\n")
    kit = os.path.join(contents, "Frameworks", "Kit.framework")
    os.makedirs(os.path.join(kit, "Versions", "A", "Resources", "en.lproj"))
    write_plist(os.path.join(kit, "Versions", "A", "Resources", "Info.plist"), {
        "CFBundleExecutable": "Kit",
        "CFBundleIdentifier": "dev.sweetpad.kit",
        "CFBundlePackageType": "FMWK",
        "CFBundleVersion": "1",
    })
    with open(os.path.join(kit, "Versions", "A", "Resources", "en.lproj", "Kit.strings"), "w") as f:
        f.write('"kit" = "Kit";\n')
    compile_mac(work, "int kit(void) { return 7; }\n", os.path.join(kit, "Versions", "A", "Kit"),
                universal, "-dynamiclib", "-install_name", "@rpath/Kit.framework/Versions/A/Kit")
    os.symlink("A", os.path.join(kit, "Versions", "Current"))
    os.symlink("Versions/Current/Kit", os.path.join(kit, "Kit"))
    os.symlink("Versions/Current/Resources", os.path.join(kit, "Resources"))
    compile_mac(work, "int thing(void) { return 3; }\n",
                os.path.join(contents, "Frameworks", "libThing.dylib"), universal, "-dynamiclib",
                "-install_name", "@rpath/libThing.dylib")
    worker = os.path.join(contents, "XPCServices", "Worker.xpc", "Contents")
    os.makedirs(os.path.join(worker, "MacOS"))
    write_plist(os.path.join(worker, "Info.plist"), {
        "CFBundleExecutable": "Worker",
        "CFBundleIdentifier": "dev.sweetpad.macfixture.worker",
        "CFBundlePackageType": "XPC!",
        "CFBundleVersion": "1",
    })
    compile_mac(work, "int main(void) { return 2; }\n", os.path.join(worker, "MacOS", "Worker"),
                ["arm64"])
    write_plist(os.path.join(root, "mac.xcent"), MAC_ENTITLEMENTS, plistlib.FMT_XML)


# What Swift Build signs in a macOS app, inside out, and whether with the entitlements.
MAC_STEPS = [
    ("Contents/Frameworks/libThing.dylib", False),
    ("Contents/Frameworks/Kit.framework/Versions/A", False),
    ("Contents/XPCServices/Worker.xpc", True),
    ("Contents/MacOS/Helper", False),
    ("", True),
]


def make_mac_expected(sign):
    entitlements = os.path.join(HERE, "input", "mac.xcent")
    for name, flags in [("Mac.app", ["-o", "runtime"]), ("Mac.adhoc.app", None)]:
        app = os.path.join(HERE, "expected", name)
        shutil.rmtree(app, ignore_errors=True)
        shutil.copytree(os.path.join(HERE, "input", "Mac.app"), app, symlinks=True)
        for step, with_entitlements in MAC_STEPS:
            extra = ["--entitlements", entitlements] if with_entitlements else []
            path = os.path.join(app, step) if step else app
            if flags is None:
                run("codesign", "--force", "--sign", "-", "--timestamp=none",
                    "--generate-entitlement-der", *extra, path, stderr=subprocess.DEVNULL)
            else:
                sign(path, *flags, *extra)
    for name, adhoc in [("Assets.bundle", False), ("Assets.adhoc.bundle", True)]:
        bundle = os.path.join(HERE, "expected", name)
        shutil.rmtree(bundle, ignore_errors=True)
        shutil.copytree(os.path.join(HERE, "input", "Assets.bundle"), bundle, symlinks=True)
        if adhoc:
            run("codesign", "--force", "--sign", "-", "--timestamp=none",
                "--generate-entitlement-der", bundle, stderr=subprocess.DEVNULL)
        else:
            sign(bundle)


def make_mac():
    with tempfile.TemporaryDirectory() as work:
        p12, _, _, _, sha1 = make_identity(work)
        keychain = make_keychain(work, p12)
        try:
            def sign(path, *flags):
                run("codesign", "--keychain", keychain, "--force", "--sign", sha1, *flags,
                    "--timestamp=none", "--generate-entitlement-der", path,
                    stderr=subprocess.DEVNULL)
            make_mac_inputs(work)
            make_mac_expected(sign)
        finally:
            run("security", "delete-keychain", keychain)


# Each platform input: the SDK, its version flag, and the architectures.
PLATFORMS = [
    ("Watch", "watchos", "-mwatchos-version-min=9.0", ["arm64_32", "arm64"]),
    ("Phone15", "iphoneos", "-mios-version-min=15.0", ["arm64"]),
    ("Phone10", "iphoneos", "-mios-version-min=10.0", ["arm64"]),
    ("TV", "appletvos", "-mtvos-version-min=17.0", ["arm64"]),
]


def make_platform_inputs(work):
    root = os.path.join(HERE, "input")
    for name, sdk, version, archs in PLATFORMS:
        path = os.path.join(work, name + ".c")
        with open(path, "w") as f:
            f.write("int main(void) { return 0; }\n")
        arch_flags = [flag for arch in archs for flag in ("-arch", arch)]
        run("xcrun", "-sdk", sdk, "clang", *arch_flags, version, "-o",
            os.path.join(root, name), path)
    info = os.path.join(work, "Info.plist")
    write_plist(info, {
        "CFBundleIdentifier": "dev.sweetpad.info",
        "CFBundleName": "Info",
        "CFBundleVersion": "1",
    }, fmt=plistlib.FMT_XML)
    path = os.path.join(work, "info.c")
    with open(path, "w") as f:
        f.write("int info(void) { return 4; }\n")
    run("xcrun", "-sdk", "macosx", "clang", "-arch", "arm64", "-mmacos-version-min=13.0",
        "-dynamiclib", "-install_name", "@rpath/libInfo.dylib",
        "-Wl,-sectcreate,__TEXT,__info_plist," + info,
        "-o", os.path.join(root, "Info.dylib"), path)
    app = os.path.join(root, "Legacy.app")
    shutil.rmtree(app, ignore_errors=True)
    os.makedirs(os.path.join(app, "en.lproj"))
    shutil.copy(os.path.join(root, "Phone10"), os.path.join(app, "Legacy"))
    write_plist(os.path.join(app, "Info.plist"), {
        "CFBundleExecutable": "Legacy",
        "CFBundleIdentifier": "dev.sweetpad.legacy",
        "CFBundlePackageType": "APPL",
        "CFBundleVersion": "1",
    })
    with open(os.path.join(app, "dot.png"), "wb") as f:
        f.write(bytes(range(64)))
    with open(os.path.join(app, "en.lproj", "Localizable.strings"), "w") as f:
        f.write('"hello" = "Hello";\n')


def make_platform_expected(sign):
    for name in [entry[0] for entry in PLATFORMS] + ["Info.dylib"]:
        stem, dot, extension = name.partition(".")
        expected = os.path.join(HERE, "expected", f"{stem}.adhoc{dot}{extension}")
        shutil.copy(os.path.join(HERE, "input", name), expected)
        run("codesign", "--force", "--sign", "-", "--timestamp=none",
            "--generate-entitlement-der", expected, stderr=subprocess.DEVNULL)
    expected = os.path.join(HERE, "expected", "Legacy.adhoc.app")
    shutil.rmtree(expected, ignore_errors=True)
    shutil.copytree(os.path.join(HERE, "input", "Legacy.app"), expected)
    run("codesign", "--force", "--sign", "-", "--timestamp=none",
        "--generate-entitlement-der", expected, stderr=subprocess.DEVNULL)
    # Both code directories under one CMS signature.
    expected = os.path.join(HERE, "expected", "Phone10")
    shutil.copy(os.path.join(HERE, "input", "Phone10"), expected)
    sign(expected)


def make_platforms():
    with tempfile.TemporaryDirectory() as work:
        p12, _, _, _, sha1 = make_identity(work)
        keychain = make_keychain(work, p12)
        try:
            def sign(path, *flags):
                run("codesign", "--keychain", keychain, "--force", "--sign", sha1, *flags,
                    "--timestamp=none", "--generate-entitlement-der", path,
                    stderr=subprocess.DEVNULL)
            make_platform_inputs(work)
            make_platform_expected(sign)
        finally:
            run("security", "delete-keychain", keychain)


def make_orphan(work):
    """orphan.p12: an identity whose issuer, a made-up authority, isn't in the file."""
    p12 = os.path.join(HERE, "orphan.p12")
    if os.path.exists(p12):
        return
    ca_key, ca = os.path.join(work, "ca.key"), os.path.join(work, "ca.pem")
    run("openssl", "req", "-x509", "-newkey", "rsa:2048", "-nodes", "-keyout", ca_key,
        "-out", ca, "-days", "36500",
        "-subj", "/CN=Sweetpad Fixture Authority/OU=G9/O=Sweetpad Fixture/C=US")
    key, request, cert = (os.path.join(work, n) for n in ["orphan.key", "orphan.csr", "orphan.pem"])
    run("openssl", "req", "-newkey", "rsa:2048", "-nodes", "-keyout", key, "-out", request,
        "-subj", "/CN=Apple Development: Sweetpad Orphan (TESTTEAM09)/OU=TESTTEAM09/O=Sweetpad Fixture/C=US")
    run("openssl", "x509", "-req", "-in", request, "-CA", ca, "-CAkey", ca_key,
        "-CAcreateserial", "-out", cert, "-days", "36500")
    run("openssl", "pkcs12", "-export", "-inkey", key, "-in", cert, "-out", p12,
        "-passout", f"pass:{PASSWORD}", "-certpbe", "PBE-SHA1-3DES", "-keypbe", "PBE-SHA1-3DES",
        "-macalg", "sha1")


def make_kinds(work):
    _, key, cert, der, _ = make_identity(work)
    make_kind_profiles(work, key, cert, der, make_kind_identities(work))
    make_orphan(work)


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
            make_orphan(work)
        finally:
            run("security", "delete-keychain", keychain)
    make_mac()
    make_platforms()


if __name__ == "__main__":
    if sys.argv[1:] == ["mac"]:
        make_mac()
    elif sys.argv[1:] == ["platforms"]:
        make_platforms()
    elif sys.argv[1:] == ["kinds"]:
        with tempfile.TemporaryDirectory() as scratch:
            make_kinds(scratch)
    else:
        main()
