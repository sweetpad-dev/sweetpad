"""Sign explicit simulator test copies; never modify source build products."""
from pathlib import Path
import plistlib
import subprocess
import tempfile


def sign(app, entitlements):
    app = Path(app)
    info = plistlib.loads((app / "Info.plist").read_bytes())
    if info.get("CFBundleSupportedPlatforms") != ["iPhoneSimulator"]:
        raise ValueError("test signing accepts only iOS simulator bundles")
    def codesign(path, entitlement_file=None):
        command = ["/usr/bin/codesign", "--force", "--sign", "-", "--timestamp=none"]
        if entitlement_file:
            command += ["--entitlements", str(entitlement_file)]
        subprocess.run([*command, str(path)], check=True, capture_output=True)
    with tempfile.TemporaryDirectory(prefix="sweetpad-simulator-entitlements-") as work:
        path = Path(work) / "entitlements.plist"
        path.write_bytes(plistlib.dumps(entitlements))
        for framework in sorted(app.rglob("*.framework"), key=lambda p: -len(p.parts)):
            codesign(framework)
        for library in app.glob("Frameworks/*.dylib"):
            codesign(library)
        for extension in app.rglob("*.appex"):
            codesign(extension, path)
        for tests in app.rglob("*.xctest"):
            codesign(tests)
        codesign(app, path)
