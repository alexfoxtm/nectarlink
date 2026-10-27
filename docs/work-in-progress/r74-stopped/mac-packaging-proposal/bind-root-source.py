#!/usr/bin/env python3
"""Root-only offline generation: bind a sealed Mac142 definition and source manifest.
Reads only its own public templates/pins and the two explicitly sealed JSON inputs.
Does not compile, package, inspect donor/artifacts/profiles, or invoke any OS tool.
"""
import argparse
import ast
import hashlib
import json
import re
from pathlib import Path, PurePosixPath

HERE = Path(__file__).absolute().parent
DEFINITION_KEYS = {'schema', 'cohort', 'source', 'tree', 'version', 'build', 'sourceFileCount', 'currentSourceFiles', 'settingsPageSHA256', 'sourceFileSHA256', 'publicAuditScriptSHA256', 'repositoryResources', 'sourceFeatureSHA256', 'approvedAddedQmlImports'}
REQUIRED_FEATURES = {'desktop/app/src/transfers.rs', 'desktop/app/src/bridge/transfers.rs', 'desktop/app/qml/IslandWindow.qml', 'desktop/app/qml/pages/PhotosPage.qml', 'desktop/app/qml/components/CallCard.qml', 'desktop/app/qml/pages/SettingsPage.qml', 'desktop/app/src/settings.rs', 'desktop/app/qml/components/Icon.qml', 'desktop/app/src/bridge/prefs.rs', 'desktop/app/qml/MirrorWindow.qml', 'desktop/app/qml/components/Segmented.qml', 'desktop/app/qml/components/ListRow.qml', 'desktop/app/qml/Icons.qml', 'desktop/app/qml/components/GlassRim.qml', 'desktop/app/qml/components/Button.qml', 'desktop/app/qml/Theme.qml', 'desktop/app/qml/components/PhoneModelPortrait.qml', 'desktop/app/qml/pages/MessagesPage.qml', 'desktop/app/build.rs', 'assets/fonts/Inter-Variable.ttf', 'desktop/app/qml/components/IconButton.qml', 'desktop/app/qml/components/Chip.qml', 'desktop/app/qml/components/Txt.qml', 'desktop/app/qml/components/NowPlaying.qml', 'desktop/app/qml/components/Card.qml', 'desktop/app/qml/components/Sheet.qml', 'assets/fonts/OFL-Inter.txt', 'desktop/app/qml/components/MaterialBackdrop.qml', 'desktop/app/qml/components/CommandPalette.qml', 'docs/design/tokens.json', 'desktop/app/qml/components/NavItem.qml', 'desktop/app/qml/MainWindow.qml', 'desktop/app/qml/Tokens.qml', 'desktop/app/qml/pages/HomePage.qml'}


def require(value, reason):
    if not value:
        raise ValueError(reason)


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def document(path):
    def pairs(rows):
        result = {}
        for key, value in rows:
            require(key not in result, 'duplicate JSON key: ' + key)
            result[key] = value
        return result
    return json.loads(path.read_text(), object_pairs_hook=pairs)


def canonical_rel(value):
    require(type(value) is str and bool(value) and '\\' not in value and '\x00' not in value, 'invalid relative path')
    p = PurePosixPath(value)
    require(not p.is_absolute() and not any(part in ('', '.', '..') for part in value.split('/')) and p.as_posix() == value, 'noncanonical/traversal relative path')
    return value


def sealed(item):
    require(type(item) is dict and set(item) == {'path', 'sha256'}, 'sealed file schema mismatch')
    p = Path(item['path'])
    require('..' not in p.parts and p.is_absolute() and p.is_file() and not any(x.is_symlink() for x in [p, *p.parents]), 'absolute regular sealed file without traversal/symlink ancestors required')
    require(type(item['sha256']) is str and re.fullmatch('[0-9a-f]{64}', item['sha256']) and sha(p) == item['sha256'], 'sealed JSON changed')
    return p


def feature_paths(files):
    # Exact complete current feature directories; the complete source manifest
    # still seals every remaining file, including all Android adaptations.
    return REQUIRED_FEATURES | {name for name in files if name.startswith(('assets/fonts/', 'assets/ui/phones/', 'assets/ui/oneui/'))}


def resource_paths(files):
    return {'LICENSE'} | {name for name in files if name.startswith(('LICENSES/', 'assets/fonts/OFL-'))}


def validate(definition, manifest):
    require(type(definition) is dict and set(definition) == DEFINITION_KEYS, 'Root source definition schema keys mismatch')
    require(definition['schema'] == 1 and definition['cohort'] == 'MAC142_R74_ROOT_SOURCE_DEFINITION', 'wrong Root source definition cohort')
    require(all(type(definition[field]) is str and re.fullmatch('[0-9a-f]{40}', definition[field]) for field in ('source', 'tree')), 'actual frozen source and tree required')
    require(definition['version'] == '0.2.4' and type(definition['build']) is int and definition['build'] == 142, 'only version0.2.4 build142 admitted')
    require(type(definition['sourceFileCount']) is int and definition['sourceFileCount'] > 0, 'actual positive source-file count required')
    require(type(manifest) is dict and set(manifest) == {'schema', 'source', 'tree', 'files'} and manifest['schema'] == 1, 'complete source manifest schema mismatch')
    require((manifest['source'], manifest['tree']) == (definition['source'], definition['tree']), 'source manifest source/tree mismatch')
    files = manifest['files']
    require(type(files) is dict and len(files) == definition['sourceFileCount'], 'complete source manifest file count mismatch')
    for name, digest in files.items():
        canonical_rel(name)
        require(type(digest) is str and re.fullmatch('[0-9a-f]{64}', digest), 'source manifest invalid SHA256')
    for name in files:
        require(not any(str(parent) in files for parent in PurePosixPath(name).parents if str(parent) != '.'), 'source manifest file/directory path collision')
    retained = document(HERE / 'RETAINED-SOURCE-PINS.json')
    for field in ('sourceFileSHA256', 'publicAuditScriptSHA256'):
        require(definition[field] == retained[field], 'retained dependency/notice/audit inputs changed: use newly qualified closure lane')
        require(all(files.get(name) == digest for name, digest in definition[field].items()), 'retained source pins differ from new frozen manifest')
    require(definition['settingsPageSHA256'] == files.get('desktop/app/qml/pages/SettingsPage.qml'), 'current Settings source pin mismatch')
    features = feature_paths(files)
    require(all(name in files for name in features), 'required R74 transfer/font/phone/OneUI/build/QML/token source absent')
    require(any(name.startswith('assets/ui/oneui/') for name in features), 'new OneUI source assets absent')
    require(definition['sourceFeatureSHA256'] == {name: files[name] for name in sorted(features)}, 'exact current transfer/font/phone/OneUI/build/QML source pin set required')
    resources = resource_paths(files)
    expected_resources = {name: {'destination': 'Contents/Resources/LICENSE.txt' if name == 'LICENSE' else 'Contents/Resources/LICENSES/' + (name.removeprefix('LICENSES/') if name.startswith('LICENSES/') else PurePosixPath(name).name), 'sha256': files[name]} for name in sorted(resources)}
    require(definition['repositoryResources'] == expected_resources, 'exact complete current repository-license and font-notice map required')
    require('LICENSES/Phone-Model-Image-Attribution.txt' in resources and any(name.startswith('LICENSES/') and 'oneui' in name.lower() for name in resources), 'current Phone attribution and OneUI notice required')
    destinations = [record['destination'] for record in expected_resources.values()]
    require(len(destinations) == len(set(destinations)), 'colliding repository resource destinations')
    require(definition['approvedAddedQmlImports'] == ['QtQuick.Controls.Basic'], 'only Basic is added relative to immutable133; Dialogs is already in the baseline')


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--binding', type=Path, required=True)
    ap.add_argument('--binding-sha256', required=True)
    ap.add_argument('--output', type=Path, required=True)
    a = ap.parse_args()
    # Reject lexical traversal and all symlink ancestors before any mutation.
    require('..' not in a.output.parts and a.output.is_absolute() and not a.output.exists() and not a.output.is_symlink(), 'fresh absolute output without traversal required')
    require(a.output.parent.is_dir() and not any(p.is_symlink() for p in [a.output.parent, *a.output.parent.parents]), 'existing regular output parent without symlink ancestors required')
    binding_path = sealed({'path': str(a.binding), 'sha256': a.binding_sha256})
    definition = document(binding_path)
    manifest = document(sealed(definition['currentSourceFiles']))
    validate(definition, manifest)
    template_path = HERE / 'assemble-r74.template.py'
    require(not template_path.is_symlink(), 'template symlink refused')
    template = template_path.read_text()
    values = {'ROOT_SOURCE_DEFINITION': definition, 'ROOT_SOURCE_DEFINITION_SHA256': a.binding_sha256, 'EXPECTED_SOURCE': definition['source'], 'EXPECTED_TREE': definition['tree'], 'EXPECTED_SOURCE_MANIFEST': definition['currentSourceFiles']['sha256'], 'EXPECTED_SETTINGS': definition['settingsPageSHA256'], 'EXPECTED_SOURCE_COUNT': definition['sourceFileCount'], 'SOURCE_FILE_PINS': definition['sourceFileSHA256'], 'AUDIT_SCRIPT_PINS': definition['publicAuditScriptSHA256'], 'REPO_RESOURCE_PINS': definition['repositoryResources'], 'SOURCE_FEATURE_PINS': definition['sourceFeatureSHA256'], 'EXPECTED_ADDED_QML_IMPORTS': definition['approvedAddedQmlImports']}
    for key, value in values.items():
        needle = key + ' = None'
        require(template.count(needle) == 1, 'template binding marker absent/ambiguous: ' + key)
        template = template.replace(needle, key + ' = ' + repr(value))
    ast.parse(template)
    with a.output.open('x') as out:
        out.write(template)
    print(json.dumps({'status': 'ROOT_GENERATED_NOT_EXECUTED', 'recipe': str(a.output), 'recipeSHA256': sha(a.output), 'rootSourceDefinitionSHA256': a.binding_sha256, 'source': definition['source'], 'tree': definition['tree'], 'sourceFileCount': definition['sourceFileCount'], 'currentSourceFilesSHA256': definition['currentSourceFiles']['sha256']}, indent=2))


if __name__ == '__main__':
    try:
        main()
    except (ValueError, KeyError, TypeError, OSError, json.JSONDecodeError) as error:
        raise SystemExit('MAC142_SOURCE_BINDING_HOLD: ' + str(error))
