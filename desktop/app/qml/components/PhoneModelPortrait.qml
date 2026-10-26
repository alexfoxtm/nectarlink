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
            anchors.fill: parent
            anchors.margins: portrait.hasStockImage ? 4 : 5
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
                    font.pixelSize: 9
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
                y: screen.roomForDetails ? 35 : (parent.height - height) / 2
                width: Math.min(44, screen.width)
                height: 44
                visible: portrait.online
                Rectangle {
                    anchors.fill: parent
                    radius: width / 2
                    visible: screen.roomForDetails
                    color: portrait.hasStockImage ? "#DD20252E" : Theme.panelColor
                }
                Shape {
                    anchors.fill: parent
                    visible: screen.roomForDetails
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
                        visible: portrait.charging
                        path: Icons.bolt
                        color: "#4ADE80"
                    }
                    Text {
                        objectName: "phoneBatteryLabel"
                        anchors.horizontalCenter: parent.horizontalCenter
                        text: portrait.hasBattery ? portrait.battery + (screen.width < 38 ? "\n%" : "%") : "—"
                        font.family: Theme.fontUi
                        font.pixelSize: screen.roomForDetails ? 14 : 12
                        horizontalAlignment: Text.AlignHCenter
                        font.weight: Font.Bold
                        color: portrait.screenInk
                        textFormat: Text.PlainText
                    }
                }
            }

            Rectangle {
                anchors.horizontalCenter: parent.horizontalCenter
                y: portrait.online ? 84 : (parent.height - height) / 2
                width: parent.width - 2
                height: statusLabel.implicitHeight + 5
                radius: 5
                visible: portrait.statusText.length > 0 && (!portrait.online || screen.roomForDetails)
                color: portrait.hasStockImage ? "#DD20252E" : Theme.panelColor
                Text {
                    id: statusLabel
                    objectName: "phoneStatusLabel"
                    anchors.centerIn: parent
                    width: parent.width - 2
                    text: portrait.statusText
                    font.family: Theme.fontUi
                    font.pixelSize: 9
                    font.weight: Font.DemiBold
                    color: portrait.screenInk
                    textFormat: Text.PlainText
                    horizontalAlignment: Text.AlignHCenter
                    wrapMode: Text.WordWrap
                }
            }
        }
    }
}
