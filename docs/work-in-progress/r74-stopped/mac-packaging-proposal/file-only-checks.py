#!/usr/bin/env python3
"""Inspect only the five public predecessor templates and this proposal's text.
Never execute the binder/assembler, follow embedded paths, or inspect artifacts.
"""
import ast
from collections import Counter
import hashlib
import json
from pathlib import Path
import re

HERE = Path(__file__).absolute().parent
PREDECESSOR = Path('/Users/AleksandrSlobodanyuk/orca/tmp/waypair-install-R66-20261026/reports/mac139-packaging')
MAPPING = {
    'bind-root-source.py': 'bind-root-source.py',
    'assemble-r71.template.py': 'assemble-r74.template.py',
    'CONFIG.template.json': 'CONFIG.template.json',
    'ROOT-SOURCE-DEFINITION.template.json': 'ROOT-SOURCE-DEFINITION.template.json',
    'RETAINED-SOURCE-PINS.json': 'RETAINED-SOURCE-PINS.json',
}
EXTRA_FEATURES = {'desktop/app/src/transfers.rs', 'desktop/app/src/bridge/transfers.rs'}


def text(path):
    assert path.is_file() and not path.is_symlink(), 'public text file required'
    return path.read_text()


def assignment(tree, name):
    return next(n.value for n in tree.body if isinstance(n, ast.Assign) and any(isinstance(t, ast.Name) and t.id == name for t in n.targets))


def adapted_string(value):
    for old, new in [('Mac139', 'Mac142'), ('MAC139', 'MAC142'), ('HOST139', 'HOST142'), ('R71', 'R74'), ('build139', 'build142'), ('assemble-r71.template.py', 'assemble-r74.template.py'), ('Waypair-Link-local-139-source.tar.gz', 'Waypair-Link-local-142-source.tar.gz')]:
        value = value.replace(old, new)
    value = value.replace('required R74 font/phone/OneUI/build/QML/token source absent', 'required R74 transfer/font/phone/OneUI/build/QML/token source absent')
    value = value.replace('exact current font/phone/OneUI/build/QML source pin set required', 'exact current transfer/font/phone/OneUI/build/QML source pin set required')
    value = value.replace('current font/phone/OneUI/build/QML feature pins missing', 'current transfer/font/phone/OneUI/build/QML feature pins missing')
    return value.replace('current font/phone/OneUI/build/QML feature source mismatch', 'current transfer/font/phone/OneUI/build/QML feature source mismatch')


class ApprovedLabels(ast.NodeTransformer):
    def visit_Constant(self, node):
        if isinstance(node.value, str):
            node.value = '142' if node.value == '139' else adapted_string(node.value)
        elif type(node.value) is int and node.value == 139:
            node.value = 142
        return node


def main():
    old = {name: text(PREDECESSOR / name) for name in MAPPING}
    new = {name: text(HERE / target) for name, target in MAPPING.items()}
    checks = []
    for name in ('bind-root-source.py', 'assemble-r71.template.py'):
        before, after = ast.parse(old[name]), ast.parse(new[name])
        if name == 'bind-root-source.py':
            previous = ast.literal_eval(assignment(before, 'REQUIRED_FEATURES'))
            current = ast.literal_eval(assignment(after, 'REQUIRED_FEATURES'))
            assert current - previous == EXTRA_FEATURES and previous <= current
            assignment(before, 'REQUIRED_FEATURES').elts = assignment(after, 'REQUIRED_FEATURES').elts
        expected = ApprovedLabels().visit(before)
        assert ast.dump(expected) == ast.dump(after), 'unexpected syntax/guard/operation change: ' + name
        checks.append('Python AST parses; full AST differs only by approved labels/build and exact two feature additions: ' + MAPPING[name])
        assert Counter(re.findall(r'[0-9a-f]{64}', old[name])) == Counter(re.findall(r'[0-9a-f]{64}', new[name])), 'retained literal hash changed'
        assert not re.search(r'Mac139|MAC139|HOST139|build139|R71|r71|local-139-source', new[name])
    previous_config, config = json.loads(old['CONFIG.template.json']), json.loads(new['CONFIG.template.json'])
    previous_config['cohort'] = 'NORMAL_HOST_ONLY_MAC142_R74_SUCCESSOR'
    previous_config['sourceArchive']['path'] = 'ROOT_REQUIRED_NEW_R74_SOURCE_ARCHIVE_ABSOLUTE_PATH'
    assert config == previous_config, 'config changed beyond cohort/archive placeholder'
    previous_definition, definition = json.loads(old['ROOT-SOURCE-DEFINITION.template.json']), json.loads(new['ROOT-SOURCE-DEFINITION.template.json'])
    previous_definition.update(cohort='MAC142_R74_ROOT_SOURCE_DEFINITION', build=142, sourceFeatureSHA256='ROOT_REQUIRED_EXACT_MAP_OF_REQUIRED_R74_TRANSFER_RUST_QML_BUILD_PREFS_CANONICAL_TOKENS_INTER_OFL_AND_ALL_FONTS_PHONES_ONEUI_PATH_TO_SHA256')
    assert definition == previous_definition, 'source definition changed beyond metadata/placeholders'
    assert definition['source'] == 'ROOT_REQUIRED_NEW_COMMIT_40_HEX' and definition['tree'] == 'ROOT_REQUIRED_NEW_TREE_40_HEX'
    assert old['RETAINED-SOURCE-PINS.json'] == new['RETAINED-SOURCE-PINS.json']
    checks.append('JSON parses; predecessor paths, proofs, hashes, descriptors and retained pins unchanged; fresh evidence remains placeholders')
    assembler = ast.parse(new['assemble-r71.template.py'])
    bind_names = ['ROOT_SOURCE_DEFINITION', 'ROOT_SOURCE_DEFINITION_SHA256', 'EXPECTED_SOURCE', 'EXPECTED_TREE', 'EXPECTED_SOURCE_MANIFEST', 'EXPECTED_SETTINGS', 'EXPECTED_SOURCE_COUNT', 'SOURCE_FILE_PINS', 'AUDIT_SCRIPT_PINS', 'REPO_RESOURCE_PINS', 'SOURCE_FEATURE_PINS', 'EXPECTED_ADDED_QML_IMPORTS']
    assert all(ast.literal_eval(assignment(assembler, name)) is None for name in bind_names)
    assert all(new['assemble-r71.template.py'].count(name + ' = None') == 1 for name in bind_names)
    checks.append('All 12 Root binding markers remain unbound and unique; no new source/tree OID or receipt is invented')
    # A static negative check: every predecessor guard condition remains present
    # in the normalized full AST above; the unbound recipe still fails closed.
    assert "require(ROOT_SOURCE_DEFINITION is not None, 'unbound source template: run bind-root-source.py first')" in new['assemble-r71.template.py']
    checks.append('Failure admission retained: unbound assembly is rejected before reading config or donor data (source inspection only)')
    report = {
        'status': 'FILE_ONLY_CHECKS_COMPLETED_SOURCE_PROPOSAL_ONLY',
        'checks': checks,
        'featurePathsAdded': sorted(EXTRA_FEATURES),
        'freshMac142SourceBinding': 'ROOT_PENDING',
        'freshMac142SDKBuild': 'ROOT_PENDING',
        'freshMac142StaticDonorImportProof': 'ROOT_PENDING',
        'strictCICleanClaimed': False,
        'strictBaselineContextFromRootInstructionNotReexecuted': {'coreLintFailures': 6, 'formatPaths': 20, 'disposition': 'RETAINED_UNRESOLVED_NOT_BLANKET_PASS'},
        'binderOrAssemblerExecuted': False,
        'protectedEmbeddedPathsFollowed': False,
        'files': {target: hashlib.sha256((HERE / target).read_bytes()).hexdigest() for target in MAPPING.values()},
    }
    (HERE / 'FILE-ONLY-CHECKS.json').write_text(json.dumps(report, indent=2) + '\n')
    print(report['status'])


if __name__ == '__main__':
    main()
