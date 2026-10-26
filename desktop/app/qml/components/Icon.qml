// SPDX-License-Identifier: GPL-3.0-or-later
import QtQuick
import QtQuick.Shapes
import app.nectarlink

// App-owned semantic icons use complete OneUI SVGs; source remains an untinted image API.
Item {
    id: icon
    property string path
    property color color: Theme.surfaceContent
    property real stroke: 1.8
    property url source: ""
    property bool selected: false
    property bool semanticTint: false
    readonly property bool hasSource: source.toString().length > 0
    readonly property bool sourcedPath: Icons.isSourced(path)
    readonly property var oneuiPath: Icons.oneuiBinding(path)
    readonly property color paintColor: semanticTint ? Icons.utilityColor(path, color) : color

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

    Image {
        anchors.fill: parent
        visible: !icon.hasSource && icon.sourcedPath
        source: visible ? Icons.svgSource(icon.path, icon.paintColor, icon.selected) : ""
        sourceSize.width: Math.max(1, Math.ceil(width * Screen.devicePixelRatio))
        sourceSize.height: Math.max(1, Math.ceil(height * Screen.devicePixelRatio))
        fillMode: Image.PreserveAspectFit
        mirror: icon.oneuiPath !== null && LayoutMirroring.enabled
                && ((icon.selected || icon.path.endsWith(":selected"))
                    ? icon.oneuiPath.selectedAutoMirrored : icon.oneuiPath.autoMirrored)
        smooth: true
    }

    // Keep arbitrary caller-authored stroked paths (including the Waypair mark) compatible.
    Shape {
        visible: !icon.hasSource && !icon.sourcedPath
        anchors.centerIn: parent
        width: 24
        height: 24
        scale: Math.min(icon.width, icon.height) / 24
        preferredRendererType: Shape.CurveRenderer
        ShapePath {
            strokeColor: icon.paintColor
            strokeWidth: icon.stroke
            fillColor: "transparent"
            fillRule: ShapePath.WindingFill
            capStyle: ShapePath.RoundCap
            joinStyle: ShapePath.RoundJoin
            PathSvg { path: icon.path }
        }
    }
}
