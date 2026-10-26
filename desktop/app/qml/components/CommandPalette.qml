// SPDX-License-Identifier: GPL-3.0-or-later
import QtQuick
import QtQuick.Effects
import app.nectarlink

// Fast, keyboard-first Command Palette modal overlay (PLAN.md §3.9 / §3.11 P1).
// Loaded lazily via a Loader in MainWindow.qml only while open, so it costs
// zero memory and zero CPU while closed.
Item {
    id: palette
    anchors.fill: parent
    z: 150

    property Item backdropSource: null
    property bool backdropSamplingEnabled: false
    signal dismissed

    property int selectedIndex: 0
    readonly property var items: {
        const raw = AppController.commandPaletteResults
        if (!raw || raw.length === 0)
            return []
        try {
            return JSON.parse(raw)
        } catch (e) {
            return []
        }
    }

    onItemsChanged: {
        if (selectedIndex >= items.length)
            selectedIndex = Math.max(0, items.length - 1)
    }

    function runSelected() {
        if (!AppController.commandPaletteOpen || items.length === 0)
            return
        const idx = Math.max(0, Math.min(selectedIndex, items.length - 1))
        const item = items[idx]
        if (item && item.id && item.enabled !== false)
            AppController.runCommandPalette(item.id)
    }

    property bool shown: false
    opacity: shown && AppController.commandPaletteOpen ? 1 : 0
    Behavior on opacity { NumberAnimation { duration: Theme.reduceMotion ? 120 : (AppController.commandPaletteOpen ? 260 : 220); easing.type: Easing.OutCubic } }

    Component.onCompleted: {
        shown = true
        searchInput.forceActiveFocus()
        Qt.callLater(() => AppController.noteCommandPaletteShown())
    }

    Shortcut {
        sequence: "Esc"
        enabled: AppController.commandPaletteOpen
        onActivated: { palette.dismissed(); AppController.closeCommandPalette() }
    }

    Rectangle {
        id: scrim
        anchors.fill: parent
        color: Theme.scrim
        TapHandler {
            onTapped: (eventPoint) => {
                const p = card.mapFromItem(scrim, eventPoint.position)
                const inside = p.x >= 0 && p.y >= 0 && p.x <= card.width && p.y <= card.height
                if (!inside) {
                    palette.dismissed()
                    AppController.closeCommandPalette()
                }
            }
        }
    }

    Rectangle {
        id: card
        anchors.horizontalCenter: parent.horizontalCenter
        anchors.bottom: parent.bottom
        anchors.bottomMargin: 8
        width: Math.min(640, parent.width - 16)
        height: Math.min(cardColumn.height, palette.height - 16)
        topLeftRadius: 36
        topRightRadius: 36
        bottomLeftRadius: 32
        bottomRightRadius: 32
        color: "transparent"
        layer.enabled: paletteMaterial.effectsSupported
        layer.smooth: true
        layer.effect: MultiEffect {
            maskEnabled: true
            maskSource: paletteMaterial.maskSource
            maskThresholdMin: 0.5
            maskSpreadAtMin: 1.0
        }
        border.width: 1
        border.color: Theme.panelBorder
        clip: true
        transform: Translate {
            id: cardSlide
            y: palette.shown && AppController.commandPaletteOpen || Theme.reduceMotion ? 0 : 24
            Behavior on y {
                enabled: !Theme.reduceMotion
                NumberAnimation { duration: AppController.commandPaletteOpen ? 260 : 220; easing.type: Easing.OutCubic }
            }
        }

        MaterialBackdrop {
            id: paletteMaterial
            anchors.fill: parent
            sourceItem: palette.backdropSource
            samplingEnabled: palette.backdropSamplingEnabled && AppController.commandPaletteOpen
            topRadius: 36
            bottomRadius: 32
            sampleRect: {
                if (!sourceItem)
                    return Qt.rect(0, 0, width, height)
                const p = sourceItem.mapFromItem(palette, card.x, card.y + cardSlide.y)
                return Qt.rect(p.x, p.y, width, height)
            }
        }

        Column {
            id: cardColumn
            width: parent.width

            Item {
                width: parent.width
                height: 16
                Rectangle {
                    anchors.horizontalCenter: parent.horizontalCenter
                    y: 4
                    width: 36; height: 4
                    radius: 2
                    color: Theme.surfaceContentVariant
                    opacity: 0.35
                }
            }

            // ---- Search bar ----
            Item {
                width: parent.width
                height: 64

                Icon {
                    id: searchIcon
                    anchors.left: parent.left
                    anchors.leftMargin: 18
                    anchors.verticalCenter: parent.verticalCenter
                    width: 18
                    height: 18
                    path: Icons.search
                    color: Theme.surfaceContentVariant
                }

                Item {
                    anchors.left: searchIcon.right
                    anchors.leftMargin: 12
                    anchors.right: escBadge.left
                    anchors.rightMargin: 10
                    anchors.top: parent.top
                    anchors.bottom: parent.bottom

                    Txt {
                        anchors.verticalCenter: parent.verticalCenter
                        visible: searchInput.text.length === 0
                        text: qsTr("Type a command, contact, or app…")
                        role: "body"
                        muted: true
                    }

                    TextInput {
                        id: searchInput
                        anchors.fill: parent
                        verticalAlignment: TextInput.AlignVCenter
                        font.family: Theme.fontUi
                        font.pixelSize: Math.round(16 * Theme.textScale)
                        color: Theme.surfaceContent
                        selectionColor: Theme.primaryContainer
                        selectedTextColor: Theme.primaryContainerContent
                        focus: AppController.commandPaletteOpen
                        enabled: AppController.commandPaletteOpen
                        clip: true
                        Accessible.name: qsTr("Command palette search")

                        onTextChanged: {
                            palette.selectedIndex = 0
                            AppController.searchCommandPalette(searchInput.text)
                        }

                        Keys.onPressed: (event) => {
                            if (event.key === Qt.Key_Down) {
                                if (palette.items.length > 0) {
                                    palette.selectedIndex = (palette.selectedIndex + 1) % palette.items.length
                                    resultsList.positionViewAtIndex(palette.selectedIndex, ListView.Contain)
                                }
                                event.accepted = true
                            } else if (event.key === Qt.Key_Up) {
                                if (palette.items.length > 0) {
                                    palette.selectedIndex = (palette.selectedIndex - 1 + palette.items.length) % palette.items.length
                                    resultsList.positionViewAtIndex(palette.selectedIndex, ListView.Contain)
                                }
                                event.accepted = true
                            } else if (event.key === Qt.Key_Return || event.key === Qt.Key_Enter) {
                                palette.runSelected()
                                event.accepted = true
                            } else if (event.key === Qt.Key_Escape) {
                                palette.dismissed()
                                AppController.closeCommandPalette()
                                event.accepted = true
                            }
                        }
                    }
                }

                Rectangle {
                    id: escBadge
                    anchors.right: parent.right
                    anchors.rightMargin: 16
                    anchors.verticalCenter: parent.verticalCenter
                    width: escLabel.implicitWidth + 12
                    height: 40
                    radius: 20
                    color: Theme.surfaceContainerHighest
                    border.width: 1
                    border.color: Theme.panelBorder
                    activeFocusOnTab: true

                    function activate() {
                        if (searchInput.text.length > 0) {
                            searchInput.text = ""
                            searchInput.forceActiveFocus()
                        } else {
                            palette.dismissed()
                            AppController.closeCommandPalette()
                        }
                    }

                    Accessible.role: Accessible.Button
                    Accessible.name: searchInput.text.length > 0 ? qsTr("Clear search") : qsTr("Close command palette")
                    Accessible.onPressAction: escBadge.activate()

                    Keys.onPressed: (event) => {
                        if (event.key === Qt.Key_Return || event.key === Qt.Key_Enter || event.key === Qt.Key_Space) {
                            escBadge.activate()
                            event.accepted = true
                        }
                    }

                    Rectangle {
                        anchors.fill: parent
                        anchors.margins: -3
                        radius: parent.radius + 3
                        color: "transparent"
                        border.width: 2
                        border.color: Theme.primary
                        visible: Theme.focusVisible(escBadge)
                    }

                    Txt {
                        id: escLabel
                        anchors.centerIn: parent
                        text: searchInput.text.length > 0 ? qsTr("Clear") : qsTr("Esc")
                        role: "caption"
                        size: 11
                        muted: true
                    }

                    HoverHandler { cursorShape: Qt.PointingHandCursor }
                    TapHandler { onTapped: escBadge.activate() }
                }
            }

            Divider { width: parent.width }

            // ---- Results list ----
            Item {
                width: parent.width
                height: palette.items.length > 0
                    ? Math.min(resultsList.contentHeight + 16, Math.max(64, palette.height - 176), 384) : 88

                ListView {
                    id: resultsList
                    anchors.fill: parent
                    anchors.margins: Theme.spacingSm
                    clip: true
                    boundsBehavior: Flickable.StopAtBounds
                    visible: palette.items.length > 0
                    model: palette.items
                    spacing: Theme.spacingXs

                    delegate: Rectangle {
                        id: row
                        required property var modelData
                        required property int index
                        readonly property bool selected: index === palette.selectedIndex
                        readonly property bool isEnabled: modelData.enabled !== false
                        readonly property string explanation: modelData.reason
                            ? qsTr("Not yet available on Mac") : (modelData.subtitle || "")

                        width: resultsList.width
                        height: Math.max(76, Math.round(64 * Theme.textScale))
                        radius: 16
                        color: selected
                            ? Theme.insetColor
                            : rowHover.hovered
                              ? Theme.insetColor
                              : "transparent"

                        Accessible.role: Accessible.ListItem
                        Accessible.name: (modelData.title || "") + (row.explanation ? ", " + row.explanation : "")
                        Accessible.onPressAction: {
                            if (AppController.commandPaletteOpen && row.isEnabled) {
                                palette.selectedIndex = row.index
                                AppController.runCommandPalette(row.modelData.id)
                            }
                        }

                        HoverHandler {
                            id: rowHover
                            cursorShape: row.isEnabled ? Qt.PointingHandCursor : Qt.ArrowCursor
                            onHoveredChanged: {
                                if (hovered)
                                    palette.selectedIndex = row.index
                            }
                        }

                        TapHandler {
                            enabled: AppController.commandPaletteOpen && row.isEnabled
                            onTapped: {
                                palette.selectedIndex = row.index
                                AppController.runCommandPalette(row.modelData.id)
                            }
                        }

                        Rectangle {
                            id: iconBox
                            anchors.left: parent.left
                            anchors.leftMargin: 10
                            anchors.verticalCenter: parent.verticalCenter
                            width: 40
                            height: 40
                            radius: Theme.radiusSm
                            color: row.selected ? Theme.primaryContainer : Theme.surfaceContainerHighest

                            Icon {
                                anchors.centerIn: parent
                                width: 20
                                height: 20
                                path: (row.modelData.icon && Icons[row.modelData.icon])
                                    ? Icons[row.modelData.icon]
                                    : Icons.terminal
                                color: row.selected ? Theme.primaryContainerContent : Theme.surfaceContent
                            }
                        }

                        Column {
                            anchors.left: iconBox.right
                            anchors.leftMargin: 12
                            anchors.right: badgePill.visible ? badgePill.left : parent.right
                            anchors.rightMargin: 12
                            anchors.verticalCenter: parent.verticalCenter
                            spacing: 1
                            opacity: row.isEnabled ? 1.0 : 0.6

                            Txt {
                                width: parent.width
                                text: row.modelData.title || ""
                                role: "label"
                                elide: Text.ElideRight
                                size: 16
                                color: row.selected ? Theme.secondaryContainerContent : Theme.surfaceContent
                            }

                            Txt {
                                width: parent.width
                                visible: row.explanation.length > 0
                                text: row.explanation
                                role: "caption"
                                elide: Text.ElideRight
                                size: 12
                                muted: !row.selected
                                color: row.selected ? Theme.secondaryContainerContent : Theme.surfaceContentVariant
                            }
                        }

                        Rectangle {
                            id: badgePill
                            readonly property string badgeText: (row.modelData.badge && row.modelData.badge.length > 0)
                                ? row.modelData.badge
                                : (row.modelData.category || "")
                            visible: badgeText.length > 0
                            anchors.right: parent.right
                            anchors.rightMargin: 12
                            anchors.verticalCenter: parent.verticalCenter
                            width: badgeLabel.implicitWidth + 14
                            height: 22
                            radius: Theme.pill(height)
                            color: row.selected ? Theme.surfaceContainerHigh : Theme.surfaceContainerHighest

                            Txt {
                                id: badgeLabel
                                anchors.centerIn: parent
                                text: badgePill.badgeText
                                role: "caption"
                                size: 11
                                muted: true
                            }
                        }
                    }
                }

                Txt {
                    anchors.centerIn: parent
                    visible: palette.items.length === 0
                    text: qsTr("No matching commands")
                    role: "body"
                    muted: true
                }
            }

            Divider { width: parent.width }

            // ---- Footer ----
            Item {
                width: parent.width
                height: 40

                Txt {
                    anchors.left: parent.left
                    anchors.leftMargin: 16
                    anchors.verticalCenter: parent.verticalCenter
                    text: qsTr("↑↓ to navigate · Enter to run · Esc to close")
                    role: "caption"
                    size: 11
                    muted: true
                }

                Txt {
                    anchors.right: parent.right
                    anchors.rightMargin: 16
                    anchors.verticalCenter: parent.verticalCenter
                    text: {
                        const hk = Qt.platform.os === "osx" ? "⌘K"
                            : (Preferences.commandPaletteHotkey.length > 0
                                ? Preferences.commandPaletteHotkey
                                : "Ctrl+K")
                        if (AppController.commandPaletteOpenMs <= 0)
                            return hk
                        const msText = AppController.commandPaletteOpenMs < 10
                            ? AppController.commandPaletteOpenMs.toFixed(1)
                            : String(Math.round(AppController.commandPaletteOpenMs))
                        return qsTr("%1 · %2 ms").arg(hk).arg(msText)
                    }
                    role: "caption"
                    size: 11
                    muted: true
                }
            }
        }
    }
}
