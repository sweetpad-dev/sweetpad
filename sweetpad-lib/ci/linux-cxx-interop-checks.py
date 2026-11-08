#!/usr/bin/env python3
"""Run the reduced MMKV generic DEBUG-thunk compiler comparison.

This intentionally returns failure while the pinned Linux compiler still
recursively instantiates used generic member functions. No SDK or MMKV checkout
is required for this reproducer. It does not modify the fixture workaround.
"""
import argparse
import json
from pathlib import Path
import subprocess

REPO = Path(__file__).resolve().parents[2]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--out', type=Path, required=True)
    parser.add_argument('--vm', default='sweetpad-linux')
    args = parser.parse_args()
    args.out.mkdir(parents=True, exist_ok=False)
    source = REPO/'sweetpad-cli/fixtures/cxx/used-template.cpp'
    flags = ['-target','arm64-apple-ios15.1-simulator','-std=c++20','-fsyntax-only',
             '-ftemplate-backtrace-limit=2',str(source)]
    compilers = {
        'native':['/Applications/Xcode.app/Contents/Developer/Toolchains/XcodeDefault.xctoolchain/usr/bin/clang++'],
        'linux':['orb','-m',args.vm,'sh','-c','exec "$HOME/swift/swift-6.4.2-RELEASE/usr/bin/clang++" "$@"','--'],
    }
    summary = {}
    for host, command in compilers.items():
        version = subprocess.run([*command,'--version'],capture_output=True,text=True,timeout=30)
        result = subprocess.run([*command,*flags],capture_output=True,text=True,timeout=60)
        (args.out/(host+'.stderr')).write_text(result.stderr)
        summary[host] = {'exitCode':result.returncode,'compiler':version.stdout.splitlines()[0],
                         'command':[*command,*flags]}
    summary['passed'] = all(summary[h]['exitCode'] == 0 for h in compilers)
    (args.out/'summary.json').write_text(json.dumps(summary,indent=2)+'\n')
    print(json.dumps(summary,indent=2))
    return int(not summary['passed'])


if __name__ == '__main__': raise SystemExit(main())
