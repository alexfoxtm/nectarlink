// SPDX-License-Identifier: GPL-3.0-or-later
import QtQuick
import QtQuick.Shapes
import app.nectarlink

// A stock portrait is selected only by an explicitly catalogued hardware model.
// This is a product illustration with a separate live-status overlay, not mirroring.
Item {
    id: portrait
    property string model: ""
    property int battery: -1
    property bool charging: false
    property int batteryFullIn: 0
    property bool online: false
    property int cellBars: -1
    property string cellType: ""
    property int wifiBars: -1
    property var screenData: null
    property bool largeStatus: false
    property string connectionText: ""
    signal connectionHelpRequested
    readonly property bool connectionInPortrait: largeStatus && handset.height >= 220 && handset.width >= 72
        && screen.y + Math.max(batteryDial.visible ? batteryDial.y + batteryDial.height : 0,
                               statusPanel.visible ? statusPanel.y + statusPanel.height : 0) * screen.detailScale + 8
           <= connectionButton.y

    readonly property bool hasStockImage: model.trim().toUpperCase() === "SM-F956B"
    readonly property bool hasBattery: online && battery >= 0 && battery <= 100
    readonly property real handsetAspect: {
        if (hasStockImage) return 343 / 791
        const aspect = screenData ? Number(screenData.aspect) : NaN
        return isFinite(aspect) && aspect >= 0.25 && aspect <= 2.5 ? aspect : 0.47
    }
    readonly property color screenInk: hasStockImage ? "#FFFFFF" : Theme.surfaceContent
    readonly property color ringColor: battery <= 20 && !charging ? Theme.error
        : (charging ? "#4ADE80" : screenInk)
    readonly property string statusText: {
        if (!online) return qsTr("Offline")
        if (charging) {
            if (!hasBattery || batteryFullIn <= 0) return qsTr("Charging")
            if (batteryFullIn < 60) return qsTr("~%1 min\nto full").arg(batteryFullIn)
            const h = Math.floor(batteryFullIn / 60)
            const m = batteryFullIn % 60
            return m > 0 ? qsTr("~%1h %2m\nto full").arg(h).arg(m)
                         : qsTr("~%1h\nto full").arg(h)
        }
        if (!hasBattery) return qsTr("Battery\nunknown")
        return battery <= 20 ? qsTr("Low battery") : ""
    }

    implicitHeight: 136
    implicitWidth: Math.max(64, Math.round(implicitHeight * handsetAspect))

    function corner(name, width) {
        const value = screenData && screenData.corners ? Number(screenData.corners[name]) : NaN
        return isFinite(value) ? Math.max(0, Math.min(width * 0.48, value * width)) : width * 0.14
    }

    function handsetPath(w, h) {
        const tl = Math.min(corner("tl", w), h * 0.48)
        const tr = Math.min(corner("tr", w), h * 0.48)
        const br = Math.min(corner("br", w), h * 0.48)
        const bl = Math.min(corner("bl", w), h * 0.48)
        return "M " + tl + " 0 H " + (w-tr) + " Q " + w + " 0 " + w + " " + tr
            + " V " + (h-br) + " Q " + w + " " + h + " " + (w-br) + " " + h
            + " H " + bl + " Q 0 " + h + " 0 " + (h-bl)
            + " V " + tl + " Q 0 0 " + tl + " 0 Z"
    }

    Item {
        id: handset
        anchors.centerIn: parent
        width: Math.min(portrait.width, portrait.height * portrait.handsetAspect)
        height: width / portrait.handsetAspect
        opacity: portrait.online ? 1 : 0.65

        // Keep the original 689×791 PNG intact; crop only the front at display time.
        Item {
            anchors.fill: parent
            visible: portrait.hasStockImage
            clip: true
            Image {
                x: -346 * handset.height / 791
                width: 689 * handset.height / 791
                height: handset.height
                source: portrait.hasStockImage ? "qrc:/waypair/phones/galaxy-z-fold6.png" : ""
                smooth: true
                mipmap: true
            }
        }
        Shape {
            anchors.fill: parent
            visible: !portrait.hasStockImage
            preferredRendererType: Shape.CurveRenderer
            ShapePath {
                strokeColor: Theme.outline
                strokeWidth: 1.5
                fillColor: Theme.panelColor
                PathSvg { path: portrait.handsetPath(handset.width, handset.height) }
            }
        }

        Item {
            id: screen
            readonly property real detailScale: portrait.largeStatus
                ? Math.max(1, Math.min(handset.height < 220 ? 1.2 : 1.65,
                                      handset.width / 64, handset.height / 136)) : 1
            x: portrait.hasStockImage ? 4 : 5
            y: x
            width: (parent.width - x * 2) / detailScale
            height: (parent.height - y * 2) / detailScale
            scale: detailScale
            transformOrigin: Item.TopLeft
            // Extremely wide/narrow reported screens use a compact battery-only layout.
            readonly property bool roomForDetails: width >= 47 && height >= 112

            Row {
                id: networkStatus
                anchors.horizontalCenter: parent.horizontalCenter
                y: 11
                height: 12
                spacing: 3
                visible: portrait.online && screen.roomForDetails
                Row {
                    height: 10
                    spacing: 1
                    visible: portrait.cellBars >= 0
                    Repeater {
                        model: 4
                        Rectangle {
                            required property int index
                            width: 2
                            height: 3 + index * 2
                            anchors.bottom: parent.bottom
                            radius: 0.7
                            color: portrait.screenInk
                            opacity: index < Math.min(4, portrait.cellBars) ? 1 : 0.25
                        }
                    }
                }
                Text {
                    visible: portrait.cellBars >= 0 && text.length > 0
                    text: portrait.cellType.length <= 3 ? portrait.cellType : ""
                    font.family: Theme.fontUi
                    font.pixelSize: portrait.largeStatus ? 11 / screen.detailScale : 9
                    font.weight: Font.DemiBold
                    color: portrait.screenInk
                    textFormat: Text.PlainText
                }
                Icon {
                    width: 12; height: 12
                    visible: portrait.wifiBars >= 0
                    path: Icons.wifi
                    stroke: 1.8
                    color: portrait.screenInk
                    opacity: portrait.wifiBars === 0 ? 0.35 : 1
                }
            }

            Item {
                id: batteryDial
                anchors.horizontalCenter: parent.horizontalCenter
                y: screen.roomForDetails ? (portrait.largeStatus ? 32 : 35) : (parent.height - height) / 2
                width: portrait.largeStatus ? screen.width : Math.min(44, screen.width)
                height: portrait.largeStatus ? 38 : 44
                visible: portrait.online
                Rectangle {
                    anchors.fill: parent
                    radius: width / 2
                    visible: screen.roomForDetails && !portrait.largeStatus
                    color: portrait.hasStockImage ? "#DD20252E" : Theme.panelColor
                }
                Shape {
                    anchors.fill: parent
                    visible: screen.roomForDetails && !portrait.largeStatus
                    preferredRendererType: Shape.CurveRenderer
                    ShapePath {
                        strokeColor: Qt.rgba(portrait.screenInk.r, portrait.screenInk.g, portrait.screenInk.b, 0.22)
                        fillColor: "transparent"
                        strokeWidth: 3
                        PathAngleArc {
                            centerX: 22; centerY: 22
                            radiusX: 19; radiusY: 19
                            startAngle: 0; sweepAngle: 360
                        }
                    }
                    ShapePath {
                        strokeColor: portrait.hasBattery ? portrait.ringColor : "transparent"
                        fillColor: "transparent"
                        strokeWidth: 3
                        capStyle: ShapePath.RoundCap
                        PathAngleArc {
                            centerX: 22; centerY: 22
                            radiusX: 19; radiusY: 19
                            startAngle: -90
                            sweepAngle: portrait.hasBattery ? 360 * portrait.battery / 100 : 0
                        }
                    }
                }
                Column {
                    anchors.centerIn: parent
                    spacing: 0
                    Icon {
                        anchors.horizontalCenter: parent.horizontalCenter
                        width: 10; height: 10
                        visible: portrait.charging && !portrait.largeStatus
                        path: Icons.bolt
                        color: "#4ADE80"
                    }
                    Text {
                        objectName: "phoneBatteryLabel"
                        anchors.horizontalCenter: parent.horizontalCenter
                        text: portrait.hasBattery ? portrait.battery + (screen.width < 38 ? "\n%" : "%") : "—"
                        font.family: Theme.fontUi
                        font.pixelSize: portrait.largeStatus ? Math.min(32, handset.width * 0.3) / screen.detailScale
                            : (screen.roomForDetails ? 14 : 12)
                        horizontalAlignment: Text.AlignHCenter
                        font.weight: Font.Bold
                        color: portrait.screenInk
                        textFormat: Text.PlainText
                    }
                }
                Rectangle {
                    anchors.horizontalCenter: parent.horizontalCenter
                    y: parent.height - height
                    width: Math.min(32, parent.width)
                    height: 2
                    radius: 1
                    visible: portrait.largeStatus && portrait.hasBattery && screen.roomForDetails
                    color: Qt.rgba(portrait.screenInk.r, portrait.screenInk.g, portrait.screenInk.b, 0.2)
                    Rectangle {
                        width: parent.width * portrait.battery / 100
                        height: parent.height
                        radius: parent.radius
                        color: portrait.battery <= 20 && !portrait.charging ? Theme.error : portrait.screenInk
                    }
                }
            }

            Rectangle {
                id: statusPanel
                anchors.horizontalCenter: parent.horizontalCenter
                y: portrait.online ? (portrait.largeStatus ? 78 : 84) : (parent.height - height) / 2
                width: parent.width - 2
                height: statusLabel.implicitHeight + 5
                radius: 5
                visible: portrait.statusText.length > 0 && (!portrait.online || screen.roomForDetails)
                color: portrait.largeStatus ? "transparent" : (portrait.hasStockImage ? "#DD20252E" : Theme.panelColor)
                Text {
                    id: statusLabel
                    objectName: "phoneStatusLabel"
                    anchors.centerIn: parent
                    width: parent.width - 2
                    text: portrait.largeStatus && portrait.charging && portrait.hasBattery && portrait.batteryFullIn > 0
                        ? qsTr("Charging") + "\n" + portrait.statusText.replace("\n", " ")
                        : portrait.statusText
                    font.family: Theme.fontUi
                    font.pixelSize: portrait.largeStatus ? 11 / screen.detailScale : 9
                    font.weight: portrait.largeStatus ? Font.Normal : Font.DemiBold
                    color: portrait.screenInk
                    opacity: portrait.largeStatus ? 0.72 : 1
                    textFormat: Text.PlainText
                    horizontalAlignment: Text.AlignHCenter
                    wrapMode: Text.WordWrap
                }
            }
        }

        // The existing connection label is also the entry to connection diagnostics.
        FocusScope {
            id: connectionButton
            visible: portrait.connectionInPortrait && portrait.connectionText.length > 0
            x: 6
            y: parent.height - height - 12
            width: parent.width - 12
            height: Math.max(36, connectionLabel.implicitHeight + 12)
            activeFocusOnTab: visible
            Accessible.role: Accessible.Button
            Accessible.name: portrait.connectionText
            Accessible.description: qsTr("Connection diagnostics")
            Accessible.onPressAction: portrait.connectionHelpRequested()
            Keys.onPressed: (event) => {
                if (!event.isAutoRepeat && (event.key === Qt.Key_Return || event.key === Qt.Key_Enter || event.key === Qt.Key_Space)) {
                    portrait.connectionHelpRequested()
                    event.accepted = true
                }
            }
            Rectangle {
                anchors.fill: parent
                radius: 6
                color: "transparent"
                border.width: Theme.focusVisible(connectionButton) ? 2 : 0
                border.color: Theme.primary
            }
            Text {
                id: connectionLabel
                anchors.centerIn: parent
                width: parent.width - 8
                text: portrait.connectionText.replace(" · ", "\n")
                font.family: Theme.fontUi
                font.pixelSize: 11
                font.weight: Font.Normal
                color: portrait.screenInk
                opacity: 0.72
                textFormat: Text.PlainText
                horizontalAlignment: Text.AlignHCenter
                wrapMode: Text.WordWrap
                maximumLineCount: 3
                elide: Text.ElideRight
            }
            HoverHandler { cursorShape: Qt.PointingHandCursor }
            TapHandler { onTapped: portrait.connectionHelpRequested() }
        }
    }
}
