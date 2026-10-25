# SPDX-License-Identifier: GPL-3.0-or-later
#!/usr/bin/env python3
"""Root-only build/assembly recipe. Never activates or signs an extension.

Run only in a disposable candidate bundle BEFORE Root signs the extension and
host using their separately qualified profiles and entitlements.
"""
import argparse
import plistlib
import re
import subprocess
from pathlib import Path

p = argparse.ArgumentParser()
p.add_argument('--source-root', type=Path, required=True)
p.add_argument('--candidate-app', type=Path, required=True)
p.add_argument('--team-id', required=True)
p.add_argument('--host-requirement-file', type=Path, required=True)
p.add_argument('--signing-input-dir', type=Path, required=True)
p.add_argument('--version', required=True)
p.add_argument('--build', required=True)
a = p.parse_args()
if not re.fullmatch(r'[A-Z0-9]{10}', a.team_id):
    p.error('Use the qualified signing team ID.')
requirement = a.host_requirement_file.read_text().strip()
if not requirement or 'REPLACE_' in requirement or a.team_id not in requirement or 'com.waypair.mac' not in requirement:
    p.error('Supply the exact qualified host designated requirement including bundle and team.')
contents = a.candidate_app / 'Contents'
if not (contents / 'Info.plist').is_file():
    p.error('Candidate .app must already exist.')
with (contents / 'Info.plist').open('rb') as f:
    host = plistlib.load(f)
if host.get('CFBundleIdentifier') != 'com.waypair.mac':
    p.error('Refusing a candidate with another host bundle ID.')
bundle = contents / 'Library/SystemExtensions/com.waypair.mac.CameraExtension.systemextension'
if bundle.exists():
    p.error('Refusing to overwrite an extension payload.')
exe = bundle / 'Contents/MacOS/WaypairCameraExtension'
exe.parent.mkdir(parents=True)
subprocess.run(['xcrun', 'clang', '-fobjc-arc', '-O2', '-mmacosx-version-min=12.3',
    str(a.source_root / 'desktop/app/src/mac/camera_extension/WaypairCameraExtension.m'),
    '-framework', 'Foundation', '-framework', 'CoreMediaIO', '-framework', 'CoreMedia',
    '-framework', 'CoreVideo', '-framework', 'Security', '-o', str(exe)], check=True)
info = dict(CFBundleIdentifier='com.waypair.mac.CameraExtension',
    CFBundleExecutable=exe.name, CFBundleName='Waypair Camera', CFBundlePackageType='SYSX',
    CFBundleShortVersionString=a.version, CFBundleVersion=a.build,
    LSMinimumSystemVersion='12.3', WaypairProducerRequirement=requirement,
    CMIOExtension={'CMIOExtensionMachServiceName': a.team_id + '.com.waypair.mac.CameraExtension'},
    NSSystemExtensionUsageDescription='Use your approved phone camera in video apps.')
with (bundle / 'Contents/Info.plist').open('wb') as f:
    plistlib.dump(info, f)
host['NSSystemExtensionUsageDescription'] = 'Use your approved phone camera in video apps.'
with (contents / 'Info.plist').open('wb') as f:
    plistlib.dump(host, f)
# These are signing INPUTS; this recipe does not claim granted entitlements.
a.signing_input_dir.mkdir(parents=True, exist_ok=True)
with (a.signing_input_dir / 'camera-extension-signing-input.plist').open('wb') as f:
    plistlib.dump({'com.apple.security.app-sandbox': True,
                   'com.apple.developer.cmio.extension': True}, f)
with (a.signing_input_dir / 'camera-host-entitlement-addition.plist').open('wb') as f:
    plistlib.dump({'com.apple.developer.system-extension.install': True}, f)
print('UNSIGNED native camera payload assembled. Root must merge host entitlements, provide qualified profiles, sign extension then host, and qualify before activation.')
