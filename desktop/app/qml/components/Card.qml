// SPDX-License-Identifier: GPL-3.0-or-later
import QtQuick
import app.nectarlink

// A grouped native surface with shared geometry and a quiet border.
// Hive and Glass retain their selected surface treatments.
Rectangle {
    id: cardRoot
    property int padding: Theme.spacingLg
    default property alias content: inner.data

    radius: Theme.cardRadius
    color: Theme.glass || Theme.hive ? Theme.cardColor : Theme.panelColor
    // Glass keeps its existing rim; opaque panels use one quiet hairline.
    border.width: Theme.glass ? 0 : 1
    border.color: Theme.hive ? Theme.cardBorder : Theme.panelBorder
    implicitHeight: inner.childrenRect.height + padding * 2
    Behavior on color { enabled: !Theme.reduceMotion; ColorAnimation { duration: Theme.fadeNormal } }

    GlassRim {
        radius: cardRoot.radius
    }

    Item {
        id: inner
        z: 1
        anchors.fill: parent
        anchors.margins: parent.padding
    }
}
