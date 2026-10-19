#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
"""Record the real minimum macOS of a closed bundle, without executing its code."""
import json
from pathlib import Path
import plistlib
import re
import subprocess
import sys

MAGIC = {bytes.fromhex(s) for s in ('cffaedfe', 'cefaedfe', 'feedfacf', 'feedface', 'cafebabe', 'bebafeca')}


def versions(text):
    found = re.findall(r'\bminos\s+([0-9]+(?:\.[0-9]+){1,2})', text)
    found += re.findall(r'cmd LC_VERSION_MIN_MACOSX\s+cmdsize \d+\s+version ([0-9]+(?:\.[0-9]+){1,2})', text)
    if not found:
        raise ValueError('Mach-O has no macOS deployment metadata')
    return [tuple((list(map(int, v.split('.'))) + [0, 0])[:3]) for v in found]


def record(app):
    plist_path = app / 'Contents/Info.plist'
    plist = plistlib.loads(plist_path.read_bytes())
    baseline = versions('minos ' + plist['LSMinimumSystemVersion'])[0]
    builds = []
    for path in sorted(app.rglob('*')):
        if path.is_symlink() or not path.is_file():
            continue
        with path.open('rb') as stream:
            if stream.read(4) not in MAGIC:
                continue
        result = subprocess.run(['vtool', '-show-build', str(path)], capture_output=True, text=True, check=True)
        for minimum in versions(result.stdout):
            builds.append((minimum, str(path.relative_to(app))))
    if not builds:
        raise ValueError('bundle contains no Mach-O code')
    maximum = max([baseline] + [minimum for minimum, _ in builds])
    declared = '.'.join(map(str, maximum[:2] if maximum[2] == 0 else maximum))
    plist['LSMinimumSystemVersion'] = declared
    plist_path.write_bytes(plistlib.dumps(plist, fmt=plistlib.FMT_XML, sort_keys=False))
    receipt = {'schema': 1, 'minimumSystemVersion': declared, 'architectureBuildsChecked': len(builds),
               'constrainingFiles': [path for minimum, path in builds if minimum == maximum]}
    (app / 'Contents/Resources/build-minimum-os.json').write_text(json.dumps(receipt, indent=2) + '\n')
    return receipt


if __name__ == '__main__':
    if len(sys.argv) != 2:
        raise SystemExit('usage: record-minimum-os.py <app> (run before signing)')
    print(json.dumps(record(Path(sys.argv[1]))))
