#!/usr/bin/env python3
"""Build and run an unedited Linux UI/logic test-plan manifest on a Mac.

Requires Xcode, the existing simulator, and a provisioned OrbStack Linux VM.
Apple SDK products remain in local --out; only JSON reports belong in CI artifacts.
"""
import argparse
import hashlib
import importlib.util
import json
from pathlib import Path
import subprocess

CI = Path(__file__).resolve().parent
REPO = CI.parent.parent
spec = importlib.util.spec_from_file_location('parity', CI / 'linux-parity.py')
parity = importlib.util.module_from_spec(spec)
spec.loader.exec_module(parity)


def hashes(products):
    """Hash app, runner, and test executables before and after test-without-building."""
    result = {}
    for info in products.rglob('Info.plist'):
        try:
            name = parity.checks.plist(info).get('CFBundleExecutable')
        except (ValueError, RuntimeError):
            continue
        executable = info.parent / name if name else None
        if executable and executable.is_file():
            result[str(executable.relative_to(products))] = hashlib.sha256(executable.read_bytes()).hexdigest()
    return result


def manifest_options(manifest):
    result = {}
    keys = ['BlueprintName', 'ProductModuleName', 'TestBundlePath', 'TestHostPath',
            'TestHostBundleIdentifier', 'IsUITestBundle', 'IsXCTRunnerHostedTestBundle',
            'UITargetAppPath', 'UITargetAppCommandLineArguments', 'CommandLineArguments',
            'TestLanguage', 'TestRegion', 'OnlyTestIdentifiers', 'SkipTestIdentifiers']
    for configuration in manifest['TestConfigurations']:
        targets = {}
        for target in configuration['TestTargets']:
            options = {key: target[key] for key in keys if key in target}
            options['EnvironmentVariables'] = {k: v for k, v in target['EnvironmentVariables'].items() if k.startswith('PLAN_')}
            targets[target['BlueprintName']] = options
        result[configuration['Name']] = targets
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--out', type=Path, required=True)
    parser.add_argument('--simulator', required=True)
    parser.add_argument('--vm', default='sweetpad-linux')
    parser.add_argument('--linux-dd', default='sweetpad-linux-clean/test-plan-checks')
    args = parser.parse_args()
    args.out = args.out.resolve()
    args.out.mkdir(parents=True, exist_ok=False)
    project = CI / 'fixture-test-plans/TestPlanProbe.xcodeproj'
    common = ['test', 'build', '--project', str(project), '--scheme', 'TestPlanProbe',
              '--destination', 'generic/platform=iOS Simulator', '--json', '--non-interactive']
    flags = ['ARCHS=arm64', 'ONLY_ACTIVE_ARCH=YES', 'CODE_SIGNING_ALLOWED=NO']
    summary = {'builds': {}, 'tests': {}}
    manifests = {}
    for host in ['native', 'linux']:
        if host == 'native':
            command = [str(REPO / 'target/debug/sweetpad'), *common, '--', '-derivedDataPath', str(args.out / 'native-dd'), *flags]
        else:
            script = '''export PATH="$HOME/bin:$HOME/swift/swift-6.4.2-RELEASE/usr/bin:$PATH"
exec "$HOME/sweetpad-target/debug/sweetpad" "$@"
'''
            home = subprocess.check_output(['orb', '-m', args.vm, 'sh', '-c', 'printf %s "$HOME"'], text=True)
            remote_dd = Path(home) / args.linux_dd
            command = ['orb', '-m', args.vm, 'bash', '-lc', script, '--', *common, '--', '-derivedDataPath', str(remote_dd), *flags]
        build = parity.invoke(command, args.out / (host + '-build'))
        summary['builds'][host] = build
        if build['exitCode'] or not build['result']['data']['built']:
            raise RuntimeError(f'{host} test build failed; see {args.out}')
        if host == 'native':
            products = args.out / 'native-dd/Build/Products'
            manifest = next(p for p in products.glob('*.xctestrun') if parity.checks.plist(p).get('TestPlan', {}).get('Name') == 'Probe')
        else:
            products = parity.checks.copy_product(remote_dd / 'Build/Products', args.out / 'linux-export', args.vm)
            manifest = products / Path(build['result']['data']['xctestrunPath']).name
        manifests[host] = manifest_options(parity.checks.plist(manifest))
        before = hashes(products)
        command = [str(REPO / 'target/debug/sweetpad'), 'test', 'run', '--project', str(project),
                   '--scheme', 'TestPlanProbe', '--xctestrun', str(manifest), '--on', args.simulator,
                   '--result-bundle', str(args.out / (host + '.xcresult')), '--json', '--non-interactive']
        tests = parity.invoke(command, args.out / (host + '-tests'), timeout=600)
        tests['executableHashes'] = before
        tests['executablesUnchanged'] = bool(before) and hashes(products) == before
        result = subprocess.check_output(['xcrun', 'xcresulttool', 'get', 'test-results', 'summary', '--path', str(args.out / (host + '.xcresult'))], text=True)
        result = json.loads(result)
        parity.save(args.out / (host + '-xcresult-summary.json'), result)
        tests['configurations'] = {c['testPlanConfiguration']['configurationName']: {'passed': c['passedTests'], 'failed': c['failedTests']} for c in result['devicesAndConfigurations']}
        tests['passed'] = tests['exitCode'] == 0 and tests['result']['data']['passed'] and tests['executablesUnchanged'] and tests['configurations'] == {'French': {'passed': 2, 'failed': 0}, 'English': {'passed': 2, 'failed': 0}}
        summary['tests'][host] = tests
        parity.save(args.out / (host + '-tests.json'), tests)
        print(host, tests['configurations'], 'unchanged executables:', tests['executablesUnchanged'], flush=True)
    summary['manifestOptionsMatch'] = manifests['native'] == manifests['linux']
    summary['manifestOptions'] = manifests
    summary['passed'] = summary['manifestOptionsMatch'] and all(r['passed'] for r in summary['tests'].values())
    parity.save(args.out / 'summary.json', summary)
    return int(not summary['passed'])


if __name__ == '__main__':
    raise SystemExit(main())
