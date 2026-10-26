// SPDX-License-Identifier: GPL-3.0-or-later
import QtQuick
import app.nectarlink

// A native rounded button. Variants: "fill" (primary action), "tonal", "outline",
// "text". Sizes: "md" (40 px) and "sm" (36 px). Works with mouse, touch and
// keyboard (Tab to focus, Enter or Space to press).
FocusScope {
    id: button
    property string text
    property string iconPath
    property string variant: "fill"
    property string size: "md"
    property bool busy: false
    signal clicked

    readonly property bool small: size === "sm"
    readonly property bool interactive: enabled && !busy
    readonly property color background: variant === "fill" ? Theme.actionColor
        : variant === "tonal" ? Theme.secondaryContainer : "transparent"
    readonly property color foreground: variant === "fill" ? Theme.actionContent
        : variant === "tonal" ? Theme.secondaryContainerContent
        : variant === "text" ? Theme.actionColor : Theme.surfaceContent

    implicitHeight: Math.max(small ? 40 : 48, row.implicitHeight + (small ? 12 : 16))
    implicitWidth: Math.max(36, row.implicitWidth + (variant === "text" ? 24 : (small ? 32 : 40)))
    activeFocusOnTab: true
    opacity: enabled ? 1 : 0.38

    Accessible.role: Accessible.Button
    Accessible.name: text
    Accessible.onPressAction: if (interactive) button.clicked()
    Keys.onPressed: (event) => {
        if (interactive && !event.isAutoRepeat && (event.key === Qt.Key_Return || event.key === Qt.Key_Enter || event.key === Qt.Key_Space)) {
            button.clicked()
            event.accepted = true
        }
    }

    Rectangle {
        id: body
        width: parent.width
        height: parent.height
        radius: Theme.pill(height)
        color: button.background
        Behavior on color { enabled: !Theme.reduceMotion; ColorAnimation { duration: Theme.fadeFast } }
        border.width: button.variant === "outline" ? 1 : 0
        border.color: Theme.panelBorder
        scale: mouse.pressed && button.interactive ? Theme.pressScale : 1
        Behavior on scale { enabled: !Theme.reduceMotion; NumberAnimation { duration: 120; easing.type: Easing.OutCubic } }

        // Hover & Press state layer.
        Rectangle {
            anchors.fill: parent
            radius: parent.radius
            color: button.variant === "fill" && !Theme.dark ? Theme.surfaceContent : button.foreground
            opacity: !button.interactive ? 0 : (mouse.pressed ? 0.14 : (mouse.containsMouse ? 0.07 : 0))
            Behavior on opacity { enabled: !Theme.reduceMotion && !mouse.pressed; NumberAnimation { duration: Theme.fadeFast } }
        }

        Row {
            id: row
            anchors.centerIn: parent
            spacing: 8
            Spinner {
                visible: button.busy
                anchors.verticalCenter: parent.verticalCenter
                width: 16; height: 16
                color: button.foreground
            }
            Icon {
                visible: button.iconPath.length > 0 && !button.busy
                anchors.verticalCenter: parent.verticalCenter
                width: button.small ? 16 : 18; height: width
                path: button.iconPath
                color: button.foreground
                semanticTint: button.variant !== "fill"
            }
            Txt {
                anchors.verticalCenter: parent.verticalCenter
                text: button.text
                role: "label"
                color: button.foreground
                size: button.small ? 14 : 16
                font.underline: false
                maximumLineCount: 1
                elide: Text.ElideRight
            }
        }
    }

    // Keyboard focus ring.
    Rectangle {
        anchors.fill: body
        anchors.margins: -4
        radius: body.radius + 4
        color: "transparent"
        border.width: 2
        border.color: Theme.primary
        visible: Theme.focusVisible(button)
    }

    MouseArea {
        id: mouse
        anchors.fill: parent
        enabled: button.interactive
        hoverEnabled: true
        cursorShape: button.interactive ? Qt.PointingHandCursor : Qt.ArrowCursor
        onClicked: button.clicked()
    }
}
