// SPDX-License-Identifier: GPL-3.0-or-later
import QtQuick
import QtTest
import app.nectarlink

Item {
    width: 600
    height: 240
    Txt { id: sample; text: "Readable interface text"; role: "headline"; size: 14 }
    Txt { id: code; y: 50; role: "code"; text: "code" }
    Button { id: button; y: 100; text: "Choose font…" }
    Segmented {
        id: segments
        y: 160
        options: [{ value: "first", label: "First" }, { value: "second", label: "Second" }]
        value: "first"
    }
    TestCase {
        name: "Typography"
        when: windowShown
        function cleanup() {
            Preferences.fontFamily = "Figtree"
            Preferences.textScalePercent = 100
        }
        function test_family_and_scale_update_text_and_control_metrics() {
            Preferences.textScalePercent = 100
            const baseButtonWidth = button.implicitWidth
            Preferences.fontFamily = "Arial"
            Preferences.textScalePercent = 150
            compare(Theme.fontUi, "Arial")
            compare(sample.family, "Arial")
            compare(code.family, "Space Mono")
            compare(sample.font.pixelSize, 21)
            compare(sample.lineHeight, Math.round(21 * sample.spec.lineHeight))
            // QFont quantizes letter spacing to a 1/64-pixel fixed-point grid.
            verify(Math.abs(sample.font.letterSpacing - sample.spec.tracking * sample.font.pixelSize) <= 1 / 64)
            tryVerify(() => button.implicitWidth > baseButtonWidth)
            tryVerify(() => segments.labelHeight > 0)
            verify(segments.implicitHeight >= segments.labelHeight + 20)
            verify(segments.selectedItem.width > 36)
        }
    }
}
