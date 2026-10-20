// SPDX-License-Identifier: GPL-3.0-or-later
import QtQuick
import QtQuick.Dialogs
import app.nectarlink

// Phone storage: browse the folders a paired phone shares and save single
// files to a folder chosen on this Mac. Read-only: no preview, upload, rename
// or delete. The listing is dropped whenever the page is hidden or destroyed.
Item {
    id: page
    property bool active: false
    property string deviceId
    property string deviceName

    readonly property bool validDevice: page.deviceId.trim().length > 0
    readonly property bool shouldOpen: page.active && PhoneBrowser.enabled && page.validDevice
    readonly property string phoneName: page.deviceName.length > 0 ? page.deviceName : qsTr("your phone")
    readonly property string status: PhoneBrowser.status
    readonly property bool loading: page.status === "loading"
    readonly property bool browsing: page.validDevice && PhoneBrowser.enabled && page.status !== "offline"

    // The one message shown instead of the listing, or "" when the list (or a
    // spinner) owns the content area.
    readonly property string stateKind: {
        if (!page.validDevice) return "noDevice"
        if (!PhoneBrowser.enabled) return "off"
        if (page.status === "offline") return "offline"
        if (["denied", "unsupported", "notFound", "invalid", "failed"].indexOf(page.status) >= 0)
            return page.status
        if (page.status === "ready" && PhoneBrowser.count === 0) return "empty"
        return ""
    }
    readonly property bool listShown: page.stateKind === "" && PhoneBrowser.count > 0

    readonly property string folderTitle: {
        const p = PhoneBrowser.path
        if (p.length === 0) return qsTr("Phone storage")
        const parts = p.split("/").filter(s => s.length > 0)
        return parts.length > 0 ? parts[parts.length - 1] : p
    }

    property string savingPath: ""
    property string savingName: ""
    property string saveMessage: ""
    property bool saveFailed: false

    function sync() {
        if (page.shouldOpen) {
            if (PhoneBrowser.status === "hidden" || PhoneBrowser.device !== page.deviceId)
                PhoneBrowser.open(page.deviceId)
        } else {
            PhoneBrowser.close()
        }
    }

    function formatBytes(bytes) {
        if (!bytes || !isFinite(bytes) || bytes <= 0) return qsTr("Empty file")
        if (bytes < 1024) return qsTr("%1 B").arg(Math.round(bytes))
        const units = [qsTr("%1 KB"), qsTr("%1 MB"), qsTr("%1 GB"), qsTr("%1 TB")]
        let value = bytes / 1024
        let unit = 0
        while (value >= 1024 && unit < units.length - 1) { value /= 1024; unit++ }
        return units[unit].arg(unit === 0 ? Math.round(value) : (value < 10 ? value.toFixed(1) : Math.round(value)))
    }

    function formatModified(ms) {
        if (!ms || !isFinite(ms) || ms <= 0) return ""
        const date = new Date(ms)
        const today = new Date()
        if (date.toDateString() === today.toDateString())
            return date.toLocaleTimeString(Qt.locale(), Locale.ShortFormat)
        return date.toLocaleDateString(
            Qt.locale(),
            date.getFullYear() === today.getFullYear() ? "d MMM" : "d MMM yyyy"
        )
    }

    // Rows move as the listing refreshes, so files and folders are named by
    // their path; the backend saves or opens one only while it's still listed.
    function requestSave(name, path) {
        if (PhoneBrowser.saving || path.length === 0)
            return
        saveDialog.pendingName = name
        saveDialog.pendingPath = path
        saveDialog.open()
    }

    function cancelPendingSave() {
        saveDialog.pendingPath = ""
    }

    function activateCurrent() {
        const item = entryList.currentItem
        if (!item)
            return
        if (item.isDir)
            PhoneBrowser.openFolder(item.path)
        else
            page.requestSave(item.name, item.path)
    }

    onShouldOpenChanged: page.sync()
    onDeviceIdChanged: {
        page.cancelPendingSave()
        page.saveMessage = ""
        page.sync()
    }
    Component.onCompleted: page.sync()
    Component.onDestruction: PhoneBrowser.close()

    visible: active

    Connections {
        target: PhoneBrowser
        ignoreUnknownSignals: true
        function onSaveFinished(ok) {
            // Only a save this page started.
            if (page.savingPath.length === 0)
                return
            page.saveFailed = !ok
            page.saveMessage = ok
                ? qsTr("Saved “%1”").arg(page.savingName)
                : qsTr("Couldn't save “%1”").arg(page.savingName)
            page.savingPath = ""
        }
        function onPathChanged() {
            page.cancelPendingSave()
            page.saveMessage = ""
            entryList.currentIndex = 0
            entryList.positionViewAtBeginning()
        }
    }

    Shortcut {
        sequences: ["Ctrl+Up", "Alt+Up"]
        enabled: page.active && page.browsing && PhoneBrowser.canUp && !page.loading
        onActivated: PhoneBrowser.up()
    }
    Shortcut {
        sequences: ["Ctrl+R", "F5"]
        enabled: page.active && page.browsing && !page.loading
        onActivated: PhoneBrowser.refresh()
    }

    FolderDialog {
        id: saveDialog
        property string pendingName: ""
        property string pendingPath: ""
        title: qsTr("Save “%1” to").arg(pendingName)
        onAccepted: {
            const path = pendingPath
            const name = pendingName
            page.cancelPendingSave()
            if (path.length === 0 || PhoneBrowser.saving)
                return
            page.saveMessage = ""
            page.savingName = name
            page.savingPath = path
            PhoneBrowser.saveFile(path, selectedFolder.toString())
        }
        onRejected: page.cancelPendingSave()
    }

    // ---- Toolbar: up, current folder, save result, refresh ----
    Item {
        id: toolbar
        visible: page.browsing
        width: parent.width
        height: visible ? 56 : 0
        readonly property bool compact: width < 540

        IconButton {
            id: upButton
            anchors.left: parent.left
            anchors.leftMargin: 12
            anchors.verticalCenter: parent.verticalCenter
            iconPath: Icons.back
            label: qsTr("Up one folder")
            enabled: PhoneBrowser.canUp && !page.loading
            onClicked: PhoneBrowser.up()
        }

        Column {
            anchors.left: upButton.right
            anchors.leftMargin: 8
            anchors.right: toolbarActions.left
            anchors.rightMargin: 12
            anchors.verticalCenter: parent.verticalCenter
            spacing: 1
            Txt {
                width: parent.width
                role: "title"
                size: 16
                elide: Text.ElideMiddle
                text: page.folderTitle
            }
            Txt {
                width: parent.width
                role: "caption"
                muted: true
                elide: Text.ElideMiddle
                text: {
                    const where = PhoneBrowser.path.length > 0 ? PhoneBrowser.path : page.phoneName
                    if (page.status !== "ready" && page.status !== "loading")
                        return where
                    const n = PhoneBrowser.count
                    const items = PhoneBrowser.truncated ? qsTr("%1+ items").arg(n)
                        : n === 1 ? qsTr("1 item") : qsTr("%1 items").arg(n)
                    return where + " · " + items
                }
            }
        }

        Row {
            id: toolbarActions
            anchors.right: parent.right
            anchors.rightMargin: 12
            anchors.verticalCenter: parent.verticalCenter
            spacing: 8

            Spinner {
                anchors.verticalCenter: parent.verticalCenter
                width: 16; height: 16
                visible: page.loading && PhoneBrowser.count > 0
            }

            Row {
                anchors.verticalCenter: parent.verticalCenter
                spacing: 6
                visible: page.saveMessage.length > 0 || PhoneBrowser.saving
                Spinner {
                    anchors.verticalCenter: parent.verticalCenter
                    width: 14; height: 14
                    visible: PhoneBrowser.saving
                }
                Icon {
                    anchors.verticalCenter: parent.verticalCenter
                    width: 14; height: 14
                    visible: !PhoneBrowser.saving
                    path: page.saveFailed ? Icons.warning : Icons.check
                    color: page.saveFailed ? Theme.error : Theme.surfaceContentVariant
                }
                Txt {
                    anchors.verticalCenter: parent.verticalCenter
                    width: Math.min(implicitWidth, toolbar.compact ? 140 : 260)
                    role: "caption"
                    muted: !page.saveFailed
                    color: page.saveFailed ? Theme.error : Theme.surfaceContentVariant
                    elide: Text.ElideMiddle
                    text: PhoneBrowser.saving ? qsTr("Saving “%1”…").arg(page.savingName) : page.saveMessage
                    Accessible.role: Accessible.StaticText
                    Accessible.name: text
                }
            }

            IconButton {
                anchors.verticalCenter: parent.verticalCenter
                iconPath: Icons.refresh
                label: qsTr("Refresh")
                enabled: !page.loading && page.status !== "hidden"
                onClicked: PhoneBrowser.refresh()
            }
        }

        Divider { anchors.bottom: parent.bottom; width: parent.width }
    }

    // Shown only when the phone capped the listing.
    Item {
        id: truncatedNote
        anchors.top: toolbar.bottom
        width: parent.width
        visible: page.listShown && PhoneBrowser.truncated
        height: visible ? 32 : 0
        Row {
            anchors.left: parent.left
            anchors.leftMargin: 20
            anchors.verticalCenter: parent.verticalCenter
            spacing: 6
            Icon {
                anchors.verticalCenter: parent.verticalCenter
                width: 14; height: 14
                path: Icons.info
                color: Theme.surfaceContentVariant
            }
            Txt {
                anchors.verticalCenter: parent.verticalCenter
                width: Math.min(implicitWidth, truncatedNote.width - 60)
                role: "caption"
                muted: true
                text: qsTr("Showing the first %1 items in this folder.").arg(PhoneBrowser.count)
            }
        }
    }

    // ---- Content ----
    Item {
        id: content
        anchors.top: truncatedNote.bottom
        anchors.bottom: parent.bottom
        width: parent.width

        Spinner {
            anchors.centerIn: parent
            visible: page.stateKind === "" && page.loading && PhoneBrowser.count === 0
            Accessible.role: Accessible.StaticText
            Accessible.name: qsTr("Loading folder")
        }

        Column {
            anchors.centerIn: parent
            width: Math.min(parent.width - 48, 420)
            spacing: 10
            visible: page.stateKind.length > 0

            Icon {
                anchors.horizontalCenter: parent.horizontalCenter
                width: 34; height: 34
                stroke: 1.5
                path: page.stateKind === "offline" || page.stateKind === "noDevice" ? Icons.phone
                    : page.stateKind === "denied" ? Icons.lock
                    : page.stateKind === "failed" || page.stateKind === "invalid" ? Icons.warning
                    : Icons.folder
                color: Theme.surfaceContentVariant
            }
            Txt {
                width: parent.width
                horizontalAlignment: Text.AlignHCenter
                wrapMode: Text.WordWrap
                elide: Text.ElideNone
                role: "title"
                size: 16
                text: {
                    switch (page.stateKind) {
                    case "noDevice": return qsTr("Phone storage")
                    case "off": return qsTr("Phone storage isn't available")
                    case "offline": return qsTr("%1 isn't connected").arg(page.phoneName)
                    case "denied": return qsTr("Allow this Mac on %1").arg(page.phoneName)
                    case "unsupported": return qsTr("Share folders on %1").arg(page.phoneName)
                    case "notFound": return qsTr("This folder is gone")
                    case "invalid": return qsTr("This folder can't be opened")
                    case "failed": return qsTr("Couldn't load this folder")
                    case "empty": return qsTr("This folder is empty")
                    }
                    return ""
                }
            }
            Txt {
                width: parent.width
                horizontalAlignment: Text.AlignHCenter
                wrapMode: Text.WordWrap
                elide: Text.ElideNone
                role: "bodySmall"
                muted: true
                visible: text.length > 0
                text: {
                    switch (page.stateKind) {
                    case "noDevice": return qsTr("Pair your phone to browse its shared folders.")
                    case "off": return qsTr("Browsing phone folders isn't available in this version yet.")
                    case "offline": return qsTr("Connect %1 to browse its storage.").arg(page.phoneName)
                    case "denied": return qsTr("Tap Allow on the storage request on %1, or turn on its storage switch for this Mac in Settings › Devices. Then try again.").arg(page.phoneName)
                    case "unsupported": return qsTr("On %1, turn on All files access or add a folder under Selected folders only in Settings › Permissions › Photos and videos. Then try again.").arg(page.phoneName)
                    case "notFound": return qsTr("It may have been moved or deleted on %1.").arg(page.phoneName)
                    case "invalid": return qsTr("Go back to the top of the phone's storage.")
                    case "failed": return qsTr("Check that %1 is nearby and unlocked, then try again.").arg(page.phoneName)
                    }
                    return ""
                }
            }
            Row {
                anchors.horizontalCenter: parent.horizontalCenter
                spacing: 8
                visible: upStateButton.visible || topButton.visible || retryButton.visible

                Button {
                    id: upStateButton
                    visible: page.stateKind === "notFound" && PhoneBrowser.canUp
                    variant: "tonal"
                    size: "sm"
                    iconPath: Icons.back
                    text: qsTr("Go up")
                    onClicked: PhoneBrowser.up()
                }
                Button {
                    id: topButton
                    visible: (page.stateKind === "notFound" || page.stateKind === "invalid") && page.shouldOpen
                    variant: upStateButton.visible ? "text" : "tonal"
                    size: "sm"
                    text: qsTr("Back to top")
                    onClicked: PhoneBrowser.open(page.deviceId)
                }
                Button {
                    id: retryButton
                    visible: page.stateKind === "failed" || page.stateKind === "denied" || page.stateKind === "unsupported"
                    variant: page.stateKind === "failed" ? "tonal" : "text"
                    size: "sm"
                    text: qsTr("Try again")
                    onClicked: PhoneBrowser.refresh()
                }
            }
        }

        ListView {
            id: entryList
            anchors.fill: parent
            topMargin: 6
            bottomMargin: 16
            visible: page.listShown
            opacity: page.loading ? 0.6 : 1
            clip: true
            boundsBehavior: Flickable.StopAtBounds
            model: PhoneBrowser
            activeFocusOnTab: true
            keyNavigationEnabled: true
            highlightFollowsCurrentItem: false
            property bool keyboardFocus: false

            Accessible.role: Accessible.List
            Accessible.name: qsTr("Files and folders in %1").arg(page.folderTitle)

            onActiveFocusChanged: {
                if (!activeFocus)
                    keyboardFocus = false
                else if (focusReason === Qt.TabFocusReason || focusReason === Qt.BacktabFocusReason)
                    keyboardFocus = true
            }
            Keys.onPressed: (event) => {
                entryList.keyboardFocus = true
                if (event.key === Qt.Key_Return || event.key === Qt.Key_Enter) {
                    page.activateCurrent()
                    event.accepted = true
                } else if (event.key === Qt.Key_Backspace && PhoneBrowser.canUp && !page.loading) {
                    PhoneBrowser.up()
                    event.accepted = true
                }
            }

            delegate: EntryRow {}
        }
    }

    // ---- A file or folder row ----
    component EntryRow: Item {
        id: row
        required property int index
        required property string name
        required property string path
        required property bool isDir
        required property real size
        required property real modified

        readonly property string details: {
            const when = page.formatModified(row.modified)
            if (row.isDir)
                return when.length > 0 ? qsTr("Folder") + " · " + when : qsTr("Folder")
            const parts = [page.formatBytes(row.size)]
            if (when.length > 0)
                parts.push(when)
            return parts.join(" · ")
        }
        readonly property bool savingThis: PhoneBrowser.saving && page.savingPath === row.path

        width: entryList.width
        height: 52

        Accessible.role: Accessible.ListItem
        Accessible.name: row.name
        Accessible.description: row.details
        Accessible.onPressAction: {
            entryList.currentIndex = row.index
            page.activateCurrent()
        }

        Rectangle {
            anchors.fill: parent
            anchors.leftMargin: 8
            anchors.rightMargin: 8
            anchors.topMargin: 1
            anchors.bottomMargin: 1
            radius: Theme.graphite ? Theme.radiusSm : Theme.radiusMd
            color: rowMouse.containsMouse
                ? Qt.rgba(Theme.surfaceContent.r, Theme.surfaceContent.g, Theme.surfaceContent.b, 0.06)
                : "transparent"
            border.width: row.ListView.isCurrentItem && entryList.activeFocus && entryList.keyboardFocus ? 2 : 0
            border.color: Theme.primary
        }

        MouseArea {
            id: rowMouse
            anchors.fill: parent
            hoverEnabled: true
            cursorShape: row.isDir ? Qt.PointingHandCursor : Qt.ArrowCursor
            onClicked: {
                entryList.keyboardFocus = false
                entryList.forceActiveFocus(Qt.MouseFocusReason)
                entryList.currentIndex = row.index
                if (row.isDir)
                    PhoneBrowser.openFolder(row.path)
            }
        }

        Rectangle {
            id: kindBox
            x: 20
            anchors.verticalCenter: parent.verticalCenter
            width: 32; height: 32
            radius: Theme.radiusSm
            color: Theme.surfaceContainerHigh
            Icon {
                anchors.centerIn: parent
                width: 18; height: 18
                path: row.isDir ? Icons.folder : Icons.clipboardList
                color: Theme.surfaceContentVariant
            }
        }

        Column {
            anchors.left: kindBox.right
            anchors.leftMargin: 12
            anchors.right: trailing.left
            anchors.rightMargin: 12
            anchors.verticalCenter: parent.verticalCenter
            spacing: 1
            Txt {
                width: parent.width
                role: "body"
                elide: Text.ElideMiddle
                text: row.name
            }
            Txt {
                width: parent.width
                role: "caption"
                muted: true
                elide: Text.ElideRight
                text: row.details
            }
        }

        Item {
            id: trailing
            anchors.right: parent.right
            anchors.rightMargin: 18
            anchors.verticalCenter: parent.verticalCenter
            width: row.isDir ? 16 : saveButton.implicitWidth
            height: row.isDir ? 16 : saveButton.implicitHeight

            Icon {
                anchors.centerIn: parent
                visible: row.isDir
                width: 16; height: 16
                path: Icons.chevronRight
                color: Theme.surfaceContentVariant
            }
            Button {
                id: saveButton
                visible: !row.isDir
                anchors.fill: parent
                activeFocusOnTab: false
                variant: "tonal"
                size: "sm"
                iconPath: Icons.download
                text: qsTr("Save…")
                busy: row.savingThis
                enabled: !PhoneBrowser.saving
                Accessible.name: qsTr("Save %1").arg(row.name)
                onClicked: {
                    entryList.currentIndex = row.index
                    page.requestSave(row.name, row.path)
                }
            }
        }
    }
}
