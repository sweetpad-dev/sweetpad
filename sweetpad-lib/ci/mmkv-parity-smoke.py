#!/usr/bin/env python3
"""Exercise both prebuilt MMKV apps through Metro, without recompilation.

Use a test simulator: this removes each existing MMKV example installation.
The temporary Metro entry tests native storage, then renders the original app.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import plistlib
import subprocess
import time
import urllib.request

CI = Path(__file__).resolve().parent
CORPUS = CI.parent / "corpus"

PROBE = r"""
import {AppRegistry} from 'react-native';
import {createMMKV} from 'react-native-mmkv';
import App from './src/App';
import {name} from './app.json';
const checks = {};
let phase = 0;
let error = null;
try {
  const storage = createMMKV({id: 'dev.sweetpad.parity'});
  phase = (storage.getNumber('launches') ?? 0) + 1;
  checks.persistence = phase === 1 || storage.getString('persistent') === 'Linux/Apple 🦀';
  storage.set('persistent', 'Linux/Apple 🦀');
  storage.set('number', -42.5);
  checks.number = storage.getNumber('number') === -42.5;
  storage.set('boolean', true);
  checks.boolean = storage.getBoolean('boolean') === true;
  const bytes = new Uint8Array([0, 1, 127, 255]);
  storage.set('buffer', bytes.buffer);
  checks.buffer = Array.from(new Uint8Array(storage.getBuffer('buffer'))).join() === bytes.join();
  storage.set('remove', 'temporary');
  checks.keys = storage.getAllKeys().includes('remove');
  storage.remove('remove');
  checks.remove = !storage.contains('remove') && storage.getString('remove') === undefined;
  checks.string = storage.getString('persistent') === 'Linux/Apple 🦀';
  storage.set('launches', phase);
} catch (e) {
  error = String(e.stack ?? e);
}
fetch('http://127.0.0.1:8081/__sweetpad_mmkv_result', {
  method: 'POST', headers: {'Content-Type': 'application/json'},
  body: JSON.stringify({phase, checks, error}),
});
AppRegistry.registerComponent(name, () => App);
"""

CONFIG = r"""
const fs = require('fs');
const config = require('./metro.config.js');
const previous = config.server?.enhanceMiddleware;
const rewrite = config.server?.rewriteRequestUrl;
config.server = {...config.server,
  rewriteRequestUrl: url => (rewrite ? rewrite(url) : url)
    .replace(/^\/index\.bundle(?=\?|$)/, '/_sweetpad_mmkv_probe.bundle'),
  enhanceMiddleware: (middleware, server) => {
    const next = previous ? previous(middleware, server) : middleware;
    return (req, res, done) => {
      if (req.url !== '/__sweetpad_mmkv_result') return next(req, res, done);
      let body = '';
      req.on('data', chunk => { body += chunk; });
      req.on('end', () => {
        fs.appendFileSync(process.env.SWEETPAD_MMKV_REPORTS, JSON.stringify(JSON.parse(body)) + '\n');
        res.end('ok');
      });
    };
  },
};
module.exports = config;
"""


def run(args):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--simulator", required=True)
    parser.add_argument("--out", type=Path, default=CORPUS / "_linux-followups")
    parser.add_argument("--corpus-root", type=Path, default=CORPUS)
    parser.add_argument("--tool-bin", type=Path, default=CORPUS / "_linux-followups/tools/js/node_modules/.bin")
    parser.add_argument("--developer-dir", default="/Applications/Xcode.app/Contents/Developer")
    args = parser.parse_args(args)
    args.out = args.out.resolve()
    evidence = args.out / "validation/mmkv-smoke"
    evidence.mkdir(parents=True, exist_ok=True)
    example = args.corpus_root.resolve() / "react-native-mmkv/example"
    pin = next(p["commit"] for p in json.loads((CI / "linux-projects.json").read_text())
               if p["slug"] == "react-native-mmkv")
    actual = subprocess.check_output(["git", "-C", str(example), "rev-parse", "HEAD"], text=True).strip()
    if actual != pin:
        raise RuntimeError(f"MMKV source pin mismatch: {actual}")
    entry = example / "_sweetpad_mmkv_probe.js"
    config = example / "_sweetpad_mmkv_metro.config.cjs"
    if entry.exists() or config.exists():
        raise RuntimeError("Temporary MMKV probe files already exist")
    try:
        urllib.request.urlopen("http://127.0.0.1:8081/status", timeout=1)
    except OSError:
        pass
    else:
        raise RuntimeError("Port 8081 already has a Metro server")
    reports = evidence / "reports.ndjson"
    reports.write_text("")
    env = dict(os.environ, DEVELOPER_DIR=args.developer_dir, SWEETPAD_MMKV_REPORTS=str(reports),
               PATH=str(args.tool_bin.resolve()) + os.pathsep + os.environ["PATH"])
    records = {}
    server = None
    try:
        entry.write_text(PROBE)
        config.write_text(CONFIG)
        with (evidence / "metro.log").open("w") as log:
            server = subprocess.Popen(["node", "../node_modules/react-native/cli.js", "start",
                "--config", str(config), "--port", "8081", "--host", "127.0.0.1",
                "--no-interactive", "--max-workers", "2"], cwd=example, env=env, stdout=log, stderr=log)
        deadline = time.monotonic() + 60
        while True:
            try:
                urllib.request.urlopen("http://127.0.0.1:8081/status", timeout=1)
                break
            except OSError:
                if server.poll() is not None or time.monotonic() > deadline:
                    raise RuntimeError("Metro did not start; inspect metro.log")
                time.sleep(0.2)

        def sim(*words, check=True):
            return subprocess.run(["xcrun", "simctl", *words], env=env, capture_output=True,
                                  text=True, check=check, timeout=90)

        seen = 0
        for host in ["xcode", "linux"]:
            build = json.loads((args.out / f"react-native-mmkv-{host}.json").read_text())
            if not build.get("ok") or not build.get("data", {}).get("built"):
                raise RuntimeError(f"No successful {host} MMKV build")
            app = (Path(build["data"]["productPath"]) if host == "xcode" else
                   args.out / "linux-products/react-native-mmkv/MmkvExample.app")
            info = plistlib.loads((app / "Info.plist").read_bytes())
            bundle = info["CFBundleIdentifier"]
            binary = app / info["CFBundleExecutable"]
            binaries = [binary, *sorted(app.glob("*.debug.dylib"))]
            def hashes():
                return {p.name: hashlib.sha256(p.read_bytes()).hexdigest() for p in binaries}
            before = hashes()
            sim("uninstall", args.simulator, bundle, check=False)
            sim("install", args.simulator, str(app))
            phases = []
            for expected in [1, 2]:
                sim("launch", "--terminate-running-process", args.simulator, bundle)
                deadline = time.monotonic() + 90
                while True:
                    rows = [json.loads(line) for line in reports.read_text().splitlines()]
                    if len(rows) > seen:
                        report = rows[seen]
                        seen += 1
                        break
                    if server.poll() is not None or time.monotonic() > deadline:
                        raise RuntimeError(f"{host}: no MMKV runtime report; inspect metro.log")
                    time.sleep(0.2)
                phases.append(report)
                if report["phase"] != expected or report["error"] or len(report["checks"]) != 7 or not all(report["checks"].values()):
                    raise RuntimeError(f"{host}: failed MMKV checks: {report}")
                time.sleep(2)
                jobs = sim("spawn", args.simulator, "launchctl", "list").stdout
                if not any((parts := line.split()) and parts[0].isdigit() and bundle in parts[-1]
                           for line in jobs.splitlines()):
                    raise RuntimeError(f"{host}: MMKV exited after the storage checks")
                sim("io", args.simulator, "screenshot", str(evidence / f"{host}-{expected}.png"))
                sim("terminate", args.simulator, bundle)
            after = hashes()
            if before != after:
                raise RuntimeError(f"{host}: prebuilt executable changed")
            records[host] = {"bundle": bundle, "running": True, "phases": phases,
                             "executableSHA256": after, "unchanged": True}
            (evidence / "summary.json").write_text(json.dumps(records, indent=2) + "\n")
            print(f"{host}: 7 storage checks pass twice, including persistence after relaunch", flush=True)
        comparison_path = args.out / "project-checks.json"
        comparisons = json.loads(comparison_path.read_text())
        if comparisons["react-native-mmkv"]["commit"] != pin:
            raise RuntimeError("MMKV comparison belongs to another source revision")
        comparisons["react-native-mmkv"]["runtime"] = records
        comparison_path.write_text(json.dumps(comparisons, indent=2) + "\n")
    finally:
        if server:
            server.terminate()
            try:
                server.wait(timeout=10)
            except subprocess.TimeoutExpired:
                server.kill()
                server.wait()
        entry.unlink(missing_ok=True)
        config.unlink(missing_ok=True)


if __name__ == "__main__":
    run(None)
