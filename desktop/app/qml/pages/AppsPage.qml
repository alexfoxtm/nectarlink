// SPDX-License-Identifier: GPL-3.0-or-later
import QtQuick
import app.nectarlink

// Apps: every app on the phone, to open one in a window of its own on this PC
// (Elevated: the phone runs it on a display of its own). Searchable; Enter
// opens the first match. Apps already open here are marked.
Item {
    id: page
    property bool active: false
    property string deviceId
    property string deviceName

    // A known host qualification gap takes precedence over phone setup advice.
    // Keep the negotiated matrix intact; it cannot qualify this host build.
    readonly property var negotiatedFeature: AppController.capsRevision >= 0 && deviceId.length > 0
        ? AppController.featureState(deviceId, "mirroring.app_windows") : ({})
    readonly property bool macGap: !Mirror.appWindowsSupported
    readonly property bool settingsEntrySupported: Mirror.settingsQualificationSupported
    readonly property var feature: negotiatedFeature
    readonly property bool available: !macGap && (feature.state === "available" || feature.state === "partial")
    readonly property string unavailableReason: {
        if (page.deviceId.length === 0)
            return qsTr("Pair your phone to open its apps in windows on this PC.")
        if (page.macGap)
            return qsTr("Opening phone apps in windows is not yet available on Mac.")
        const reason = page.feature && typeof page.feature.reason === "string" ? page.feature.reason.trim() : ""
        return reason.length > 0 ? reason : qsTranslate("Mirror", "App windows are unavailable for this connection.")
    }

    readonly property var apps: {
        if (Mirror.appsDevice !== deviceId)
            return []
        try { return JSON.parse(Mirror.apps) } catch (e) { return [] }
    }
    readonly property var recent: {
        if (Mirror.appsDevice !== deviceId)
            return []
        try { return JSON.parse(Mirror.recentApps) } catch (e) { return [] }
    }
    // Packages with a window open on this PC.
    readonly property var openPkgs: {
        try {
            const open = {}
            JSON.parse(Mirror.windows).forEach(w => {
                if (w.device === page.deviceId && w.app && w.pkg && w.phase !== "ended")
                    open[w.pkg] = true
            })
            return open
        } catch (e) {
            return ({})
        }
    }
    readonly property string query: search.text.trim().toLocaleLowerCase()
    readonly property var shown: query.length === 0 ? apps
        : apps.filter(a => a.label.toLocaleLowerCase().indexOf(query) >= 0 || a.pkg.indexOf(query) >= 0)
    readonly property string loadState: Mirror.appsDevice === deviceId ? Mirror.appsState : ""

    function launch(app) {
        Mirror.startApp(deviceId, app.pkg, app.label)
    }

    // Loads the list the first time the page shows for a phone; Refresh reloads it.
    function loadIfNeeded() {
        if (!active || !available || deviceId.length === 0)
            return
        if (Mirror.appsDevice !== deviceId || Mirror.appsState === "" || Mirror.appsState === "failed")
            Mirror.loadApps(deviceId)
    }
    onActiveChanged: if (active) { loadIfNeeded(); search.forceActiveFocus() }
    onDeviceIdChanged: { search.text = ""; loadIfNeeded() }
    onAvailableChanged: loadIfNeeded()
    Component.onCompleted: loadIfNeeded()

    // ---- No phone, or opening apps isn't available yet ----
    Column {
        id: unavailablePlaceholder
        objectName: "appsUnavailablePlaceholder"
        anchors.centerIn: parent
        width: Math.max(0, Math.min(parent.width - 32, 420))
        spacing: 16
        visible: page.deviceId.length === 0 || !page.available
        Accessible.ignored: !visible
        Icon {
            objectName: "appsUnavailableIcon"
            Accessible.ignored: true
            anchors.horizontalCenter: parent.horizontalCenter
            width: 40; height: 40
            stroke: 1.5
            path: Icons.apps
            color: Theme.surfaceContentVariant
        }
        Txt {
            width: parent.width
            horizontalAlignment: Text.AlignHCenter
            objectName: "appsUnavailableHeading"
            role: "title"
            text: page.deviceId.length === 0 ? qsTr("Phone apps")
                : qsTr("Open %1's apps here").arg(page.deviceName)
            Accessible.role: Accessible.StaticText
            Accessible.name: text
            Accessible.ignored: !unavailablePlaceholder.visible
        }
        Txt {
            objectName: "appsUnavailableReason"
            width: parent.width
            horizontalAlignment: Text.AlignHCenter
            wrapMode: Text.WordWrap
            role: "body"
            muted: true
            text: page.unavailableReason
            Accessible.role: Accessible.StaticText
            Accessible.name: text
            Accessible.ignored: !unavailablePlaceholder.visible
        }
        Txt {
            objectName: "appsUnavailableSettingsGuidance"
            width: parent.width
            visible: page.deviceId.length > 0 && page.macGap && page.settingsEntrySupported
            horizontalAlignment: Text.AlignHCenter
            wrapMode: Text.WordWrap
            role: "body"
            muted: true
            text: qsTr("In Waypair Link on your phone, allow this computer to open Settings once, then open the window here. Scrolling needs the phone-control allowance.")
            Accessible.role: Accessible.StaticText
            Accessible.name: text
            Accessible.ignored: !unavailablePlaceholder.visible || !visible
        }
        Button {
            objectName: "appsUnavailableSettingsButton"
            anchors.horizontalCenter: parent.horizontalCenter
            visible: page.deviceId.length > 0 && page.macGap && page.settingsEntrySupported
            Accessible.ignored: !unavailablePlaceholder.visible || !visible
            enabled: page.openPkgs["com.android.settings"] !== true
            text: qsTr("Open Settings window")
            onClicked: Mirror.startSettingsQualification(page.deviceId)
        }
        LockChip {
            objectName: "appsUnavailableLockChip"
            Accessible.ignored: !unavailablePlaceholder.visible || !visible
            anchors.horizontalCenter: parent.horizontalCenter
            visible: !page.macGap && page.deviceId.length > 0 && label.length > 0
            feature: page.feature
        }
    }

    // ---- The apps ----
    Item {
        id: content
        visible: page.deviceId.length > 0 && page.available
        anchors.fill: parent
        anchors.leftMargin: page.width < 640 ? 16 : 24
        anchors.rightMargin: page.width < 640 ? 16 : 24
        anchors.topMargin: 24

        Column {
            id: phoneContext
            objectName: "appsPhoneContext"
            anchors.top: parent.top
            width: parent.width
            spacing: 4

            Txt {
                width: parent.width
                role: "title"
                size: 24
                weight: Font.DemiBold
                text: qsTr("Phone apps")
            }
            Txt {
                width: Math.min(parent.width, 320)
                role: "bodySmall"
                muted: true
                objectName: "appsPhoneName"
                text: page.deviceName
                Accessible.name: page.deviceName
                elide: Text.ElideRight
                maximumLineCount: 1
                wrapMode: Text.NoWrap
            }
        }

        Row {
            id: toolbar
            objectName: "appsToolbar"
            anchors.top: phoneContext.bottom
            anchors.topMargin: 16
            width: parent.width
            height: 40
            spacing: 12

            Rectangle {
                objectName: "appsSearchSurface"
                width: Math.min(460, parent.width - refresh.width - parent.spacing)
                height: parent.height
                radius: Theme.controlRadius
                color: Theme.panelColor
                border.width: 1
                border.color: Theme.outlineVariant
                Rectangle {
                    anchors.fill: parent
                    anchors.margins: -4
                    radius: parent.radius + 4
                    color: "transparent"
                    border.width: 2
                    border.color: Theme.focusColor
                    visible: Theme.focusVisible(search)
                }
                Icon {
                    id: searchIcon
                    anchors.left: parent.left
                    anchors.leftMargin: 14
                    anchors.verticalCenter: parent.verticalCenter
                    width: 18; height: 18
                    path: Icons.search
                    color: Theme.surfaceContentVariant
                }
                TextInput {
                    id: search
                    objectName: "appsSearchField"
                    anchors.left: searchIcon.right
                    anchors.leftMargin: 10
                    anchors.right: parent.right
                    anchors.rightMargin: 14
                    anchors.verticalCenter: parent.verticalCenter
                    font.family: Theme.fontUi
                    font.pixelSize: 14
                    color: Theme.surfaceContent
                    selectionColor: Theme.primaryContainer
                    selectedTextColor: Theme.primaryContainerContent
                    clip: true
                    Accessible.name: qsTr("Search apps")
                    Keys.onReturnPressed: if (page.shown.length > 0) page.launch(page.shown[0])
                    Keys.onEnterPressed: if (page.shown.length > 0) page.launch(page.shown[0])
                    Keys.onEscapePressed: text = ""
                    Keys.onDownPressed: grid.focusFirstApp()
                    Txt {
                        anchors.left: parent.left
                        anchors.right: parent.right
                        anchors.verticalCenter: parent.verticalCenter
                        visible: search.text.length === 0
                        clip: true
                        role: "body"
                        muted: true
                        text: qsTr("Search %1's apps").arg(page.deviceName)
                        elide: Text.ElideRight
                        maximumLineCount: 1
                        wrapMode: Text.NoWrap
                    }
                }
            }
            IconButton {
                id: refresh
                anchors.verticalCenter: parent.verticalCenter
                iconPath: Icons.refresh
                label: qsTr("Refresh")
                enabled: page.loadState !== "loading"
                onClicked: Mirror.loadApps(page.deviceId)
            }
        }

        // Loading, failed or empty.
        Column {
            id: loadStateMessage
            objectName: "appsLoadState"
            anchors.top: toolbar.bottom
            anchors.topMargin: 32
            width: parent.width
            visible: page.loadState !== "ready" || page.shown.length === 0
            spacing: 12
            Spinner {
                anchors.horizontalCenter: parent.horizontalCenter
                visible: page.loadState === "loading" || page.loadState === ""
                width: 24; height: 24
            }
            Txt {
                width: parent.width
                horizontalAlignment: Text.AlignHCenter
                wrapMode: Text.WordWrap
                role: "body"
                color: page.loadState === "failed" ? Theme.error : Theme.surfaceContentVariant
                text: page.loadState === "failed" ? Mirror.appsError
                    : page.loadState === "ready" ? (page.query.length > 0 ? qsTr("No apps match.") : qsTr("No apps to open."))
                    : qsTr("Getting the phone's apps…")
            }
            Button {
                objectName: "appsRetryButton"
                anchors.horizontalCenter: parent.horizontalCenter
                visible: page.loadState === "failed"
                variant: "tonal"
                size: "sm"
                text: qsTr("Try again")
                onClicked: Mirror.loadApps(page.deviceId)
            }
        }

        GridView {
            id: grid
            objectName: "appsGrid"
            anchors.top: toolbar.bottom
            anchors.topMargin: 16
            anchors.bottom: parent.bottom
            width: parent.width
            visible: page.loadState === "ready" && page.shown.length > 0
            clip: true
            model: page.shown
            cellWidth: width < 104 ? width : Math.max(104, Math.min(116,
                Math.floor(width / Math.max(1, Math.round(width / 112)))))
            cellHeight: 112
            keyNavigationEnabled: true
            activeFocusOnTab: true
            boundsBehavior: Flickable.StopAtBounds

            function focusIndex(index) {
                if (index < 0 || index >= count)
                    return
                currentIndex = index
                positionViewAtIndex(index, GridView.Contain)
                Qt.callLater(function() {
                    if (currentItem)
                        currentItem.forceActiveFocus()
                })
            }

            function focusFirstApp() { focusIndex(0) }
            reuseItems: true
            cacheBuffer: 400

            header: Column {
                width: grid.width
                visible: page.query.length === 0 && page.recent.length > 0
                height: visible ? implicitHeight : 0
                spacing: 8
                bottomPadding: 8

                Txt {
                    role: "label"
                    muted: true
                    text: qsTr("Recent")
                }
                Row {
                    width: parent.width
                    clip: true
                    spacing: 0
                    Repeater {
                        model: page.recent
                        delegate: AppCell {
                            required property var modelData
                            app: modelData
                            width: grid.cellWidth
                        }
                    }
                }
                Divider {}
                Txt {
                    topPadding: 4
                    role: "label"
                    muted: true
                    text: qsTr("All apps")
                }
            }

            delegate: AppCell {
                required property var modelData
                app: modelData
                gridNavigation: true
                width: grid.cellWidth
            }
        }
    }

    component AppCell: Item {
        id: cell
        property var app: ({})
        property bool gridNavigation: false
        required property int index
        readonly property bool isOpen: page.openPkgs[app.pkg] === true
        height: grid.cellHeight
        activeFocusOnTab: true
        Accessible.role: Accessible.Button
        Accessible.name: isOpen ? qsTr("%1, open").arg(app.label) : app.label
        Accessible.onPressAction: page.launch(app)
        Keys.onLeftPressed: if (gridNavigation) grid.focusIndex(index - 1)
        Keys.onRightPressed: if (gridNavigation) grid.focusIndex(index + 1)
        Keys.onUpPressed: if (gridNavigation) grid.focusIndex(index - Math.max(1, Math.floor(grid.width / grid.cellWidth)))
        Keys.onDownPressed: if (gridNavigation) grid.focusIndex(index + Math.max(1, Math.floor(grid.width / grid.cellWidth)))
        Keys.onReturnPressed: page.launch(app)
        Keys.onEnterPressed: page.launch(app)
        Keys.onSpacePressed: page.launch(app)

        Rectangle {
            anchors.fill: parent
            anchors.margins: 4
            radius: Theme.cardRadius
            color: Theme.surfaceContent
            opacity: tap.pressed ? 0.12 : (hover.hovered ? 0.06 : 0)
        }
        Rectangle {
            anchors.fill: parent
            anchors.margins: 4
            radius: Theme.cardRadius
            color: "transparent"
            border.width: Theme.focusVisible(cell) ? 2 : 0
            border.color: Theme.primary
        }
        Column {
            anchors.centerIn: parent
            width: parent.width - 12
            spacing: 6
            Item {
                anchors.horizontalCenter: parent.horizontalCenter
                width: 44; height: 60
                Image {
                    id: icon
                    anchors.top: parent.top
                    anchors.horizontalCenter: parent.horizontalCenter
                    width: 44; height: 44
                    source: cell.app.icon || ""
                    sourceSize: Qt.size(96, 96)
                    asynchronous: true
                    smooth: true
                    mipmap: true
                    visible: status === Image.Ready
                }
                Avatar {
                    anchors.top: parent.top
                    anchors.horizontalCenter: parent.horizontalCenter
                    width: 44; height: 44
                    visible: icon.status !== Image.Ready
                    name: cell.app.label || ""
                }
                // Open in a window on this PC.
                Txt {
                    visible: cell.isOpen
                    anchors.horizontalCenter: parent.horizontalCenter
                    anchors.top: parent.top
                    anchors.topMargin: 47
                    role: "caption"
                    size: 11
                    color: Theme.primary
                    text: qsTr("Open")
                    Accessible.ignored: true
                }
            }
            Txt {
                width: parent.width
                horizontalAlignment: Text.AlignHCenter
                role: "bodySmall"
                text: cell.app.label || ""
                elide: Text.ElideRight
                maximumLineCount: 2
                wrapMode: Text.WordWrap
            }
        }
        HoverHandler { id: hover; cursorShape: Qt.PointingHandCursor }
        TapHandler { id: tap; onTapped: page.launch(cell.app) }
    }
}
