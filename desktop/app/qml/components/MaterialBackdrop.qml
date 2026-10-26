// SPDX-License-Identifier: GPL-3.0-or-later
import QtQuick
import QtQuick.Window
import QtQuick.Effects
import app.nectarlink

// Original same-window material. The caller supplies a content sibling that
// excludes this overlay branch and an explicit, reactive source rectangle.
Item {
    id: material
    property Item sourceItem: null
    property rect sampleRect: Qt.rect(0, 0, width, height)
    property bool samplingEnabled: true
    property real topRadius: 28
    property real bottomRadius: 28
    property color fallbackColor: Theme.sheetColor
    property real tintOpacity: Theme.dark ? 0.84 : 0.90
    readonly property Item maskSource: cornerMask
    readonly property bool effectsSupported: GraphicsInfo.api !== GraphicsInfo.Software
        && GraphicsInfo.api !== GraphicsInfo.Unknown
    readonly property real textureScale: Math.min(1, 1024 / Math.max(1, width), 1024 / Math.max(1, height))
    readonly property bool sampling: effectsSupported && samplingEnabled && visible && width > 0 && height > 0
        && sourceItem !== null && sourceItem !== material
        && sourceItem.visible && sourceItem.Window.window !== null
        && sampleRect.width > 0 && sampleRect.height > 0
        && sampleRect.x < sourceItem.width && sampleRect.y < sourceItem.height
        && sampleRect.x + sampleRect.width > 0 && sampleRect.y + sampleRect.height > 0
        && sourceItem.Window.window === material.Window.window
        && material.Window.window !== null && material.Window.window.active
        && material.Window.window.visible
        && material.Window.window.visibility !== Window.Minimized
        && material.Window.window.visibility !== Window.Hidden

    // This base remains under the effect on unsupported renderers or absent sources.
    Rectangle {
        anchors.fill: parent
        topLeftRadius: material.topRadius
        topRightRadius: material.topRadius
        bottomLeftRadius: material.bottomRadius
        bottomRightRadius: material.bottomRadius
        color: material.fallbackColor
    }
    ShaderEffectSource {
        id: contentTexture
        sourceItem: material.sampling ? material.sourceItem : null
        sourceRect: material.sampleRect
        textureSize: Qt.size(Math.max(1, Math.round(material.width * material.textureScale)),
                             Math.max(1, Math.round(material.height * material.textureScale)))
        live: material.sampling
        recursive: false
        hideSource: false
        visible: false
        smooth: true
    }
    Rectangle {
        id: cornerMask
        anchors.fill: parent
        topLeftRadius: material.topRadius
        topRightRadius: material.topRadius
        bottomLeftRadius: material.bottomRadius
        bottomRightRadius: material.bottomRadius
        color: "white"
        visible: false
        layer.enabled: material.effectsSupported
        layer.smooth: true
    }
    MultiEffect {
        anchors.fill: parent
        visible: material.sampling
        source: contentTexture
        autoPaddingEnabled: false
        blurEnabled: true
        blurMax: 24
        blur: 0.65
        saturation: 0.0
        maskEnabled: true
        maskSource: cornerMask
        maskThresholdMin: 0.5
        maskSpreadAtMin: 1.0
    }
    Rectangle {
        anchors.fill: parent
        topLeftRadius: material.topRadius
        topRightRadius: material.topRadius
        bottomLeftRadius: material.bottomRadius
        bottomRightRadius: material.bottomRadius
        color: material.fallbackColor
        opacity: material.sampling ? material.tintOpacity : 1
    }
}
