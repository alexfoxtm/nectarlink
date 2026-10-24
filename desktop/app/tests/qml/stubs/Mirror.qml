// SPDX-License-Identifier: GPL-3.0-or-later
pragma Singleton
import QtQuick

// Passive Mirror bridge for QML component tests; no backend or device access.
QtObject {
    property string windows: "[]"
    property bool appWindowsSupported: false
    property bool settingsQualificationSupported: false
    property string appsDevice: ""
    property string appsState: ""
    property string apps: "[]"
    property string recentApps: "[]"
    property string appsError: ""
    property bool muted: false
    property var calls: []

    function record(name, args) {
        calls = calls.concat([[name].concat(args)])
    }
    function dropFiles(urls) { record("dropFiles", [urls]) }
    function enterWindow(key) { record("enterWindow", [key]) }
    function leaveWindow() { record("leaveWindow", []) }
    function paste(key) { record("paste", [key]) }
    function press(key, name) { record("press", [key, name]) }
    function requestControl(key) { record("requestControl", [key]) }
    function resize(key, width, height) { record("resize", [key, width, height]) }
    function saveGeometry(key, x, y, width, height, userResized) { record("saveGeometry", [key, x, y, width, height, userResized]) }
    function screenshotClipboard(key) { record("screenshotClipboard", [key]) }
    function screenshotFile(key) { record("screenshotFile", [key]) }
    function scroll(key, x, y, dx, dy) { record("scroll", [key, x, y, dx, dy]) }
    function setScreenOff(key, on) { record("setScreenOff", [key, on]) }
    function setStayAwake(key, on) { record("setStayAwake", [key, on]) }
    function stop(key) { record("stop", [key]) }
    function syncClipboardOnFocus(key) { record("syncClipboardOnFocus", [key]) }
    function text(key, value) { record("text", [key, value]) }
    function toggleRecording(key) { record("toggleRecording", [key]) }
    function toggleSound() { record("toggleSound", []) }
    function touch(key, phase, x, y) { record("touch", [key, phase, x, y]) }
    function reopen(key) { record("reopen", [key]) }
}
