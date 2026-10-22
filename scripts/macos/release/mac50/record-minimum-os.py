#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
"""Record the real minimum macOS of a closed bundle, without executing its code."""
import json
from pathlib import Path
import plistlib
import re
from macho_probe import checked_output
import sys

MAGIC = {bytes.fromhex(s) for s in ('cffaedfe', 'cefaedfe', 'feedfacf', 'feedface', 'cafebabe', 'bebafeca')}


def versions(text, expected_architectures=None):
    headers = list(re.finditer(r'(?m)^.* \(architecture ([^)]+)\):[ \t]*$', text))
    slices = [(match.group(1), text[match.end():headers[i + 1].start() if i + 1 < len(headers) else len(text)])
              for i, match in enumerate(headers)] if headers else [('native', text)]
    if expected_architectures is not None:
        observed = [architecture for architecture, _ in slices]
        if headers:
            if len(observed) != len(set(observed)) or set(observed) != set(expected_architectures):
                raise ValueError('vtool architecture coverage mismatch')
        elif len(expected_architectures) != 1:
            raise ValueError('vtool missing universal architecture headers')
    result = []
    for architecture, body in slices:
        found = re.findall(r'\bminos\s+([0-9]+(?:\.[0-9]+){1,2})', body)
        found += re.findall(r'cmd LC_VERSION_MIN_MACOSX\s+cmdsize \d+\s+version ([0-9]+(?:\.[0-9]+){1,2})', body)
        if not found:
            raise ValueError(f'architecture {architecture} has no macOS deployment metadata')
        result.append(max(tuple((list(map(int, v.split('.'))) + [0, 0])[:3]) for v in found))
    return result


def record(app):
    plist_path = app / 'Contents/Info.plist'
    plist = plistlib.loads(plist_path.read_bytes())
    baseline = versions('minos ' + plist['LSMinimumSystemVersion'])[0]
    builds = []
    files_checked = 0
    for path in sorted(app.rglob('*')):
        if path.is_symlink() or not path.is_file():
            continue
        with path.open('rb') as stream:
            if stream.read(4) not in MAGIC:
                continue
        architectures = checked_output('lipo', '-archs', str(path)).split()
        if not architectures or len(architectures) != len(set(architectures)) or not set(architectures) <= {'arm64', 'x86_64'} or 'arm64' not in architectures:
            raise ValueError(f'{path}: unknown architecture coverage')
        output = checked_output('vtool', '-show-build', str(path))
        files_checked += 1
        try:
            minima = versions(output, architectures)
        except ValueError as error:
            raise ValueError(f"{path}: {error}") from error
        for minimum in minima:
            builds.append((minimum, str(path.relative_to(app))))
    if not builds:
        raise ValueError('bundle contains no Mach-O code')
    maximum = max([baseline] + [minimum for minimum, _ in builds])
    declared = '.'.join(map(str, maximum[:2] if maximum[2] == 0 else maximum))
    plist['LSMinimumSystemVersion'] = declared
    plist_path.write_bytes(plistlib.dumps(plist, fmt=plistlib.FMT_XML, sort_keys=False))
    receipt = {'schema': 1, 'minimumSystemVersion': declared, 'machOFiles': files_checked, 'architectureBuildsChecked': len(builds),
               'constrainingFiles': [path for minimum, path in builds if minimum == maximum]}
    (app / 'Contents/Resources/build-minimum-os.json').write_text(json.dumps(receipt, indent=2) + '\n')
    return receipt


if __name__ == '__main__':
    if len(sys.argv) != 2:
        raise SystemExit('usage: record-minimum-os.py <app> (run before signing)')
    print(json.dumps(record(Path(sys.argv[1]))))
