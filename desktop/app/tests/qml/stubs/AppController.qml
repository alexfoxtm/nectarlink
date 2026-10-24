// SPDX-License-Identifier: GPL-3.0-or-later
pragma Singleton
import QtQuick

// Stands in for the Rust AppController in QML tests: still, light, no
// phone or wallpaper colors.
QtObject {
    property bool systemDark: false
    property bool reduceMotion: true
    property bool powerSaver: false
    property string phoneColors: ""
    property string wallpaperColors: ""
    property string currentPage: "home"
    property int capsRevision: -1
    property var appFeatureStates: ({})
    property string settingsSection: ""
    property bool doctorBusy: false
    property var doctorChecks: []
    property var commandPaletteResults: []
    property double commandPaletteOpenMs: 0
    function featureState(deviceId, feature) { return appFeatureStates[deviceId + "|" + feature] || ({}) }
    function closeCommandPalette() {}
    function noteCommandPaletteShown() {}
    function runCommandPalette(id) {}
    function searchCommandPalette(query) {}
    function runDoctor() {}
    function doctorFix(id) {}
    function wifiNetworks() { return "[]" }
    function hotspotNetwork(device) { return "" }
    function setHotspotNetwork(device, network) {}
    function joinPhoneHotspot(device) {}
    function cancelPhoneHotspotJoin(device) {}
    function phoneHotspotJoining(device) { return false }
    function requestWifiDiscoveryPermission() {}
}
