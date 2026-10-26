// SPDX-License-Identifier: GPL-3.0-or-later
import QtQuick
import app.nectarlink

// A native switch. Click, Space or Enter flips it; `toggled` reports the
// user's choice (bind `checked` to the source of truth).
FocusScope {
    id: toggle
    property bool checked: false
    property string label
    signal toggled(bool checked)

    implicitWidth: 46
    implicitHeight: 36
    activeFocusOnTab: true
    opacity: enabled ? 1 : 0.38

    Accessible.role: Accessible.CheckBox
    Accessible.name: label
    Accessible.checkable: true
    Accessible.checked: checked
    Accessible.onToggleAction: if (toggle.enabled) toggle.toggled(!toggle.checked)
    Keys.onPressed: (event) => {
        if (!toggle.enabled) return
        if (event.key === Qt.Key_Space || event.key === Qt.Key_Return || event.key === Qt.Key_Enter) {
            toggle.toggled(!toggle.checked)
            event.accepted = true
        }
    }

    Rectangle {
        id: track
        anchors.centerIn: parent
        width: parent.width
        height: 28
        radius: height / 2
        color: toggle.checked ? Theme.actionColor : Theme.surfaceContainerHighest
        border.width: 0
        Behavior on color { ColorAnimation { duration: Theme.fadeFast } }

        Rectangle {
            id: thumb
            property real size: 20
            width: mouse.pressed ? 22 : size
            height: size
            radius: height / 2
            anchors.verticalCenter: parent.verticalCenter
            x: toggle.checked ? parent.width - width - 4 : 4
            color: toggle.checked ? Theme.actionContent : (Theme.dark ? "#CFC2E4" : "#FFFFFF")
            border.width: 0
            // Quick and settled, never springy: a switch must feel immediate.
            Behavior on x { enabled: !Theme.reduceMotion; NumberAnimation { duration: Theme.reduceMotion ? 0 : 140; easing.type: Easing.OutCubic } }
            Behavior on width { enabled: !Theme.reduceMotion; NumberAnimation { duration: Theme.fadeFast; easing.type: Easing.OutCubic } }
            Behavior on height { enabled: !Theme.reduceMotion; NumberAnimation { duration: Theme.fadeFast; easing.type: Easing.OutCubic } }
            Behavior on color { enabled: !Theme.reduceMotion; ColorAnimation { duration: Theme.fadeFast } }
        }
    }
    Rectangle {
        anchors.fill: track
        anchors.margins: -4
        radius: track.radius + 4
        color: "transparent"
        border.width: 2
        border.color: Theme.primary
        visible: Theme.focusVisible(toggle)
    }
    MouseArea {
        id: mouse
        anchors.fill: parent
        hoverEnabled: true
        cursorShape: Qt.PointingHandCursor
        onClicked: if (toggle.enabled) toggle.toggled(!toggle.checked)
    }
}
