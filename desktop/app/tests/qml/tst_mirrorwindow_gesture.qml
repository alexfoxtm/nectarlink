// SPDX-License-Identifier: GPL-3.0-or-later
import QtQuick
import QtQuick.Window
import QtTest
import app.nectarlink

// This loads production MirrorWindow.qml and WindowScreenGeometry. The test
// harness uses the existing passive Mirror and VideoView adapters.
TestCase {
    id: testCase
    name: "MirrorWindow passive QML behavior"
    when: windowShown

    property var component
    property var mirrorWindow
    property var gesture
    property var videoView

    function createMirror(canControl, phase, extraInfo) {
        Mirror.calls = []
        const info = {
            key: "passive-test", name: "Fixture", phase: phase,
            canControl: canControl, viewOnly: false
        }
        if (extraInfo) {
            for (const key in extraInfo)
                info[key] = extraInfo[key]
        }
        Mirror.windows = JSON.stringify([info])
        component = Qt.createComponent(Qt.resolvedUrl("../../qml/MirrorWindow.qml"))
        compare(component.status, Component.Ready, component.errorString())
        mirrorWindow = component.createObject(null, { mirrorKey: "passive-test" })
        verify(mirrorWindow !== null)
        gesture = findChild(mirrorWindow, "mirrorGestureMouse")
        verify(gesture !== null)
        videoView = findChild(mirrorWindow, "mirrorVideo")
        verify(videoView !== null)
        // Let the real Window bind to its owning QScreen before a decoded frame arrives.
        wait(0)
        tryVerify(() => mirrorWindow.visible
            && mirrorWindow.visibility === Window.Windowed
            && videoView.width > 0 && videoView.height > 0
            && mirrorWindow.availableArea.width > 0
            && mirrorWindow.availableArea.height > 0)
        videoView.frameSize = Qt.size(420, 860)
        tryVerify(() => mirrorWindow.sized)
    }

    function cleanup() {
        if (mirrorWindow)
            mirrorWindow.destroy()
        mirrorWindow = null
        gesture = null
        videoView = null
        component = null
        Mirror.windows = "[]"
        Mirror.calls = []
    }

    function test_control_revocation_clears_local_drag_before_regrant() {
        createMirror(true, "showing")
        mousePress(gesture, 80, 80, Qt.LeftButton)
        verify(gesture.down)
        mouseMove(gesture, 100, 100)
        tryVerify(() => gesture.lastMove > 0)
        const touchCountBeforeRevocation = Mirror.calls.filter(call => call[0] === "touch").length
        verify(touchCountBeforeRevocation >= 2)

        Mirror.windows = JSON.stringify([{
            key: "passive-test", name: "Fixture", phase: "showing",
            canControl: false, viewOnly: false
        }])
        tryVerify(() => !gesture.down)
        compare(gesture.lastMove, 0)

        Mirror.windows = JSON.stringify([{
            key: "passive-test", name: "Fixture", phase: "showing",
            canControl: true, viewOnly: false
        }])
        wait(0)
        mouseMove(gesture, 100, 100)
        mouseRelease(gesture, 100, 100, Qt.LeftButton)
        compare(Mirror.calls.filter(call => call[0] === "touch").length, touchCountBeforeRevocation,
                "revoked drag must not resume under a later grant")
    }

    function test_phase_loss_cancels_local_drag_without_backend_cancel() {
        createMirror(true, "showing")
        mousePress(gesture, 80, 80, Qt.LeftButton)
        verify(gesture.down)
        mouseMove(gesture, 100, 100)
        tryVerify(() => gesture.lastMove > 0)
        const touchCountBeforePhaseLoss = Mirror.calls.filter(call => call[0] === "touch").length
        verify(touchCountBeforePhaseLoss >= 2)
        Mirror.windows = JSON.stringify([{
            key: "passive-test", name: "Fixture", phase: "connecting",
            canControl: true, viewOnly: false
        }])
        tryVerify(() => !gesture.down)
        compare(gesture.lastMove, 0)
        compare(Mirror.calls.filter(call => call[0] === "touch").length, touchCountBeforePhaseLoss,
                "phase loss clears local state without emitting a touch cancel")
    }

    function test_settings_window_allows_only_approved_wheel_scrolling() {
        createMirror(true, "showing", {
            app: true, title: "Settings", settingsQualification: true,
            canControl: true, viewOnly: false, controlReason: "unsupported",
            canScroll: true, scrollReason: "available", canRequestControl: true,
            canReopen: true, canScreenOff: true, stayAwake: true, screenOff: true,
            sound: true
        })

        verify(!mirrorWindow.canControl, "the restriction marker must never grant phone control")
        verify(mirrorWindow.viewOnly)
        verify(mirrorWindow.canScroll)
        verify(!mirrorWindow.canRequestControl)
        verify(!mirrorWindow.canReopen)
        verify(!mirrorWindow.canScreenOff)
        verify(!videoView.acceptsText)
        verify(!findChild(mirrorWindow, "mirrorDropArea").enabled)
        verify(!findChild(mirrorWindow, "mirrorStayAwake").visible)
        verify(!findChild(mirrorWindow, "mirrorScreenOff").visible)
        verify(!findChild(mirrorWindow, "mirrorSound").visible)
        verify(!findChild(mirrorWindow, "mirrorCopyShot").visible)
        verify(!findChild(mirrorWindow, "mirrorRequestControl").visible)

        mirrorWindow.width = Math.max(500, mirrorWindow.width)
        const toolbar = findChild(mirrorWindow, "mirrorToolbar")
        const saveShot = findChild(mirrorWindow, "mirrorSaveShot")
        tryVerify(() => toolbar.visible && !toolbar.overflowMode && saveShot.visible)
        mirrorWindow.requestActivate()
        tryVerify(() => mirrorWindow.active)
        videoView.forceActiveFocus()
        tryVerify(() => videoView.activeFocus)
        keyClick(Qt.Key_F6)
        tryVerify(() => saveShot.activeFocus)

        Mirror.calls = []
        mouseClick(gesture, 80, 80, Qt.LeftButton)
        mouseClick(gesture, 80, 80, Qt.RightButton)
        mouseClick(gesture, 80, 80, Qt.MiddleButton)
        videoView.forceActiveFocus()
        tryVerify(() => mirrorWindow.active && videoView.activeFocus)
        keyClick(Qt.Key_A)
        keyClick(Qt.Key_V, Qt.ControlModifier)
        keyClick(Qt.Key_C, Qt.ControlModifier | Qt.ShiftModifier)
        mouseWheel(gesture, 80, 80, 0, 120, Qt.NoButton, Qt.NoModifier)

        compare(Mirror.calls.filter(call => call[0] === "scroll").length, 1)
        const blockedNames = ["requestControl", "touch", "press", "text", "paste",
                              "screenshotClipboard", "dropFiles", "syncClipboardOnFocus",
                              "enterWindow", "leaveWindow", "setScreenOff", "setStayAwake", "toggleSound"]
        for (let i = 0; i < blockedNames.length; i++) {
            const blocked = blockedNames[i]
            compare(Mirror.calls.filter(call => call[0] === blocked).length, 0,
                    blocked + " must be blocked for this window")
        }

        Mirror.windows = JSON.stringify([{
            key: "passive-test", name: "Fixture", phase: "ended", app: true,
            settingsQualification: true, canControl: false, viewOnly: true,
            canScroll: false, canRequestControl: false, canReopen: false
        }])
        tryVerify(() => !findChild(mirrorWindow, "mirrorPreFrameReopen").visible)
        verify(findChild(mirrorWindow, "mirrorPreFrameDetail").text.indexOf("new Settings window") >= 0)
    }

    function test_settings_window_wheel_without_allowance_does_not_request_control() {
        createMirror(false, "showing", {
            app: true, title: "Settings", settingsQualification: true,
            canControl: false, viewOnly: true, controlReason: "ownerApproval",
            canScroll: false, scrollReason: "ownerApproval", canRequestControl: false,
            canReopen: false, canScreenOff: false, stayAwake: false, screenOff: false
        })
        Mirror.calls = []

        mouseWheel(gesture, 80, 80, 0, 120, Qt.NoButton, Qt.NoModifier)
        mouseClick(gesture, 80, 80, Qt.LeftButton)

        compare(Mirror.calls.filter(call => call[0] === "scroll").length, 0)
        compare(Mirror.calls.filter(call => call[0] === "requestControl").length, 0)
        compare(Mirror.calls.filter(call => call[0] === "touch").length, 0)
    }


    function test_first_portrait_is_centered_and_rotation_stays_in_available_area() {
        createMirror(false, "showing", { app: false })
        tryVerify(() => mirrorWindow.sized)
        const area = mirrorWindow.availableArea
        verify(area.width > 0 && area.height > 0)
        const firstCenterX = mirrorWindow.x + mirrorWindow.width / 2
        const firstCenterY = mirrorWindow.y + mirrorWindow.height / 2
        verify(Math.abs(firstCenterX - (area.x + area.width / 2)) <= 1)
        verify(Math.abs(firstCenterY - (area.y + area.height / 2)) <= 1)
        verify(mirrorWindow.x >= area.x)
        verify(mirrorWindow.y >= area.y)
        verify(mirrorWindow.x + mirrorWindow.width <= area.x + area.width + 1)
        verify(mirrorWindow.y + mirrorWindow.height <= area.y + area.height + 1)

        // Synthetic landscape sample; available bounds come from the real
        // owning QScreen and this does not simulate multiple monitors.
        videoView.frameSize = Qt.size(700, 420)
        tryVerify(() => Math.abs(mirrorWindow.width / mirrorWindow.height - (700 / 420)) < 0.02)
        verify(Math.abs(mirrorWindow.x + mirrorWindow.width / 2 - firstCenterX) <= 2)
        verify(Math.abs(mirrorWindow.y + mirrorWindow.height / 2 - firstCenterY) <= 2)
        verify(mirrorWindow.x >= area.x)
        verify(mirrorWindow.y >= area.y)
        verify(mirrorWindow.x + mirrorWindow.width <= area.x + area.width + 1)
        verify(mirrorWindow.y + mirrorWindow.height <= area.y + area.height + 1)
    }

    function test_stale_offscreen_saved_window_is_fitted_and_clamped() {
        createMirror(false, "showing", {
            app: true, savedWidth: 1600, savedHeight: 1600,
            savedX: 100000, savedY: 100000
        })
        tryVerify(() => mirrorWindow.sized)
        const area = mirrorWindow.availableArea
        verify(area.width > 0 && area.height > 0)
        verify(Math.abs(mirrorWindow.width / mirrorWindow.height - 1) < 0.02,
               "restored dimensions keep their saved aspect ratio")
        verify(mirrorWindow.width <= area.width * 0.95 + 1)
        verify(mirrorWindow.minimumWidth <= area.width * 0.95 + 1,
               "minimum width must allow the restored window to fit the owning screen")
        verify(mirrorWindow.minimumHeight <= area.height * 0.95 + 1,
               "minimum height must allow the restored window to fit the owning screen")
        verify(mirrorWindow.height <= area.height * 0.95 + 1)
        verify(mirrorWindow.x + mirrorWindow.width <= area.x + area.width + 1,
               "stale right-edge position is clamped")
        verify(mirrorWindow.y + mirrorWindow.height <= area.y + area.height + 1,
               "stale bottom-edge position is clamped")
    }
}
