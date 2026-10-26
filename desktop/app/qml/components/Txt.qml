// SPDX-License-Identifier: GPL-3.0-or-later
import QtQuick
import app.nectarlink

// Text in one of the design's type roles: displayLarge, displaySmall,
// headline, title, body, bodySmall, label, caption, code.
Text {
    id: txt
    property string role: "body"
    // Secondary text uses the variant color.
    property bool muted: false

    readonly property var spec: Theme.typography[role] || Theme.typography.body
    readonly property bool isDisplay: role === "displayLarge" || role === "displaySmall" || role === "headline"
    // Graphite keeps its bundled mono labels; sentence case stays readable in controls.
    readonly property bool graphiteLabel: Theme.graphite && (role === "label" || role === "caption")

    // Pixel size; set this (not font.pixelSize) to resize, so the role's
    // tracking scales with it.
    property real size: Math.round(graphiteLabel ? Theme.labelStyle.size : spec.size)
    // Set this (not font.weight) to change the weight: Inter is a variable
    // font, and its weight axis is set to match, so in-between weights such
    // as 650 render exactly instead of snapping to a named style.
    property int weight: graphiteLabel ? Theme.labelStyle.weight : (isDisplay && Theme.graphite ? 400 : spec.weight)

    readonly property real scaledSize: Math.max(1, Math.round(size * Theme.textScale))
    readonly property string family: role === "code" || graphiteLabel
        ? Theme.fontMono : (isDisplay ? Theme.fontDisplay : Theme.fontUi)

    color: muted ? Theme.surfaceContentVariant : Theme.surfaceContent
    font.family: family
    font.pixelSize: scaledSize
    font.weight: weight
    font.variableAxes: (family === "Figtree" || family === "Inter") ? { "wght": weight } : {}
    font.letterSpacing: (graphiteLabel ? 0 : spec.tracking) * scaledSize
    font.capitalization: Font.MixedCase
    // Token line heights are CSS-style (a multiple of the font size), not
    // a multiple of the font's own line spacing.
    lineHeight: Math.round(scaledSize * spec.lineHeight)
    lineHeightMode: Text.FixedHeight
    verticalAlignment: Text.AlignVCenter
    textFormat: Text.PlainText
    elide: Text.ElideRight
}
