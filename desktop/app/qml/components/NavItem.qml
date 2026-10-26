// SPDX-License-Identifier: GPL-3.0-or-later
import QtQuick
import QtQuick.Controls as Controls
import app.nectarlink

// A destination adapts from a compact rail to a labeled desktop sidebar.
FocusScope {
    id: item
    property string iconPath
    property string text
    property bool selected: false
    property bool expanded: false
    readonly property bool compactMac: !expanded && (Qt.platform.os === "osx" || Qt.platform.os === "macos")
    readonly property alias hovered: mouse.containsMouse
    property bool activeHover: mouse.containsMouse
    signal clicked

    implicitWidth: 72
    implicitHeight: expanded ? 40 : (compactMac ? 54 : 52)
    activeFocusOnTab: true

    Controls.ToolTip.visible: !item.expanded && mouse.containsMouse
    Controls.ToolTip.text: item.text
    Controls.ToolTip.delay: 450

    Accessible.role: Accessible.PageTab
    Accessible.name: text
    Accessible.selected: selected
    Accessible.onPressAction: item.clicked()
    Keys.onPressed: (event) => {
        if (event.key === Qt.Key_Return || event.key === Qt.Key_Enter || event.key === Qt.Key_Space) {
            item.clicked()
            event.accepted = true
        }
    }

    Rectangle {
        id: pill
        anchors.horizontalCenter: parent.horizontalCenter
        y: item.compactMac ? 0 : 2
        width: item.expanded ? item.width : (item.compactMac ? 54 : 52)
        height: item.expanded ? 36 : (item.compactMac ? 52 : 30)
        radius: item.expanded ? Theme.controlRadius : 26
        color: "transparent"
        scale: mouse.pressed ? Theme.pressScale : 1.0
        Behavior on scale { enabled: !Theme.reduceMotion; NumberAnimation { duration: 150; easing.type: Easing.OutCubic } }

        Rectangle {
            anchors.centerIn: parent
            height: parent.height
            width: item.compactMac || item.selected || item.activeHover ? parent.width : 24
            radius: parent.radius
            color: item.selected ? Theme.railSelected : Theme.railContent
            opacity: item.selected ? 1.0 : (item.activeHover ? 0.08 : 0.0)
            border.width: 0
            border.color: Theme.panelBorder
            Behavior on width {
                enabled: !Theme.reduceMotion
                NumberAnimation { duration: 150; easing.type: Easing.OutCubic }
            }
            Behavior on opacity {
                enabled: !Theme.reduceMotion
                NumberAnimation { duration: Theme.fadeFast; easing.type: Easing.OutCubic }
            }
            Behavior on color {
                enabled: !Theme.reduceMotion && !item.compactMac
                ColorAnimation { duration: Theme.fadeFast }
            }
        }

        Icon {
            anchors.verticalCenter: item.compactMac ? undefined : parent.verticalCenter
            y: item.compactMac ? 7 : 0
            anchors.horizontalCenter: item.expanded ? undefined : parent.horizontalCenter
            x: item.expanded ? Theme.spacingMd : 0
            width: item.compactMac ? 24 : 20; height: width
            path: item.iconPath
            selected: item.selected
            color: item.selected || item.activeHover ? Theme.railContent : Theme.railMuted
            scale: item.compactMac || Theme.reduceMotion ? 1.0 : (item.selected ? 1.05 : (item.activeHover ? 1.02 : 1.0))
            Behavior on scale {
                enabled: !Theme.reduceMotion
                NumberAnimation { duration: 160; easing.type: Easing.OutCubic }
            }
            Behavior on color {
                enabled: !Theme.reduceMotion && !item.compactMac
                ColorAnimation { duration: Theme.fadeFast }
            }
        }
    }
    Rectangle {
        visible: false // The selected circular surface already identifies the current page.
        x: 0
        y: 17
        width: 3; height: 20
        radius: 1.5
        color: Theme.railContent
    }
    Txt {
        anchors.left: item.expanded ? pill.left : parent.left
        anchors.right: item.expanded ? pill.right : parent.right
        anchors.leftMargin: item.expanded ? 44 : 0
        anchors.rightMargin: item.expanded ? Theme.spacingMd : 0
        anchors.top: item.expanded || item.compactMac ? pill.top : pill.bottom
        anchors.topMargin: item.compactMac ? 34 : (item.expanded ? 0 : 4)
        height: item.compactMac ? 14 : (item.expanded ? pill.height : implicitHeight)
        horizontalAlignment: item.expanded ? Text.AlignLeft : Text.AlignHCenter
        verticalAlignment: Text.AlignVCenter
        elide: Text.ElideRight
        text: item.text
        role: item.expanded ? "body" : "caption"
        size: item.compactMac ? 10 : Math.round(graphiteLabel ? Theme.labelStyle.size : spec.size)
        weight: item.selected ? 600 : 500
        color: item.selected || item.activeHover ? Theme.railContent : Theme.railMuted
        Behavior on color {
            enabled: !Theme.reduceMotion && !item.compactMac
            ColorAnimation { duration: Theme.fadeFast }
        }
    }
    Rectangle {
        anchors.fill: pill
        anchors.margins: item.compactMac ? 0 : -4
        radius: pill.radius + (item.compactMac ? 0 : 4)
        color: "transparent"
        border.width: 2
        border.color: Theme.railContent
        visible: Theme.focusVisible(item)
    }
    MouseArea {
        id: mouse
        anchors.fill: parent
        hoverEnabled: true
        cursorShape: Qt.PointingHandCursor
        onClicked: item.clicked()
    }
}
