#!/usr/bin/env python3
"""Compare property mutations from the iOS corpus with Xcode's compiler.

One representative occurrence per element/attribute pair is changed. Boolean,
numeric, text, and observed enum alternatives are covered. This is a finite
probe set, not proof of every value or combination supported by Interface Builder.
"""
import argparse
from collections import Counter, defaultdict
from concurrent.futures import ThreadPoolExecutor
from datetime import datetime, timezone
import hashlib
import json
import os
import plistlib
from pathlib import Path
import re
import shutil
import subprocess
import xml.etree.ElementTree as ET

import sweep

REPO = sweep.REPO
SKIP = {'id', 'key', 'firstItem', 'secondItem', 'destination', 'reference',
        'sceneID', 'sceneMemberID', 'tableViewCell', 'initialViewController',
        'placeholderIdentifier', 'property', 'selector', 'version', 'toolsVersion',
        'systemVersion', 'targetRuntime', 'keyPath', 'relationship',
        'customModuleProvider', 'name', 'catalog', 'systemColor', 'customColorSpace',
        'image', 'identifier', 'reuseIdentifier', 'storyboardIdentifier'}
DESIGN = {'document', 'dependencies', 'deployment', 'plugIn', 'capability',
          'device', 'point', 'simulatedScreenMetrics', 'simulatedOrientationMetrics',
          'simulatedStatusBarMetrics', 'simulatedToolbarMetrics', 'simulatedNavigationBarMetrics',
          'simulatedTabBarMetrics', 'nil', 'rect' }
TEXT = {'text', 'title', 'placeholder', 'prompt', 'customClass', 'customModule',
        'accessibilityLabel', 'accessibilityHint', 'restorationIdentifier'}


def alternative(attribute, original, values):
    if original in {'YES', 'NO'}:
        return 'NO' if original == 'YES' else 'YES'
    if attribute in TEXT:
        return original + 'Audit' if attribute.startswith('custom') else original + ' audit'
    others = sorted(value for value in values if value != original)
    if others:
        return others[0]
    if re.fullmatch(r'-?\d+(\.\d+)?', original):
        if attribute == 'priority':
            return '750' if original != '750' else '500'
        number = float(original)
        return str(number + 1 if abs(number) >= 1 else 0.5 if number == 0 else 0)
    return None


def candidates(corpus):
    values = defaultdict(Counter)
    examples = {}
    for path, document in sweep.inputs(corpus, 'iOS.CocoaTouch'):
        for index, element in enumerate(document.iter()):
            for attribute, value in element.attrib.items():
                if element.tag in DESIGN or attribute in SKIP:
                    continue
                key = element.tag, attribute
                values[key][value] += 1
                examples.setdefault(key, (path, index, value))
    result = []
    for (element, attribute), counts in sorted(values.items(), key=lambda item: -sum(item[1].values())):
        path, index, original = examples[element, attribute]
        value = alternative(attribute, original, counts)
        if value is not None:
            result.append({'path': str(path.relative_to(corpus)), 'element': element,
                           'attribute': attribute, 'original': original, 'mutated': value,
                           'occurrences': sum(counts.values()), 'element_index': index})
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--corpus', type=Path, default=REPO / 'sweetpad-lib/corpus')
    parser.add_argument('--binary', type=Path, default=REPO / 'target/debug/sweetpad')
    parser.add_argument('--linux-vm')
    parser.add_argument('--max-cases', type=int)
    parser.add_argument('--out', required=True, type=Path)
    args = parser.parse_args()
    corpus, out = args.corpus.resolve(), args.out.resolve()
    out.mkdir(parents=True, exist_ok=False)
    binary = args.binary if args.linux_vm else args.binary.resolve()
    assert binary.is_absolute(), binary
    prefix = ['orb', '-m', args.linux_vm] if args.linux_vm else []
    (out / 'ibtool').symlink_to(binary)
    comparison, dump = out / 'comparison', out / 'canonical'
    for folder in [comparison, comparison / 'expected', comparison / 'actual', dump]:
        folder.mkdir(exist_ok=True)
    rows = candidates(corpus)
    if args.max_cases:
        rows = rows[:args.max_cases]

    def compile_case(item):
        row = dict(item)
        identity = json.dumps(row, sort_keys=True)
        name = hashlib.sha256(identity.encode()).hexdigest()[:12] + '_' + row['element'] + '_' + row['attribute']
        row['name'] = name
        case = out / 'cases' / name
        source = corpus / row['path']
        folder = case / ('Base.lproj' if source.parent.name == 'Base.lproj' else 'input')
        folder.mkdir(parents=True)
        mutated = folder / source.name
        document = ET.parse(source)
        element = list(document.iter())[row['element_index']]
        assert element.tag == row['element'] and element.get(row['attribute']) == row['original']
        element.set(row['attribute'], row['mutated'])
        document.write(mutated, encoding='utf-8', xml_declaration=True)
        suffix = '.storyboardc' if source.suffix == '.storyboard' else '.nib'
        flags = ['--module', 'App', '--target-device', 'iphone', '--target-device', 'ipad',
                 '--minimum-deployment-target', '18.0', '--output-format', 'human-readable-text']
        for side, tool in [('ours', prefix + [str(out / 'ibtool')]), ('xcode', ['xcrun', 'ibtool'])]:
            product = case / (side + suffix)
            row[side] = sweep.run([*tool, *flags, '--compile', str(product), str(mutated)], case / (side + '-log'))
        row['structural_match'] = None
        if row['ours']['exit_code'] == row['xcode']['exit_code'] == 0:
            shutil.copyfile(mutated, comparison / (name + source.suffix))
            copy = shutil.copytree if suffix == '.storyboardc' else shutil.copyfile
            copy(case / ('xcode' + suffix), comparison / 'expected' / (name + suffix))
            copy(case / ('ours' + suffix), comparison / 'actual' / (name + suffix))
        print(row['element'] + '.' + row['attribute'], row['original'], '->', row['mutated'],
              'ours=' + str(row['ours']['exit_code']), 'xcode=' + str(row['xcode']['exit_code']), flush=True)
        return row

    with ThreadPoolExecutor(max_workers=2) as pool:
        rows = list(pool.map(compile_case, rows))
    env = dict(os.environ, IBTOOL_FIXTURES=str(comparison), IBTOOL_ACTUAL=str(comparison / 'actual'), IBTOOL_DUMP=str(dump))
    harness = sweep.run(['cargo', 'test', '-p', 'sweetpad-cli', '--lib',
                         'cli::ibtool::tests::each_document_compiles_as_xcode_compiles_it',
                         '--', '--exact', '--nocapture'], out / 'comparison-log', env)
    for row in rows:
        if row['ours']['exit_code'] or row['xcode']['exit_code']:
            continue
        files = sorted(dump.glob(row['name'] + '-*.got'))
        differences = [file.name for file in files if file.read_bytes() != file.with_suffix('.want').read_bytes()]
        suffix = '.storyboardc' if Path(row['path']).suffix == '.storyboard' else '.nib'
        ours, native = comparison / 'actual' / (row['name'] + suffix), comparison / 'expected' / (row['name'] + suffix)
        if suffix == '.storyboardc':
            if sorted(p.name for p in ours.iterdir()) != sorted(p.name for p in native.iterdir()):
                differences.append('compiled filenames')
            else:
                for file in ours.glob('*.plist'):
                    if plistlib.loads(file.read_bytes()) != plistlib.loads((native / file.name).read_bytes()):
                        differences.append(file.name)
        row['differences'] = differences
        row['structural_match'] = bool(files) and not differences
    summary = {'total': len(rows), 'native_accepted': sum(r['xcode']['exit_code'] == 0 for r in rows),
               'matching_acceptance': sum((r['ours']['exit_code'] == 0) == (r['xcode']['exit_code'] == 0) for r in rows),
               'structural_matches': sum(r['structural_match'] is True for r in rows)}
    report = {'recorded_at': datetime.now(timezone.utc).isoformat(), 'summary': summary,
              'scope': 'one finite mutation per selected element/attribute pair; excludes geometry and design metadata',
              'corpus': str(corpus), 'binary': str(binary), 'linux_vm': args.linux_vm,
              'comparison_harness': harness, 'cases': rows}
    (out / 'report.json').write_text(json.dumps(report, indent=2) + '\n')
    print(json.dumps(summary, indent=2))
    return 0 if summary['structural_matches'] == summary['native_accepted'] and summary['matching_acceptance'] == len(rows) else 1


if __name__ == '__main__':
    raise SystemExit(main())
