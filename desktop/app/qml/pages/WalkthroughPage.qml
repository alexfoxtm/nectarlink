// SPDX-License-Identifier: GPL-3.0-or-later
import QtQuick
import QtQuick.Layouts
import app.nectarlink

// First-run setup walkthrough (7 steps): Welcome, Privacy, Install phone app,
// Pair, Choose what to share, Optional setup, and You are set.
FocusScope {
    id: walkthrough
    focus: true

    property bool active: true
    property string deviceId: ""
    property string deviceName: ""

    signal completed()
    signal openGuideRequested()
    signal doctorRequested()

    readonly property int totalSteps: 7
    // Matches the APK name on the GitHub release (see .github/workflows/release.yml).
    // The Waypair Android app is versioned on its own (e.g. 0.1.4-local.5), so the
    // Mac build can't know the file name and shows a placeholder to fill in.
    readonly property bool adbVersionPlaceholder: Qt.platform.os === "osx"
    readonly property string adbCommand: adbVersionPlaceholder
        ? "adb install -r Waypair-Link-<version>-android.apk"
        : ("adb install -r Nectarlink-" + AppController.version + "-android.apk")
    property int step: Math.max(0, Math.min(totalSteps - 1, Preferences.walkthroughStep))
    property alias currentStep: walkthrough.step

    // Primary paired device (if any) for Step 5 per-device toggles.
    property string activeDeviceId: ""
    property string activeDeviceName: ""
    readonly property bool hasPairedDevice: AppController.hasDevices || activeDeviceId !== ""

    property real pairJoinProgress: hasPairedDevice ? 1.0 : 0.35
    Behavior on pairJoinProgress {
        enabled: !Theme.reduceMotion
        NumberAnimation { duration: 420; easing.type: Easing.OutCubic }
    }

    onStepChanged: {
        if (Preferences.walkthroughStep !== step)
            Preferences.walkthroughStep = step
        if (bodyFlick)
            bodyFlick.contentY = 0
        if (honeycombCanvas)
            honeycombCanvas.requestPaint()
    }

    onHasPairedDeviceChanged: {
        if (honeycombCanvas)
            honeycombCanvas.requestPaint()
    }

    onPairJoinProgressChanged: {
        if (honeycombCanvas)
            honeycombCanvas.requestPaint()
    }

    Connections {
        target: Preferences
        function onWalkthroughStepChanged() {
            let clamped = Math.max(0, Math.min(walkthrough.totalSteps - 1, Preferences.walkthroughStep))
            if (walkthrough.step !== clamped)
                walkthrough.step = clamped
        }
        function onStartWithWindowsChanged() {
            if (honeycombCanvas)
                honeycombCanvas.requestPaint()
        }
    }

    property var deviceTogglesMap: ({
        notifications: true,
        clipboard: true,
        files: true,
        sms: true,
        calls: true,
        media: true,
        storage: false,
        remote_input: false,
        pc_mirror: false
    })

    readonly property var defaultOnLabels: [
        "Notifications",
        "Clipboard",
        "Send files",
        "Now playing",
        "Messages and Calls",
        "Photos",
        "Find my phone"
    ]

    readonly property var optInLabels: [
        "Screen mirroring",
        "Remote input",
        "Share PC folders"
    ]

    readonly property var stepTitles: [
        "Welcome to Waypair Link",
        "Your data stays on your devices",
        "Install Waypair Link on your phone",
        "Pair your phone and PC",
        "Choose what to share",
        "Optional setup",
        "You are set"
    ]

    readonly property var stepSubtitles: [
        Qt.platform.os === "osx"
            ? "Your Android phone and your Mac, working together on your local network."
            : "Your Android phone and your PC, working together on your local network.",
        "No accounts, no cloud servers, and no telemetry.",
        "Get the Android app from GitHub Releases or install the APK over USB.",
        "Scan the QR code from the Waypair Link Android app, or pick your phone from nearby devices.",
        "Safe features are on by default. Higher-trust access stays off until you turn it on.",
        Qt.platform.os === "osx"
            ? "Three quick checks so notifications and discovery work reliably on your Mac."
            : "Three quick checks so notifications and discovery work reliably on your PC.",
        "Reopen it from Settings or the Command Palette."
    ]

    function refreshDeviceToggles() {
        let _rev = AppController.togglesRevision
        let count = DeviceList.count
        if (count <= 0) {
            activeDeviceId = ""
            activeDeviceName = ""
            return
        }
        let id = DeviceList.deviceIdAt(0) || ""
        let name = DeviceList.deviceNameAt(0) || "Your phone"
        activeDeviceId = id
        activeDeviceName = name
        if (id === "")
            return
        let list = AppController.deviceToggles(id)
        let next = {
            notifications: true,
            clipboard: true,
            files: true,
            sms: true,
            calls: true,
            media: true,
            storage: false,
            remote_input: false,
            pc_mirror: false
        }
        for (let i = 0; i < list.length; i++) {
            let entry = list[i]
            if (entry && entry.name !== undefined)
                next[entry.name] = Boolean(entry.on)
        }
        deviceTogglesMap = next
        if (honeycombCanvas)
            honeycombCanvas.requestPaint()
    }

    function isDeviceToggleOn(name) {
        return Boolean(deviceTogglesMap[name])
    }

    function setDeviceToggleValue(name, on) {
        let next = Object.assign({}, deviceTogglesMap)
        next[name] = on
        deviceTogglesMap = next
        if (activeDeviceId !== "")
            AppController.setDeviceToggle(activeDeviceId, name, on)
        if (honeycombCanvas)
            honeycombCanvas.requestPaint()
    }

    readonly property var parsedDoctorChecks: {
        let raw = AppController.doctorChecks
        if (!raw || raw === "")
            return []
        try {
            return JSON.parse(raw)
        } catch (e) {
            return []
        }
    }

    readonly property bool doctorAllPassed: {
        if (parsedDoctorChecks.length === 0)
            return false
        for (let i = 0; i < parsedDoctorChecks.length; i++) {
            if (parsedDoctorChecks[i].status !== "ok")
                return false
        }
        return true
    }

    readonly property string doctorSummaryLabel: {
        if (AppController.doctorBusy)
            return "Checking firewall and local network..."
        if (parsedDoctorChecks.length === 0)
            return Qt.platform.os === "osx"
                ? "Run a quick check of the local network and discovery."
                : "Run a quick check to verify Windows Firewall and local discovery."
        let problems = 0
        for (let i = 0; i < parsedDoctorChecks.length; i++) {
            if (parsedDoctorChecks[i].status !== "ok")
                problems++
        }
        return problems === 0
            ? "All network and firewall checks passed."
            : (problems + (problems === 1 ? " item needs attention." : " items need attention."))
    }

    function goNext() {
        if (step < totalSteps - 1) {
            step = step + 1
        } else {
            finishWalkthrough()
        }
    }

    function goBack() {
        if (step > 0)
            step = step - 1
    }

    function skipWalkthrough() {
        Preferences.walkthroughCompleted = true
        walkthrough.completed()
    }

    function finishWalkthrough() {
        Preferences.walkthroughCompleted = true
        walkthrough.completed()
    }

    function finishAndOpenGuide() {
        Preferences.walkthroughCompleted = true
        walkthrough.openGuideRequested()
    }

    Component.onCompleted: {
        refreshDeviceToggles()
        AppController.refreshToastsEnabled()
    }

    Connections {
        target: AppController
        function onHasDevicesChanged() { walkthrough.refreshDeviceToggles() }
        function onTogglesRevisionChanged() { walkthrough.refreshDeviceToggles() }
        function onToastsEnabledChanged() {
            if (honeycombCanvas)
                honeycombCanvas.requestPaint()
        }
        function onDoctorChecksChanged() {
            if (honeycombCanvas)
                honeycombCanvas.requestPaint()
        }
    }

    Connections {
        target: Theme
        function onDarkChanged() {
            if (honeycombCanvas)
                honeycombCanvas.requestPaint()
        }
    }

    Keys.onPressed: event => {
        if (event.key === Qt.Key_Escape) {
            skipWalkthrough()
            event.accepted = true
        } else if (event.key === Qt.Key_Right && (event.modifiers & Qt.AltModifier)) {
            goNext()
            event.accepted = true
        } else if (event.key === Qt.Key_Left && (event.modifiers & Qt.AltModifier)) {
            goBack()
            event.accepted = true
        }
    }

    ColumnLayout {
        anchors.fill: parent
        anchors.leftMargin: 24
        anchors.rightMargin: 24
        anchors.topMargin: Theme.topBarHeight + 4
        anchors.bottomMargin: 24
        spacing: 16

        // Single top progress header: 7-segment progress bar + Skip setup action
        RowLayout {
            Layout.fillWidth: true
            spacing: 16

            RowLayout {
                id: progressTrack
                Layout.fillWidth: true
                spacing: 6
                Accessible.role: Accessible.ProgressBar
                Accessible.name: "Step " + (walkthrough.currentStep + 1) + " of " + walkthrough.totalSteps

                Repeater {
                    model: walkthrough.totalSteps
                    delegate: Rectangle {
                        id: progressStep
                        required property int index
                        Layout.fillWidth: true
                        Layout.preferredHeight: 6
                        implicitWidth: 64
                        implicitHeight: 6
                        radius: 3
                        color: index < walkthrough.step
                            ? Qt.rgba(Theme.primary.r, Theme.primary.g, Theme.primary.b, 0.55)
                            : (index === walkthrough.step ? Theme.primary : Theme.surfaceContainerHighest)

                        Behavior on color {
                            enabled: !Theme.reduceMotion
                            ColorAnimation { duration: Theme.fadeFast }
                        }

                        activeFocusOnTab: true
                        border.width: Theme.focusVisible(progressStep) ? 2 : 0
                        border.color: Theme.primary
                        Accessible.role: Accessible.Button
                        Accessible.onPressAction: walkthrough.step = index
                        Keys.onPressed: (event) => {
                            if (event.key === Qt.Key_Return || event.key === Qt.Key_Enter || event.key === Qt.Key_Space) {
                                walkthrough.step = index
                                event.accepted = true
                            }
                        }
                        Accessible.name: "Go to step " + (index + 1) + ": " + walkthrough.stepTitles[index]
                        MouseArea {
                            anchors.fill: parent
                            anchors.topMargin: -6
                            anchors.bottomMargin: -6
                            cursorShape: Qt.PointingHandCursor
                            onClicked: walkthrough.step = parent.index
                        }
                    }
                }
            }

            Button {
                text: qsTr("Skip setup")
                variant: "tonal"
                size: "sm"
                visible: walkthrough.step < walkthrough.totalSteps - 1
                Accessible.name: "Skip setup walkthrough"
                onClicked: walkthrough.skipWalkthrough()
            }
        }

        // Scrollable, vertically centered step content area
        Flickable {
            id: bodyFlick
            Layout.fillWidth: true
            Layout.fillHeight: true
            contentWidth: width
            contentHeight: Math.max(height, contentColumn.implicitHeight + 16)
            clip: true
            boundsBehavior: Flickable.StopAtBounds

            ColumnLayout {
                id: contentColumn
                width: Math.min(bodyFlick.width, 640)
                x: Math.floor((bodyFlick.width - width) / 2)
                y: Math.max(4, Math.floor((bodyFlick.height - implicitHeight) / 2))
                spacing: 24

                // Existing step illustration on a quiet, theme-aware hero surface.
                Rectangle {
                    Layout.fillWidth: true
                    Layout.preferredHeight: 148
                    implicitWidth: 640
                    implicitHeight: 148
                    radius: Theme.heroRadius
                    color: Theme.heroWash

                    Canvas {
                        id: honeycombCanvas
                        anchors.fill: parent
                        onPaint: {
                            const ctx = getContext("2d")
                            ctx.reset()
                            ctx.fillStyle = Qt.rgba(Theme.primary.r, Theme.primary.g, Theme.primary.b, 0.06).toString()
                            ctx.beginPath()
                            ctx.arc(width * 0.5, height * 0.5, 64, 0, Math.PI * 2)
                            ctx.fill()
                        }
                    }
                    Row {
                        anchors.centerIn: parent
                        spacing: 18
                        Rectangle {
                            width: 64; height: 64; radius: 10
                            anchors.verticalCenter: parent.verticalCenter
                            color: Qt.rgba(Theme.panelColor.r, Theme.panelColor.g, Theme.panelColor.b, 0.66)
                            Icon {
                                anchors.centerIn: parent
                                width: 27; height: 27
                                path: Icons.phone
                                color: Theme.primary
                            }
                        }
                        Rectangle {
                            width: 88; height: 88; radius: 12
                            color: Theme.panelColor
                            Icon {
                                anchors.centerIn: parent
                                width: 38; height: 38
                                path: [Icons.mark, Icons.shield, Icons.download, Icons.qr, Icons.check, Icons.settings, Icons.check][walkthrough.step]
                                stroke: 2
                                color: Theme.primary
                            }
                        }
                        Rectangle {
                            width: 64; height: 64; radius: 10
                            anchors.verticalCenter: parent.verticalCenter
                            color: Qt.rgba(Theme.panelColor.r, Theme.panelColor.g, Theme.panelColor.b, 0.66)
                            Icon {
                                anchors.centerIn: parent
                                width: 27; height: 27
                                path: Icons.laptop
                                color: Theme.primary
                            }
                        }
                    }
                }

                // Consistent Step Title and Subtitle block
                ColumnLayout {
                    Layout.fillWidth: true
                    spacing: 4

                    Txt {
                        Layout.fillWidth: true
                        text: walkthrough.stepTitles[walkthrough.step] || ""
                        role: "headline"
                        size: 30
                        wrapMode: Text.WordWrap
                    }

                    Txt {
                        Layout.fillWidth: true
                        text: walkthrough.stepSubtitles[walkthrough.step] || ""
                        role: "body"
                        color: Theme.surfaceContentVariant
                        wrapMode: Text.WordWrap
                    }
                }

                // Single Unified Card per step
                Card {
                    Layout.fillWidth: true
                    padding: 24
                    implicitHeight: stepCardBody.implicitHeight + padding * 2

                    ColumnLayout {
                        id: stepCardBody
                        width: parent.width
                        height: implicitHeight
                        spacing: 14

                        // STEP 0: Welcome
                        ColumnLayout {
                            Layout.fillWidth: true
                            spacing: 14
                            visible: walkthrough.step === 0

                            GridLayout {
                                Layout.fillWidth: true
                                columns: width >= 480 ? 2 : 1
                                columnSpacing: 12
                                rowSpacing: 12

                                Repeater {
                                    model: [
                                        {
                                            icon: Icons.bell,
                                            title: Qt.platform.os === "osx" ? qsTr("Notifications and Messages") : qsTr("Notifications and Calls"),
                                            body: Qt.platform.os === "osx"
                                                ? "See phone notifications and messages on your Mac when you enable access."
                                                : "Reply to notifications, read SMS threads, and answer calls from your PC."
                                        },
                                        {
                                            icon: Icons.send,
                                            title: qsTr("Clipboard and Send files"),
                                            body: "Copy text or images once, or drop files and folders at full Wi-Fi speed."
                                        },
                                        {
                                            icon: Icons.mirror,
                                            title: Qt.platform.os === "osx" ? qsTr("More Mac features in development") : qsTr("Mirroring, Camera, and Audio"),
                                            body: Qt.platform.os === "osx"
                                                ? "Screen mirroring, webcam, PC audio and call audio are not ready in this macOS port yet."
                                                : "Stream your phone screen, use your phone as a webcam, or route PC audio."
                                        },
                                        {
                                            icon: Icons.lock,
                                            title: qsTr("Direct local QUIC link"),
                                            body: "Everything runs directly between your devices with zero cloud accounts."
                                        }
                                    ]

                                    delegate: Rectangle {
                                        required property var modelData
                                        Layout.fillWidth: true
                                        implicitWidth: 260
                                        implicitHeight: tileCol.implicitHeight + 24
                                        radius: Theme.radiusMd
                                        color: Theme.surfaceContainerLow
                                        border.width: 1
                                        border.color: Theme.outlineVariant

                                        RowLayout {
                                            anchors.fill: parent
                                            anchors.margins: 12
                                            spacing: 12

                                            Rectangle {
                                                Layout.preferredWidth: 34
                                                Layout.preferredHeight: 34
                                                implicitWidth: 34
                                                implicitHeight: 34
                                                Layout.alignment: Qt.AlignTop
                                                radius: 10
                                                color: Theme.primaryContainer

                                                Icon {
                                                    anchors.centerIn: parent
                                                    path: modelData.icon
                                                    width: 18
                                                    height: 18
                                                    color: Theme.primaryContainerContent
                                                }
                                            }

                                            ColumnLayout {
                                                id: tileCol
                                                Layout.fillWidth: true
                                                spacing: 3

                                                Txt {
                                                    Layout.fillWidth: true
                                                    text: modelData.title
                                                    role: "title"
                                                    wrapMode: Text.WordWrap
                                                }

                                                Txt {
                                                    Layout.fillWidth: true
                                                    text: modelData.body
                                                    role: "caption"
                                                    color: Theme.surfaceContentVariant
                                                    wrapMode: Text.WordWrap
                                                }
                                            }
                                        }
                                    }
                                }
                            }

                            Rectangle {
                                Layout.fillWidth: true
                                implicitWidth: 520
                                implicitHeight: 40
                                radius: Theme.radiusSm
                                color: walkthrough.hasPairedDevice
                                    ? Qt.rgba(0.18, 0.72, 0.42, 0.12)
                                    : Theme.surfaceContainerLow
                                border.width: 1
                                border.color: walkthrough.hasPairedDevice
                                    ? Qt.rgba(0.18, 0.72, 0.42, 0.35)
                                    : Theme.outlineVariant

                                RowLayout {
                                    anchors.fill: parent
                                    anchors.leftMargin: 12
                                    anchors.rightMargin: 12
                                    spacing: 10

                                    Icon {
                                        path: walkthrough.hasPairedDevice ? Icons.check : Icons.phone
                                        width: 16
                                        height: 16
                                        color: walkthrough.hasPairedDevice ? Qt.rgba(0.22, 0.80, 0.48, 1.0) : Theme.primary
                                    }

                                    Txt {
                                        Layout.fillWidth: true
                                        text: walkthrough.hasPairedDevice
                                            ? ("Paired with " + (walkthrough.activeDeviceName || "your phone") + ".")
                                            : "No phone paired yet. You can pair in Step 4 or skip setup anytime."
                                        role: "caption"
                                        color: Theme.surfaceContent
                                        elide: Text.ElideRight
                                    }
                                }
                            }
                        }

                        // STEP 1: Privacy promise
                        ColumnLayout {
                            Layout.fillWidth: true
                            spacing: 12
                            visible: walkthrough.step === 1

                            Repeater {
                                model: [
                                    {
                                        icon: Icons.lock,
                                        title: qsTr("End-to-end encrypted over QUIC"),
                                        body: "All traffic between your phone and PC uses TLS 1.3 with device certificates pinned during pairing."
                                    },
                                    {
                                        icon: Icons.shield,
                                        title: qsTr("Protected device keys"),
                                        body: Qt.platform.os === "osx"
                                            ? qsTr("Your phone’s keys are protected by Android Keystore, and your Mac’s keys by macOS Keychain.")
                                            : qsTr("Device keys are protected by Windows DPAPI and the Android Keystore.")
                                    },
                                    {
                                        icon: Icons.wifi,
                                        title: qsTr("Local network by default"),
                                        body: "Pairing and transfers stay on your local network unless you turn on \"Connect over mobile data\", and automatic update checks can be turned off in Settings."
                                    }
                                ]

                                delegate: Rectangle {
                                    required property var modelData
                                    Layout.fillWidth: true
                                    implicitWidth: 520
                                    implicitHeight: privacyRow.implicitHeight + 24
                                    radius: Theme.radiusMd
                                    color: Theme.surfaceContainerLow
                                    border.width: 1
                                    border.color: Theme.outlineVariant

                                    RowLayout {
                                        id: privacyRow
                                        anchors.fill: parent
                                        anchors.margins: 12
                                        spacing: 12

                                        Rectangle {
                                            Layout.preferredWidth: 34
                                            Layout.preferredHeight: 34
                                            implicitWidth: 34
                                            implicitHeight: 34
                                            Layout.alignment: Qt.AlignTop
                                            radius: 10
                                            color: Theme.primaryContainer

                                            Icon {
                                                anchors.centerIn: parent
                                                path: modelData.icon
                                                width: 18
                                                height: 18
                                                color: Theme.primaryContainerContent
                                            }
                                        }

                                        ColumnLayout {
                                            Layout.fillWidth: true
                                            spacing: 3

                                            Txt {
                                                Layout.fillWidth: true
                                                text: modelData.title
                                                role: "title"
                                                wrapMode: Text.WordWrap
                                            }

                                            Txt {
                                                Layout.fillWidth: true
                                                text: modelData.body
                                                role: "caption"
                                                color: Theme.surfaceContentVariant
                                                wrapMode: Text.WordWrap
                                            }
                                        }
                                    }
                                }
                            }
                        }

                        // STEP 2: Install NectarLink on your phone
                        ColumnLayout {
                            Layout.fillWidth: true
                            spacing: 12
                            visible: walkthrough.step === 2

                            Rectangle {
                                Layout.fillWidth: true
                                implicitWidth: 520
                                implicitHeight: releaseCol.implicitHeight + 28
                                radius: Theme.radiusMd
                                color: Theme.surfaceContainerLow
                                border.width: 1
                                border.color: Theme.outlineVariant

                                ColumnLayout {
                                    id: releaseCol
                                    anchors.fill: parent
                                    anchors.margins: 14
                                    spacing: 10

                                    RowLayout {
                                        Layout.fillWidth: true
                                        spacing: 10

                                        Icon {
                                            path: Icons.download
                                            width: 18
                                            height: 18
                                            color: Theme.primary
                                        }

                                        Txt {
                                            Layout.fillWidth: true
                                            text: qsTr("Option 1: Download the Android APK (Android 8.0+)")
                                            role: "title"
                                            wrapMode: Text.WordWrap
                                        }
                                    }

                                    Txt {
                                        Layout.fillWidth: true
                                        text: qsTr("Open the latest GitHub Release on your phone or copy the release link.")
                                        role: "caption"
                                        color: Theme.surfaceContentVariant
                                        wrapMode: Text.WordWrap
                                    }

                                    RowLayout {
                                        Layout.fillWidth: true
                                        spacing: 8

                                        Button {
                                            text: qsTr("Open release page")
                                            iconPath: Icons.openExternal
                                            variant: "tonal"
                                            size: "sm"
                                            Accessible.name: "Open Android release page in browser"
                                            onClicked: Qt.openUrlExternally("https://github.com/thtbee/nectarlink/releases/latest")
                                        }

                                        Button {
                                            id: copyUrlBtn
                                            property bool copied: false
                                            text: copied ? "Copied link" : "Copy link"
                                            iconPath: copied ? Icons.check : Icons.copy
                                            variant: "outline"
                                            size: "sm"
                                            Accessible.name: "Copy Android release URL"
                                            onClicked: {
                                                AppController.copyText("https://github.com/thtbee/nectarlink/releases/latest")
                                                copied = true
                                                resetCopyUrlTimer.restart()
                                            }
                                            Timer {
                                                id: resetCopyUrlTimer
                                                interval: 2000
                                                onTriggered: copyUrlBtn.copied = false
                                            }
                                        }

                                        Item { Layout.fillWidth: true }
                                    }
                                }
                            }

                            Rectangle {
                                Layout.fillWidth: true
                                implicitWidth: 520
                                implicitHeight: adbCol.implicitHeight + 28
                                radius: Theme.radiusMd
                                color: Theme.surfaceContainerLow
                                border.width: 1
                                border.color: Theme.outlineVariant

                                ColumnLayout {
                                    id: adbCol
                                    anchors.fill: parent
                                    anchors.margins: 14
                                    spacing: 10

                                    RowLayout {
                                        Layout.fillWidth: true
                                        spacing: 10

                                        Icon {
                                            path: Icons.terminal
                                            width: 18
                                            height: 18
                                            color: Theme.primary
                                        }

                                        Txt {
                                            Layout.fillWidth: true
                                            text: qsTr("Option 2: Install over USB with ADB")
                                            role: "title"
                                            wrapMode: Text.WordWrap
                                        }
                                    }

                                    RowLayout {
                                        Layout.fillWidth: true
                                        spacing: 8

                                        Rectangle {
                                            Layout.fillWidth: true
                                            implicitWidth: 320
                                            implicitHeight: 34
                                            radius: Theme.radiusSm
                                            color: Theme.surfaceContainer
                                            border.width: 1
                                            border.color: Theme.outlineVariant

                                            Txt {
                                                anchors.verticalCenter: parent.verticalCenter
                                                anchors.left: parent.left
                                                anchors.right: parent.right
                                                anchors.leftMargin: 10
                                                anchors.rightMargin: 10
                                                text: walkthrough.adbCommand
                                                textFormat: Text.PlainText
                                                role: "code"
                                                size: 12
                                                elide: Text.ElideRight
                                            }
                                        }

                                        Button {
                                            id: copyAdbBtn
                                            property bool copied: false
                                            text: copied ? "Copied" : "Copy command"
                                            iconPath: copied ? Icons.check : Icons.copy
                                            variant: "tonal"
                                            size: "sm"
                                            Accessible.name: "Copy ADB install command"
                                            onClicked: {
                                                AppController.copyText(walkthrough.adbCommand)
                                                copied = true
                                                resetCopyAdbTimer.restart()
                                            }
                                            Timer {
                                                id: resetCopyAdbTimer
                                                interval: 2000
                                                onTriggered: copyAdbBtn.copied = false
                                            }
                                        }
                                    }

                                    Txt {
                                        Layout.fillWidth: true
                                        visible: walkthrough.adbVersionPlaceholder
                                        text: qsTr("Replace <version> with the version in the name of the APK you downloaded.")
                                        textFormat: Text.PlainText
                                        role: "caption"
                                        muted: true
                                        wrapMode: Text.WordWrap
                                    }
                                }
                            }
                        }

                        // STEP 3: Pair your phone and PC
                        ColumnLayout {
                            Layout.fillWidth: true
                            spacing: 14
                            visible: walkthrough.step === 3

                            Rectangle {
                                Layout.fillWidth: true
                                implicitWidth: 520
                                implicitHeight: 42
                                radius: Theme.radiusSm
                                visible: walkthrough.hasPairedDevice
                                color: Qt.rgba(0.18, 0.72, 0.42, 0.14)
                                border.width: 1
                                border.color: Qt.rgba(0.18, 0.72, 0.42, 0.40)

                                RowLayout {
                                    anchors.fill: parent
                                    anchors.leftMargin: 12
                                    anchors.rightMargin: 12
                                    spacing: 10

                                    Icon {
                                        path: Icons.check
                                        width: 16
                                        height: 16
                                        color: Qt.rgba(0.22, 0.80, 0.48, 1.0)
                                    }

                                    Txt {
                                        Layout.fillWidth: true
                                        text: qsTr("Paired with %1. You can continue to the next step.")
                                            .arg(walkthrough.activeDeviceName || qsTr("your phone"))
                                        role: "caption"
                                        color: Theme.surfaceContent
                                        elide: Text.ElideRight
                                    }
                                }
                            }

                            PairingPanel {
                                Layout.fillWidth: true
                                implicitWidth: 520
                                cancellable: false
                                compact: true
                                running: walkthrough.visible && walkthrough.step === 3
                            }
                        }

                        // STEP 4: Choose what to share
                        ColumnLayout {
                            Layout.fillWidth: true
                            spacing: 14
                            visible: walkthrough.step === 4

                            // B.4: Read-only two-column summary when no phone is paired yet
                            ColumnLayout {
                                Layout.fillWidth: true
                                spacing: 14
                                visible: !walkthrough.hasPairedDevice

                                GridLayout {
                                    Layout.fillWidth: true
                                    columns: width >= 480 ? 2 : 1
                                    columnSpacing: 16
                                    rowSpacing: 16

                                    // Left column: On by default
                                    Rectangle {
                                        Layout.fillWidth: true
                                        Layout.alignment: Qt.AlignTop
                                        implicitWidth: 260
                                        implicitHeight: onDefaultCol.implicitHeight + 28
                                        radius: Theme.radiusMd
                                        color: Theme.surfaceContainerLow
                                        border.width: 1
                                        border.color: Theme.outlineVariant

                                        ColumnLayout {
                                            id: onDefaultCol
                                            anchors.fill: parent
                                            anchors.margins: 14
                                            spacing: 10

                                            RowLayout {
                                                Layout.fillWidth: true
                                                spacing: 8

                                                Rectangle {
                                                    Layout.preferredWidth: 8
                                                    Layout.preferredHeight: 8
                                                    implicitWidth: 8
                                                    implicitHeight: 8
                                                    radius: 4
                                                    color: Theme.primary
                                                }

                                                Txt {
                                                    Layout.fillWidth: true
                                                    text: qsTr("On by default")
                                                    role: "title"
                                                }
                                            }

                                            Repeater {
                                                model: walkthrough.defaultOnLabels
                                                delegate: RowLayout {
                                                    required property string modelData
                                                    Layout.fillWidth: true
                                                    spacing: 10

                                                    Icon {
                                                        path: Icons.check
                                                        width: 14
                                                        height: 14
                                                        color: Theme.primary
                                                    }

                                                    Txt {
                                                        Layout.fillWidth: true
                                                        text: modelData
                                                        role: "body"
                                                    }
                                                }
                                            }
                                        }
                                    }

                                    // Right column: Off until you turn them on
                                    Rectangle {
                                        Layout.fillWidth: true
                                        Layout.alignment: Qt.AlignTop
                                        implicitWidth: 260
                                        implicitHeight: optInCol.implicitHeight + 28
                                        radius: Theme.radiusMd
                                        color: Theme.surfaceContainerLow
                                        border.width: 1
                                        border.color: Theme.outlineVariant

                                        ColumnLayout {
                                            id: optInCol
                                            anchors.fill: parent
                                            anchors.margins: 14
                                            spacing: 10

                                            RowLayout {
                                                Layout.fillWidth: true
                                                spacing: 8

                                                Rectangle {
                                                    Layout.preferredWidth: 8
                                                    Layout.preferredHeight: 8
                                                    implicitWidth: 8
                                                    implicitHeight: 8
                                                    radius: 4
                                                    color: "transparent"
                                                    border.width: 1.5
                                                    border.color: Theme.surfaceContentVariant
                                                }

                                                Txt {
                                                    Layout.fillWidth: true
                                                    text: qsTr("Off until you turn them on")
                                                    role: "title"
                                                }
                                            }

                                            Repeater {
                                                model: walkthrough.optInLabels
                                                delegate: RowLayout {
                                                    required property string modelData
                                                    Layout.fillWidth: true
                                                    spacing: 10

                                                    Icon {
                                                        path: Icons.lock
                                                        width: 14
                                                        height: 14
                                                        color: Theme.surfaceContentVariant
                                                    }

                                                    Txt {
                                                        Layout.fillWidth: true
                                                        text: modelData
                                                        role: "body"
                                                        color: Theme.surfaceContentVariant
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }

                                Txt {
                                    Layout.fillWidth: true
                                    text: qsTr("Change these per phone on Home after pairing.")
                                    role: "caption"
                                    color: Theme.surfaceContentVariant
                                }
                            }

                            // Interactive toggles when a phone IS paired
                            ColumnLayout {
                                Layout.fillWidth: true
                                spacing: 14
                                visible: walkthrough.hasPairedDevice

                                Txt {
                                    Layout.fillWidth: true
                                    text: qsTr("Sharing settings for %1").arg(walkthrough.activeDeviceName || qsTr("your phone"))
                                    role: "label"
                                    color: Theme.primary
                                }

                                GridLayout {
                                    Layout.fillWidth: true
                                    columns: width >= 500 ? 2 : 1
                                    columnSpacing: 16
                                    rowSpacing: 16

                                    // Left column: On by default switches
                                    Rectangle {
                                        Layout.fillWidth: true
                                        Layout.alignment: Qt.AlignTop
                                        implicitWidth: 260
                                        implicitHeight: pairedDefaultCol.implicitHeight + 28
                                        radius: Theme.radiusMd
                                        color: Theme.surfaceContainerLow
                                        border.width: 1
                                        border.color: Theme.outlineVariant

                                        ColumnLayout {
                                            id: pairedDefaultCol
                                            anchors.fill: parent
                                            anchors.margins: 14
                                            spacing: 10

                                            Txt {
                                                Layout.fillWidth: true
                                                text: qsTr("On by default")
                                                role: "title"
                                            }

                                            Repeater {
                                                model: [
                                                    { key: "notifications", label: qsTr("Notifications") },
                                                    { key: "clipboard", label: qsTr("Clipboard") },
                                                    { key: "files", label: qsTr("Send files") },
                                                    { key: "media", label: qsTr("Now playing") },
                                                    { key: "sms", label: qsTr("Messages") },
                                                    { key: "calls", label: qsTr("Calls") }
                                                ]

                                                delegate: RowLayout {
                                                    required property var modelData
                                                    Layout.fillWidth: true
                                                    spacing: 10

                                                    Txt {
                                                        Layout.fillWidth: true
                                                        text: modelData.label
                                                        role: "body"
                                                    }

                                                    Toggle {
                                                        label: modelData.label
                                                        checked: walkthrough.isDeviceToggleOn(modelData.key)
                                                        onToggled: next => walkthrough.setDeviceToggleValue(modelData.key, next)
                                                    }
                                                }
                                            }
                                        }
                                    }

                                    // Right column: Opt-in switches
                                    Rectangle {
                                        Layout.fillWidth: true
                                        Layout.alignment: Qt.AlignTop
                                        implicitWidth: 260
                                        implicitHeight: pairedOptInCol.implicitHeight + 28
                                        radius: Theme.radiusMd
                                        color: Theme.surfaceContainerLow
                                        border.width: 1
                                        border.color: Theme.outlineVariant

                                        ColumnLayout {
                                            id: pairedOptInCol
                                            anchors.fill: parent
                                            anchors.margins: 14
                                            spacing: 10

                                            Txt {
                                                Layout.fillWidth: true
                                                text: qsTr("Off until you turn them on")
                                                role: "title"
                                            }

                                            Repeater {
                                                model: [
                                                    { key: "pc_mirror", label: qsTr("Screen mirroring") },
                                                    { key: "remote_input", label: qsTr("Remote input") },
                                                    { key: "storage", label: qsTr("Share PC folders") }
                                                ]

                                                delegate: RowLayout {
                                                    required property var modelData
                                                    Layout.fillWidth: true
                                                    spacing: 10

                                                    Txt {
                                                        Layout.fillWidth: true
                                                        text: modelData.label
                                                        role: "body"
                                                    }

                                                    Toggle {
                                                        label: modelData.label
                                                        checked: walkthrough.isDeviceToggleOn(modelData.key)
                                                        onToggled: next => walkthrough.setDeviceToggleValue(modelData.key, next)
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }

                        // STEP 5: Optional setup
                        ColumnLayout {
                            Layout.fillWidth: true
                            spacing: 12
                            visible: walkthrough.step === 5

                            // 1. Windows notifications
                            Rectangle {
                                Layout.fillWidth: true
                                implicitWidth: 520
                                implicitHeight: toastRow.implicitHeight + 24
                                radius: Theme.radiusMd
                                color: Theme.surfaceContainerLow
                                border.width: 1
                                border.color: Theme.outlineVariant

                                RowLayout {
                                    id: toastRow
                                    anchors.fill: parent
                                    anchors.margins: 12
                                    spacing: 12

                                    Rectangle {
                                        Layout.preferredWidth: 34
                                        Layout.preferredHeight: 34
                                        implicitWidth: 34
                                        implicitHeight: 34
                                        radius: 10
                                        color: Theme.primaryContainer

                                        Icon {
                                            anchors.centerIn: parent
                                            path: Icons.bell
                                            width: 18
                                            height: 18
                                            color: Theme.primaryContainerContent
                                        }
                                    }

                                    ColumnLayout {
                                        Layout.fillWidth: true
                                        spacing: 2

                                        Txt {
                                            Layout.fillWidth: true
                                            text: Qt.platform.os === "osx" ? qsTr("Notifications") : qsTr("Windows notifications")
                                            role: "title"
                                        }

                                        Txt {
                                            Layout.fillWidth: true
                                            text: Qt.platform.os === "osx"
                                                ? "Banners follow your macOS notification settings. Their on or off state isn't checked yet."
                                                : AppController.toastsEnabled
                                                ? "Enabled in Windows Settings."
                                                : "Notifications are off for Waypair Link in Windows Settings."
                                            role: "caption"
                                            color: Theme.surfaceContentVariant
                                            wrapMode: Text.WordWrap
                                        }
                                    }

                                    RowLayout {
                                        spacing: 8

                                        Button {
                                            text: qsTr("Windows Settings")
                                            variant: "tonal"
                                            size: "sm"
                                            visible: !AppController.toastsEnabled
                                            Accessible.name: "Open Windows notification settings"
                                            onClicked: Qt.openUrlExternally("ms-settings:notifications")
                                        }

                                        Button {
                                            text: qsTr("Recheck")
                                            iconPath: Icons.refresh
                                            variant: "text"
                                            size: "sm"
                                            visible: Qt.platform.os !== "osx"
                                            Accessible.name: "Recheck Windows notification status"
                                            onClicked: AppController.refreshToastsEnabled()
                                        }
                                    }
                                }
                            }

                            // 2. Start with Windows
                            Rectangle {
                                Layout.fillWidth: true
                                implicitWidth: 520
                                implicitHeight: startupRow.implicitHeight + 24
                                radius: Theme.radiusMd
                                color: Theme.surfaceContainerLow
                                border.width: 1
                                border.color: Theme.outlineVariant

                                RowLayout {
                                    id: startupRow
                                    anchors.fill: parent
                                    anchors.margins: 12
                                    spacing: 12

                                    Rectangle {
                                        Layout.preferredWidth: 34
                                        Layout.preferredHeight: 34
                                        implicitWidth: 34
                                        implicitHeight: 34
                                        radius: 10
                                        color: Theme.primaryContainer

                                        Icon {
                                            anchors.centerIn: parent
                                            path: Icons.power
                                            width: 18
                                            height: 18
                                            color: Theme.primaryContainerContent
                                        }
                                    }

                                    ColumnLayout {
                                        Layout.fillWidth: true
                                        spacing: 2

                                        Txt {
                                            Layout.fillWidth: true
                                            text: Qt.platform.os === "osx" ? qsTr("Open at login") : qsTr("Start with Windows")
                                            role: "title"
                                        }

                                        Txt {
                                            Layout.fillWidth: true
                                            text: Qt.platform.os === "osx"
                                                ? qsTr("Open the window when you log in.")
                                                : qsTr("Keep Waypair Link ready in the system tray when you sign in.")
                                            role: "caption"
                                            color: Theme.surfaceContentVariant
                                            wrapMode: Text.WordWrap
                                        }
                                    }

                                    Toggle {
                                        label: Qt.platform.os === "osx" ? qsTr("Open at login") : qsTr("Start with Windows")
                                        checked: Preferences.startWithWindows
                                        onToggled: next => Preferences.startWithWindows = next
                                    }
                                }
                            }

                            // 3. Firewall and network check
                            Rectangle {
                                Layout.fillWidth: true
                                implicitWidth: 520
                                implicitHeight: firewallCol.implicitHeight + 24
                                radius: Theme.radiusMd
                                color: Theme.surfaceContainerLow
                                border.width: 1
                                border.color: Theme.outlineVariant

                                ColumnLayout {
                                    id: firewallCol
                                    anchors.fill: parent
                                    anchors.margins: 12
                                    spacing: 10

                                    RowLayout {
                                        Layout.fillWidth: true
                                        spacing: 12

                                        Rectangle {
                                            Layout.preferredWidth: 34
                                            Layout.preferredHeight: 34
                                            implicitWidth: 34
                                            implicitHeight: 34
                                            radius: 10
                                            color: Theme.primaryContainer

                                            Icon {
                                                anchors.centerIn: parent
                                                path: Icons.bolt
                                                width: 18
                                                height: 18
                                                color: Theme.primaryContainerContent
                                            }
                                        }

                                        ColumnLayout {
                                            Layout.fillWidth: true
                                            spacing: 2

                                            Txt {
                                                Layout.fillWidth: true
                                                text: qsTr("Firewall and network check")
                                                role: "title"
                                            }

                                            Txt {
                                                Layout.fillWidth: true
                                                text: walkthrough.doctorSummaryLabel
                                                role: "caption"
                                                color: Theme.surfaceContentVariant
                                                wrapMode: Text.WordWrap
                                            }
                                        }

                                        Button {
                                            text: AppController.doctorBusy ? "Checking..." : "Connection Doctor"
                                            iconPath: Icons.bolt
                                            variant: "tonal"
                                            size: "sm"
                                            enabled: !AppController.doctorBusy
                                            Accessible.name: "Run firewall and network check"
                                            onClicked: {
                                                AppController.runDoctor()
                                                walkthrough.doctorRequested()
                                            }
                                        }
                                    }
                                }
                            }
                        }

                        // STEP 6: You are set (no duplicate action buttons inside card)
                        ColumnLayout {
                            Layout.fillWidth: true
                            spacing: 12
                            visible: walkthrough.step === 6

                            Repeater {
                                model: [
                                    {
                                        icon: Icons.phone,
                                        title: walkthrough.hasPairedDevice
                                            ? ("Paired with " + (walkthrough.activeDeviceName || "your phone"))
                                            : "Pair a phone anytime from Home",
                                        body: walkthrough.hasPairedDevice
                                            ? "Your phone is linked and ready on your local network."
                                            : "Open the Waypair Link Android app whenever you are ready to scan the QR code."
                                    },
                                    {
                                        icon: Icons.search,
                                        title: Qt.platform.os === "osx" ? qsTr("Command Palette (Command-K)") : qsTr("Command Palette (Ctrl+K)"),
                                        body: Qt.platform.os === "osx"
                                            ? "Press Command-K from anywhere in the app to jump to features, send files, or run actions."
                                            : "Press Ctrl+K from anywhere in the app to jump to features, send files, or run actions."
                                    },
                                    {
                                        icon: Icons.book,
                                        title: qsTr("Features guide"),
                                        body: "Browse every feature in Waypair Link, see what permissions it needs, and jump straight to it."
                                    }
                                ]

                                delegate: Rectangle {
                                    required property var modelData
                                    Layout.fillWidth: true
                                    implicitWidth: 520
                                    implicitHeight: doneRow.implicitHeight + 24
                                    radius: Theme.radiusMd
                                    color: Theme.surfaceContainerLow
                                    border.width: 1
                                    border.color: Theme.outlineVariant

                                    RowLayout {
                                        id: doneRow
                                        anchors.fill: parent
                                        anchors.margins: 12
                                        spacing: 12

                                        Rectangle {
                                            Layout.preferredWidth: 34
                                            Layout.preferredHeight: 34
                                            implicitWidth: 34
                                            implicitHeight: 34
                                            Layout.alignment: Qt.AlignTop
                                            radius: 10
                                            color: Theme.primaryContainer

                                            Icon {
                                                anchors.centerIn: parent
                                                path: modelData.icon
                                                width: 18
                                                height: 18
                                                color: Theme.primaryContainerContent
                                            }
                                        }

                                        ColumnLayout {
                                            Layout.fillWidth: true
                                            spacing: 2

                                            Txt {
                                                Layout.fillWidth: true
                                                text: modelData.title
                                                role: "title"
                                                wrapMode: Text.WordWrap
                                            }

                                            Txt {
                                                Layout.fillWidth: true
                                                text: modelData.body
                                                role: "caption"
                                                color: Theme.surfaceContentVariant
                                                wrapMode: Text.WordWrap
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        // Bottom navigation footer: Back on left, Next or Open features guide + Done on right
        RowLayout {
            Layout.fillWidth: true
            spacing: 12

            Button {
                text: qsTr("Back")
                iconPath: Icons.back
                variant: "tonal"
                size: "md"
                visible: walkthrough.step > 0
                Accessible.name: "Back to previous step"
                onClicked: walkthrough.goBack()
            }

            Item { Layout.fillWidth: true }

            Button {
                text: qsTr("Open features guide")
                iconPath: Icons.book
                variant: "tonal"
                size: "md"
                visible: walkthrough.step === walkthrough.totalSteps - 1
                Accessible.name: "Finish setup and open Features guide"
                onClicked: walkthrough.finishAndOpenGuide()
            }

            Button {
                text: walkthrough.step === walkthrough.totalSteps - 1
                    ? "Done"
                    : (walkthrough.step === 0 ? "Get started" : "Next")
                iconPath: walkthrough.step === walkthrough.totalSteps - 1 ? Icons.check : Icons.chevronRight
                variant: "fill"
                size: "md"
                Accessible.name: walkthrough.step === walkthrough.totalSteps - 1 ? "Finish walkthrough" : "Next step"
                onClicked: walkthrough.goNext()
            }
        }
    }
}
