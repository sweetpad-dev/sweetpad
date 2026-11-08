#!/usr/bin/env python3
"""Prepare and build fresh hybrid fixtures on Linux with executable auditing.

Uses an already provisioned Apple SDK and Linux Swift toolchain. Application
dependencies and build products start in a fresh output directory. No signing
identity, physical device, macOS executable, or native dependency preparation is
needed. Source checkouts and reports remain available after build-cache cleanup.
"""
import argparse
import importlib.util
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys

CI = Path(__file__).resolve().parent
REPO = CI.parent.parent


def load(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--out", type=Path, required=True)
    parser.add_argument("--only", default="")
    parser.add_argument("--configuration", choices=["Debug", "Release"], default="Release")
    parser.add_argument("--destination", default="generic/platform=iOS")
    parser.add_argument("--tools-root", type=Path, default=Path.home()/".cache/sweetpad/framework-tools")
    parser.add_argument("--flutter-root", type=Path, default=Path.home()/"src/flutter")
    parser.add_argument("--flutter-compiler", type=Path, default=Path.home()/"src/sweetpad-dart-ios-snapshot/out/SweetpadIOS/gen_snapshot_product")
    parser.add_argument("--sweetpad", type=Path, default=Path.home()/"sweetpad-target/debug/sweetpad")
    parser.add_argument("--keep-build-cache", action="store_true")
    args = parser.parse_args()
    if sys.platform != "linux": parser.error("run on Linux")
    out = args.out.resolve()
    out.mkdir(parents=True, exist_ok=False)
    for directory in ["reports", "sources", "products", "derived"]: (out/directory).mkdir()
    env = dict(os.environ)
    prefixes = [args.tools_root/"js/node_modules/.bin", args.tools_root/"node/bin", args.tools_root/"ruby/bin",
                Path.home()/"bin", Path.home()/"swift/swift-6.4.2-RELEASE/usr/bin"]
    env["PATH"] = os.pathsep.join(map(str,prefixes)) + os.pathsep + env["PATH"]
    for name in ["SWEETPAD_CODE_SIGN_P12", "SWEETPAD_CODE_SIGN_P12_PASSWORD", "SWEETPAD_PROVISIONING_PROFILES"]: env.pop(name,None)
    selected = [p for p in json.loads((CI/"linux-projects.json").read_text()) if p.get("framework") and args.only in p["slug"]]
    if not selected: parser.error("no hybrid fixtures match")
    summary = {"host":"linux", "configuration":args.configuration, "destination":args.destination, "projects":{}}

    def audited(command, slug, phase):
        audit = out/"reports"/(slug+"-"+phase+"-audit")
        with (out/"reports"/(slug+"-"+phase+".json")).open("w") as stdout, (out/"reports"/(slug+"-"+phase+".stderr")).open("w") as stderr:
            result = subprocess.run([sys.executable,str(CI/"linux-exec-audit.py"),"--out",str(audit),"--",*map(str,command)],env=env,cwd=REPO,stdout=stdout,stderr=stderr,timeout=3600)
        if result.returncode: raise RuntimeError(f"{slug} {phase}: inspect {out/'reports'}")
        report = json.loads((audit/"summary.json").read_text())
        assert report["passed"] and not report["violations"]
        return report

    local = load("local_engine", CI/"linux-flutter-local-engine.py")
    try:
        for project in selected:
            slug = project["slug"]
            prep = audited([sys.executable,"-B",CI/"linux-project-prepare.py","--only",slug,
                "--corpus-root",out/"sources","--tool-bin",args.tools_root/"js/node_modules/.bin",
                "--gem-home",args.tools_root/"gems","--flutter-root",args.flutter_root,
                "--out",out/"reports"/(slug+"-dependencies.json")],slug,"prepare")
            root = out/"sources"/slug
            settings = [s.replace("@SOURCE_ROOT@",str(root)).replace("@HOME@",str(Path.home())) for s in project.get("linuxSettings",[])]
            if project["framework"] == "Flutter":
                settings.extend(f"{k}={v}" for k,v in local.prepare(args.flutter_root,args.flutter_compiler,out/"flutter-engine").items())
            container = "workspace" if "workspace" in project else "project"
            command = [args.sweetpad,"build","--"+container,root/project[container],"--scheme",project["scheme"],
                "--configuration",args.configuration,"--destination",args.destination,"--json","--non-interactive",
                "--","-derivedDataPath",out/"derived"/slug,"ARCHS=arm64","ONLY_ACTIVE_ARCH=YES","CODE_SIGNING_ALLOWED=NO",*settings]
            build = audited(command,slug,"build")
            result = json.loads((out/"reports"/(slug+"-build.json")).read_text())
            assert result["ok"] and result["data"]["built"]
            app = Path(result["data"]["productPath"])
            destination = out/"products"/slug/app.name
            destination.parent.mkdir()
            shutil.move(str(app),destination)
            summary["projects"][slug] = {"commit":project["commit"],"passed":True,"product":str(destination),
                "preparationTools":len(prep["tools"]),"buildTools":len(build["tools"]),"macOSExecutables":0,
                "files":sum(1 for p in destination.rglob('*') if p.is_file())}
            (out/"summary.json").write_text(json.dumps(summary,indent=2)+"\n")
            print(f"{slug}: fresh Linux preparation and {args.configuration} build pass",flush=True)
            if not args.keep_build_cache:
                # Preserve SourcePackages and application sources; only discard
                # this run's generated intermediates and module/index caches.
                derived = out/"derived"/slug
                for relative in ["Build/Intermediates.noindex","ModuleCache.noindex","SDKStatCaches.noindex","Index.noindex"]:
                    shutil.rmtree(derived/relative,ignore_errors=True)
    except Exception as error:
        summary["error"] = str(error)
        (out/"summary.json").write_text(json.dumps(summary,indent=2)+"\n")
        raise


if __name__ == "__main__":
    main()
