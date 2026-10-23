# SOURCE-ONLY successor; authored, NOT executed by worker.
# Consult BUILD-RECIPE-ADDENDUM.md and INVENTORY.json before Root execution; no installer here.
"""Mac120 normal candidate build+package (Root externally sealed source/tree, build 120 / 0.2.1).

Successor to READONLY next-mac47-8ba-operator/build-mac47-candidate.py.
All original 46/47 files remain immutable. This file lives in
scripts/macos/release/mac120 and writes ONLY to
  Root output/mac120-candidate/R1.

Scope: normal identity only; no registration, launch, install, publication,
no device/permission/updater action. Root reviews/runs; timestamp signing may
use Apple network. Apple Development notarization limitation unchanged; minOS
27 guard preserved. No legal certification. Both app-window flags OFF.
Standard build regenerates exact; no forced duplicate fresh-compile claim.
No Android build is part of this operator.

Signing order incorporates Root-reviewed R3 correction: standalone Mach-O
leaves first, then nested bundles inside-out, then outer app exactly once.
Bundle executables are derived from Info.plist and never treated as
independent leaf requests. No premature main/parent strict verification, no
`--deep` signing, `--timestamp` always. ADB pre-signed bytes never resigned.
Final deep strict verification is the review/readback gate, not the signer.
"""
from pathlib import Path
import datetime
import gzip
import hashlib
import json
import os
import shutil
import subprocess
import traceback
import xml.etree.ElementTree as ET

from mac120_guard import validate_inputs, OUTPUT, ROOT, SOURCE, TREE
root = ROOT
tmp = Path(os.environ['MAC120_SCRATCH']).resolve(strict=True)
arch_ref = Path(os.environ['MAC120_QT_TRANSLATIONS_SOURCE']).resolve(strict=True)
qmake = os.environ['MAC120_QMAKE']
macdeployqt = os.environ['MAC120_MACDEPLOYQT']
lrelease = os.environ['MAC120_LRELEASE']
qt_cellar = Path(os.environ['MAC120_QT_TRANSLATIONS_CELLAR']).resolve(strict=True)
pins = validate_inputs()
assert not OUTPUT.exists() and not OUTPUT.is_symlink(), 'Fresh cohort must be absent; preserve prior evidence'
OUTPUT.mkdir(mode=0o700, exist_ok=False)
out = OUTPUT / 'mac120-candidate/R1'
out.mkdir(mode=0o700, parents=True, exist_ok=False)
source = SOURCE
base47 = '8ba8ad5f46e4c096e6cd3160dc736ebf5ec77c8f'
old_mac_archive46 = 'ddbde39fccc4efe4e49195bee321c4108a905b5657f3b9a57787e0cf18c26d89'
old_mac_archive47 = '3569097b8b80bceb4cd6403d7b01833de4d263b531034d70b7346dfdab7fd899'
old_unsigned_46 = '3484f6c3df73daa564763c148ae5f20b368d2e812d8812d404715290efebe13a'
old_unsigned_47_informational = '7c1d916c4f00fa7e192f3ac344b774ea65ca81e877e200ea07ac7518f263e6cd'
env = os.environ.copy()
env['PYTHONDONTWRITEBYTECODE'] = '1'
env.update(CARGO_TARGET_DIR=str((tmp / 'cargo-target').resolve()), CARGO_BUILD_JOBS='4', CARGO_NET_OFFLINE='true', QMAKE=qmake)
record = {'at': datetime.datetime.now(datetime.timezone.utc).isoformat(), 'source': source, 'tree': TREE, 'parentSource': pins['parentSource'], 'base47': base47, 'branch': pins['branch'], 'build': 120, 'channel': '0.2.1', 'scope': 'Root-only normal candidate build/package/sign; no registration, launch, install, publication, device or permission action; codesign timestamp may use Apple network', 'flags': 'Both app-window qualification flags OFF; no feature enable', 'signingOrder': 'standalone leaves then inside-out bundles then outer; bundle executables via Info.plist; no premature parent verification; no --deep sign; --timestamp always', 'steps': []}

def digest(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()

def step(name, command, timeout=2400):
    print(name, flush=True)
    with (out / (name + '.log')).open('x') as log:
        p = subprocess.run(command, cwd=root, env=env, stdout=log, stderr=subprocess.STDOUT, timeout=timeout)
    record['steps'].append({'name': name, 'command': command, 'exit': p.returncode})
    if p.returncode:
        raise RuntimeError(name + ' failed; log retained')

try:
    assert subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=root, text=True).strip() == source
    assert subprocess.check_output(['git', 'rev-parse', 'HEAD^{tree}'], cwd=root, text=True).strip() == TREE
    assert subprocess.check_output(['git', 'rev-parse', source + '^{tree}'], cwd=root, text=True).strip() == TREE
    assert not subprocess.check_output(['git', 'status', '--porcelain', '--untracked-files=no'], cwd=root, text=True).strip()
    assert hashlib.sha256(subprocess.check_output(['git', 'show', source + ':core/nectarlink-core/src/features.rs'], cwd=root)).hexdigest() == pins['finalSourceSha256']['core/nectarlink-core/src/features.rs']
    assert hashlib.sha256(subprocess.check_output(['git', 'show', source + ':core/nectarlink-core/src/config.rs'], cwd=root)).hexdigest() == pins['finalSourceSha256']['core/nectarlink-core/src/config.rs']
    # Final composed source pins; historical BT guide remains a separate receipt.
    assert hashlib.sha256(subprocess.check_output(['git', 'show', source + ':desktop/app/qml/pages/CallsPage.qml'], cwd=root)).hexdigest() == pins['finalSourceSha256']['desktop/app/qml/pages/CallsPage.qml']
    assert hashlib.sha256(subprocess.check_output(['git', 'show', source + ':desktop/app/qml/components/CallCard.qml'], cwd=root)).hexdigest() == pins['finalSourceSha256']['desktop/app/qml/components/CallCard.qml']
    assert hashlib.sha256(subprocess.check_output(['git', 'show', source + ':desktop/app/i18n/template/nectarlink.ts'], cwd=root)).hexdigest() == pins['finalSourceSha256']['desktop/app/i18n/template/nectarlink.ts']
    assert hashlib.sha256(subprocess.check_output(['git', 'show', source + ':desktop/app/i18n/nectarlink_zh_CN.ts'], cwd=root)).hexdigest() == pins['finalSourceSha256']['desktop/app/i18n/nectarlink_zh_CN.ts']
    for dep in [tmp / 'cargo-target', tmp / 'license-texts', tmp / 'Sparkle.framework', tmp / 'Sparkle-LICENSE', tmp / 'adb-vendor/adb', tmp / 'adb-vendor/adb-seal.json', tmp / 'WaypairUpdater-14.4', Path(os.environ['MAC120_SIGN_MAC42']).resolve(strict=True), Path(qmake), Path(macdeployqt), Path(lrelease)]:
        assert Path(dep).exists(), str(dep)
    assert (tmp / 'mac41-PROVENANCE.json').is_file()
    assert (arch_ref / 'SOURCE.json').is_file()
    step('rust-notices', ['cargo', 'xtask', 'notices', str(out / 'RUST-NOTICES.txt'), '--target', 'aarch64-apple-darwin', '--license-texts', str(tmp / 'license-texts')])
    step('release-build', ['cargo', 'build', '--locked', '--release', '-p', 'nectarlink-desktop'])
    binary = Path(env['CARGO_TARGET_DIR']) / 'release/nectarlink-desktop'
    record['unsignedBinary'] = {'sha256': digest(binary), 'bytes': binary.stat().st_size}
    record['unsignedBinaryDeltaVs46'] = {'old': old_unsigned_46, 'same': record['unsignedBinary']['sha256'] == old_unsigned_46}
    record['unsignedBinaryVs47Informational'] = {'old47': old_unsigned_47_informational, 'sameAs47': record['unsignedBinary']['sha256'] == old_unsigned_47_informational, 'meaning': 'Historical47 informational only; Mac120 source differs50; frozen48 and frozen50 binary reuse rejected'}
    assert record['unsignedBinary']['sha256'] != old_unsigned_46, 'new source must not reuse frozen Mac46 binary'
    assert record['unsignedBinary']['sha256'] != pins['baseline48']['unsignedBinarySha256'], 'Mac120 must not reuse frozen Mac48 unsigned binary'
    record['unsignedBinaryVs48'] = {'old48': pins['baseline48']['unsignedBinarySha256'], 'sameAs48': False}
    assert record['unsignedBinary']['sha256'] != pins['baseline50']['unsignedBinarySha256'], 'Mac120 must not reuse frozen50 unsigned binary'
    assert record['unsignedBinary']['sha256'] != pins['baseline51']['unsignedBinarySha256'], 'Mac120 must not reuse frozen51 unsigned binary'
    assert record['unsignedBinary']['sha256'] != pins['baseline52']['unsignedBinarySha256'], 'Mac120 must not reuse frozen52 binary'
    assert record['unsignedBinary']['sha256'] != pins['baseline119']['unsignedBinarySha256'], 'Mac120 cannot reuse119 binary'
    record['baseline119'] = pins['baseline119']
    record['baseline52'] = pins['baseline52']
    record['baseline51'] = pins['baseline51']
    record['baseline50'] = pins['baseline50']
    record['baseline48'] = pins['baseline48']
    record['deploymentPolicy'] = pins['deploymentPolicy']
    archive = out / 'Waypair-Link-0.2.1-macos-source.tar.gz'
    archive.write_bytes(gzip.compress(subprocess.check_output(['git', 'archive', '--format=tar', '--prefix=waypair-link-source/', source], cwd=root), mtime=0))
    record['archive'] = {'path': str(archive), 'sha256': digest(archive), 'bytes': archive.stat().st_size, 'published': False}
    assert record['archive']['sha256'] not in (old_mac_archive46, old_mac_archive47, pins['baseline48']['sourceArchiveSha256']), 'Source archive reused frozen46/47/48'
    assert record['archive']['sha256'] != pins['baseline50']['sourceArchiveSha256'], 'Source archive reused frozen50'
    assert record['archive']['sha256'] != pins['baseline51']['sourceArchiveSha256'], 'Mac120 must not reuse frozen51 source archive'
    assert record['archive']['sha256'] != pins['baseline52']['sourceArchiveSha256'], 'Mac120 must not reuse frozen52 source archive'
    assert record['archive']['sha256'] != pins['baseline119']['sourceArchiveSha256'], 'Mac120 cannot reuse119 archive'
    old = json.loads((tmp / 'mac41-PROVENANCE.json').read_text())
    sparkle = tmp / 'Sparkle.framework'
    notices = out / 'notices'
    command = ['python3', 'scripts/macos/prepare-notices.py', 'prepare', '--rust-notices', str(out / 'RUST-NOTICES.txt'), '--license-texts', str(tmp / 'license-texts'), '--sparkle-framework', str(sparkle), '--sparkle-license', str(tmp / 'Sparkle-LICENSE'), '--adb-pin', 'scripts/macos/adb-platform-tools.json', '--source-commit', source, '--source-archive-url', 'https://pub-a57457f4da43415ea6335550ee9e2983.r2.dev/waypair/nectar/macos/releases/v0.2.1/' + archive.name, '--source-archive-sha256', digest(archive), '--source-repository', 'https://github.com/alexfoxtm/nectarlink', '--output-dir', str(notices)]
    for keg in old['homebrew']:
        command.extend(['--keg', str(Path(os.environ['MAC120_HOMEBREW_CELLAR']) / keg['name'] / keg['version'])])
    step('prepare-notices', command)
    data_keg = qt_cellar
    data_source = arch_ref
    source_info = json.loads((data_source / 'SOURCE.json').read_text())
    assert source_info['sha256'] == '85929c0c30d6f273f23bd879bb69803ea17b010ab73cab2f8abf95357f6f6bbb'
    assert digest(data_source / 'qttranslations-everywhere-src-6.12.0.tar.xz') == source_info['sha256']
    data_dir = notices / 'third-party/qt-translations'
    data_dir.mkdir()
    for filename, source_path in [('qttranslations.rb', data_keg / '.brew/qttranslations.rb'), ('homebrew-sbom.spdx.json', data_keg / 'sbom.spdx.json'), ('qttranslations-6.12.0.spdx', data_keg / 'share/qt/sbom/qttranslations-6.12.0.spdx'), ('BSD-3-Clause.txt', data_source / 'BSD-3-Clause.txt')]:
        shutil.copyfile(source_path, data_dir / filename)
    copyright_line = 'Copyright (C) The Qt Company Ltd. and other contributors.'
    assert copyright_line in (data_dir / 'qttranslations-6.12.0.spdx').read_text()
    catalog_data = (data_keg / 'share/qt/translations/qtbase_zh_CN.qm').resolve(strict=True)
    component = {'name': 'qttranslations', 'version': '6.12.0', 'kind': 'data-only translation catalog; no Mach-O', 'license': 'BSD-3-Clause', 'copyright': copyright_line, 'source': {'url': source_info['url'], 'sha256': source_info['sha256']}, 'bundlePath': 'Resources/translations/qtbase_zh_CN.qm', 'sha256': digest(catalog_data), 'supportingFiles': 'third-party/qt-translations/'}
    assert component['sha256'] == '0481a9b92a6f0578400e960a7d5ee70217a6a450eaf7f98b7d32b8dbd0d58616'
    text = 'Qt standard Chinese translation catalog\n=======================================\n\n' + copyright_line + '\n\nVersion: 6.12.0\nLicense: BSD-3-Clause\nSource: ' + source_info['url'] + '\nSource SHA-256: ' + source_info['sha256'] + '\nBundled data: Contents/' + component['bundlePath'] + '\nData SHA-256: ' + component['sha256'] + '\n\nVerbatim Qt copyright/SBOM, Homebrew formula and source/bottle SBOM are in third-party/qt-translations/.\n\nVerbatim license text from the checked Qt source archive:\n\n' + (data_source / 'BSD-3-Clause.txt').read_text()
    (notices / 'QT-TRANSLATIONS-NOTICE.txt').write_text(text)
    provenance_path = notices / 'third-party/PROVENANCE.json'
    provenance = json.loads(provenance_path.read_text())
    provenance['dataComponents'] = [component]
    provenance_path.write_text(json.dumps(provenance, indent=2) + '\n')
    manifest_path = notices / 'third-party/notices-manifest.json'
    manifest = json.loads(manifest_path.read_text())
    old_covers = manifest['covers']
    manifest['files'] = {str(p.relative_to(notices)): digest(p) for p in sorted(notices.rglob('*')) if p.is_file() and p != manifest_path}
    assert manifest['covers'] == old_covers == provenance['covers']
    manifest_path.write_text(json.dumps(manifest, indent=2) + '\n')
    step('verify-data-notice-manifest', ['python3', 'scripts/macos/prepare-notices.py', 'verify', str(notices)])
    record['dataComponent'] = component
    vendor = tmp / 'adb-vendor'
    stage = out / 'normal-mac120.noindex'
    app = stage / 'Waypair Link.app'
    step('package-normal-app', ['bash', str(Path(__file__).resolve().parent / 'package-app.sh'), '--binary', str(binary), '--output-dir', str(stage), '--app-name', 'Waypair Link', '--bundle-id', 'com.waypair.mac', '--version', '0.2.1', '--build-version', '120', '--macdeployqt', macdeployqt, '--updater-helper', str(tmp / 'WaypairUpdater-14.4'), '--sparkle-framework', str(sparkle), '--adb-helper', str(vendor / 'adb'), '--adb-seal', str(vendor / 'adb-seal.json'), '--adb-notice', str(vendor / 'ADB-NOTICE.txt'), '--notices-dir', str(notices)])
    assert not list(app.rglob('*.systemextension')) and not list(app.rglob('*.appex')), 'Normal Mac120 excludes prototype extensions'
    assert not (app / 'Contents/Library/SystemExtensions').exists(), 'CMIO extension embedding prohibited'
    catalog = root / 'desktop/app/i18n/nectarlink_zh_CN.ts'
    qm = app / 'Contents/Resources/nectarlink_zh_CN.qm'
    step('compile-offered-catalog', [lrelease, str(catalog), '-qm', str(qm)])
    translations = Path(subprocess.check_output([qmake, '-query', 'QT_INSTALL_TRANSLATIONS'], text=True).strip())
    qt_catalog = (translations / 'qtbase_zh_CN.qm').resolve(strict=True)
    assert qt_catalog.is_relative_to(qt_cellar)
    assert qt_catalog.is_file() and not qt_catalog.is_symlink(), qt_catalog
    (app / 'Contents/Resources/translations').mkdir(exist_ok=True)
    shutil.copyfile(qt_catalog, app / 'Contents/Resources/translations/qtbase_zh_CN.qm')
    messages = ET.parse(catalog).findall('.//message')
    template_messages = ET.parse(root / 'desktop/app/i18n/template/nectarlink.ts').findall('.//message')
    record['catalogCounts'] = {'translationMessages': len(messages), 'templateMessages': len(template_messages)}
    record['translation'] = {'tsSha256': digest(catalog), 'qmSha256': digest(qm), 'qtSource': str(qt_catalog), 'qtQmSha256': digest(qt_catalog)}
    assert record['translation']['tsSha256'] == pins['finalSourceSha256']['desktop/app/i18n/nectarlink_zh_CN.ts'], 'offered catalog must match the externally sealed final source'
    # Corrected signer: reuse qualified 46 signer lineage (hash-guarded) but emit
    # phase-order-fixed body. validate_inputs() already hash-guards the frozen
    # operator and input helper bytes; old source guards remain immutable.
    import plistlib
    entitlements = out / 'call-audio-entitlements.plist'
    entitlements.write_bytes(plistlib.dumps({'com.apple.security.device.audio-input': True}))
    signer_src = Path(os.environ['MAC120_SIGNER_REFERENCE']).resolve(strict=True)
    assert digest(signer_src) == 'd8f8c31e51c1fbd604206f440331d6dbaee6ee87719737df4769471c8488050b'
    signer_text = signer_src.read_text()
    assert signer_text.count('80f731ee19e02914c4634d57e8f7b1adddd63fd2') >= 1
    # Emit corrected signer: leaves, then inside-out bundles, then outer.
    # Bundle executables derived from Info.plist; adb never resigned; no --deep
    # sign; --timestamp always; entitlements only main exe file + outer app.
    corrected = '''from pathlib import Path
import subprocess, sys, json, hashlib, datetime, plistlib
app = Path(sys.argv[1]).resolve()
assert app.suffix == '.app' and app.is_dir()
assert app == Path("''' + str(app) + '''").resolve()
identity = '80f731ee19e02914c4634d57e8f7b1adddd63fd2'
entitlements = "''' + str(entitlements) + '''"
magic = {bytes.fromhex(x) for x in ['cffaedfe', 'cefaedfe', 'feedfacf', 'feedface', 'cafebabe', 'bebafeca']}
def bundle_executable(bundle):
    plist_path = bundle / ('Resources/Info.plist' if bundle.suffix == '.framework' else 'Contents/Info.plist')
    name = plistlib.loads(plist_path.read_bytes())['CFBundleExecutable']
    assert isinstance(name, str) and name and Path(name).name == name
    exe = bundle / (name if bundle.suffix == '.framework' else 'Contents/MacOS/' + name)
    assert exe.resolve().is_relative_to(bundle.resolve()) and exe.is_file()
    return exe.resolve()
macho = []
bundles = []
for p in app.rglob('*'):
    if p.is_symlink():
        continue
    if p.is_dir() and p.suffix in {'.app', '.xpc', '.framework', '.bundle'}:
        bundles.append(p)
    if p.is_file():
        with p.open('rb') as f:
            if f.read(4) in magic:
                macho.append(p)
adb = app / 'Contents/Helpers/adb'
if adb.exists():
    seal = json.loads((app / 'Contents/Resources/PhoneSetup/adb-seal.json').read_text())
    assert not adb.is_symlink() and seal['schema'] == 1 and seal['relativePath'] == 'Helpers/adb'
    assert adb.stat().st_size == seal['size'] and hashlib.sha256(adb.read_bytes()).hexdigest() == seal['sha256']
    subprocess.run(['codesign', '--verify', '--strict', str(adb)], check=True)
    info = subprocess.run(['codesign', '-dv', '--verbose=4', str(adb)], capture_output=True, text=True, check=True).stderr
    assert 'TeamIdentifier=5H4D7574Z4' in info and 'Identifier=com.waypair.adb' in info
    leaf = subprocess.run(['codesign', '-d', '--extract-certificates=' + str(app.parent.parent / 'adb-sign-cert'), str(adb)], capture_output=True, text=True, check=True)
    cert = app.parent.parent / 'adb-sign-cert0'
    assert hashlib.sha1(cert.read_bytes()).hexdigest() == identity
owners = {bundle_executable(b): b for b in bundles + [app]}
assert len(owners) == len(bundles) + 1 and set(owners) <= set(macho)
leaves = sorted([p for p in macho if p not in set(owners) and p != adb], key=lambda x: (-len(x.parts), str(x)))
ordered_bundles = sorted(bundles, key=lambda x: (-len(x.parts), str(x)))
for p in leaves:
    subprocess.run(['codesign', '--force', '--options', 'runtime', '--timestamp', '--sign', identity] + (['--entitlements', entitlements] if p == app / 'Contents/MacOS/nectarlink-desktop' else []) + [str(p)], check=True, timeout=240)
for p in ordered_bundles + [app]:
    subprocess.run(['codesign', '--force', '--options', 'runtime', '--timestamp', '--sign', identity] + (['--entitlements', entitlements] if p == app else []) + [str(p)], check=True, timeout=240)
print(json.dumps({'at': datetime.datetime.now(datetime.timezone.utc).isoformat(), 'app': str(app), 'identitySha1': identity, 'leaves': len(leaves), 'machOTotal': len(macho), 'bundles': len(bundles) + 1, 'hardenedRuntime': True, 'extraEntitlements': 'main executable/app audio-input only', 'order': 'leaves-then-insideout-bundles-then-outer'}))
'''
    (out / 'sign-normal-mac120.py').write_text(corrected)
    step('sign-normal-app', ['python3', str(out / 'sign-normal-mac120.py'), str(app)])
    assert subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=root, text=True).strip() == source
    assert subprocess.check_output(['git', 'rev-parse', 'HEAD^{tree}'], cwd=root, text=True).strip() == TREE
    assert not subprocess.check_output(['git', 'status', '--porcelain', '--untracked-files=no'], cwd=root, text=True).strip()
    manifest = {}
    for p in sorted(app.rglob('*')):
        if p.is_symlink():
            manifest[str(p.relative_to(app))] = {'symlink': os.readlink(p)}
        elif p.is_file():
            manifest[str(p.relative_to(app))] = {'bytes': p.stat().st_size, 'sha256': digest(p)}
    (out / 'SIGNED-BUNDLE-MANIFEST.json').write_text(json.dumps(manifest, indent=2) + '\n')
    record['bundle'] = str(app)
    record['bundleManifestSha256'] = digest(out / 'SIGNED-BUNDLE-MANIFEST.json')
    assert record['bundleManifestSha256'] != '97b3c1978c5aca1c14fb73dd6c468dd7d470f596087df1c3474d3f991666faf0', 'bundle manifest reused frozen 46'
    assert record['bundleManifestSha256'] != 'f00f22025ebfe8c70a09f10565067cb1a17084dbfb4e274a49c0b5a8272eca2a', 'bundle manifest reused frozen 47 sealed'
    assert record['bundleManifestSha256'] != pins['baseline48']['manifestSha256'], 'Bundle manifest reused frozen48'
    assert record['bundleManifestSha256'] != pins['baseline50']['manifestSha256'], 'Bundle manifest reused frozen50'
    validate_inputs()
    record['status'] = 'SIGNED_NORMAL_CANDIDATE_BUILT'
except Exception:
    record['status'] = 'FAILED'
    record['error'] = traceback.format_exc()
finally:
    record['finishedAt'] = datetime.datetime.now(datetime.timezone.utc).isoformat()
    (out / 'BUILD-PACKAGE-RESULT.json').write_text(json.dumps(record, indent=2) + '\n')
    print('Normal Mac120 candidate ' + record['status'], flush=True)
raise SystemExit(0 if record['status'] == 'SIGNED_NORMAL_CANDIDATE_BUILT' else 1)
