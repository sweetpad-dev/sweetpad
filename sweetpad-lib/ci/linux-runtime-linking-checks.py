#!/usr/bin/env python3
"""Separate Swift code-generation differences from linker policy.

Both linkers consume the exact same arm64 object. Compare all load-command
versions and weak/required kinds without normalization. A weak force-load marker
alone must not be promoted to a required runtime dependency.
"""
import argparse
import json
import os
from pathlib import Path
import subprocess
import sys

sys.path.insert(0, str(Path(__file__).resolve().parent))
import bundle_comparison


def run(command, **kwargs):
    result = subprocess.run(list(map(str, command)), capture_output=True, text=True, timeout=90, **kwargs)
    if result.returncode: raise RuntimeError(result.stderr)
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--out', type=Path, required=True)
    parser.add_argument('--vm', default='sweetpad-linux')
    parser.add_argument('--developer-dir', type=Path, default=Path('/Applications/Xcode.app/Contents/Developer'))
    args = parser.parse_args()
    args.out.mkdir(parents=True, exist_ok=False)
    args.out = args.out.resolve()
    sdk = args.developer_dir/'Platforms/iPhoneOS.platform/Developer/SDKs/iPhoneOS.sdk'
    env = dict(os.environ, DEVELOPER_DIR=str(args.developer_dir))
    version = json.loads((sdk/'SDKSettings.json').read_text())['Version']
    remote_sdk = run(['orb','-m',args.vm,'sh','-c','printf %s "$HOME/.swiftpm/swift-sdks/darwin.artifactbundle"']).stdout
    cases = [
        ('darwin-implicit', '__swift_FORCE_LOAD_$_swift_Builtin_float', 'swift_Builtin_float', True, []),
        ('darwin-explicit', '__swift_FORCE_LOAD_$_swift_Builtin_float', 'swift_Builtin_float', True, ['-lswiftDarwin']),
        ('objc-weak', '__swift_FORCE_LOAD_$_swiftObjectiveC', 'swiftObjectiveC', True, []),
        ('objc-required', '_$sSo8NSObjectC10ObjectiveCE4hash4intoys6HasherVz_tF', 'swiftObjectiveC', False, []),
        ('core-audio-weak', '__swift_FORCE_LOAD_$_swiftCoreAudio', 'swiftCoreAudio', True, []),
    ]
    records = []
    for minimum in ['15.1','17.6','18.0','27.0']:
        for label, symbol, library, weak, extra in cases:
            stem = args.out/(minimum+'-'+label)
            source = Path(str(stem)+'.s')
            source.write_text('.section __DATA,__const\n.globl _probe\n.p2align 3\n_probe:\n.quad "'+symbol+'"\n'+
                ('.weak_reference "'+symbol+'"\n' if weak else '')+'.linker_option "-l'+library+'"\n')
            obj = Path(str(stem)+'.o')
            run(['xcrun','clang','-target',f'arm64-apple-ios{minimum}','-isysroot',sdk,'-c',source,'-o',obj],env=env)
            common = ['-dylib','-arch','arm64','-platform_version','ios',minimum,version]
            native, linux = Path(str(stem)+'-native.dylib'), Path(str(stem)+'-linux.dylib')
            native_command = ['xcrun','ld',*common,'-syslibroot',sdk,obj,'-L'+str(sdk/'usr/lib/swift'),'-lSystem',*extra,'-o',native]
            remote_platform = remote_sdk+'/Developer/Platforms/iPhoneOS.platform/Developer/SDKs/iPhoneOS.sdk'
            linux_command = ['orb','-m',args.vm,remote_sdk+'/toolset/bin/ld64.lld',*common,'-syslibroot',remote_platform,obj,
                '-L'+remote_platform+'/usr/lib/swift','-lSystem',*extra,'-o',linux]
            run(native_command,env=env); run(linux_command)
            a,b = bundle_comparison.macho(native),bundle_comparison.macho(linux)
            records.append({'minimum':minimum,'case':label,'native':a,'linux':b,'differences':bundle_comparison.differences(a,b),
                            'nativeCommand':list(map(str,native_command)),'linuxCommand':list(map(str,linux_command))})
    (args.out/'summary.json').write_text(json.dumps(records,indent=2)+'\n')
    print(f'{sum(not r["differences"] for r in records)}/{len(records)} identical-input native/Linux links match')
    return int(any(r['differences'] for r in records))


if __name__ == '__main__': raise SystemExit(main())
