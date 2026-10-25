#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
"""Root-only native camera pre-sign assembly. Never signs, installs or activates.

Config is an externally sealed Root input. Provisioning qualification is Root
operator evidence, not inferred from plist decoding. Missing evidence fails closed.
"""
import argparse
import datetime as dt
import hashlib
import json
import plistlib
import re
import shutil
import subprocess
import tarfile
from pathlib import Path

HOST = 'com.waypair.mac'
EXT = HOST + '.CameraExtension'
REL = Path('Library/SystemExtensions') / (EXT + '.systemextension')
EXE = 'WaypairCameraExtension'
MACHO = {b'\xfe\xed\xfa\xce', b'\xce\xfa\xed\xfe', b'\xfe\xed\xfa\xcf', b'\xcf\xfa\xed\xfe', b'\xca\xfe\xba\xbe', b'\xbe\xba\xfe\xca', b'\xca\xfe\xba\xbf', b'\xbf\xba\xfe\xca'}


def require(ok, message):
    if not ok:
        raise ValueError(message)


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def regular(path):
    path = Path(path)
    require(path.is_absolute() and path.is_file(), 'absolute regular input required')
    require(not any(p.is_symlink() for p in [path, *path.parents]), 'symlinked input refused')
    return path


def sealed(item):
    path = regular(item['path'])
    require(re.fullmatch('[0-9a-f]{64}', item['sha256']) is not None, 'invalid input SHA256')
    require(digest(path) == item['sha256'], 'sealed input changed: ' + str(path))
    return path


def run(args):
    return subprocess.check_output([str(a) for a in args], text=True, stderr=subprocess.STDOUT)


def version(value):
    require(re.fullmatch(r'\d+\.\d+(?:\.\d+)?', value) is not None, 'invalid OS version')
    return tuple(int(x) for x in value.split('.')) + (0,) * (3 - len(value.split('.')))


def profile(item, expected_id, required, team, certificate):
    path = sealed(item['profile'])
    # CMS decoding alone does not establish Apple's grant/trust; the independent
    # Root qualification receipt below is mandatory and binds these same bytes.
    raw = subprocess.check_output(['/usr/bin/security', 'cms', '-D', '-i', str(path)])
    data = plistlib.loads(raw)
    require(data.get('UUID') == item['uuid'], 'profile UUID mismatch')
    require(data.get('TeamIdentifier') == [team], 'profile team mismatch')
    expiry = data.get('ExpirationDate')
    require(isinstance(expiry, dt.datetime), 'profile expiration missing')
    require(expiry.replace(tzinfo=dt.timezone.utc) > dt.datetime.now(dt.timezone.utc), 'expired profile')
    require('OSX' in data.get('Platform', []), 'macOS profile required')
    certs = {hashlib.sha256(bytes(c)).hexdigest() for c in data.get('DeveloperCertificates', [])}
    require(certificate in certs, 'qualified signing certificate absent from profile')
    grants = data.get('Entitlements', {})
    appid = grants.get('com.apple.application-identifier', grants.get('application-identifier'))
    require(appid == team + '.' + expected_id, 'exact profile application identifier required')
    require(all(grants[key] == appid for key in ['com.apple.application-identifier', 'application-identifier'] if key in grants), 'conflicting profile application identifiers')
    require(grants.get('com.apple.developer.team-identifier') == team, 'profile entitlement team mismatch')
    require(all(grants.get(key, False) is False for key in ['get-task-allow', 'com.apple.security.get-task-allow']), 'debug profile refused')
    for key, value in required.items():
        require(grants.get(key) == value, 'profile does not grant required entitlement: ' + key)
    return path


def binary_audit(path, architectures, minimum, allowed_loads):
    require(set(run(['/usr/bin/lipo', '-archs', path]).split()) == set(architectures), 'architecture mismatch')
    build = run(['/usr/bin/xcrun', 'vtool', '-show-build', path])
    versions = re.findall(r'^\s*minos\s+(\d+(?:\.\d+){1,2})\s*$', build, re.M)
    require(len(versions) == len(architectures), 'missing per-slice deployment metadata')
    require(all(version(v) >= version(minimum) for v in versions), 'deployment target below qualified minimum')
    loads = run(['/usr/bin/otool', '-L', path])
    dependencies = [line.strip().split(' (compatibility version')[0] for line in loads.splitlines() if line.startswith('\t')]
    require(dependencies and set(dependencies) == set(allowed_loads), 'load closure differs from exact Root-qualified set')
    require(all(n.startswith('/System/Library/Frameworks/') or n.startswith('/usr/lib/') for n in dependencies), 'unexpected extension non-system dependency')
    return {'architectures': sorted(architectures), 'minimumOSBySlice': versions, 'loads': sorted(dependencies)}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--config', required=True, type=Path)
    parser.add_argument('--config-sha256', required=True)
    args = parser.parse_args()
    cfg_path = sealed({'path': str(args.config), 'sha256': args.config_sha256})
    c = json.loads(cfg_path.read_text())
    require(c['schema'] == 1 and c['cohort'] == 'LATER_NATIVE_CAMERA_UI_NOT_FOCUS_MAC134', 'wrong cohort')
    require(str(c['build']) != '134' and re.fullmatch(r'\d+', str(c['build'])) is not None, 'Root must allocate later build')
    require(re.fullmatch(r'\d+\.\d+\.\d+', c['version']) is not None, 'Root must bind version')
    team = c['teamID']
    require(re.fullmatch('[A-Z0-9]{10}', team) is not None, 'qualified team required')
    certificate = c['signingCertificateDER_SHA256']
    require(re.fullmatch('[0-9a-f]{64}', certificate) is not None, 'qualified certificate pin required')
    require(c['architectures'] in [['arm64'], ['x86_64']], 'frozen recipe supports one native architecture; universal requires reviewed successor')
    require(version(c['hostMinimumOS']) >= version('14.4'), 'host minimum must retain baseline14.4')
    require(c['extensionMinimumOS'] == '12.3', 'frozen recipe minimum is12.3; change requires reviewed recipe successor')
    source = Path(c['sourceRoot'])
    require(source.is_absolute() and source.is_dir() and not source.is_symlink(), 'absolute source root required')
    out = Path(c['outputDirectory'])
    require(out.is_absolute() and not out.exists() and not out.is_symlink(), 'fresh absolute output required')
    require(source not in out.parents and out not in source.parents, 'source/output must be separate')
    require(not any(p.is_symlink() for p in [out.parent, *out.parent.parents]), 'symlinked output parent refused')
    qpath = sealed(c['rootQualification'])
    q = json.loads(qpath.read_text())
    require(q['status'] == 'ROOT_QUALIFIED_NATIVE_CAMERA_SIGNING_INPUTS', 'Root qualification missing')
    require(q['teamID'] == team and q['signingCertificateDER_SHA256'] == certificate, 'qualification signer mismatch')
    for gate in ['appleProfileSignatureAndTrustVerified', 'certificatePrivateKeyAvailableToRoot', 'hostAndExtensionGrantsQualified', 'sourceCleanAndArchiveMatchesActualHEAD', 'predecessorIndependentResourcesPreserved']:
        require(q.get(gate) is True, 'Root qualification gate missing: ' + gate)
    require(re.fullmatch('[0-9a-f]{40}', c['sourceHEAD']) is not None and re.fullmatch('[0-9a-f]{40}', c['sourceTree']) is not None, 'actual source HEAD/tree required')
    require(q['sourceHEAD'] == c['sourceHEAD'] and q['sourceTree'] == c['sourceTree'], 'qualification source mismatch')
    manifest = json.loads(sealed(c['cameraV4Manifest']).read_text())
    require(manifest['version'] == 4 and len(manifest['files']) == 8, 'exact camera v4 manifest required')
    require(q['inputSHA256']['cameraV4Manifest'] == c['cameraV4Manifest']['sha256'], 'qualified v4 manifest binding mismatch')
    expected_camera = {row['path']: row['post_sha256'] for row in manifest['files']}
    tests_input = c.get('cameraTestsOnlyManifest')
    if tests_input is not None:
        tests = json.loads(sealed(tests_input).read_text())
        expected_transition = {'path': 'desktop/app/src/webcam.rs',
                               'pre_sha256': 'b68ea2e7997ba04a1079836f9da78234cc0eccd4ef613fe468160a28df003f04',
                               'post_sha256': '6649fff8b8c481731a6ced040b501c17f5231b78f7b9e756d7e1f9d3a1a1e094'}
        require(tests.get('base') == 'MANIFEST-v4.json' and tests.get('files') == [expected_transition], 'only exact accepted tests-only successor admitted')
        require(expected_camera[expected_transition['path']] == expected_transition['pre_sha256'], 'tests-only predecessor mismatch')
        require(q['inputSHA256']['cameraTestsOnlyManifest'] == tests_input['sha256'], 'qualified tests-only manifest binding mismatch')
        expected_camera[expected_transition['path']] = expected_transition['post_sha256']
    else:
        require(q['inputSHA256'].get('cameraTestsOnlyManifest') is None, 'qualification expects missing tests-only input')
    for name, sha in expected_camera.items():
        relative = Path(name)
        require(not relative.is_absolute() and '..' not in relative.parts, 'unsafe source path')
        require(digest(regular(source / relative)) == sha, 'approved camera source mismatch: ' + str(relative))
    require({item['relativePath'] for item in c['packagingSources']} == {'scripts/macos/package-app.sh', 'scripts/macos/prepare-notices.py', 'scripts/macos/package-native-camera-candidate.py'}, 'exact modern packaging source inventory required')
    for item in c['packagingSources']:
        require(digest(regular(source / item['relativePath'])) == item['sha256'], 'packaging source pin mismatch')
    archive = sealed(c['sourceArchive'])
    require(q['sourceArchiveSHA256'] == digest(archive), 'Root source archive binding mismatch')
    with tarfile.open(archive, 'r:*') as tar:
        members = tar.getmembers()
        require(bool(members), 'empty corresponding-source archive')
        seen = set()
        for member in members:
            parts = Path(member.name).parts
            require(parts and parts[0] == 'waypair-link-source' and '..' not in parts, 'unsafe source archive member')
            if member.isdir():
                continue
            require(member.isfile() and member.name not in seen, 'nonregular/duplicate archive source refused')
            seen.add(member.name)
            require(hashlib.sha256(tar.extractfile(member).read()).hexdigest() == digest(regular(source.joinpath(*parts[1:]))), 'archive/current source mismatch')
        for row in manifest['files']:
            require('waypair-link-source/' + row['path'] in seen, 'camera source missing from archive')
        require('waypair-link-source/scripts/macos/package-native-camera-candidate.py' in seen, 'wrapper missing from own-source archive')
    requirement_file = sealed(c['hostRequirement'])
    requirement = requirement_file.read_text().strip()
    require(requirement and requirement == q['canonicalHostDesignatedRequirement'], 'exact qualified host requirement mismatch')
    # Qualification receipt binds exact complete entitlement files, not a handful
    # of substring checks or a newly invented replacement for host permissions.
    baseline_file = sealed(c['baselineHostEntitlements'])
    host_ent_file = sealed(c['hostEntitlements'])
    ext_ent_file = sealed(c['extensionEntitlements'])
    baseline = plistlib.loads(baseline_file.read_bytes())
    host_ent = plistlib.loads(host_ent_file.read_bytes())
    ext_ent = plistlib.loads(ext_ent_file.read_bytes())
    expected_host = dict(baseline)
    expected_host['com.apple.developer.system-extension.install'] = True
    require(host_ent == expected_host, 'host entitlements must preserve baseline plus exact install grant')
    require(ext_ent == {'com.apple.security.app-sandbox': True, 'com.apple.developer.cmio.extension': True}, 'unexpected extension entitlement input')
    for field in ['baselineHostEntitlements', 'hostEntitlements', 'extensionEntitlements', 'hostRequirement']:
        require(q['inputSHA256'][field] == c[field]['sha256'], 'qualification input binding mismatch')
    host_profile = profile(c['hostProvisioning'], HOST, {'com.apple.developer.system-extension.install': True}, team, certificate)
    ext_profile = profile(c['extensionProvisioning'], EXT, ext_ent, team, certificate)
    for field in ['hostProvisioning', 'extensionProvisioning']:
        require(q['inputSHA256'][field] == c[field]['profile']['sha256'], 'qualification profile binding mismatch')
    base = c['basePackage']
    binary = sealed(base['binary'])
    require(q['hostBinarySHA256'] == digest(binary), 'host binary/source qualification missing')
    tools = {name: sealed(base[name]) for name in ['macdeployqt', 'updaterHelper', 'adbHelper', 'adbSeal', 'adbNotice']}
    # Existing approved notices/Sparkle closure is verified by public packager;
    # no discovery fallback to an installed application is permitted.
    notices = Path(base['noticesDirectory'])
    sparkle = Path(base['sparkleFramework'])
    require(notices.is_absolute() and sparkle.is_absolute(), 'explicit local notices/Sparkle required')
    for field in ['noticesManifestSHA256', 'sparkleInputManifestSHA256']:
        require(q[field] == base[field], 'Root dependency closure binding mismatch')
    require(digest(regular(notices / 'third-party/notices-manifest.json')) == base['noticesManifestSHA256'], 'notice manifest changed')
    provenance = json.loads(regular(notices / 'third-party/PROVENANCE.json').read_text())
    require(provenance['source']['commit'] == c['sourceHEAD'] and provenance['source']['archiveSha256'] == c['sourceArchive']['sha256'], 'notice source offer does not bind actual source/archive')
    require(provenance['source']['archive'] == c['sourceArchiveOfferURL'] and q['sourceArchiveOfferURL'] == c['sourceArchiveOfferURL'], 'source offer URL qualification mismatch')
    sparkle_manifest = json.loads(sealed(base['sparkleInputManifest']).read_text())
    require(digest(sealed(base['sparkleInputManifest'])) == base['sparkleInputManifestSHA256'], 'Sparkle manifest changed')
    require(set(sparkle_manifest) == {str(p.relative_to(sparkle)) for p in sparkle.rglob('*') if p.is_file() and not p.is_symlink()}, 'incomplete Sparkle regular-file inventory')
    for rel, sha in sparkle_manifest.items():
        require(not Path(rel).is_absolute() and '..' not in Path(rel).parts, 'unsafe Sparkle pin')
        require(digest(regular(sparkle / rel)) == sha, 'Sparkle input changed')
    require(q['preservedIndependentResources'] == c['preservedIndependentResources'], 'predecessor resource inventory binding mismatch')
    for item in c['preservedIndependentResources']:
        resource = sealed(item['input'])
        require(resource.read_bytes()[:4] not in MACHO, 'independent resource must not introduce executable code')
    # Recheck input identities immediately before mutations. Output is never an
    # installed app; failures leave an inspectable unsigned disposable candidate.
    out.mkdir(parents=True)
    app = out / 'Waypair Link.app'
    recipe = source / 'scripts/macos/build-native-camera-extension.py'
    packager = source / 'scripts/macos/package-app.sh'
    notices_script = source / 'scripts/macos/prepare-notices.py'
    run(['bash', packager, '--binary', binary, '--output-dir', out, '--app-name', 'Waypair Link', '--bundle-id', HOST, '--version', c['version'], '--build-version', str(c['build']), '--macdeployqt', tools['macdeployqt'], '--qml-dir', source / 'desktop/app/qml', '--updater-helper', tools['updaterHelper'], '--sparkle-framework', sparkle, '--adb-helper', tools['adbHelper'], '--adb-seal', tools['adbSeal'], '--adb-notice', tools['adbNotice'], '--notices-dir', notices])
    run(['python3', recipe, '--source-root', source, '--candidate-app', app, '--team-id', team, '--host-requirement-file', requirement_file, '--signing-input-dir', out / 'generated-signing-inputs', '--version', c['version'], '--build', str(c['build'])])
    contents = app / 'Contents'
    ext = contents / REL
    info = plistlib.loads((ext / 'Contents/Info.plist').read_bytes())
    require(info['CFBundleIdentifier'] == EXT and info['CFBundleExecutable'] == EXE and info['CFBundlePackageType'] == 'SYSX', 'extension bundle identity mismatch')
    require(info['CFBundleVersion'] == str(c['build']) and info['CFBundleShortVersionString'] == c['version'], 'extension version mismatch')
    require(info['LSMinimumSystemVersion'] == c['extensionMinimumOS'], 'extension plist deployment mismatch')
    require(info['WaypairProducerRequirement'] == requirement and info['CMIOExtension']['CMIOExtensionMachServiceName'] == team + '.' + EXT, 'extension requirement/service mismatch')
    host_info = plistlib.loads((contents / 'Info.plist').read_bytes())
    require(host_info['CFBundleIdentifier'] == HOST and host_info['CFBundleVersion'] == str(c['build']) and host_info['CFBundleShortVersionString'] == c['version'], 'host identity/version mismatch')
    require(version(host_info['LSMinimumSystemVersion']) >= version(c['hostMinimumOS']), 'host Info.plist minimum lowered')
    require(bool(host_info.get('NSSystemExtensionUsageDescription')), 'host usage copy missing')
    extension_files = {str(p.relative_to(ext)) for p in ext.rglob('*') if p.is_file()}
    require(extension_files == {'Contents/Info.plist', 'Contents/MacOS/' + EXE}, 'unexpected unsigned extension payload')
    require(not any(p.is_symlink() for p in ext.rglob('*')), 'extension symlink refused')
    require(set(contents.glob('Library/SystemExtensions/*')) == {ext}, 'unexpected extension bundle')
    audit = binary_audit(ext / 'Contents/MacOS' / EXE, c['architectures'], c['extensionMinimumOS'], c['extensionSystemLoads'])
    require(set(run(['/usr/bin/lipo', '-archs', contents / 'MacOS/nectarlink-desktop']).split()) == set(c['architectures']), 'host architecture mismatch')
    host_build = run(['/usr/bin/xcrun', 'vtool', '-show-build', contents / 'MacOS/nectarlink-desktop'])
    host_minos = re.findall(r'^\s*minos\s+(\d+(?:\.\d+){1,2})\s*$', host_build, re.M)
    require(len(host_minos) == len(c['architectures']) and all(version(v) >= version(c['hostMinimumOS']) for v in host_minos), 'host Mach-O deployment mismatch')
    for destination in [contents / 'embedded.provisionprofile', ext / 'Contents/embedded.provisionprofile']:
        require(not destination.exists() and not destination.is_symlink(), 'profile overwrite refused')
    shutil.copyfile(host_profile, contents / 'embedded.provisionprofile')
    shutil.copyfile(ext_profile, ext / 'Contents/embedded.provisionprofile')
    source_dest = contents / 'Resources/Source' / archive.name
    source_dest.parent.mkdir()
    shutil.copyfile(archive, source_dest)
    for item in c['preservedIndependentResources']:
        rel = Path(item['bundleRelativePath'])
        require(not rel.is_absolute() and '..' not in rel.parts and str(rel).startswith('Contents/Resources/camera-helper/'), 'unexpected independent resource destination')
        dest = app / rel
        require(not dest.exists() and not dest.is_symlink(), 'resource overwrite refused')
        dest.parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(sealed(item['input']), dest)
    listing = contents / 'Resources/LICENSES/BUNDLED-COMPONENTS.txt'
    # The public packager created this first-pass file itself. Regenerate only
    # this known generated listing to include the newly nested first-party code.
    require(listing.is_file() and not listing.is_symlink(), 'generated component listing missing')
    listing.unlink()
    run(['python3', notices_script, 'check-bundle', notices, app])
    require(digest(source_dest) == c['sourceArchive']['sha256'], 'embedded source mismatch')
    seal = json.loads((contents / 'Resources/PhoneSetup/adb-seal.json').read_text())
    adb = contents / 'Helpers/adb'
    require(seal['schema'] == 1 and seal['relativePath'] == 'Helpers/adb' and seal['sha256'] == digest(adb) and seal['size'] == adb.stat().st_size and digest(adb) == base['adbHelper']['sha256'], 'ADB seal changed')
    resources = contents / 'Resources'
    require(not [p for p in resources.iterdir() if (p.name.startswith('nectarlink_zh') and p.suffix == '.qm') or (p.name.startswith('zh') and p.suffix == '.lproj')], 'Chinese host resources present')
    require(not [p for p in (resources / 'translations').glob('*.qm') if re.search(r'_zh(?:[_.-]|$)', p.stem)], 'Chinese host Qt catalogs present')
    result = {'status': 'UNSIGNED_PRE_SIGN_ASSEMBLY_ONLY_NOT_ACTIVATION_READY', 'configSHA256': args.config_sha256, 'sourceHEAD': c['sourceHEAD'], 'sourceTree': c['sourceTree'], 'app': str(app), 'extension': str(ext), 'extensionBinaryAudit': audit, 'hostMinimumOSBySlice': host_minos, 'files': {str(p.relative_to(app)): digest(p) for p in sorted(app.rglob('*')) if p.is_file() and not p.is_symlink()}, 'remaining': ['Root extension-first then host signing', 'signed entitlement/profile/team/designated requirement verification', 'deep strict full bundle verification', 'notarization/distribution qualification if required', 'explicit permitted activation', 'real consumer approval and frames at every supported mode', 'actual sensor/encoder/output quality qualification']}
    (out / 'UNSIGNED-ASSEMBLY-RESULT.json').write_text(json.dumps(result, indent=2) + '\n')
    print(result['status'])


if __name__ == '__main__':
    try:
        main()
    except (ValueError, KeyError, OSError, subprocess.CalledProcessError, plistlib.InvalidFileException, tarfile.TarError) as error:
        raise SystemExit('NATIVE_CAMERA_PACKAGING_HOLD: ' + str(error))
