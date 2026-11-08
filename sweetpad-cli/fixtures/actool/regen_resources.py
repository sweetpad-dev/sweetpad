#!/usr/bin/env python3
"""Capture native sticker and Icon Composer output contracts on a Mac."""
import json
from pathlib import Path
import plistlib
import shutil
import subprocess
import tempfile

HERE = Path(__file__).resolve().parent


def capture(source, icon, extra):
    with tempfile.TemporaryDirectory(prefix='sweetpad-native-assets-') as tmp:
        out = Path(tmp)
        subprocess.run(['xcrun', 'actool', str(HERE / 'resources' / source), '--compile', tmp,
                        '--platform', 'iphonesimulator', '--minimum-deployment-target', '18.0',
                        '--target-device', 'iphone', '--target-device', 'ipad', '--app-icon', icon,
                        '--bundle-identifier', 'dev.sweetpad.Fixture',
                        '--output-partial-info-plist', str(out / 'partial.plist'), *extra],
                       check=True, stdout=subprocess.DEVNULL)
        result = {'partialInfo': plistlib.loads((out / 'partial.plist').read_bytes()),
                  'files': sorted(str(p.relative_to(out)) for p in out.rglob('*') if p.is_file())}
        if icon == 'MessageIcon':
            result['packInfo'] = plistlib.loads((out / 'Ordered Pack.stickerpack/Info.plist').read_bytes())
        else:
            shutil.copy(out / (icon + '60x60@2x.png'), HERE / ('expected/' + ('modern' if icon == 'Simple' else icon.lower()) + '-light.png'))
            assets = json.loads(subprocess.check_output(['xcrun', 'assetutil', '--info', str(out / 'Assets.car')]))
            appearances = {'UIAppearanceDark': 1, 'ISAppearanceTintable': 10}
            idioms = {'phone': 1, 'pad': 2}
            result['staticIconRenditions'] = sorted([
                [idioms[a['Idiom']], appearances.get(a.get('Appearance'), 0), a['PixelWidth'], a['PixelHeight']]
                for a in assets if a.get('AssetType') == 'Icon Image' and a.get('Name') == icon])
        return result


def main():
    for name, source, icon, extra in [
        ('stickers', 'Stickers.xcassets', 'MessageIcon', ['--include-sticker-content', '--stickers-icon-role', 'extension',
                                                       '--sticker-pack-identifier-prefix', 'dev.sweetpad.Fixture.sticker-pack']),
        ('modern', 'Simple.icon', 'Simple', [str(HERE / 'resources/Shared.xcassets')]),
        ('system', 'System.icon', 'System', []),
    ]:
        result = capture(source, icon, extra)
        (HERE / 'expected' / (name + '.json')).write_text(json.dumps(result, indent=2) + '\n')


if __name__ == '__main__':
    main()
