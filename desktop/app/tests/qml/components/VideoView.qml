// SPDX-License-Identifier: GPL-3.0-or-later
import QtQuick

// Passive adapter for the native VideoView type used by production QML.
// It supplies geometry and the public property/signal surface only.
Item {
    property size frameSize: Qt.size(0, 0)
    property string stream: ""
    property string windowIcon: ""
    property string windowAppId: ""
    property bool acceptsText: false
    signal textCommitted(string text)

    function pictureRect() {
        return Qt.rect(0, 0, width, height)
    }
}
