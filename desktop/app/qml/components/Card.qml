// SPDX-License-Identifier: GPL-3.0-or-later
import QtQuick
import app.nectarlink

// A grouped native surface with shared geometry and a quiet border.
// Mac panels use the existing QML glass rim over the neutral ambient layer.
Rectangle {
    id: cardRoot
    property int padding: Theme.spacingLg
    default property alias content: inner.data

    radius: Theme.cardRadius
    color: Theme.panelColor
    border.width: 1
    border.color: Theme.panelBorder
    implicitHeight: inner.childrenRect.height + padding * 2
    Behavior on color { enabled: !Theme.reduceMotion; ColorAnimation { duration: Theme.fadeNormal } }

    GlassRim {
        radius: cardRoot.radius
        strength: 0.3
    }

    Item {
        id: inner
        z: 1
        anchors.fill: parent
        anchors.margins: parent.padding
    }
}
