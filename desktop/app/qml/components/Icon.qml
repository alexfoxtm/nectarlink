// SPDX-License-Identifier: GPL-3.0-or-later
import QtQuick
import QtQuick.Shapes
import app.nectarlink

// Mac uses authored Phosphor duotone/fill; other paths keep the existing API.
Item {
    id: icon
    property string path
    property color color: Theme.surfaceContent
    property real stroke: 1.8
    property url source: ""
    property bool selected: false
    readonly property bool hasSource: source.toString().length > 0
    readonly property bool sourcedPath: Icons.isSourced(path)
    readonly property var phosphorPath: Theme.onMac ? Icons.phosphor(path) : null
    readonly property color paintColor: Icons.utilityColor(path, color)

    implicitWidth: 20
    implicitHeight: 20

    Image {
        anchors.fill: parent
        visible: icon.hasSource
        source: icon.source
        sourceSize.width: 128
        sourceSize.height: 128
        fillMode: Image.PreserveAspectFit
        smooth: true
    }

    Shape {
        visible: !icon.hasSource && icon.phosphorPath !== null
        anchors.centerIn: parent
        width: 256
        height: 256
        scale: Math.min(icon.width, icon.height) / 256
        preferredRendererType: Shape.CurveRenderer
        ShapePath {
            strokeColor: "transparent"
            strokeWidth: 0
            fillColor: Qt.rgba(icon.paintColor.r, icon.paintColor.g, icon.paintColor.b, icon.paintColor.a * 0.2)
            fillRule: ShapePath.WindingFill
            PathSvg { path: icon.phosphorPath && !icon.selected ? icon.phosphorPath.background : "" }
        }
        ShapePath {
            strokeColor: "transparent"
            strokeWidth: 0
            fillColor: icon.paintColor
            fillRule: ShapePath.WindingFill
            PathSvg { path: icon.phosphorPath ? (icon.selected ? icon.phosphorPath.filled : icon.phosphorPath.foreground) : "" }
        }
    }

    Shape {
        visible: !icon.hasSource && icon.phosphorPath === null
        anchors.centerIn: parent
        width: 24
        height: 24
        scale: Math.min(icon.width, icon.height) / 24
        preferredRendererType: Shape.CurveRenderer
        ShapePath {
            strokeColor: icon.sourcedPath ? "transparent" : icon.paintColor
            strokeWidth: icon.sourcedPath ? 0 : icon.stroke
            fillColor: icon.sourcedPath ? icon.paintColor : "transparent"
            fillRule: ShapePath.WindingFill
            capStyle: ShapePath.RoundCap
            joinStyle: ShapePath.RoundJoin
            PathSvg { path: icon.selected ? Icons.selectedPath(icon.path) : icon.path }
        }
    }
}
