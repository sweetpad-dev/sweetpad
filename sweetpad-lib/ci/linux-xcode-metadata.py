#!/usr/bin/env python3
"""Answer dependency generators' metadata queries from the imported Apple SDK.

This never builds a project or invokes an Apple executable. Unsupported queries
fail instead of pretending that Xcode is installed on Linux.
"""
import os
from pathlib import Path
import plistlib
import shutil
import sys


def query(tool, args, bundle):
    if tool == "command" and len(args) == 2 and args[0] == "-v":
        return shutil.which(args[1])
    if tool == "xcodebuild" and args == ["-version"]:
        info = plistlib.loads((bundle / "version.plist").read_bytes())
        return "Xcode " + info["CFBundleShortVersionString"] + "\nBuild version " + info["ProductBuildVersion"]
    if tool == "xcode-select" and args in [["-p"], ["--print-path"]]:
        return str(bundle / "Developer")
    raise ValueError(f"unsupported metadata query: {tool} {' '.join(args)}")


if __name__ == "__main__":
    bundle = Path(os.environ.get("SWEETPAD_DARWIN_SDK", Path.home() / ".swiftpm/swift-sdks/darwin.artifactbundle"))
    try:
        result = query(sys.argv[1], sys.argv[2:], bundle)
        if result is None:
            raise SystemExit(1)
        print(result)
    except (ValueError, OSError, KeyError) as error:
        print(str(error), file=sys.stderr)
        raise SystemExit(1)
