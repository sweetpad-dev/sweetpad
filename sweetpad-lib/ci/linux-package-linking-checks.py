#!/usr/bin/env python3
"""Compare complete dynamic-package exports through nested response files.

The native linker combines objects with -r. The Linux SDK substitutes an archive
named .o. The adapter must load every member, including unused public APIs and
those APIs' autolinked libraries. Raw LLD is the negative control.
"""
import argparse
import json
import os
from pathlib import Path
import shlex
import subprocess
import sys

CI = Path(__file__).resolve().parent
REPO = CI.parent.parent
sys.path.insert(0,str(CI))
import bundle_comparison


def run(command, **kwargs):
    result = subprocess.run(list(map(str,command)),capture_output=True,text=True,timeout=90,**kwargs)
    if result.returncode: raise RuntimeError(result.stderr)
    return result.stdout


def objects(out, compiler, sdk, env=None):
    sources = {'entry':'.text\n.globl _entry\n.p2align 2\n_entry:\nb _used\n',
               'used':'.text\n.globl _used\n.p2align 2\n_used:\nret\n',
               'unused':'.text\n.globl _unused\n.p2align 2\n_unused:\nb _sqlite3_libversion\n.linker_option "-lsqlite3"\n'}
    for name,source in sources.items():
        path=out/(name+'.s');path.write_text(source)
        run([*compiler,'-target','arm64-apple-ios18.0','-isysroot',sdk,'-c',path,'-o',out/(name+'.o')],env=env)


def worker(args):
    bundle = Path.home()/'.swiftpm/swift-sdks/darwin.artifactbundle'
    sdk = bundle/'Developer/Platforms/iPhoneOS.platform/Developer/SDKs/iPhoneOS.sdk'
    raw = bundle/'toolset/bin/ld64.lld'
    args.out.mkdir(parents=True,exist_ok=True)
    args.sdk_version = json.loads((sdk/'SDKSettings.json').read_text())['Version']
    if not (args.out/'entry.o').exists():
        objects(args.out,[Path.home()/'swift/swift-6.4.2-RELEASE/usr/bin/clang'],sdk)
    for name,target in [('ld64.lld',Path.home()/'sweetpad-target/debug/sweetpad'),('sdk-ld64.lld',raw)]:
        link = args.out/name
        link.unlink(missing_ok=True); link.symlink_to(target)
    archive = args.out/'Package module ü.o'
    run(['llvm-ar-18','rcs',archive,args.out/'used.o',args.out/'unused.o'])
    common = ['-dylib','-arch','arm64','-platform_version','ios','18.0',args.sdk_version,
              '-syslibroot',str(sdk),'-dead_strip','-dead_strip_dylibs',str(args.out/'entry.o'),'-lSystem']
    nested = args.out/'package inputs.rsp'
    nested.write_text(shlex.quote(str(archive))+'\n')
    records = {}
    for host,linker in [('raw',raw),('wrapped',args.out/'ld64.lld')]:
        output = args.out/(host+'.dylib');response = args.out/(host+'.rsp')
        response.write_text(shlex.join([*common,'@'+str(nested),'-o',str(output)])+'\n')
        run([linker,'@'+str(response)])
        exports = [line.split()[-1] for line in run(['llvm-nm-18','--defined-only','--extern-only',output]).splitlines() if line.split()]
        records[host] = {'exports':sorted(exports),'machO':bundle_comparison.macho(output)}
    expected = ['_entry','_unused','_used']
    libraries = records['wrapped']['machO']['arm64']['libraries']
    records['passed'] = (records['wrapped']['exports']==expected and '_unused' not in records['raw']['exports'] and
                         any(l['path']=='/usr/lib/libsqlite3.dylib' for l in libraries))
    (args.out/'linux.json').write_text(json.dumps(records,indent=2)+'\n')
    return records


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--out',type=Path,required=True)
    parser.add_argument('--vm',default='sweetpad-linux')
    parser.add_argument('--worker',action='store_true',help=argparse.SUPPRESS)
    parser.add_argument('--sdk-version',default='27.2',help=argparse.SUPPRESS)
    args = parser.parse_args();args.out=args.out.resolve()
    if args.worker: return int(not worker(args)['passed'])
    args.out.mkdir(parents=True,exist_ok=False)
    sdk = Path('/Applications/Xcode.app/Contents/Developer/Platforms/iPhoneOS.platform/Developer/SDKs/iPhoneOS.sdk')
    args.sdk_version = json.loads((sdk/'SDKSettings.json').read_text())['Version']
    env = dict(os.environ,DEVELOPER_DIR='/Applications/Xcode.app/Contents/Developer')
    objects(args.out,['xcrun','clang'],sdk,env)
    combined = args.out/'native-prelinked.o'
    common = ['-arch','arm64','-platform_version','ios','18.0',args.sdk_version,'-syslibroot',sdk]
    run(['xcrun','ld','-r',*common,args.out/'used.o',args.out/'unused.o','-o',combined],env=env)
    native=args.out/'native.dylib'
    run(['xcrun','ld','-dylib',*common,'-dead_strip','-dead_strip_dylibs',args.out/'entry.o',combined,'-lSystem','-o',native],env=env)
    run(['orb','-m',args.vm,'python3',Path(__file__).resolve(),'--worker','--out',args.out,'--sdk-version',args.sdk_version])
    records=json.loads((args.out/'linux.json').read_text())
    exports=sorted(line.split()[-1] for line in run(['xcrun','nm','-gU',native],env=env).splitlines() if line.split())
    metadata=bundle_comparison.macho(native)
    records['native']={'exports':exports,'machO':metadata}
    records['differences']=bundle_comparison.differences(metadata,records['wrapped']['machO'])
    records['passed'] &= exports==records['wrapped']['exports'] and not records['differences']
    (args.out/'summary.json').write_text(json.dumps(records,indent=2)+'\n')
    print('Complete dynamic-package exports and library loads:',records['passed'])
    return int(not records['passed'])


if __name__=='__main__': raise SystemExit(main())
