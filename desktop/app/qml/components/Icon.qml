// SPDX-License-Identifier: GPL-3.0-or-later
import QtQuick
import QtQuick.Shapes
import app.nectarlink

// Sourced Fluent paths use their authored fill; custom paths keep the stroke API.
Item {
    id: icon
    property string path
    property color color: Theme.surfaceContent
    property real stroke: 1.8
    property url source: ""
    property bool selected: false
    readonly property bool hasSource: source.toString().length > 0
    readonly property bool sourcedPath: Icons.isSourced(path)

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
        visible: !icon.hasSource
        anchors.centerIn: parent
        width: 24
        height: 24
        scale: Math.min(icon.width, icon.height) / 24
        preferredRendererType: Shape.CurveRenderer
        ShapePath {
            strokeColor: icon.sourcedPath ? "transparent" : icon.color
            strokeWidth: icon.sourcedPath ? 0 : icon.stroke
            fillColor: icon.sourcedPath ? icon.color : "transparent"
            fillRule: ShapePath.WindingFill
            capStyle: ShapePath.RoundCap
            joinStyle: ShapePath.RoundJoin
            PathSvg { path: icon.selected ? Icons.selectedPath(icon.path) : icon.path }
        }
    }
}
