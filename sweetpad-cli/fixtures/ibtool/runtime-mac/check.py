#!/usr/bin/env python3
"""Build an AppKit checker, optionally replace its NIBs with Linux output, and run.

The executable is built on macOS. --linux-vm validates Linux's resource compiler
by using its NIBs in that same executable; this does not claim a Linux app build.
"""
import argparse
from datetime import datetime, timezone
import json
from pathlib import Path
import shlex
import shutil
import subprocess

HERE = Path(__file__).resolve().parent
REPO = HERE.parents[3]


def run(command, log, timeout=120):
    result = subprocess.run(command, capture_output=True, text=True, timeout=timeout)
    log.with_suffix('.stdout').write_text(result.stdout)
    log.with_suffix('.stderr').write_text(result.stderr)
    log.with_suffix('.command.json').write_text(json.dumps(command, indent=2) + '\n')
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--linux-vm')
    parser.add_argument('--standin', action='store_true')
    parser.add_argument('--reference', type=Path, help='Require checks and observations to match this native report')
    parser.add_argument('--out', required=True, type=Path)
    args = parser.parse_args()
    out = args.out.resolve()
    out.mkdir(parents=True, exist_ok=False)
    subprocess.run(['xcodegen', 'generate', '--spec', str(HERE / 'project.yml')], check=True)
    command = [str(REPO / 'target/debug/sweetpad'), '-C', str(HERE), 'build', '--scheme', 'IBMacAudit', '--on', 'mac', '--non-interactive', '-o', 'json', '--', '-derivedDataPath', str(out / 'derived-data'), 'CODE_SIGNING_ALLOWED=NO', '-resultBundlePath', str(out / 'build.xcresult')]
    build = run(command, out / 'build', timeout=1200)
    if build.returncode:
        print(build.stderr)
        return build.returncode
    envelope = json.loads(build.stdout)
    assert envelope['ok'] and envelope['data']['built'], envelope
    product = Path(envelope['data']['productPath'])
    assert product.is_absolute() and product.name == 'IBMacAudit.app' and product.is_dir(), product
    if args.linux_vm or args.standin:
        staged = out / product.name
        shutil.copytree(product, staged)
        product = staged
        binary = Path('/home/hyzyla_home/sweetpad-target/debug/sweetpad') if args.linux_vm else REPO / 'target/debug/sweetpad'
        tool = out / 'ibtool'
        tool.symlink_to(binary)
        documents = [(name, ".xib") for name in ['MacLayoutAdaptive', 'MacRecessedPopup', 'MacAccessibility', 'MacConstraintPrecision', 'MacBindingNoOptions', 'MacCell', 'MacOutlineView', 'MacOutlineSidebar', 'MacOutlineOptions', 'MacRecessedButton', 'MacCollectionView', 'MacCollectionOptions', 'MacLegacyCollection', 'MacCollectionDefaults', 'MacSeparator', 'MacTablePrototype', 'MacControls', 'MacCheckButtons', 'MacControlPriorities', 'MacTableView', 'MacTableViewDefaultSpacing', 'MacTableColumns', 'MacTableOverflow', 'MacTableMinimum', 'MacTableFixedColumn', 'MacTableSmallColumn', 'MacScrollerPriorities', 'MacControlVariants', 'MacWindowVariants', 'MacBindings', 'MacPopup', 'MacPopupVariants', 'MacImages', 'MacGrid', 'MacGridDefaults', 'MacGridVariants', 'MacSecureField', 'MacWebView', 'MacWebViewVariants', 'MacSystemRed', 'MacConstraintVariants', 'MacPlaceholderConstraint', 'MacMenus', 'MacMenuVariants', 'MacDefaultPriorities', 'MacCustomPriorities', 'MacViewProperties', 'MacScrollView', 'MacScrollViewVariants', 'MacScrollZoomIn', 'MacScrollZoomOut', 'MacTextView', 'MacTextViewVariants', 'MacTextViewClearBackground', 'MacTextViewCalibratedColor', 'MacDefaultsShared', 'MacDefaultsPrivate', 'MacDefaultsDeferred', 'MacStack', 'MacStackViews', 'MacStackDefaults', 'MacStackMetadata', 'MacCustomField', 'MacFirstResponder', 'MacStringValues', 'MacFont-subheadline', 'MacFont-headline', 'MacFont-callout', 'MacFont-title3', 'MacFont-custom', 'MacNamedFonts', 'MacLayoutSheet', 'MacLayoutProgress', 'MacLayoutMultiline', 'MacProgressRanges', 'MacGridMixed', 'MacGridIntrinsic', 'MacTextFieldVariants', 'MacTextColorEditable', 'MacTextColorLabel', 'Base.lproj/MacLocalizedControls']]
        documents.extend((name, ".storyboard") for name in ["MacSceneTable", "MacMenuStoryboard", "MacViewController", "MacUnnamedController", "MacConnectedController", "MacMenuAndController", "MacWindowController", "MacCustomWindowController", "MacWindowRelationship", "MacCustomWindowRelationship", "MacSceneObjects", "MacSceneLayout", "MacShowSegue", "MacUnnamedWindowContent", "MacSplitController", "MacEmbedSplit"])
        for name, extension in documents:
            command = [str(tool), '--module', 'App', '--minimum-deployment-target', '12', '--compile', str(product / 'Contents/Resources' / (name + ('.storyboardc' if extension == '.storyboard' else '.nib'))), str(HERE.parent / (name + extension))]
            if args.linux_vm:
                command = ['orb', '-m', args.linux_vm, 'bash', '-lc', 'exec ' + shlex.join(command)]
            result = run(command, out / ('compile-' + name.replace('/', '-')))
            assert result.returncode == 0, (name, result.stdout, result.stderr)
    result = run([str(product / 'Contents/MacOS/IBMacAudit')], out / 'runtime', timeout=60)
    checks = [line for line in result.stdout.splitlines() if line.startswith('IBMACAUDIT ')]
    passed = result.returncode == 0 and 'IBMACAUDIT done' in checks and not any(line.endswith('=false') for line in checks)
    observations = [line for line in result.stdout.splitlines() if line.startswith('IBMACOBSERVE ')]
    report = {'recorded_at': datetime.now(timezone.utc).isoformat(), 'resource_compiler': 'linux' if args.linux_vm else 'standin' if args.standin else 'xcode', 'product': str(product), 'checks': checks, 'observations': observations, 'passed': passed}
    if args.reference:
        reference = json.loads(args.reference.read_text())
        matches = reference['passed'] and checks == reference['checks'] and observations == reference.get('observations', [])
        report.update(reference=str(args.reference.resolve()), matches_reference=matches)
        passed = report['passed'] = passed and matches
    (out / 'report.json').write_text(json.dumps(report, indent=2) + '\n')
    print(passed, len(checks) - 1)
    return 0 if passed else 1


if __name__ == '__main__':
    raise SystemExit(main())
