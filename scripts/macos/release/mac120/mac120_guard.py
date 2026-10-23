"""Parameterized Mac120 Root guards. Private paths/pins come from Root inputs."""
from pathlib import Path
import hashlib
import json
import os
import re
import subprocess
import sys
import tomllib

ROOT = Path(os.environ['WAYPAIR_REPO_ROOT']).resolve(strict=True)
OUTPUT = Path(os.environ['MAC120_OUTPUT_ROOT']).resolve()
SOURCE = os.environ['MAC120_SOURCE_COMMIT']
TREE = os.environ['MAC120_SOURCE_TREE']
PARENT = os.environ['MAC120_PARENT_COMMIT']
PARENT_TREE = os.environ['MAC120_PARENT_TREE']
INPUTS = Path(os.environ['MAC120_ROOT_INPUTS']).resolve(strict=True)
PINS = json.loads(INPUTS.read_text())
INPUTS_SHA256 = os.environ['MAC120_ROOT_INPUTS_SHA256']
PINS.update({'source': SOURCE, 'tree': TREE, 'parentSource': PARENT, 'parentTree': PARENT_TREE, 'outputRoot': str(OUTPUT)})


def sha(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def validate_selected_file(env_name, pin_group, basename, allow_symlink=False):
    """Bind the file that will execute/be consumed to its immutable V3 pin."""
    matches = [(Path(path), digest) for path, digest in PINS[pin_group].items()
               if Path(path).name == basename]
    if len(matches) != 1:
        raise RuntimeError('expected exactly one retained pin for ' + basename)
    pinned_path, expected = matches[0]
    selected_path = Path(os.environ[env_name]).absolute()
    if selected_path.is_symlink() and not allow_symlink:
        raise RuntimeError(env_name + ' must not be a symlink')
    resolved = selected_path.resolve(strict=True)
    pinned_resolved = pinned_path.resolve(strict=True)
    if resolved != pinned_resolved or not resolved.is_file() or resolved.is_symlink() or sha(resolved) != expected:
        raise RuntimeError(env_name + ' differs from its immutable V3 pin')


def git(*args):
    return subprocess.check_output(['git', '-C', str(ROOT), *args], stderr=subprocess.DEVNULL)


def bundle_manifest(app):
    result = {}
    for path in sorted(Path(app).rglob('*')):
        rel = str(path.relative_to(app))
        if path.is_symlink():
            result[rel] = {'symlink': os.readlink(path)}
        elif path.is_file():
            result[rel] = {'bytes': path.stat().st_size, 'sha256': sha(path)}
    return result


def validate_inputs():
    if not __debug__ or sys.flags.optimize:
        raise RuntimeError('Do not disable operator assertions with -O/PYTHONOPTIMIZE')
    for name in ('RUSTFLAGS', 'CARGO_ENCODED_RUSTFLAGS', 'DEVELOPER_DIR', 'PYTHONOPTIMIZE'):
        if os.environ.get(name):
            raise RuntimeError(name + ' unexpectedly configured; HOLD')
    for name, value in (('MAC120_SOURCE_COMMIT', SOURCE), ('MAC120_SOURCE_TREE', TREE),
                        ('MAC120_PARENT_COMMIT', PARENT), ('MAC120_PARENT_TREE', PARENT_TREE)):
        if re.fullmatch(r'[0-9a-f]{40}', value) is None:
            raise RuntimeError(name + ' must be an externally sealed full Git id')
    if sha(INPUTS) != INPUTS_SHA256 or re.fullmatch(r'[0-9a-f]{64}', INPUTS_SHA256) is None:
        raise RuntimeError('external Root input seal changed after binding')
    if PINS.get('build') != 120 or PINS.get('channel') != '0.2.1':
        raise RuntimeError('Mac120/local.26 identity pin mismatch')
    if PINS.get('branch') != git('symbolic-ref', '--short', 'HEAD').decode().strip():
        raise RuntimeError('expected release branch differs from current branch')
    if git('rev-parse', 'HEAD').decode().strip() != SOURCE or git('rev-parse', 'HEAD^{tree}').decode().strip() != TREE:
        raise RuntimeError('HEAD/tree differ from Root external final-source pins')
    if git('rev-parse', SOURCE + '^').decode().strip() != PARENT:
        raise RuntimeError('final source parent differs from Root external parent pin')
    if git('rev-parse', PARENT + '^{tree}').decode().strip() != PARENT_TREE:
        raise RuntimeError('final source parent tree differs from Root external pin')
    if git('status', '--porcelain', '--untracked-files=no').strip():
        raise RuntimeError('tracked source is dirty; preserve and HOLD')
    allowed = set(PINS.get('allowedUntrackedSha256', {}))
    untracked = set(git('ls-files', '--others', '--exclude-standard', '-z').decode().split('\0')) - {''}
    if untracked - allowed:
        raise RuntimeError('unexpected untracked files; preserve and HOLD')
    for rel in untracked:
        path = ROOT / rel
        if path.is_symlink() or sha(path) != PINS['allowedUntrackedSha256'][rel]:
            raise RuntimeError('allowed untracked input changed; preserve and HOLD')
    baseline51 = PINS['baseline51']
    if baseline51['source'] != '3e6f14decdf525c628619f734b2363e96e0b3d6e' or baseline51['tree'] != '2aecfaa9ce5a9eace4530b7643329dae79a6c39f' or baseline51['build'] != 51 or baseline51['channel'] != '0.1.4-local.23' or baseline51['manifestSha256'] != '90524520c1d697fcee360cf0206133ad587337bed5c19201ce0223b96de5ee9f':
        raise RuntimeError('installed Mac51 baseline cannot be repinned')
    if sha(baseline51['manifestPath']) != baseline51['manifestSha256']:
        raise RuntimeError('immutable Mac51 signed manifest changed')
    subprocess.run(['git', '-C', str(ROOT), 'merge-base', '--is-ancestor', baseline51['source'], SOURCE], check=True)
    # Preserve the Mac50 baseline and frozen historical per-file bytes.
    baseline50 = PINS['baseline50']
    expected50 = {'source':'79d6d051ec0f7b5437fed25980bbb0a55de9395c','tree':'6909e6532dd23c2bfe7beca2b2f71981d53449e5','build':50,'channel':'0.1.4-local.22','manifestSha256':'a7fa5c2a447c0811300a3a9d4d024984de92519d43e7f3be4fd04087a49813be','sourceArchiveSha256':'ab0b2b5a4378c2ee3651ddc4f45effb61480423cdb8d73e43656abcbad73ec18','unsignedBinarySha256':'c40fc0fcdf1fd739734afc5e7f4081c3aadf32d54241f3834a1e781e8ca2f10f','dmgSha256':'9a69424d5d99cfa5a849db469ead6cdbc411e4f1b412ee1e343f5c11141299e4','dmgBytes':86781459,'minimumSystemVersion':'27.0'}
    for key, value in expected50.items():
        actual_key = 'manifestSha256' if key == 'manifestSha256' else key
        if baseline50.get(actual_key) != value:
            raise RuntimeError('Mac50 baseline pin changed: ' + key)
    manifest = Path(os.environ['MAC120_MAC50_MANIFEST']).resolve(strict=True)
    if sha(manifest) != expected50['manifestSha256']:
        raise RuntimeError('frozen Mac50 signed bundle manifest mismatch')
    if not (manifest.is_file() and not manifest.is_symlink()):
        raise RuntimeError('Mac50 baseline manifest is not a regular file')
    for field in ('frozenOperatorPins','retainedInputs','provenanceReceiptPins','frozenArtifactPins','successful48Operators','successful49Operators','actual49ReceiptPins'):
        for path, expected in PINS[field].items():
            p = Path(path)
            qt_tool = field == 'retainedInputs' and str(p) in {
                '/opt/homebrew/bin/qmake6', '/opt/homebrew/bin/macdeployqt', '/opt/homebrew/bin/lrelease'
            }
            resolved = p.resolve(strict=True) if qt_tool else p
            if (p.is_symlink() and not qt_tool) or not resolved.is_file() or resolved.is_symlink() or sha(resolved) != expected:
                raise RuntimeError(field + ' bytes changed: ' + path)
    for env_name, group, basename, allow_symlink in (
        ('MAC120_QMAKE', 'retainedInputs', 'qmake6', True),
        ('MAC120_MACDEPLOYQT', 'retainedInputs', 'macdeployqt', True),
        ('MAC120_LRELEASE', 'retainedInputs', 'lrelease', True),
        ('MAC120_UPDATE_SIGNER', 'retainedInputs', 'mac-update-signing', False),
        ('MAC120_SIGN_MAC42', 'retainedInputs', 'sign-mac42-bundle.py', False),
        ('MAC120_SIGNER_REFERENCE', 'frozenOperatorPins', 'sign-normal-mac46.py', False),
        ('MAC120_ADB_SEAL', 'retainedInputs', 'adb-seal.json', False),
    ):
        validate_selected_file(env_name, group, basename, allow_symlink)
    retained = PINS['retainedPins']
    if retained.get('bundleId') != 'com.waypair.mac' or retained.get('team') != '5H4D7574Z4' or retained.get('leafSha1') != '80f731ee19e02914c4634d57e8f7b1adddd63fd2':
        raise RuntimeError('bundle identity/signing pins changed')
    if retained.get('minOS') != PINS['deploymentPolicy']['minimumSystemVersion'] or retained.get('minOS') != '27.0':
        raise RuntimeError('minimum macOS must remain 27.0')
    # Existing exact source pins remain guards for previously reviewed Mac50/Mac120 inputs.
    for rel, expected in PINS.get('finalSourceSha256', {}).items():
        actual = hashlib.sha256(git('show', f'{SOURCE}:{rel}')).hexdigest()
        if actual != expected:
            raise RuntimeError('pinned source path changed; Root must refresh review: ' + rel)
        if sha(ROOT / rel) != expected:
            raise RuntimeError('working source differs from pinned commit: ' + rel)
    for rel, expected in PINS.get('sourceClosureSha256', {}).items():
        actual = hashlib.sha256(git('show', f'{SOURCE}:{rel}')).hexdigest()
        if actual != expected or sha(ROOT / rel) != expected:
            raise RuntimeError('source-closure recipe differs from sealed final commit: ' + rel)
    if OUTPUT.is_symlink() or OUTPUT.exists():
        if OUTPUT.is_symlink() or not OUTPUT.is_dir() or OUTPUT.resolve() != OUTPUT:
            raise RuntimeError('Root output root is unsafe; preserve it and HOLD')
    canonical = b'version=0.2.1\nbuild=120\n'
    if git('show', SOURCE + ':release-version.properties') != canonical or (ROOT / 'release-version.properties').read_bytes() != canonical:
        raise RuntimeError('canonical release version must be exactly 0.2.1 / 120')
    cargo = tomllib.loads(git('show', SOURCE + ':Cargo.toml').decode())
    if cargo['workspace']['package']['version'] != '0.2.1':
        raise RuntimeError('Cargo/CLI semantic version differs from canonical release')
    for rel, markers in {
        'desktop/app/build.rs': ('release-version.properties', 'CARGO_PKG_VERSION', 'WAYPAIR_RELEASE_BUILD'),
        'desktop/app/src/bridge/app.rs': ('CARGO_PKG_VERSION', 'WAYPAIR_RELEASE_BUILD'),
    }.items():
        text = git('show', SOURCE + ':' + rel).decode()
        if any(marker not in text for marker in markers):
            raise RuntimeError('desktop metadata is not canonically bound: ' + rel)
    baseline52 = PINS['baseline52']
    if baseline52.get('source') != '01ff613ab54041d8d73936b208a06921273da998' or baseline52.get('tree') != 'f6a2b701a56ac61df56e41869918bbf71a75f985' or baseline52.get('build') != 52 or baseline52.get('channel') != '0.1.4-local.24':
        raise RuntimeError('historical Mac52 source identity changed')
    if baseline52.get('manifestSha256') != '03912def664cb1b2993df0da86e40667f23666a52e87e638db5cc668633a9dc5' or sha(baseline52['manifestPath']) != baseline52['manifestSha256']:
        raise RuntimeError('historical Mac52 manifest changed')
    subprocess.run(['git', '-C', str(ROOT), 'merge-base', '--is-ancestor', baseline52['source'], SOURCE], check=True)
    baseline119 = PINS['baseline119']
    if baseline119.get('source') != '078be482b0ba0be46fa9096cb6b7282208de0a51' or baseline119.get('tree') != '14337f3a6c71aaa621a1c6c5f5e7d4d505a83d74' or baseline119.get('build') != 119 or baseline119.get('channel') != '0.2.0':
        raise RuntimeError('historical Mac119 source identity changed')
    if baseline119.get('manifestSha256') != 'b6343239cb505cc0cdcc56e94ff30022c3df65cbd853f24148236fb7bb3d48d3' or sha(baseline119['manifestPath']) != baseline119['manifestSha256']:
        raise RuntimeError('historical Mac119 signed manifest changed')
    subprocess.run(['git', '-C', str(ROOT), 'merge-base', '--is-ancestor', baseline119['source'], SOURCE], check=True)
    return PINS
