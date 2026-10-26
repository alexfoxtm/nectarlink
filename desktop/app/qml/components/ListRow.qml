// SPDX-License-Identifier: GPL-3.0-or-later
import QtQuick
import app.nectarlink

// A settings-style row: optional icon, title and description, and a
// trailing control placed in `trailing`.
Item {
    id: row
    property string iconPath
    property string title
    property string description
    property bool stackBelow: false
    default property alias trailing: trailingSlot.data

    readonly property bool stackTrailing: stackBelow || (trailingSlot.childrenRect.width > row.width * 0.52 && row.width > 0)

    implicitHeight: stackTrailing
        ? texts.implicitHeight + trailingSlot.childrenRect.height + 34
        : Math.max(row.description.length > 0 ? 76 : 64, texts.implicitHeight + 24, trailingSlot.childrenRect.height + 24)

    Rectangle {
        id: iconWell
        visible: row.iconPath.length > 0
        width: 40; height: 40
        radius: 20
        anchors.left: parent.left
        anchors.verticalCenter: row.stackTrailing ? texts.verticalCenter : parent.verticalCenter
        color: Theme.insetColor
        Icon {
            anchors.centerIn: parent
            width: 20; height: 20
            path: row.iconPath
            color: Theme.surfaceContentVariant
        }
    }
    Column {
        id: texts
        anchors.left: iconWell.visible ? iconWell.right : parent.left
        anchors.leftMargin: iconWell.visible ? 12 : 0
        anchors.right: row.stackTrailing ? parent.right : trailingSlot.left
        anchors.rightMargin: row.stackTrailing ? 0 : 16
        y: row.stackTrailing ? 12 : Math.round((row.height - height) / 2)
        spacing: Theme.spacingXs
        Txt {
            width: parent.width
            text: row.title
            role: "body"
            size: 16
            wrapMode: Text.WordWrap
            elide: Text.ElideNone
        }
        Txt {
            visible: row.description.length > 0
            width: parent.width
            text: row.description
            role: "bodySmall"
            muted: true
            wrapMode: Text.WordWrap
            elide: Text.ElideNone
        }
    }
    Item {
        id: trailingSlot
        anchors.left: row.stackTrailing ? texts.left : undefined
        anchors.right: row.stackTrailing ? undefined : parent.right
        y: row.stackTrailing ? (texts.y + texts.implicitHeight + 10) : Math.round((row.height - height) / 2)
        width: childrenRect.width
        height: childrenRect.height
    }
}
