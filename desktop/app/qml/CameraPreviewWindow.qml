// SPDX-License-Identifier: GPL-3.0-or-later
import QtQuick
import QtQuick.Window
import QtQuick.Controls as Controls
import app.nectarlink

Window {
    id: win
    visible: true
    color: "black"
    width: 960
    height: 540
    minimumWidth: 320
    minimumHeight: 180
    title: qsTr("Camera preview · %1p · Escape to stop").arg(Webcam.height)
    onClosing: Webcam.showPreviewWindow(false)

    Shortcut {
        sequence: "Escape"
        context: Qt.WindowShortcut
        onActivated: win.close()
    }

    VideoView {
        anchors.fill: parent
        stream: "webcam"
        focus: true
        Accessible.role: Accessible.Grouping
        Accessible.name: qsTr("Phone camera preview")
        Accessible.description: qsTr("Camera preview for window capture. Press Escape or close this window to stop the camera. Right-click for Stop.")
        MouseArea {
            anchors.fill: parent
            acceptedButtons: Qt.RightButton
            onClicked: stopMenu.popup()
        }
    }

    Controls.Menu {
        id: stopMenu
        Controls.MenuItem {
            text: qsTr("Stop camera preview")
            onTriggered: win.close()
        }
    }

    // Keep the captured client area clean while live. Pending and failures
    // remain visible instead of presenting a frozen image as live video.
    Rectangle {
        anchors.fill: parent
        color: "black"
        visible: Webcam.phase !== "streaming"
        Column {
            anchors.centerIn: parent
            width: Math.max(0, parent.width - 48)
            spacing: 16
            Txt {
                width: parent.width
                color: "white"
                horizontalAlignment: Text.AlignHCenter
                wrapMode: Text.WordWrap
                text: Webcam.statusText.length > 0 ? Webcam.statusText
                    : qsTr("Start camera preview from Camera & Audio settings.")
                Accessible.role: Accessible.StaticText
                Accessible.name: text
            }
            Button {
                anchors.horizontalCenter: parent.horizontalCenter
                text: qsTr("Stop and close")
                onClicked: win.close()
            }
        }
    }
}
