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

    function createMirror(canControl, phase, extraInfo, skipFrame) {
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
        if (!skipFrame) {
            videoView.frameSize = Qt.size(420, 860)
            tryVerify(() => mirrorWindow.sized)
        }
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

    function test_close_key_closes_only_the_active_ended_settings_window() {
        createMirror(false, "ended", { app: true, settingsQualification: true })
        const otherInfo = {
            key: "other-mirror", name: "Other fixture", phase: "showing",
            canControl: false, viewOnly: true
        }
        const ownedInfo = JSON.parse(Mirror.windows)[0]
        Mirror.windows = JSON.stringify([ownedInfo, otherInfo])
        const otherWindow = component.createObject(null, { mirrorKey: "other-mirror" })
        verify(otherWindow !== null)
        try {
            tryVerify(() => otherWindow.visible)
            mirrorWindow.requestActivate()
            tryVerify(() => mirrorWindow.active)
            Mirror.calls = []
            // Qt maps ControlModifier to Command on macOS.
            keyClick(Qt.Key_W, Qt.ControlModifier)
            tryVerify(() => !mirrorWindow.visible)
            verify(otherWindow.visible, "closing one mirror must preserve the other window")
            const stops = Mirror.calls.filter(call => call[0] === "stop")
            compare(stops.length, 1)
            compare(stops[0][1], "passive-test")
        } finally {
            otherWindow.destroy()
        }
    }

    function test_close_key_in_an_unrelated_window_does_not_stop_the_mirror() {
        createMirror(false, "ended", { app: true, settingsQualification: true })
        const unrelatedWindow = Qt.createQmlObject(
            'import QtQuick; import QtQuick.Window; Window { visible: true; width: 240; height: 240 }',
            testCase)
        verify(unrelatedWindow !== null)
        try {
            unrelatedWindow.requestActivate()
            tryVerify(() => unrelatedWindow.active && !mirrorWindow.active)
            Mirror.calls = []
            keyClick(Qt.Key_W, Qt.ControlModifier)
            wait(0)
            verify(mirrorWindow.visible)
            compare(Mirror.calls.filter(call => call[0] === "stop").length, 0)
        } finally {
            unrelatedWindow.destroy()
        }
    }

    function test_app_close_after_video_keeps_existing_wording() {
        createMirror(false, "showing", { app: true, title: "Calculator" })
        tryVerify(() => mirrorWindow.videoStarted)
        videoView.frameSize = Qt.size(0, 0)
        Mirror.windows = JSON.stringify([{
            key: "passive-test", name: "Fixture", phase: "ended",
            app: true, title: "Calculator", canReopen: true
        }])
        tryCompare(findChild(mirrorWindow, "mirrorPreFrameTitle"), "text", "Calculator closed")
        compare(findChild(mirrorWindow, "mirrorPreFrameDetail").text, "The app closed on Fixture.")
        verify(findChild(mirrorWindow, "mirrorPreFrameReopen").visible)
        verify(!findChild(mirrorWindow, "mirrorPreFrameStop").visible)

        Mirror.windows = JSON.stringify([{
            key: "passive-test", name: "Fixture", phase: "asking",
            app: true, title: "Calculator"
        }])
        tryVerify(() => !mirrorWindow.videoStarted)
    }

    function test_app_end_before_video_is_truthful_and_preserves_reason() {
        createMirror(false, "showing", { app: true, title: "Settings" }, true)
        verify(!mirrorWindow.videoStarted)
        for (const title of ["Settings", "Calculator"]) {
            Mirror.windows = JSON.stringify([{
                key: "passive-test", name: "Fixture", phase: "ended",
                app: true, title: title, canReopen: true
            }])
            tryCompare(findChild(mirrorWindow, "mirrorPreFrameTitle"), "text", title + " window ended")
            compare(findChild(mirrorWindow, "mirrorPreFrameDetail").text, "The window ended before video started.")
            verify(findChild(mirrorWindow, "mirrorPreFrameReopen").visible)
            verify(!findChild(mirrorWindow, "mirrorPreFrameStop").visible)
        }

        Mirror.windows = JSON.stringify([{
            key: "passive-test", name: "Fixture", phase: "ended",
            app: true, title: "Settings", reason: "The phone isn't connected.",
            settingsQualification: true, canReopen: false
        }])
        tryCompare(findChild(mirrorWindow, "mirrorPreFrameTitle"), "text", "Settings window ended")
        compare(findChild(mirrorWindow, "mirrorPreFrameDetail").text, "The phone isn't connected.")
        verify(!findChild(mirrorWindow, "mirrorPreFrameReopen").visible)
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

    function test_user_resize_intent_survives_reopen() {
        createMirror(false, "showing", { app: true })
        tryVerify(() => mirrorWindow.readyForResize)
        verify(!mirrorWindow.userResized)
        Mirror.calls = []
        mirrorWindow.width = Math.max(260, mirrorWindow.width + 20)
        mirrorWindow.height = Math.max(260, mirrorWindow.height + 20)
        tryVerify(() => mirrorWindow.userResized)
        const saved = Mirror.calls.filter(call => call[0] === "saveGeometry").pop()
        verify(saved !== undefined)
        compare(saved[1], "passive-test", "geometry belongs to this exact mirror window")
        compare(saved[6], true)
        const savedInfo = {
            app: true, savedX: saved[2], savedY: saved[3],
            savedWidth: saved[4], savedHeight: saved[5], savedUserResized: saved[6]
        }
        cleanup()
        createMirror(false, "showing", savedInfo)
        tryVerify(() => mirrorWindow.readyForResize)
        verify(mirrorWindow.userResized)
        verify(Math.abs(mirrorWindow.width / mirrorWindow.height
                        - savedInfo.savedWidth / savedInfo.savedHeight) < 0.02)
        videoView.frameSize = Qt.size(700, 420)
        wait(0)
        verify(mirrorWindow.userResized)
        verify(Math.abs(mirrorWindow.width / mirrorWindow.height
                        - savedInfo.savedWidth / savedInfo.savedHeight) < 0.02)
    }

    function test_natural_size_action_restores_natural_frame_in_place() {
        createMirror(false, "showing", { app: true })
        tryVerify(() => mirrorWindow.readyForResize)
        const naturalWidth = mirrorWindow.width
        const naturalHeight = mirrorWindow.height
        const naturalSize = findChild(mirrorWindow, "mirrorNaturalSize")
        verify(naturalSize !== null)
        verify(!naturalSize.visible, "nothing to restore before a manual resize")
        mirrorWindow.width = naturalWidth + 60
        mirrorWindow.height = Math.max(260, naturalHeight - 60)
        tryVerify(() => mirrorWindow.userResized && naturalSize.visible)
        // The phone's app display follows the manual size.
        videoView.frameSize = Qt.size(mirrorWindow.width * 2, mirrorWindow.height * 2)
        // An interior point where both sizes fit, so the reset has nothing to
        // clamp. The platform may adjust a requested position, so the test uses
        // the one this window saves once the move has settled.
        const area = mirrorWindow.availableArea
        const largestWidth = Math.max(naturalWidth, mirrorWindow.width)
        const largestHeight = Math.max(naturalHeight, mirrorWindow.height)
        Mirror.calls = []
        mirrorWindow.x = area.x + Math.floor((area.width - largestWidth) / 4)
        mirrorWindow.y = area.y + Math.floor((area.height - largestHeight) / 4)
        tryVerify(() => Mirror.calls.some(call => call[0] === "saveGeometry"))
        const placedX = mirrorWindow.x
        const placedY = mirrorWindow.y
        compare(Mirror.calls.filter(call => call[0] === "saveGeometry").pop().slice(1, 4),
                ["passive-test", placedX, placedY])
        verify(placedX > area.x && placedY > area.y, "an interior position")
        verify(placedX + largestWidth <= area.x + area.width && placedY + largestHeight <= area.y + area.height)
        const sameWindow = mirrorWindow
        Mirror.calls = []

        naturalSize.clicked()

        tryVerify(() => mirrorWindow.width === naturalWidth && mirrorWindow.height === naturalHeight)
        verify(mirrorWindow === sameWindow && mirrorWindow.visible)
        compare(mirrorWindow.x, placedX)
        compare(mirrorWindow.y, placedY)
        tryVerify(() => Mirror.calls.some(call => call[0] === "saveGeometry"))
        compare(Mirror.calls.filter(call => call[0] === "saveGeometry").pop(),
                ["saveGeometry", "passive-test", placedX, placedY, naturalWidth, naturalHeight, false])
        compare(Mirror.calls.filter(call => call[0] === "resize").pop(),
                ["resize", "passive-test", naturalWidth, naturalHeight])
        compare(Mirror.calls.filter(call => call[0] === "stop").length, 0)
        verify(!mirrorWindow.userResized)
        verify(!naturalSize.visible)
    }

    function test_natural_size_action_keeps_override_outside_a_normal_window() {
        createMirror(false, "showing", { app: true })
        tryVerify(() => mirrorWindow.readyForResize)
        mirrorWindow.width = mirrorWindow.width + 60
        tryVerify(() => mirrorWindow.userResized)
        const overrideWidth = mirrorWindow.width
        mirrorWindow.visibility = Window.Minimized
        tryVerify(() => mirrorWindow.visibility === Window.Minimized)
        verify(!mirrorWindow.canResetNaturalSize)
        Mirror.calls = []

        mirrorWindow.resetToNaturalSize()
        wait(400)

        verify(mirrorWindow.userResized, "the explicit override is kept")
        compare(mirrorWindow.width, overrideWidth)
        compare(Mirror.calls.filter(call => call[0] === "saveGeometry" || call[0] === "resize").length, 0)
    }

    function test_reopened_app_stream_keeps_its_natural_frame() {
        createMirror(false, "showing", { app: true })
        tryVerify(() => mirrorWindow.readyForResize)
        const naturalWidth = mirrorWindow.width
        const naturalHeight = mirrorWindow.height
        mirrorWindow.width = naturalWidth + 60
        mirrorWindow.height = Math.max(260, naturalHeight - 60)
        tryVerify(() => mirrorWindow.userResized)
        const overrideWidth = mirrorWindow.width
        const overrideHeight = mirrorWindow.height
        const reopened = phase => JSON.stringify([{
            key: "passive-test", name: "Fixture", phase: phase,
            canControl: false, viewOnly: false, app: true
        }])
        // Reopen in place: the stream clears and is showing before its first frame.
        videoView.frameSize = Qt.size(0, 0)
        Mirror.windows = reopened("asking")
        tryVerify(() => mirrorWindow.phase === "asking")
        Mirror.calls = []
        Mirror.windows = reopened("showing")
        tryVerify(() => mirrorWindow.phase === "showing")
        wait(400)
        compare(Mirror.calls.filter(call => call[0] === "resize").length, 0,
                "no resize reaches the phone before the reopened stream's first frame")

        videoView.frameSize = Qt.size(420, 860)
        tryVerify(() => Mirror.calls.some(call => call[0] === "resize"))
        compare(Mirror.calls.filter(call => call[0] === "resize").pop(),
                ["resize", "passive-test", overrideWidth, overrideHeight])
        verify(mirrorWindow.userResized)
        // The phone's app display follows the override.
        videoView.frameSize = Qt.size(overrideWidth * 2, overrideHeight * 2)
        findChild(mirrorWindow, "mirrorNaturalSize").clicked()
        tryVerify(() => mirrorWindow.width === naturalWidth && mirrorWindow.height === naturalHeight)
        verify(!mirrorWindow.userResized)
    }

    function test_automatic_or_legacy_size_does_not_override_natural_frame_data() {
        return [
            { tag: "automatic", intent: false },
            { tag: "legacy", intent: undefined }
        ]
    }

    function test_automatic_or_legacy_size_does_not_override_natural_frame(data) {
        createMirror(false, "showing", {
            app: true, savedWidth: 1600, savedHeight: 1600,
            savedUserResized: data.intent, savedX: 100000, savedY: 100000
        })
        tryVerify(() => mirrorWindow.readyForResize)
        verify(!mirrorWindow.userResized)
        verify(Math.abs(mirrorWindow.width / mirrorWindow.height - 420 / 860) < 0.02,
               "automatic or unproven legacy dimensions must not become a user override")
        const area = mirrorWindow.availableArea
        verify(mirrorWindow.x + mirrorWindow.width <= area.x + area.width + 1)
        verify(mirrorWindow.y + mirrorWindow.height <= area.y + area.height + 1)
        const saved = Mirror.calls.filter(call => call[0] === "saveGeometry").pop()
        verify(saved !== undefined)
        compare(saved[1], "passive-test")
        compare(saved[6], false)
        mirrorWindow.visibility = Window.Minimized
        tryVerify(() => mirrorWindow.visibility === Window.Minimized)
        mirrorWindow.visibility = Window.Windowed
        tryVerify(() => mirrorWindow.readyForResize)
        verify(!mirrorWindow.userResized, "restore must not create resize intent")
    }

    function test_stale_offscreen_saved_window_is_fitted_and_clamped() {
        createMirror(false, "showing", {
            app: true, savedWidth: 1600, savedHeight: 1600, savedUserResized: true,
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

    // Stream config can enter showing before the first decoded frame.
    // An already sized app must not send resize, and a zero frame must
    // not become the natural capture, until a positive frame exists.
    function test_showing_before_first_frame_does_not_send_resize_or_capture_natural_size() {
        createMirror(false, "connecting", { app: true }, true)
        verify(!mirrorWindow.sized)
        verify(mirrorWindow.frame.width <= 0)
        verify(mirrorWindow.naturalFrame.width <= 0)

        videoView.frameSize = Qt.size(420, 860)
        tryVerify(() => mirrorWindow.sized && mirrorWindow.naturalFrame.width === 420)
        // Drain a timer armed while the phase is still connecting.
        wait(400)

        videoView.frameSize = Qt.size(0, 0)
        tryVerify(() => mirrorWindow.naturalFrame.width === 0)
        verify(mirrorWindow.sized, "a missing frame keeps the open app window sized")
        verify(mirrorWindow.phase === "connecting")
        const keptWidth = mirrorWindow.width
        const keptHeight = mirrorWindow.height
        const current = JSON.parse(Mirror.windows)[0]
        current.phase = "showing"
        Mirror.calls = []
        Mirror.windows = JSON.stringify([current])
        tryVerify(() => mirrorWindow.phase === "showing" && mirrorWindow.frame.width <= 0)
        wait(400)

        compare(Mirror.calls.filter(call => call[0] === "resize").length, 0)
        verify(mirrorWindow.naturalFrame.width <= 0,
               "an absent frame is not the natural capture")
        compare(mirrorWindow.width, keptWidth)
        compare(mirrorWindow.height, keptHeight)

        // A manual resize still at frame 0 arms the timer through onWidthChanged;
        // the timer fires (userResized is committed) but must not reach the phone.
        tryVerify(() => mirrorWindow.readyForResize)
        verify(!mirrorWindow.userResized)
        const resizedWidth = keptWidth + 40
        mirrorWindow.width = resizedWidth
        wait(400)
        tryVerify(() => mirrorWindow.userResized)
        compare(Mirror.calls.filter(call => call[0] === "resize").length, 0,
                "a timer armed before the first frame does not resize the phone")
        verify(mirrorWindow.naturalFrame.width <= 0)
        compare(mirrorWindow.width, resizedWidth)
        compare(mirrorWindow.height, keptHeight)
    }
}
