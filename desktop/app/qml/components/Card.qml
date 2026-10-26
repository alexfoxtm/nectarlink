// SPDX-License-Identifier: GPL-3.0-or-later
import QtQuick
import app.nectarlink

// A grouped native surface with shared geometry and a quiet border.
// All saved appearance preferences share the native panel surface.
Rectangle {
    id: cardRoot
    property int padding: Theme.spacingLg
    default property alias content: inner.data

    radius: Theme.cardRadius
    color: Theme.panelColor
    border.width: 0
    border.color: Theme.panelBorder
    implicitHeight: inner.childrenRect.height + padding * 2
    Behavior on color { enabled: !Theme.reduceMotion; ColorAnimation { duration: Theme.fadeNormal } }

    Item {
        id: inner
        z: 1
        anchors.fill: parent
        anchors.margins: parent.padding
    }
}
