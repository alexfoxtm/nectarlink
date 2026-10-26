// SPDX-License-Identifier: GPL-3.0-or-later
import QtQuick
import app.nectarlink

// A segmented choice: `options` is a list of { value, label }.
Rectangle {
    id: segmented
    property var options: []
    property string value
    signal picked(string value)

    readonly property int selectedIndex: {
        const opts = options || []
        for (let i = 0; i < opts.length; i++) {
            if (opts[i].value === value)
                return i
        }
        return -1
    }
    readonly property Item selectedItem: (selectedIndex >= 0 && selectedIndex < segRepeater.count)
        ? segRepeater.itemAt(selectedIndex) : null

    implicitHeight: 42
    implicitWidth: row.implicitWidth + 6
    radius: Theme.controlRadius
    color: Theme.insetColor
    border.width: 1
    border.color: Theme.panelBorder

    Accessible.role: Accessible.PageTabList

    // Sliding selection pill behind the segments.
    Rectangle {
        id: slideIndicator
        visible: segmented.selectedItem !== null
        x: segmented.selectedItem ? row.x + segmented.selectedItem.x : row.x
        y: 3
        width: segmented.selectedItem ? segmented.selectedItem.width : 0
        height: segmented.height - 6
        radius: Math.max(2, Theme.controlRadius - 3)
        color: Theme.panelColor
        border.width: 1
        border.color: Theme.panelBorder
        Behavior on x {
            enabled: !Theme.reduceMotion && slideIndicator.width > 0
            SpringAnimation { spring: Theme.springSnappy; damping: Theme.dampingSnappy }
        }
        Behavior on width {
            enabled: !Theme.reduceMotion && slideIndicator.width > 0
            SpringAnimation { spring: Theme.springSnappy; damping: Theme.dampingSnappy }
        }
    }

    Row {
        id: row
        anchors.centerIn: parent
        spacing: 2
        Repeater {
            id: segRepeater
            model: segmented.options
            delegate: FocusScope {
                id: option
                required property var modelData
                required property int index
                readonly property bool selected: modelData.value === segmented.value
                width: Math.max(36, label.implicitWidth + 26)
                height: segmented.height - 6
                activeFocusOnTab: true
                scale: segMouse.pressed && !Theme.reduceMotion ? Theme.pressScale : 1.0
                Behavior on scale {
                    enabled: !Theme.reduceMotion
                    NumberAnimation { duration: 120; easing.type: Easing.OutCubic }
                }
                Accessible.role: Accessible.PageTab
                Accessible.name: modelData.label
                Accessible.selected: selected
                Accessible.onPressAction: if (segmented.enabled && option.enabled) segmented.picked(modelData.value)
                Keys.onPressed: (event) => {
                    if (!segmented.enabled || !option.enabled) return
                    if (event.key === Qt.Key_Return || event.key === Qt.Key_Enter || event.key === Qt.Key_Space) {
                        segmented.picked(modelData.value)
                        event.accepted = true
                    }
                }
                Rectangle {
                    anchors.fill: parent
                    radius: Math.max(2, Theme.controlRadius - 3)
                    color: "transparent"
                    Rectangle {
                        anchors.fill: parent
                        anchors.margins: -4
                        radius: parent.radius + 4
                        color: "transparent"
                        border.width: 2
                        border.color: Theme.primary
                        visible: Theme.focusVisible(option)
                    }
                }
                Txt {
                    id: label
                    anchors.centerIn: parent
                    text: option.modelData.label
                    role: "label"
                    color: option.selected
                        ? Theme.surfaceContent
                        : (segMouse.containsMouse ? Theme.surfaceContent : Theme.surfaceContentVariant)
                    Behavior on color { enabled: !Theme.reduceMotion; ColorAnimation { duration: Theme.fadeFast } }
                }
                MouseArea {
                    id: segMouse
                    anchors.fill: parent
                    hoverEnabled: true
                    cursorShape: Qt.PointingHandCursor
                    onClicked: if (segmented.enabled && option.enabled) segmented.picked(option.modelData.value)
                }
            }
        }
    }
}
