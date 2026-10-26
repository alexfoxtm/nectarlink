// SPDX-License-Identifier: GPL-3.0-or-later
import QtQuick
import app.nectarlink

// A modal dialog over the window: a scrim and a card that springs in.
// Escape or a click on the scrim closes it (unless `dismissable` is false).
Item {
    id: sheet
    property bool opened: false
    property bool dismissable: true
    property real cardWidth: 460
    default property alias content: body.data
    signal dismissed

    function open() {
        opened = true
        if (!sheet.activeFocus)
            sheet.forceActiveFocus(Qt.OtherFocusReason)
    }
    function close() { opened = false }

    anchors.fill: parent
    visible: opened || fade.running || cardFade.running
    z: 100

    Shortcut {
        sequence: "Esc"
        enabled: sheet.opened && sheet.dismissable
        onActivated: {
            sheet.close()
            sheet.dismissed()
        }
    }

    Rectangle {
        id: scrim
        anchors.fill: parent
        color: Theme.scrim
        opacity: sheet.opened ? 1 : 0
        Behavior on opacity { NumberAnimation { id: fade; duration: Theme.reduceMotion ? 120 : 180 } }
        // Only taps outside the card: a tap on the card (or a button in it)
        // reaches this handler too.
        TapHandler {
            onTapped: (eventPoint) => {
                const p = card.mapFromItem(scrim, eventPoint.position)
                const inside = p.x >= 0 && p.y >= 0 && p.x <= card.width && p.y <= card.height
                if (!inside && sheet.dismissable) { sheet.close(); sheet.dismissed() }
            }
        }
    }

    Rectangle {
        id: card
        anchors.horizontalCenter: parent.horizontalCenter
        anchors.bottom: parent.bottom
        anchors.bottomMargin: 8
        width: Math.min(sheet.cardWidth, parent.width - 16)
        height: Math.min(body.childrenRect.height + 52, parent.height - 16)
        topLeftRadius: 36
        topRightRadius: 36
        bottomLeftRadius: 32
        bottomRightRadius: 32
        color: Theme.sheetColor
        border.width: 1
        border.color: Theme.panelBorder
        opacity: sheet.opened ? 1 : 0
        scale: sheet.opened || Theme.reduceMotion ? 1 : 0.98
        Behavior on opacity { NumberAnimation { id: cardFade; duration: Theme.reduceMotion ? 120 : (sheet.opened ? 260 : 220); easing.type: Easing.OutCubic } }
        Behavior on scale { enabled: !Theme.reduceMotion; SpringAnimation { spring: Theme.springStandard; damping: 1.0 } }
        clip: true

        Rectangle {
            anchors.horizontalCenter: parent.horizontalCenter
            y: 4
            width: 36; height: 4
            radius: 2
            color: Theme.surfaceContentVariant
            opacity: 0.35
        }

        Flickable {
            anchors.fill: parent
            anchors.leftMargin: 16
            anchors.rightMargin: 16
            anchors.topMargin: 28
            anchors.bottomMargin: 24
            contentWidth: width
            contentHeight: body.childrenRect.height
            interactive: contentHeight > height
            boundsBehavior: Flickable.StopAtBounds
            clip: true
            Item {
                id: body
                width: parent.width
                height: childrenRect.height
            }
        }
    }

    focus: opened
    Keys.onEscapePressed: if (sheet.dismissable) { sheet.close(); sheet.dismissed() }
}
