# SOURCE-ONLY successor; authored, NOT executed by worker.
# Consult BUILD-RECIPE-ADDENDUM.md and INVENTORY.json before Root execution; no installer here.
"""Mac121 DMG + existing-key Ed signature (Root-bound source, local.26).

Successor to READONLY next-mac47-8ba-operator/dmg-mac47.py.
Writes ONLY to mac121-candidate/R1 under Root-selected output directory.
Uses the retained Root-selected update signer through MAC121_UPDATE_SIGNER
sign branch only (loads stored key, refuses mismatch); never init/rotate.
No installation/publication; bundle manifest must be unchanged.
"""
from pathlib import Path
import datetime
import hashlib
import json
import plistlib
import subprocess
import os

from mac121_guard import validate_inputs, OUTPUT, SOURCE, TREE, PINS, bundle_manifest
pins = validate_inputs()
out = OUTPUT / 'mac121-candidate/R1'
build = json.loads((out / 'BUILD-PACKAGE-RESULT.json').read_text())
assert build['status'] == 'SIGNED_NORMAL_CANDIDATE_BUILT'
assert build['source'] == SOURCE
assert build['build'] == 121 and build['channel'] == '0.2.2'
app = Path(build['bundle'])
assert app == out / 'normal-mac121.noindex/Waypair Link.app'
review = json.loads((OUTPUT / 'mac121-review/R1/REVIEW.json').read_text())
assert review['verdict'] == 'STATIC_GO_APP_ONLY' and not review['failures']
assert review['source'] == build['source'] and review['build'] == 121
assert review['tree'] == build['tree'] == TREE
assert review['channel'] == build['channel'] == '0.2.2'
assert review['finalTargetCount'] == 190 and review['allTargetsExactPolicy'] is True
assert review['bundleManifestSha256'] == build['bundleManifestSha256']
assert bundle_manifest(app) == json.loads((out / 'SIGNED-BUNDLE-MANIFEST.json').read_text())
manifest = out / 'SIGNED-BUNDLE-MANIFEST.json'
before = hashlib.sha256(manifest.read_bytes()).hexdigest()
assert before == build['bundleManifestSha256']
assert before not in ('97b3c1978c5aca1c14fb73dd6c468dd7d470f596087df1c3474d3f991666faf0', 'f00f22025ebfe8c70a09f10565067cb1a17084dbfb4e274a49c0b5a8272eca2a')
dmg = out / 'Waypair-Link-0.2.2.dmg'
assert not dmg.exists() and not dmg.is_symlink()
for name in ('ED-SIGNATURE.json', 'DMG-RESULT.json', 'DMG.log'):
    assert not (out / name).exists() and not (out / name).is_symlink(), 'Prior DMG attempt: HOLD'
signer_bin = Path(os.environ['MAC121_UPDATE_SIGNER']).resolve(strict=True)
assert signer_bin.is_file(), 'update signer unavailable; stop exact'
commands = [['hdiutil', 'create', '-volname', 'Waypair Link', '-srcfolder', str(app.parent), '-format', 'UDZO', str(dmg)], ['hdiutil', 'verify', str(dmg)]]
with (out / 'DMG.log').open('x') as log:
    for command in commands:
        subprocess.run(command, stdout=log, stderr=subprocess.STDOUT, check=True, timeout=600)
p = subprocess.run([str(signer_bin), 'sign', str(dmg)], capture_output=True, text=True, check=True)
signature = json.loads(p.stdout)
expected = plistlib.loads((app / 'Contents/Info.plist').read_bytes())['SUPublicEDKey']
assert expected == 'uMzztqVF0M7BMYkWsL48ZJDvSxrqoVNSvqBKxYdu8ss='
assert signature['publicKey'] == expected
assert signature['sha256'] == hashlib.sha256(dmg.read_bytes()).hexdigest()
assert signature['sizeBytes'] == dmg.stat().st_size, 'Ed signature must bind exact51 DMG bytes'
(out / 'ED-SIGNATURE.json').write_text(json.dumps(signature, indent=2) + '\n')
assert hashlib.sha256(manifest.read_bytes()).hexdigest() == before
assert bundle_manifest(app) == json.loads(manifest.read_text())
record = {'at': datetime.datetime.now(datetime.timezone.utc).isoformat(), 'source': build['source'], 'tree': TREE, 'build': 121, 'channel': '0.2.2', 'bundleManifestSha256': before, 'path': str(dmg), 'sha256': hashlib.sha256(dmg.read_bytes()).hexdigest(), 'bytes': dmg.stat().st_size, 'hdiutilVerify': True, 'existingEdPublicKeyMatched': True, 'noKeyCreationOrRotation': True, 'bundleManifestUnchanged': True, 'normalInstallationOrPublication': False}
assert record['sha256'] not in ('d5b40abd3b4c6de113e239452e1c8742c2b2cac2a939db9a8b8161cc70a5536f', '719c45312882ebd2daaf5645e94855dd035a8416a76877d8b2f1cbd7c82a6161'), 'DMG reused frozen 46/47'
assert before != pins['baseline48']['manifestSha256'], 'Bundle manifest reused frozen48'
assert record['sha256'] != pins['baseline48']['dmgSha256'], 'DMG reused frozen48'
record['edSignatureBindsExactDmg'] = True
assert record['sha256'] != pins['baseline50']['dmgSha256']
assert before != pins['baseline50']['manifestSha256']
assert record['sha256'] != pins['baseline52']['dmgSha256']
assert before != pins['baseline52']['manifestSha256']
assert record['sha256'] != pins['baseline119']['dmgSha256']
assert before != pins['baseline119']['manifestSha256']
assert record['sha256'] != pins['baseline120']['dmgSha256']
assert before != pins['baseline120']['manifestSha256']
validate_inputs()
(out / 'DMG-RESULT.json').write_text(json.dumps(record, indent=2) + '\n')
print('Normal Mac121 DMG built and existing-key signed', flush=True)
