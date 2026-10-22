"""Parameterized Mac50 Root guards. Private paths/pins come from Root inputs."""
from pathlib import Path
import hashlib
import json
import os
import re
import subprocess
import sys

ROOT = Path(os.environ['WAYPAIR_REPO_ROOT']).resolve(strict=True)
OUTPUT = Path(os.environ['MAC50_OUTPUT_ROOT']).resolve()
SOURCE = os.environ['MAC50_SOURCE_COMMIT']
TREE = os.environ['MAC50_SOURCE_TREE']
PARENT = os.environ['MAC50_PARENT_COMMIT']
PARENT_TREE = os.environ['MAC50_PARENT_TREE']
INPUTS = Path(os.environ['MAC50_ROOT_INPUTS']).resolve(strict=True)
PINS = json.loads(INPUTS.read_text())
INPUTS_SHA256 = os.environ['MAC50_ROOT_INPUTS_SHA256']
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
    for name, value in (('MAC50_SOURCE_COMMIT', SOURCE), ('MAC50_SOURCE_TREE', TREE),
                        ('MAC50_PARENT_COMMIT', PARENT), ('MAC50_PARENT_TREE', PARENT_TREE)):
        if re.fullmatch(r'[0-9a-f]{40}', value) is None:
            raise RuntimeError(name + ' must be an externally sealed full Git id')
    if sha(INPUTS) != INPUTS_SHA256 or re.fullmatch(r'[0-9a-f]{64}', INPUTS_SHA256) is None:
        raise RuntimeError('external Root input seal changed after binding')
    if PINS.get('build') != 50 or PINS.get('channel') != '0.1.4-local.22':
        raise RuntimeError('Mac50/local.22 identity pin mismatch')
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
    # Preserve the Mac49 baseline and frozen historical per-file bytes.
    baseline49 = PINS['baseline49']
    expected49 = {'source':'289088db2f1e75403bae7e166a618a5e83888c0a','tree':'a4c882af69e62ab01ce303fb9ea26c174935c33f','build':49,'channel':'0.1.4-local.21','manifestSha256':'635e38d24091562ac05b49efbfa571385b10e2cdc4f75856ccddb4337edc7102','sourceArchiveSha256':'33a3ec7fed5ffbb7f55da832a2ca34aa0eba3282ed610b5a7881bbb1f3373f3b','unsignedBinarySha256':'46ecdc74cd6b5f85899acf32a9cd5f7f174ad3de592435411d95e9891a66d681','dmgSha256':'a2cc3c02dbc3f78d964debc8fd3e9f4f25b1c62db8b7feae9c6e1ca840cb16af','dmgBytes':84991188,'minimumSystemVersion':'27.0'}
    for key, value in expected49.items():
        actual_key = 'manifestSha256' if key == 'manifestSha256' else key
        if baseline49.get(actual_key) != value:
            raise RuntimeError('Mac49 baseline pin changed: ' + key)
    manifest = Path(os.environ['MAC50_MAC49_MANIFEST']).resolve(strict=True)
    if sha(manifest) != expected49['manifestSha256']:
        raise RuntimeError('frozen Mac49 signed bundle manifest mismatch')
    if not (manifest.is_file() and not manifest.is_symlink()):
        raise RuntimeError('Mac49 baseline manifest is not a regular file')
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
        ('MAC50_QMAKE', 'retainedInputs', 'qmake6', True),
        ('MAC50_MACDEPLOYQT', 'retainedInputs', 'macdeployqt', True),
        ('MAC50_LRELEASE', 'retainedInputs', 'lrelease', True),
        ('MAC50_UPDATE_SIGNER', 'retainedInputs', 'mac-update-signing', False),
        ('MAC50_SIGN_MAC42', 'retainedInputs', 'sign-mac42-bundle.py', False),
        ('MAC50_SIGNER_REFERENCE', 'frozenOperatorPins', 'sign-normal-mac46.py', False),
        ('MAC50_ADB_SEAL', 'retainedInputs', 'adb-seal.json', False),
    ):
        validate_selected_file(env_name, group, basename, allow_symlink)
    retained = PINS['retainedPins']
    if retained.get('bundleId') != 'com.waypair.mac' or retained.get('team') != '5H4D7574Z4' or retained.get('leafSha1') != '80f731ee19e02914c4634d57e8f7b1adddd63fd2':
        raise RuntimeError('bundle identity/signing pins changed')
    if retained.get('minOS') != PINS['deploymentPolicy']['minimumSystemVersion'] or retained.get('minOS') != '27.0':
        raise RuntimeError('minimum macOS must remain 27.0')
    # Existing exact source pins remain guards for previously reviewed Mac49/Mac50 inputs.
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
    return PINS
