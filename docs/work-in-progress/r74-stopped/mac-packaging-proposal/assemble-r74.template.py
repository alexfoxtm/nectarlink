#!/usr/bin/env python3
"""Root-only R74 HOST-ONLY Mac142 successor from immutable Mac133.
Does not sign, install, activate, inspect profiles, run Git, or build product code.
Public notice/minimum-OS checks invoke existing local platform audit tools only
when Root executes this recipe. No historical release wrapper is called.
"""
import argparse
import hashlib
import json
import os
import plistlib
import re
import shutil
import subprocess
import tarfile
from pathlib import Path, PurePosixPath

# Root's sealed definition is inserted here by bind-root-source.py.
# The template refuses execution until every late-bound source value is sealed.
ROOT_SOURCE_DEFINITION = None
ROOT_SOURCE_DEFINITION_SHA256 = None
EXPECTED_SOURCE = None
EXPECTED_TREE = None
EXPECTED_SOURCE_MANIFEST = None
EXPECTED_SETTINGS = None
EXPECTED_SOURCE_COUNT = None
SOURCE_FILE_PINS = None
AUDIT_SCRIPT_PINS = None
REPO_RESOURCE_PINS = None
SOURCE_FEATURE_PINS = None
EXPECTED_ADDED_QML_IMPORTS = None
# END ROOT SOURCE BINDING
EXPECTED_LOAD_CHANGES = [{'from': '/opt/homebrew/opt/qtdeclarative/lib/QtQuick.framework/Versions/A/QtQuick',
  'to': '@executable_path/../Frameworks/QtQuick.framework/Versions/A/QtQuick'},
 {'from': '/opt/homebrew/opt/qtbase/lib/QtOpenGL.framework/Versions/A/QtOpenGL',
  'to': '@executable_path/../Frameworks/QtOpenGL.framework/Versions/A/QtOpenGL'},
 {'from': '/opt/homebrew/opt/qtsvg/lib/QtSvg.framework/Versions/A/QtSvg',
  'to': '@executable_path/../Frameworks/QtSvg.framework/Versions/A/QtSvg'},
 {'from': '/opt/homebrew/opt/qtdeclarative/lib/QtQml.framework/Versions/A/QtQml',
  'to': '@executable_path/../Frameworks/QtQml.framework/Versions/A/QtQml'},
 {'from': '/opt/homebrew/opt/qtbase/lib/QtNetwork.framework/Versions/A/QtNetwork',
  'to': '@executable_path/../Frameworks/QtNetwork.framework/Versions/A/QtNetwork'},
 {'from': '/opt/homebrew/opt/qtbase/lib/QtGui.framework/Versions/A/QtGui',
  'to': '@executable_path/../Frameworks/QtGui.framework/Versions/A/QtGui'},
 {'from': '/opt/homebrew/opt/qtbase/lib/QtCore.framework/Versions/A/QtCore',
  'to': '@executable_path/../Frameworks/QtCore.framework/Versions/A/QtCore'}]
BASE_MANIFEST = '4211679f147968a02505f23dcb50a8edaf1f1b3b1e1b66e446f2c40395e21688'
OLD_SOURCE_SHA = '62fa5f3a50b72560320739fbf5dd724ee48a5c97a409c97a0ce7296a5c0b4e46'
MAIN = 'Contents/MacOS/nectarlink-desktop'
LICENSES = 'Contents/Resources/LICENSES'
OLD_ARCHIVE = 'Contents/Resources/Source/Waypair-Link-local-133-source.tar.gz'
NEW_ARCHIVE = 'Contents/Resources/Source/Waypair-Link-local-142-source.tar.gz'
MACHO = {bytes.fromhex(s) for s in ['cffaedfe','cefaedfe','feedfacf','feedface','cafebabe','bebafeca','cafebabf','bfbafeca']}

CONFIG_KEYS = {'baselineApp', 'freshQmlImportResolution', 'rpathDelete', 'basicPayloadProof', 'freshImportResolution', 'freshBuildEvidence', 'cohort', 'binary', 'sourceBinding', 'rootSourceInputs', 'sourceArchive', 'freshStaticManifest', 'rpathAdd', 'outputDirectory', 'freshStaticWitness', 'currentSourceFiles', 'rootSourceDefinition', 'baselineManifest', 'loadChanges', 'schema', 'priorICU78Addendum', 'normalHostEntitlements', 'rootDependencyReceipt'}

def require(value, reason):
    if not value:
        raise ValueError(reason)


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def sealed(item):
    require(type(item) is dict and set(item) == {'path', 'sha256'}, 'sealed input schema mismatch')
    p = Path(item['path'])
    require('..' not in p.parts, 'parent-traversal sealed path refused')
    require(p.is_absolute() and p.is_file() and not any(x.is_symlink() for x in [p, *p.parents]), 'regular absolute sealed file required')
    require(re.fullmatch('[0-9a-f]{64}', item['sha256']) and sha(p) == item['sha256'], 'sealed file changed: ' + str(p))
    return p


def canonical_rel(value):
    require(type(value) is str and bool(value) and '\\' not in value and '\x00' not in value, 'invalid relative path')
    p = PurePosixPath(value)
    require(not p.is_absolute() and not any(part in ('', '.', '..') for part in value.split('/')) and p.as_posix() == value, 'noncanonical/traversal relative path')
    return p


def json_document(path):
    def pairs(rows):
        result = {}
        for key, value in rows:
            require(key not in result, 'duplicate JSON key: ' + key)
            result[key] = value
        return result
    return json.loads(path.read_text(), object_pairs_hook=pairs)


def inventory(root):
    result = {}
    for p in sorted(root.rglob('*')):
        name = str(p.relative_to(root))
        if p.is_symlink():
            target = os.readlink(p)
            require(not Path(target).is_absolute() and root.resolve() in p.resolve().parents, 'external/dangling bundle symlink')
            result[name] = {'symlink': target}
        elif p.is_file():
            result[name] = {'bytes': p.stat().st_size, 'sha256': sha(p)}
    return result


def run(args):
    return subprocess.check_output([str(a) for a in args], text=True, stderr=subprocess.STDOUT)


def load_records(binary):
    records = []
    for line in run(['/usr/bin/otool', '-L', binary]).splitlines():
        if line.startswith('\t'):
            name, sep, metadata = line.strip().partition(' (compatibility version ')
            require(bool(sep), 'unknown otool load syntax')
            records.append((name, metadata))
    require(bool(records), 'empty Mach-O load inventory')
    return records


def rpaths(binary):
    text = run(['/usr/bin/otool', '-l', binary])
    return re.findall(r'cmd LC_RPATH\s+cmdsize \d+\s+path (.*?) \(offset \d+\)', text)


def clean_output(path, baseline):
    require('..' not in path.parts and '..' not in baseline.parts, 'parent-traversal path refused')
    require(path.is_absolute() and not path.exists() and not path.is_symlink(), 'fresh output required')
    require(not any(p.is_symlink() for p in [path.parent, *path.parent.parents]), 'symlink output parent refused')
    require(baseline not in path.parents and path not in baseline.parents, 'output must be separate from immutable baseline')
    return path


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--config', required=True, type=Path)
    ap.add_argument('--config-sha256', required=True)
    a = ap.parse_args()
    require(ROOT_SOURCE_DEFINITION is not None, 'unbound source template: run bind-root-source.py first')
    c = json_document(sealed({'path': str(a.config), 'sha256': a.config_sha256}))
    require(set(c) == CONFIG_KEYS, 'config schema keys differ: ' + str(set(c) ^ CONFIG_KEYS))
    definition_path = sealed(c['rootSourceDefinition'])
    require(sha(definition_path) == ROOT_SOURCE_DEFINITION_SHA256 and json_document(definition_path) == ROOT_SOURCE_DEFINITION, 'generated recipe/source definition mismatch')
    require(c['schema'] == 1 and c['cohort'] == 'NORMAL_HOST_ONLY_MAC142_R74_SUCCESSOR', 'wrong cohort')
    source_inputs = json_document(sealed(c['rootSourceInputs']))
    SOURCE, TREE = source_inputs['source'], source_inputs['tree']
    require(SOURCE == EXPECTED_SOURCE and TREE == EXPECTED_TREE, 'exact frozen build142 source/tree required')
    require(source_inputs['version'] == '0.2.4' and source_inputs['build'] == 142 and source_inputs['profileFreeReleaseCompile'] is True, 'new normal build142 compile receipt required')
    BINARY = source_inputs['binarySHA256']
    require(re.fullmatch('[0-9a-f]{64}', BINARY) and re.fullmatch('[0-9a-f]{64}', source_inputs['archiveSHA256']) and source_inputs['sourceMembers'] == EXPECTED_SOURCE_COUNT, 'actual new binary and exact frozen archive seal required')
    baseline = Path(c['baselineApp'])
    require(baseline.is_absolute() and baseline.is_dir() and not any(p.is_symlink() for p in [baseline, *baseline.parents]), 'absolute immutable baseline required')
    manifest_path = sealed(c['baselineManifest'])
    require(sha(manifest_path) == BASE_MANIFEST, 'only immutable Mac133 baseline admitted')
    before = json.loads(manifest_path.read_text())
    require(inventory(baseline) == before and len(before) == 2283, 'Mac133 bytes/symlink inventory differs')
    require(sha(baseline / OLD_ARCHIVE) == OLD_SOURCE_SHA, 'old archive pin mismatch')
    info = plistlib.loads((baseline / 'Contents/Info.plist').read_bytes())
    require((info['CFBundleIdentifier'], info['CFBundleShortVersionString'], info['CFBundleVersion'], info['CFBundleExecutable']) == ('com.waypair.mac', '0.2.4', '133', 'nectarlink-desktop'), 'wrong baseline bundle')
    require(info['LSMinimumSystemVersion'] == '27.0' and 'NSSystemExtensionUsageDescription' not in info, 'unexpected baseline platform/native usage metadata')
    require(not list(baseline.rglob('*.systemextension')) and not list(baseline.rglob('*.appex')), 'native payload forbidden')
    require(not list(baseline.rglob('embedded.provisionprofile')), 'profile copy not admitted by host-only recipe')
    source_manifest_path = sealed(c['currentSourceFiles'])
    require(sha(source_manifest_path) == EXPECTED_SOURCE_MANIFEST, 'exact R74 complete source-file manifest required')
    source_manifest = json_document(source_manifest_path)
    require(set(source_manifest) == {'schema', 'source', 'tree', 'files'} and source_manifest['schema'] == 1, 'source manifest schema mismatch')
    require((source_manifest['source'], source_manifest['tree']) == (SOURCE, TREE) and len(source_manifest['files']) == EXPECTED_SOURCE_COUNT, 'complete source manifest binding mismatch')
    require(c['currentSourceFiles'] == ROOT_SOURCE_DEFINITION['currentSourceFiles'], 'config source manifest differs from sealed definition')
    for name, digest in source_manifest['files'].items():
        canonical_rel(name)
        require(type(digest) is str and re.fullmatch('[0-9a-f]{64}', digest), 'source manifest hash invalid')
    expected_licenses = {'LICENSE', *(name for name in source_manifest['files'] if name.startswith('LICENSES/')), *(name for name in source_manifest['files'] if name.startswith('assets/fonts/OFL-'))}
    require(set(REPO_RESOURCE_PINS) == expected_licenses, 'every current repository license/font notice must be bundled')
    archive = sealed(c['sourceArchive'])
    binary = sealed(c['binary'])
    require(sha(binary) == BINARY and c['binary']['path'] == source_inputs['binary'], 'new Root-compiled binary/path mismatch')
    require(sha(archive) == source_inputs['archiveSHA256'] and c['sourceArchive']['path'] == source_inputs['archive'], 'new Root-sealed source archive/path mismatch')
    entitlements_file = sealed(c['normalHostEntitlements'])
    require(sha(entitlements_file) == '289696af9834a7ee41aca4c1cd3aa95fc38f9ae2e83655b1d4b86c1ccab771ee', 'normal host entitlement input changed')
    require(plistlib.loads(entitlements_file.read_bytes()) == {'com.apple.security.device.audio-input': True}, 'only approved normal host audio-input entitlement admitted')
    rust = baseline / LICENSES / 'THIRD-PARTY-NOTICES.txt'
    root_receipt = json.loads(sealed(c['rootDependencyReceipt']).read_text())
    require(root_receipt['status'] == 'ROOT_QUALIFIED_MAC142_RETAINED_DONOR_STATIC_COMPATIBILITY', 'Root dependency/source qualification missing')
    require(root_receipt['sourceHEAD'] == SOURCE and root_receipt['sourceTree'] == TREE and root_receipt['binarySHA256'] == BINARY and root_receipt['sourceArchiveSHA256'] == sha(archive), 'Root new actual source/binary/archive mismatch')
    require(root_receipt['rustNoticesSHA256'] == sha(rust), 'current Rust notices binding missing')
    require(root_receipt['teamID'] == '5H4D7574Z4' and root_receipt['baselineSignedManifestSHA256'] == BASE_MANIFEST, 'identity/baseline changed')
    for field in ['completeCleanSourceArchiveMatchesActualHEAD', 'freshMainQtImportsResolveRetainedDonorExports', 'freshAddedQmlImportsResolveRetainedPayload', 'retainedQtCoreICU78ImportsResolveBundledExports', 'basicImportCoveredByPreservedPayload', 'sourceBindingGO', 'newBinaryBuildPassed', 'unchangedDependencyInputsPermitRustNoticeReuse', 'normalHostEntitlementsUnchanged', 'nativeProfileGateHOLD']:
        require(root_receipt.get(field) is True, 'Root gate missing: ' + field)
    require(root_receipt['completeIncomingDependencyByteEquivalenceClaimed'] is False and root_receipt['runtimeSemanticEquivalenceClaimed'] is False, 'no broad dependency/runtime equivalence claim admitted')
    require(root_receipt.get('linkedDependencyInputsEquivalentToMac133', False) is False, 'obsolete broad equivalence flag forbidden')
    require(root_receipt['staticCompatibilityBinarySHA256'] == BINARY, 'fresh static compatibility evidence must bind new compiled main')
    addendum = sealed(c['priorICU78Addendum'])
    require(sha(addendum) == 'aa9f731c71f36ec8d0f96fa613834a52e2f4ab294a134f6338e24f8184fed483', 'immutable ICU78 interpretation addendum required')
    require(root_receipt['priorICU78AddendumSHA256'] == sha(addendum), 'ICU78 addendum qualification binding mismatch')
    # These files are actual Root evidence, not a synthesized boolean receipt.
    # Assembly seals their bytes; independent review must assess their contents.
    evidence_fields = {
        'freshBuildEvidence': 'freshBuildEvidenceSHA256',
        'freshStaticWitness': 'freshStaticWitnessSHA256',
        'freshStaticManifest': 'freshStaticManifestSHA256',
        'freshImportResolution': 'freshImportResolutionSHA256',
        'freshQmlImportResolution': 'freshQmlImportResolutionSHA256',
    }
    for config_field, receipt_field in evidence_fields.items():
        evidence = sealed(c[config_field])
        require(root_receipt[receipt_field] == sha(evidence), 'fresh Root evidence seal mismatch: ' + config_field)
    require(source_inputs['buildEvidenceSHA256'] == root_receipt['freshBuildEvidenceSHA256'], 'compile evidence/source-input receipt mismatch')
    source_binding_path = sealed(c['sourceBinding'])
    require(root_receipt['sourceBindingSHA256'] == sha(source_binding_path), 'source binding review seal missing')
    source_binding = json.loads(source_binding_path.read_text())
    require(source_binding['verdict'] == 'GO' and source_binding['scope'] == 'MAC142_R74_FROZEN_SOURCE_BINDING', 'independent current source binding review required')
    require((source_binding['sourceHEAD'], source_binding['sourceTree'], source_binding['sourceArchiveSHA256'], source_binding['settingsPageSHA256']) == (SOURCE, TREE, sha(archive), EXPECTED_SETTINGS), 'source review must bind current source/archive/Settings')
    require(source_binding['newDependencies'] is False and source_binding['nativePayloadChanged'] is False, 'source review scope changed')
    source_review_evidence = sealed(source_binding['reviewEvidence'])
    require(root_receipt['sourceBindingReviewEvidenceSHA256'] == sha(source_review_evidence), 'source review evidence seal missing')
    require(root_receipt['selectorSettingsPageSHA256'] == EXPECTED_SETTINGS, 'current Settings source binding missing')
    require(root_receipt['sourceFileSHA256'] == SOURCE_FILE_PINS and root_receipt['publicAuditScriptSHA256'] == AUDIT_SCRIPT_PINS, 'exact unchanged dependency/audit source pins required')
    require(source_binding['sourceFeatureSHA256'] == SOURCE_FEATURE_PINS and root_receipt['sourceFeatureSHA256'] == SOURCE_FEATURE_PINS, 'current transfer/font/phone/OneUI/build/QML feature pins missing')
    require(source_binding['rootSourceDefinitionSHA256'] == ROOT_SOURCE_DEFINITION_SHA256 and root_receipt['rootSourceDefinitionSHA256'] == ROOT_SOURCE_DEFINITION_SHA256, 'Root source definition review seal missing')
    require(sha(sealed(c['basicPayloadProof'])) == 'e419d8b67959031f1b0d256ed1eb04b415354a3d98c627611dac2b2f739d986f', 'exact immutable Basic payload proof required')
    basic_proof = json.loads(sealed(c['basicPayloadProof']).read_text())
    for row in basic_proof['files']:
        path = baseline / row['path']
        require(path.is_file() and sha(path) == row['sha256'], 'Basic donor payload changed')
        if 'symlink' in row:
            require(path.is_symlink() and os.readlink(path) == row['symlink'] and str(path.resolve().relative_to(baseline.resolve())) == row['target'], 'Basic plugin symlink target changed')
        else:
            require(not path.is_symlink(), 'Basic regular payload changed type')
    require('module QtQuick.Controls.Basic' in (baseline / 'Contents/Resources/qml/QtQuick/Controls/Basic/qmldir').read_text(), 'Basic module declaration absent')
    require(root_receipt['loadChanges'] == c['loadChanges'] and root_receipt['rpathDelete'] == c['rpathDelete'] and root_receipt['rpathAdd'] == c['rpathAdd'], 'relocation changes not Root-qualified')
    output = clean_output(Path(c['outputDirectory']), baseline)
    # Inspect archive bytes directly; neither Git nor a primary checkout is read.
    with tarfile.open(archive, 'r:*') as t:
        members = t.getmembers()
        require(sum(m.isfile() for m in members) == source_inputs['sourceMembers'] and source_inputs['sourceMembers'] > 0, 'new source regular file count mismatch')
        files, seen = {}, set()
        for m in members:
            name = m.name.removesuffix('/') if m.isdir() else m.name
            parts = canonical_rel(name).parts
            require(parts[0] == 'waypair-link-source' and (len(parts) > 1 or m.isdir()), 'unsafe source archive root/member')
            require(name not in seen, 'duplicate source archive path')
            seen.add(name)
            if m.isdir():
                require(any(key.startswith('/'.join(parts[1:]) + '/') for key in source_manifest['files']) if len(parts) > 1 else True, 'extra source archive directory')
                continue
            require(m.isfile(), 'nonregular source archive member')
            files[name] = t.extractfile(m).read()
        prefix = 'waypair-link-source/'
        require({name.removeprefix(prefix): hashlib.sha256(data).hexdigest() for name, data in files.items()} == source_manifest['files'], 'complete current archive differs from frozen R74 source files')
        with tarfile.open(baseline / OLD_ARCHIVE, 'r:*') as old:
            old_files = {m.name: m for m in old.getmembers() if m.isfile()}
            descriptors = {name for name in old_files.keys() | files.keys() if name.endswith('/Cargo.toml') or name.endswith('/Cargo.lock') or '/xtask/' in name}
            require(len(descriptors) == 16 and all(name in old_files and name in files and old.extractfile(old_files[name]).read() == files[name] for name in descriptors), 'dependency/notice descriptors changed: use rebuilt dependency/notices lane')
        def qml_imports(mapping):
            return {match.group(1) for name, data in mapping.items() if '/desktop/app/qml/' in name and name.endswith('.qml') for match in re.finditer(r'^\s*import\s+([^\s;]+)', data.decode(), re.M)}
        with tarfile.open(baseline / OLD_ARCHIVE, 'r:*') as old:
            old_qml = {m.name: old.extractfile(m).read() for m in old.getmembers() if m.isfile() and '/desktop/app/qml/' in m.name and m.name.endswith('.qml')}
        added_imports = qml_imports(files) - qml_imports(old_qml)
        require(added_imports == set(EXPECTED_ADDED_QML_IMPORTS), 'only current reviewed retained QML module additions are admitted')
        qml_proof = json_document(sealed(c['freshQmlImportResolution']))
        require((qml_proof['sourceHEAD'], qml_proof['sourceTree'], qml_proof['binarySHA256'], qml_proof['sourceArchiveSHA256'], qml_proof['baselineSignedManifestSHA256']) == (SOURCE, TREE, BINARY, sha(archive), BASE_MANIFEST), 'fresh QML resolution source/binary/donor binding mismatch')
        require(set(qml_proof['modules']) == added_imports, 'fresh retained QML proof must cover exactly current added modules')
        for module, row in qml_proof['modules'].items():
            qmldir = str(canonical_rel(row['qmldir']))
            require(row['importsResolveRetainedPayload'] is True and row['files'] and qmldir in row['files'], 'QML module/type/plugin resolution evidence missing')
            for name, record in row['files'].items():
                canonical_rel(name)
                require(name.startswith('Contents/Resources/qml/') or name.startswith('Contents/Frameworks/') or name.startswith('Contents/PlugIns/'), 'non-QML retained payload proof path')
                require(name in before and type(record) is dict and before[name] == record, 'QML module payload differs from immutable Mac133')
            require(re.search(r'^module\s+' + re.escape(module) + r'\s*$', (baseline / qmldir).read_text(), re.M), 'retained QML module declaration missing')
            sealed(row['reviewEvidence'])
        for name, expected in SOURCE_FEATURE_PINS.items():
            require(hashlib.sha256(files[prefix + name]).hexdigest() == expected, 'current transfer/font/phone/OneUI/build/QML feature source mismatch')
        settings_page = files[prefix + 'desktop/app/qml/pages/SettingsPage.qml']
        require(hashlib.sha256(settings_page).hexdigest() == root_receipt['selectorSettingsPageSHA256'], 'reviewed selector postimage/source mismatch')
        aliases = re.findall(r'^\s*import\s+QtQuick\.Controls\.Basic\s+as\s+(\w+)\s*$', settings_page.decode(), re.M)
        require(len(aliases) == 1 and re.search(r'\b' + re.escape(aliases[0]) + r'\.ComboBox\s*\{', settings_page.decode()), 'reviewed Basic ComboBox alias/type missing')
        for name, expected in root_receipt['sourceFileSHA256'].items():
            require(hashlib.sha256(files[prefix + name]).hexdigest() == expected, 'source receipt member mismatch')
        audit_scripts = ['scripts/macos/prepare-notices.py', 'scripts/macos/record-minimum-os.py']
        require(set(root_receipt['publicAuditScriptSHA256']) == set(audit_scripts), 'exact public audit script inputs required')
        for name in audit_scripts:
            require(hashlib.sha256(files[prefix + name]).hexdigest() == root_receipt['publicAuditScriptSHA256'][name], 'public audit source pin mismatch')
    old_loads = load_records(baseline / MAIN)
    incoming = load_records(binary)
    require([{k: row[k] for k in ('from', 'to')} for row in c['loadChanges']] == EXPECTED_LOAD_CHANGES, 'exact seven approved load rewrites required')
    ordinals = [row['ordinal'] for row in c['loadChanges']]
    require(len(ordinals) == 7 and len(set(ordinals)) == 7 and all(type(n) is int and 1 <= n <= len(incoming) for n in ordinals), 'seven explicit unique 1-based load ordinals required')
    for row in c['loadChanges']:
        require(incoming[row['ordinal'] - 1][0] == row['from'], 'rewrite ordinal does not match current main load table')
    require(root_receipt['orderedIncomingLoads'] == [{'ordinal': i, 'name': name, 'metadata': meta} for i, (name, meta) in enumerate(incoming, 1)], 'fresh actual ordered load table missing')
    replacements = {row['from']: row['to'] for row in c['loadChanges']}
    require(len(replacements) == 7 and all(sum(name == old for name, _ in incoming) == 1 for old in replacements), 'duplicate/unused load mapping')
    mapped = [(replacements.get(name, name), meta) for name, meta in incoming]
    require(sorted(mapped) == sorted(old_loads), 'new dependency identity/version inventory differs from sealed Mac133')
    require(c['rpathDelete'] == ['/opt/homebrew/lib'] and c['rpathAdd'] == [], 'only approved rpath deletion admitted')
    # Matching load metadata is scoped static admission only. Root separately
    # qualifies fresh main imports against the preserved donor Qt/ICU78 cohort.
    require(len(set(c['rpathDelete'])) == len(c['rpathDelete']) and len(set(c['rpathAdd'])) == len(c['rpathAdd']), 'duplicate rpath instruction')
    incoming_paths = rpaths(binary)
    require(all(path in incoming_paths for path in c['rpathDelete']), 'unused rpath removal')
    final_paths = [p for p in incoming_paths if p not in c['rpathDelete']] + c['rpathAdd']
    require(all(p.startswith('@executable_path/') or p.startswith('@loader_path/') for p in final_paths), 'absolute/non-bundle rpath refused')
    require(final_paths and len(final_paths) == len(set(final_paths)), 'missing/duplicate final bundle rpath')
    output.mkdir(parents=True)
    app = output / 'Waypair Link.app'
    shutil.copytree(baseline, app, symlinks=True)
    require(inventory(app) == before, 'clone did not preserve exact baseline bytes')
    dest_binary = app / MAIN
    shutil.copyfile(binary, dest_binary)
    dest_binary.chmod((baseline / MAIN).stat().st_mode & 0o777)
    require(sha(dest_binary) == BINARY, 'copied compiled main differs before relocation')
    for old_name, new_name in replacements.items():
        run(['/usr/bin/install_name_tool', '-change', old_name, new_name, dest_binary])
    for path in c['rpathDelete']:
        run(['/usr/bin/install_name_tool', '-delete_rpath', path, dest_binary])
    for path in c['rpathAdd']:
        run(['/usr/bin/install_name_tool', '-add_rpath', path, dest_binary])
    require(load_records(dest_binary) == mapped and rpaths(dest_binary) == final_paths, 'post-relocation closure/order differs')
    # Resolve every main load into the preserved bundle or the system. All
    # non-main code remains byte-identical to the qualified baseline below.
    executable_dir = dest_binary.parent
    for name, _ in old_loads:
        if name.startswith('/System/Library/') or name.startswith('/usr/lib/'):
            continue
        candidates = []
        if name.startswith('@executable_path/') or name.startswith('@loader_path/'):
            candidates = [executable_dir / name.split('/', 1)[1]]
        elif name.startswith('@rpath/'):
            candidates = [executable_dir / p.split('/', 1)[1] / name.split('/', 1)[1] for p in final_paths]
        require(candidates and any(p.is_file() and app.resolve() in p.resolve().parents for p in candidates), 'main load not closed into preserved bundle')
    info['CFBundleVersion'] = '142'
    (app / 'Contents/Info.plist').write_bytes(plistlib.dumps(info, sort_keys=False))
    outer_signature = app / 'Contents/_CodeSignature'
    require(outer_signature.is_dir() and not outer_signature.is_symlink(), 'expected outer signature directory missing')
    require({p.name for p in outer_signature.iterdir()} == {'CodeResources'}, 'unexpected outer signature content')
    shutil.rmtree(outer_signature)
    (app / OLD_ARCHIVE).unlink()
    shutil.copyfile(archive, app / NEW_ARCHIVE)
    licenses = app / LICENSES
    notice_manifest = json.loads((licenses / 'third-party/notices-manifest.json').read_text())
    for source_name, record in REPO_RESOURCE_PINS.items():
        data = files[prefix + source_name]
        require(hashlib.sha256(data).hexdigest() == record['sha256'], 'current resource source pin mismatch')
        destination = app / record['destination']
        destination.parent.mkdir(parents=True, exist_ok=True)
        require(not destination.is_symlink(), 'resource target symlink refused')
        destination.write_bytes(data)
        if record['destination'].startswith(LICENSES + '/'):
            notice_manifest['files'][record['destination'].removeprefix(LICENSES + '/')] = record['sha256']
    provenance = json.loads((licenses / 'third-party/PROVENANCE.json').read_text())
    require(provenance['source']['commit'] == '558e1cc6b49ff904b4b651930d7316661143f9cf' and provenance['source']['archiveSha256'] == OLD_SOURCE_SHA, 'old source offer mismatch')
    provenance['source'] = {'commit': SOURCE, 'archive': 'bundle:Resources/Source/Waypair-Link-local-142-source.tar.gz', 'archiveSha256': sha(archive), 'repository': 'https://github.com/alexfoxtm/nectarlink', 'published': False}
    (licenses / 'third-party/PROVENANCE.json').write_text(json.dumps(provenance, indent=2) + '\n')
    (licenses / 'SOURCE-OFFER.txt').write_text('Source code for local Waypair Link build142\n\nThis unpublished personal host-only redesign candidate was built from commit\n  ' + SOURCE + '\nThe complete source is included in this app at\n  ' + NEW_ARCHIVE + '\n  SHA-256 ' + sha(archive) + '\nNative Camera Extension payload is not bundled; its provisioning gate remains HOLD.\nNo source push, publication, installation or activation is claimed.\n\nWaypair Link is free software under GNU GPL version3 or later (LICENSE.txt in Resources); core libraries use MPL2.0 (MPL-2.0.txt in this folder). Exact bundled third-party sources: THIRD-PARTY-SOURCES.txt.\n')
    require(rust.read_text().startswith('Waypair Link for macOS: third-party notices\n') and 'apple-darwin build)' in rust.read_text(), 'not a macOS Rust notice output')
    require(sha(licenses / 'THIRD-PARTY-NOTICES.txt') == sha(rust), 'unchanged Rust dependency notice was modified')
    changed_notices = ['SOURCE-OFFER.txt', 'third-party/PROVENANCE.json']
    for name in changed_notices:
        require(name in notice_manifest['files'], 'notice replacement not present in original inventory')
        notice_manifest['files'][name] = sha(licenses / name)
    (licenses / 'third-party/notices-manifest.json').write_text(json.dumps(notice_manifest, indent=2) + '\n')
    notice_dir = output / 'notices'
    notice_dir.mkdir()
    for name in list(notice_manifest['files']) + ['third-party/notices-manifest.json']:
        rel = Path(name)
        require(not rel.is_absolute() and '..' not in rel.parts, 'unsafe prepared notice path')
        dest = notice_dir / rel
        dest.parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(licenses / rel, dest)
    audit_dir = output / 'public-audit-scripts'
    audit_dir.mkdir()
    for name in audit_scripts:
        (audit_dir / Path(name).name).write_bytes(files[prefix + name])
    notices_script = audit_dir / 'prepare-notices.py'
    (licenses / 'BUNDLED-COMPONENTS.txt').unlink()
    run(['python3', notices_script, 'verify', notice_dir])
    run(['python3', notices_script, 'check-bundle', notice_dir, app])
    run(['python3', audit_dir / 'record-minimum-os.py', app])
    finished_info = plistlib.loads((app / 'Contents/Info.plist').read_bytes())
    require(tuple(map(int, finished_info['LSMinimumSystemVersion'].split('.'))) >= (27, 0), 'effective Mac133 minimum lowered')
    allowed_changes = {MAIN, 'Contents/Info.plist', 'Contents/_CodeSignature/CodeResources', OLD_ARCHIVE, NEW_ARCHIVE, LICENSES + '/SOURCE-OFFER.txt', LICENSES + '/third-party/PROVENANCE.json', LICENSES + '/third-party/notices-manifest.json', LICENSES + '/BUNDLED-COMPONENTS.txt', 'Contents/Resources/build-minimum-os.json'}
    allowed_changes |= {record['destination'] for record in REPO_RESOURCE_PINS.values()}
    for record in REPO_RESOURCE_PINS.values():
        require(sha(app / record['destination']) == record['sha256'], 'current license/font resource missing or changed')
    after = inventory(app)
    difference = {name for name in before.keys() | after.keys() if before.get(name) != after.get(name)}
    require(difference <= allowed_changes, 'unexpected changed resource/code path: ' + str(difference - allowed_changes))
    require(not list(app.rglob('*.systemextension')) and not list(app.rglob('*.appex')) and not list(app.rglob('embedded.provisionprofile')) and 'NSSystemExtensionUsageDescription' not in finished_info, 'native payload/profile/metadata introduced')
    require(sha(app / NEW_ARCHIVE) == sha(archive), 'own source archive changed')
    seal = json.loads((app / 'Contents/Resources/PhoneSetup/adb-seal.json').read_text())
    adb = app / 'Contents/Helpers/adb'
    require(seal['schema'] == 1 and seal['relativePath'] == 'Helpers/adb' and seal['sha256'] == sha(adb) and seal['size'] == adb.stat().st_size, 'sealed ADB mismatch')
    resources = app / 'Contents/Resources'
    require(not [p for p in resources.iterdir() if (p.name.startswith('nectarlink_zh') and p.suffix == '.qm') or (p.name.startswith('zh') and p.suffix == '.lproj')], 'Chinese host resources present')
    require(not [p for p in (resources / 'translations').glob('*.qm') if re.search(r'_zh(?:[_.-]|$)', p.stem)], 'Chinese host Qt catalogs present')
    require(inventory(baseline) == before, 'immutable predecessor changed')
    result = {'rootSourceDefinitionSHA256': ROOT_SOURCE_DEFINITION_SHA256, 'sourceFileCount': EXPECTED_SOURCE_COUNT, 'sourceFeatureSHA256': SOURCE_FEATURE_PINS, 'status': 'UNSIGNED_NORMAL_HOST_ONLY_MAC142_NOT_INSTALLED_NOT_PUBLISHED', 'source': SOURCE, 'tree': TREE, 'version': '0.2.4', 'build': 142, 'rootCompiledBinarySHA256': BINARY, 'relocatedMainSHA256': sha(dest_binary), 'sourceArchiveSHA256': sha(archive), 'baselineSignedManifestSHA256': BASE_MANIFEST, 'nativeExtensionIncluded': False, 'nativeProfileGate': 'HOLD', 'addedQmlImports': sorted(added_imports), 'basicModulePayloadPreserved': True, 'sourceBindingSHA256': sha(source_binding_path), 'sourceBindingReviewEvidenceSHA256': sha(source_review_evidence), 'freshRootEvidenceSHA256': {field: c[field]['sha256'] for field in evidence_fields}, 'dependencyQualificationScope': 'RETAINED_DONOR_QTCORE_ICU78_STATIC_COMPATIBILITY_ONLY', 'completeIncomingDependencyByteEquivalenceClaimed': False, 'runtimeSemanticEquivalenceClaimed': False, 'priorICU78AddendumSHA256': sha(addendum), 'nativeInstallEntitlementAdded': False, 'normalHostEntitlementsSHA256': sha(entitlements_file), 'app': str(app), 'changedPaths': sorted(difference), 'configSHA256': a.config_sha256, 'currentSourceFilesManifestSHA256': EXPECTED_SOURCE_MANIFEST, 'currentLicenseResourcesSHA256': {name: row['sha256'] for name, row in REPO_RESOURCE_PINS.items()}, 'allOtherBytesAndSymlinksMatchMac133': True, 'installed': False, 'published': False, 'signing': 'ROOT_ONLY_UNPERFORMED'}
    (output / 'UNSIGNED-HOST142-RESULT.json').write_text(json.dumps(result, indent=2) + '\n')
    (output / 'UNSIGNED-HOST142-BUNDLE-MANIFEST.json').write_text(json.dumps(after, indent=2) + '\n')
    print(result['status'])


if __name__ == '__main__':
    try:
        main()
    except (ValueError, KeyError, OSError, subprocess.CalledProcessError, tarfile.TarError) as error:
        raise SystemExit('HOST142_PACKAGING_HOLD: ' + str(error))
