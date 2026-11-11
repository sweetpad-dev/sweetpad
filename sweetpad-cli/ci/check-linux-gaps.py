#!/usr/bin/env python3
"""Exercise Linux scripts, clean, configuration, and build/deploy routing.

Compilation is real. Device transport is a local fixture; this check never
contacts, installs on, or launches anything on an actual phone.
"""
import argparse
import json
import os
import platform
import re
from pathlib import Path
import shutil
import subprocess
import tempfile
import xml.etree.ElementTree as ET

REPO = Path(__file__).resolve().parents[2]


def check(binary, out):
    binary, out = binary.resolve(), out.resolve()
    out.mkdir(parents=True, exist_ok=False)
    source = out / "source"
    shutil.copytree(REPO / "sweetpad-lib/ci/fixture-linux-script", source)
    project = source / "LinuxApp.xcodeproj"
    config = source / "sweetpad.toml"
    config.write_text('[xcodebuild]\nargs = []\n')
    env = dict(os.environ, XDG_STATE_HOME=str(out / "state"), XDG_CACHE_HOME=str(out / "cache"))
    trace = out / "scheme-actions.ndjson"
    env["GAP_ACTION_LOG"] = str(trace)
    common = ["--project", str(project), "--scheme", "LinuxApp", "--json", "--non-interactive"]

    def invoke(name, arguments, success=True, environment=None):
        result = subprocess.run([str(binary), *arguments], cwd=source, env=environment or env,
                                capture_output=True, text=True, timeout=600)
        (out / (name + ".stdout")).write_text(result.stdout)
        (out / (name + ".stderr")).write_text(result.stderr)
        if (result.returncode == 0) != success:
            raise AssertionError(f"{name}: exit {result.returncode}; see {out / (name + '.stderr')}")
        return json.loads(result.stdout if success else result.stderr)

    build = ["build", *common, "--destination", "generic/platform=iOS Simulator,arch=arm64", "--", "ARCHS=arm64", "CODE_SIGNING_ALLOWED=NO"]
    built=invoke("script-build", build)["data"]
    assert built["built"]
    def rows():
        return [json.loads(line) for line in trace.read_text().splitlines()]

    initial = rows()
    assert [row["phase"] for row in initial] == ["BuildAction.PreActions", "BuildAction.PreActions.NoEnvironment", "BuildAction.PostActions"]
    assert all(Path(row["cwd"]).resolve() == Path(tempfile.gettempdir()).resolve() for row in initial)
    settings = initial[0]["environment"]
    assert settings["SRCROOT"] == str(source) and settings["CONFIGURATION"] == "Debug"
    assert settings["SDKROOT"].startswith("iphonesimulator") and not settings["SDKROOT"].startswith("/")
    assert Path(settings["PLATFORM_DIR"]).is_dir() and Path(settings["DEVELOPER_DIR"]).is_dir()
    assert Path(settings["BUILT_PRODUCTS_DIR"]).is_dir() and settings["TARGET_NAME"] == "LinuxApp"
    assert all(value is None for value in initial[1]["environment"].values())
    trace.unlink()
    pre = invoke("scheme-pre-failure", build, False, dict(env, GAP_PRE_EXIT="1"))
    assert pre["error"]["code"] == "build_failure" and "BuildAction.PreActions" in pre["error"]["message"]
    assert [row["phase"] for row in rows()] == ["BuildAction.PreActions"]
    trace.unlink()
    post = invoke("scheme-post-failure", build, False, dict(env, GAP_POST_EXIT="1"))
    assert post["error"]["code"] == "build_failure" and "BuildAction.PostActions" in post["error"]["message"]
    assert len(rows()) == 3
    trace.unlink()
    derived=Path(built["productPath"]).parents[3]
    proofs = list(derived.rglob("sandbox-proof.txt"))
    assert len(proofs) == 1 and proofs[0].read_text() == "script-ok"
    assert invoke("clean", ["clean", *common])["ok"]
    assert not (derived / "Build/Products").exists()
    assert not (derived / "Build/Intermediates.noindex").exists()
    assert invoke("rebuild", build)["data"]["built"]
    assert invoke("purge", ["clean", *common, "--purge"])["ok"]
    assert not derived.exists()

    config.write_text('[xcodebuild]\nargs = ["-allowProvisioningUpdates"]\n')
    invalid = invoke("configured-xcode-flag", build, success=False)
    assert "sweetpad.toml" in invalid["error"]["message"] and "-allowProvisioningUpdates" in invalid["error"]["message"]
    config.write_text('[xcodebuild]\nargs = []\n')
    missing = dict(env, SWEETPAD_DEVICE_PYTHON=str(out / "missing-python"),
                   XDG_STATE_HOME=str(out / "no-device-state"))
    missing.pop("SWEETPAD_DEVICE_NETWORK_CONFIG", None)
    discovery = invoke("discovery-error", ["destination", "list", "--json", "--non-interactive"], False, missing)
    assert "Linux device transport" in discovery["error"]["message"]
    picker = invoke("no-mac-picker", ["run", *common, "--no-logs"], False, missing)
    assert "Linux device transport" in picker["error"]["message"]

    adapter = out / "fixture-python"
    adapter.write_text('''#!/usr/bin/python3
import json,os,sys
from pathlib import Path
request=json.loads(sys.argv[-1])
with open(os.environ['FIXTURE_TRANSPORT_LOG'],'a') as log: log.write(json.dumps(request)+'\\n')
if request['action']=='list':
 print(json.dumps([{'UniqueDeviceID':'00000000-0000000000000001','DeviceName':'Fixture iPhone','DeviceClass':'iPhone','ProductVersion':'27.2','ConnectionType':'USB'}]))
elif request['action']=='install':
 assert Path(request['app']).is_dir()
 print(json.dumps({'completed':True}))
elif request['action']=='launch': print(json.dumps({'pid':4242}))
else: raise RuntimeError('Unexpected fixture action: '+request['action'])
''')
    adapter.chmod(0o755)
    fake = dict(env, SWEETPAD_DEVICE_PYTHON=str(adapter), FIXTURE_TRANSPORT_LOG=str(out / "transport.ndjson"))
    fake.pop("SWEETPAD_DEVICE_NETWORK_CONFIG", None)
    phone_build = ["--", "ARCHS=arm64", "CODE_SIGNING_ALLOWED=NO"]
    assert invoke("app-install-swift-build", ["app", "install", *common, "--device-id", "00000000-0000000000000001", *phone_build], environment=fake)["ok"]
    assert invoke("run-swift-build", ["run", *common, "--device-id", "00000000-0000000000000001", "--no-logs", *phone_build], environment=fake)["ok"]
    assert invoke("build-on-device-name", ["build", *common, "--on", "Fixture iPhone", *phone_build], environment=fake)["data"]["built"]
    assert invoke("build-on-device", ["build", *common, "--on", "device", *phone_build], environment=fake)["data"]["built"]
    picker_env=dict(fake,XDG_STATE_HOME=str(out / "picker-state"))
    assert invoke("pick-linux-device", ["run", *common, "--no-logs", *phone_build], environment=picker_env)["ok"]
    destinations=invoke("list-linux-device",["destination","list","--json","--non-interactive"],environment=fake)["data"]["destinations"]
    assert len(destinations)==1 and destinations[0]["kind"]=="device"
    actions = [json.loads(line)["action"] for line in (out / "transport.ndjson").read_text().splitlines()]
    assert actions.count("install") == 3 and actions.count("launch") == 2
    trace.unlink()
    archive = invoke("scheme-archive", ["archive", *common, "--destination", "generic/platform=iOS", "--no-export",
                     "--output-file", str(out / "archive"), *phone_build])
    assert archive["ok"]
    assert [row["phase"] for row in rows()] == ["BuildAction.PreActions", "BuildAction.PreActions.NoEnvironment", "BuildAction.PostActions", "ArchiveAction.PreActions", "ArchiveAction.PostActions"]
    assert all(row["environment"]["CONFIGURATION"] == "Release" for row in rows() if "NoEnvironment" not in row["phase"])
    archive_env = rows()[-1]["environment"]
    assert Path(archive_env["ARCHIVE_PATH"]).joinpath("Info.plist").is_file()
    assert archive_env["ARCHIVE_DSYMS_PATH"] == archive_env["ARCHIVE_PATH"] + "/dSYMs"
    assert Path(archive_env["ARCHIVE_PRODUCTS_PATH"]).joinpath("Applications/LinuxApp.app").is_dir()
    assert rows()[-2]["environment"]["ARCHIVE_PATH"] is None
    trace.unlink()
    pbx=project / "project.pbxproj"
    text=pbx.read_text()
    text=re.sub(r'shellScript = "(?:[^"\\]|\\.)*";', lambda _: 'shellScript = '+json.dumps("echo expected-script-failure >&2\nexit 1\n")+';',text,count=1)
    pbx.write_text(text)
    human=[arg for arg in build if arg!='--json']
    failure=subprocess.run([str(binary),*human],cwd=source,env=env,capture_output=True,text=True,timeout=600)
    (out/'human-failure.stdout').write_text(failure.stdout)
    (out/'human-failure.stderr').write_text(failure.stderr)
    assert failure.returncode != 0 and 'read full build log:' in failure.stderr
    assert [row['phase'] for row in rows()] == ['BuildAction.PreActions', 'BuildAction.PreActions.NoEnvironment']
    scheme_file = project / "xcshareddata/xcschemes/LinuxApp.xcscheme"
    tree = ET.parse(scheme_file)
    action = tree.find("BuildAction/PreActions/ExecutionAction")
    original_type = action.get("ActionType")
    action.set("ActionType", "UnsupportedFixtureAction")
    tree.write(scheme_file, encoding="unicode")
    trace.unlink()
    unsupported = invoke("unsupported-scheme-action", build, False)
    assert "unsupported BuildAction.PreActions" in unsupported["error"]["message"]
    assert not trace.exists()
    action.set("ActionType", original_type)
    reference = action.find("ActionContent/EnvironmentBuildable/BuildableReference")
    reference.set("BlueprintIdentifier", "FFFFFFFFFFFFFFFFFFFFFFFF")
    tree.write(scheme_file, encoding="unicode")
    missing_target = invoke("missing-scheme-action-target", build, False)
    assert "references missing target" in missing_target["error"]["message"]
    assert not trace.exists()
    summary = {"passed": True, "compilation": f"real Linux {platform.machine()} host", "targetArchitecture": "arm64", "transport": "local fixture, no hardware",
               "checks": ["sandboxed project script", "generated scheme source", "scheme action environment/order/failures", "archive actions", "clean", "purge", "configuration flag error", "discovery error",
                          "Linux picker", "app install engine", "run engine", "build --on device name"]}
    (out / "summary.json").write_text(json.dumps(summary, indent=2) + "\n")
    print(json.dumps(summary, indent=2))


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--out", type=Path, required=True)
    args = parser.parse_args()
    check(args.binary, args.out)
