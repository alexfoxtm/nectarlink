// SPDX-License-Identifier: GPL-3.0-or-later
import QtQuick
import app.nectarlink

// A compact status label with optional icon. `selected` uses the indicator colors.
Rectangle {
    id: chip
    property string text
    property string iconPath
    property bool selected: false
    property bool interactive: false
    signal clicked()

    implicitHeight: interactive ? 40 : 28
    implicitWidth: row.implicitWidth + 24
    radius: Theme.pill(height)
    color: selected ? Theme.primaryContainer : Theme.insetColor
    Behavior on color { enabled: !Theme.reduceMotion; ColorAnimation { duration: Theme.fadeFast } }
    border.width: 1
    border.color: Theme.panelBorder
    scale: chip.interactive && mouse.pressed && !Theme.reduceMotion ? Theme.pressScale : 1.0
    Behavior on scale { enabled: !Theme.reduceMotion; NumberAnimation { duration: 120; easing.type: Easing.OutCubic } }

    activeFocusOnTab: interactive && visible && enabled
    Accessible.role: interactive ? Accessible.Button : Accessible.StaticText
    Accessible.name: chip.text
    Accessible.onPressAction: if (chip.interactive) chip.clicked()
    Keys.onPressed: (event) => {
        if (chip.interactive && (event.key === Qt.Key_Return || event.key === Qt.Key_Enter || event.key === Qt.Key_Space)) {
            chip.clicked()
            event.accepted = true
        }
    }

    Rectangle {
        anchors.fill: parent
        radius: chip.radius
        color: label.color
        opacity: !chip.interactive ? 0 : (mouse.pressed ? 0.14 : (mouse.containsMouse ? 0.07 : 0))
        Behavior on opacity { enabled: !Theme.reduceMotion; NumberAnimation { duration: Theme.fadeFast } }
    }

    Rectangle {
        anchors.fill: parent
        anchors.margins: -4
        radius: chip.radius + 4
        color: "transparent"
        border.width: 2
        border.color: Theme.primary
        visible: chip.interactive && Theme.focusVisible(chip)
    }

    Row {
        id: row
        anchors.centerIn: parent
        spacing: Theme.spacingXs
        Icon {
            visible: chip.iconPath.length > 0
            anchors.verticalCenter: parent.verticalCenter
            width: 15; height: 15
            path: chip.iconPath
            color: label.color
        }
        Txt {
            id: label
            anchors.verticalCenter: parent.verticalCenter
            text: chip.text
            role: "label"
            color: chip.selected ? Theme.primaryContainerContent : Theme.surfaceContentVariant
        }
    }

    MouseArea {
        id: mouse
        anchors.fill: parent
        enabled: chip.interactive
        hoverEnabled: chip.interactive
        cursorShape: chip.interactive ? Qt.PointingHandCursor : Qt.ArrowCursor
        onClicked: chip.clicked()
    }
}
