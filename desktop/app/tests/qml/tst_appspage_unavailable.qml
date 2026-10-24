// SPDX-License-Identifier: GPL-3.0-or-later
import QtQuick
import QtQuick.Window
import QtTest
import app.nectarlink

// Loads production AppsPage inside a visible top-level test Window. The app/controller
// adapters are passive and only supply connection/capability values.
TestCase {
    id: testCase
    name: "AppsPage unavailable accessibility"
    when: windowShown

    property var pageComponent
    property var pageHost
    property var appsPage

    function init() {
        AppController.capsRevision = 1
        AppController.appFeatureStates = ({})
        Mirror.appWindowsSupported = true
        Mirror.settingsQualificationSupported = false
        Mirror.appsDevice = ""
        Mirror.appsState = ""
        Mirror.apps = "[]"
        Mirror.recentApps = "[]"
        Mirror.appsError = ""
        Mirror.windows = "[]"
    }

    function createVisiblePage(featureState) {
        AppController.appFeatureStates = ({ "phone-1|mirroring.app_windows": featureState })
        pageComponent = Qt.createComponent(Qt.resolvedUrl("../../qml/pages/AppsPage.qml"))
        compare(pageComponent.status, Component.Ready, pageComponent.errorString())
        pageHost = Qt.createQmlObject(
            'import QtQuick.Window; Window { width: 480; height: 640; visible: true }',
            testCase,
            "appsPageTestHost")
        verify(pageHost !== null)
        tryVerify(() => pageHost.visible && pageHost.contentItem !== null)
        appsPage = pageComponent.createObject(pageHost.contentItem, {
            deviceId: "phone-1",
            deviceName: "Pixel",
            width: 480,
            height: 640
        })
        verify(appsPage !== null)
        return appsPage
    }

    function cleanup() {
        if (appsPage) appsPage.destroy()
        appsPage = null
        pageComponent = null
        if (pageHost) pageHost.destroy()
        pageHost = null
        AppController.appFeatureStates = ({})
        Mirror.appsDevice = ""
        Mirror.appsState = ""
        Mirror.apps = "[]"
        Mirror.recentApps = "[]"
        Mirror.windows = "[]"
    }

    function test_unavailable_connection_reason_is_neutral_and_accessible() {
        const page = createVisiblePage({ state: "unsupported", reason: "" })
        const placeholder = findChild(page, "appsUnavailablePlaceholder")
        const heading = findChild(page, "appsUnavailableHeading")
        const reason = findChild(page, "appsUnavailableReason")
        verify(pageHost.visible && page.visible && placeholder.visible)
        compare(heading.text, "Open Pixel's apps here")
        compare(reason.text, "App windows are unavailable for this connection.")
        verify(reason.text.indexOf("Wireless debugging") < 0)
        compare(heading.Accessible.role, Accessible.StaticText)
        compare(reason.Accessible.role, Accessible.StaticText)
        compare(heading.Accessible.name, heading.text)
        compare(reason.Accessible.name, reason.text)
        verify(!heading.Accessible.ignored)
        verify(!reason.Accessible.ignored)
    }

    function test_hidden_placeholder_is_not_announced_after_capability_changes() {
        const page = createVisiblePage({ state: "unsupported", reason: "" })
        const placeholder = findChild(page, "appsUnavailablePlaceholder")
        const icon = findChild(page, "appsUnavailableIcon")
        const lockChip = findChild(page, "appsUnavailableLockChip")
        const heading = findChild(page, "appsUnavailableHeading")
        const reason = findChild(page, "appsUnavailableReason")
        const settingsGuidance = findChild(page, "appsUnavailableSettingsGuidance")
        const settingsButton = findChild(page, "appsUnavailableSettingsButton")
        verify(pageHost.visible && placeholder.visible && !reason.Accessible.ignored)

        // The same production page changes to the normal Apps content state.
        AppController.appFeatureStates = ({ "phone-1|mirroring.app_windows": { state: "available", reason: "" } })
        tryVerify(() => page.available && !placeholder.visible)
        verify(placeholder.Accessible.ignored)
        verify(icon.Accessible.ignored)
        verify(lockChip.Accessible.ignored)
        verify(heading.Accessible.ignored)
        verify(reason.Accessible.ignored)
        verify(settingsGuidance.Accessible.ignored)
        verify(settingsButton.Accessible.ignored)
    }
}
