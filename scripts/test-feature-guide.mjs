// SPDX-License-Identifier: GPL-3.0-or-later
// Runs the actual QML catalogue/status functions with disposable model values.
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { test } from 'node:test';
import vm from 'node:vm';

const qml = readFileSync(new URL('../desktop/app/qml/pages/FeaturesGuidePage.qml', import.meta.url), 'utf8');
const core = readFileSync(new URL('../core/nectarlink-core/src/features.rs', import.meta.url), 'utf8');
const catalogue = qml.match(/readonly property var allFeatures:\s*(\[[\s\S]*?\n    \])/)[1];
const functionSource = name => {
    const match = qml.match(new RegExp(`    function ${name}\\([^)]*\\) \\{[\\s\\S]*?\\n    \\}`));
    assert.ok(match, `actual QML function ${name} must exist`);
    return match[0].trim();
};
function fixture(onMac = true) {
    const guide = { onMac, deviceId: 'synthetic-phone', hasConnectedPhone: true,
        deviceTogglesMap: { notifications: true, clipboard: true, files: true,
            messages: true, photos: true, calls: true, media: true, mirroring: true } };
    const context = vm.createContext({ guide, Qt: { platform: { os: onMac ? 'osx' : 'windows' } },
        Icons: new Proxy({}, { get: (_, key) => String(key) }), qsTr: text => text,
        Preferences: {}, AppController: { capsRevision: 1, togglesRevision: 1,
            featureState: () => ({ state: 'available' }), deviceToggles: () => [] },
        DeviceList: { count: 1, rowOf: () => 0, deviceOnlineAt: () => true } });
    return { context, guide, items: vm.runInContext(catalogue, context),
        status: vm.runInContext(`(${functionSource('statusForFeature')})`, context),
        sync: vm.runInContext(`(${functionSource('syncSelectedDevice')})`, context) };
}

test('catalogue uses real core IDs and distinguishes available/basic/local features', () => {
    const f = fixture();
    const ids = new Set([...core.matchAll(/id:\s*"([a-z_.]+)"/g)].map(m => m[1]));
    for (const item of f.items.filter(item => item.capKey)) {
        assert.ok(ids.has(item.capKey), `${item.id}: unknown core ID ${item.capKey}`);
    }
    const sms = f.items.find(item => item.id === 'messages');
    assert.equal(f.status(sms).label, 'Ready');
    f.context.AppController.featureState = () => ({ state: 'partial' });
    assert.equal(f.status(sms).label, 'Basic mode');
    assert.equal(f.status(f.items.find(item => item.id === 'deck')).label, 'Local controls only');
    const windows = fixture(false);
    assert.equal(windows.status(windows.items.find(item => item.id === 'messages')).label, 'Ready');
});

test('locked, missing, unknown and offline features never falsely report ready', () => {
    const f = fixture();
    const sms = f.items.find(item => item.id === 'messages');
    f.context.AppController.featureState = () => ({ state: 'locked', action: 'grantPermission', target: 'sms' });
    assert.equal(f.status(sms).label, 'Needs phone permission');
    f.context.AppController.featureState = () => ({ state: 'unknown' });
    assert.equal(f.status(sms).label, 'Check requirements');
    assert.equal(f.status(f.items.find(item => item.id === 'type_on_phone')).label, 'Not yet available on Mac');
    f.context.DeviceList.deviceOnlineAt = () => false;
    f.sync();
    assert.equal(f.context.deviceOnline, false, 'online state must come from the link, not a missing reason string');
    f.guide.hasConnectedPhone = false;
    assert.equal(f.status(sms).visible, false);
});
