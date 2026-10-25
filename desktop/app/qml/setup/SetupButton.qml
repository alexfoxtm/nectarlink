// SPDX-License-Identifier: GPL-3.0-or-later
import QtQuick

// A button in the setup window: filled (the main action) or quiet.
Item {
    id: button

    property string text
    property bool filled: true
    property color fill
    property color ink
    property color focusColor
    signal clicked()

    implicitWidth: Math.max(filled ? 168 : 120, label.implicitWidth + 32)
    implicitHeight: 40
    activeFocusOnTab: true
    opacity: enabled ? 1 : 0.5

    Accessible.role: Accessible.Button
    Accessible.name: text
    Accessible.onPressAction: button.clicked()
    Keys.onReturnPressed: button.clicked()
    Keys.onEnterPressed: button.clicked()
    Keys.onSpacePressed: button.clicked()

    Rectangle {
        anchors.fill: parent
        radius: 12
        color: button.filled ? button.fill : "transparent"
        border.width: button.filled ? 0 : 1
        border.color: Qt.rgba(button.ink.r, button.ink.g, button.ink.b, 0.22)
        // Keep the label and hit target stable while pressed.

        Rectangle {
            anchors.fill: parent
            radius: parent.radius
            color: button.filled ? "#FFFFFF" : button.ink
            opacity: area.pressed ? 0.14 : (area.containsMouse ? 0.08 : 0)
            Behavior on opacity { NumberAnimation { duration: 140 } }
        }
    }
    Rectangle {
        anchors.fill: parent
        anchors.margins: -4
        radius: 12
        color: "transparent"
        border.width: 2
        border.color: button.focusColor
        visible: button.activeFocus
    }
    Text {
        id: label
        anchors.centerIn: parent
        text: button.text
        font.family: "Figtree"
        font.pixelSize: 14
        font.weight: Font.DemiBold
        color: button.ink
    }
    MouseArea {
        id: area
        anchors.fill: parent
        hoverEnabled: true
        cursorShape: Qt.PointingHandCursor
        onClicked: button.clicked()
    }
}
