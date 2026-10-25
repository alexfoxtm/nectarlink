// SPDX-License-Identifier: GPL-3.0-or-later
import QtQuick
import app.nectarlink

// First run: pair a phone. Also covers the moment before the core is ready
// and the (rare) case where it failed to start.
Item {
    id: page

    readonly property string status: AppController.status

    Flickable {
        id: welcomeFlick
        anchors.fill: parent
        contentWidth: width
        contentHeight: Math.max(height, welcomeContent.implicitHeight + 48)
        clip: true
        boundsBehavior: Flickable.StopAtBounds

        Column {
            id: welcomeContent
            x: Math.round((welcomeFlick.width - width) / 2)
            y: Math.max(24, Math.round((welcomeFlick.height - implicitHeight) / 2))
            width: Math.min(560, welcomeFlick.width - 48)
            spacing: 24

            Rectangle {
                width: parent.width
                height: 120
                radius: Theme.heroRadius
                color: Theme.heroWash
                Icon {
                    anchors.centerIn: parent
                    width: 40; height: 40
                    path: Icons.mark
                    stroke: 2.4
                    color: Theme.primary
                }
            }
            Column {
                width: parent.width
                spacing: 6
                Txt {
                    width: parent.width
                    horizontalAlignment: Text.AlignLeft
                    text: qsTr("Welcome to Waypair Link")
                    role: "displaySmall"
                    size: 30
                    wrapMode: Text.WordWrap
                }
                Txt {
                    width: parent.width
                    horizontalAlignment: Text.AlignLeft
                    wrapMode: Text.WordWrap
                    role: "body"
                    muted: true
                    text: page.status === "failed"
                          ? qsTr("Waypair Link couldn't start.")
                          : qsTr("Your phone and this PC, together. Pair once and they find each other from then on.")
                }
            }

            Card {
                width: parent.width
                padding: 24
                visible: page.status === "ready"
                PairingPanel {
                    width: parent.width
                    cancellable: false
                    running: page.visible && page.status === "ready"
                }
            }

            Spinner {
                anchors.horizontalCenter: parent.horizontalCenter
                visible: page.status === "starting"
                width: 28; height: 28
            }

            Column {
                width: parent.width
                spacing: 12
                visible: page.status === "failed"
                Txt {
                    width: parent.width
                    horizontalAlignment: Text.AlignLeft
                    wrapMode: Text.WordWrap
                    role: "code"
                    size: 12
                    muted: true
                    text: AppController.error
                }
                Button {
                    anchors.horizontalCenter: parent.horizontalCenter
                    variant: "tonal"
                    iconPath: Icons.folder
                    text: qsTr("Open the logs folder")
                    onClicked: Qt.openUrlExternally(AppController.logsUrl())
                }
            }
        }
    }
}
