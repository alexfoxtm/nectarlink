// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.ui.home

import app.nectarlink.android.core.Device
import app.nectarlink.core.DeviceKind
import app.nectarlink.core.Feature
import app.nectarlink.core.FeatureStatus
import app.nectarlink.core.Link
import app.nectarlink.core.Upgrade
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class HomeCapabilitiesTest {
    private fun device(link: Link, features: List<Feature>) = Device(
        id = "pc",
        name = "Mac",
        kind = DeviceKind.DESKTOP,
        model = null,
        pairedAt = 1L,
        link = link,
        features = features,
    )

    private fun online() = Link.Online(relayed = false, usb = false, rttMs = 5u)
    private fun offline() = Link.Offline(null)

    private fun available(id: String) = Feature(id, FeatureStatus.Available)
    private fun locked(id: String, action: String, target: String) =
        Feature(id, FeatureStatus.Locked(Upgrade(action, target, 0.toUByte())))

    @Test
    fun `negotiated features keep Control Deck and Webcam reachable when online`() {
        val pc = device(
            online(),
            listOf(
                available(HOME_FEATURE_INPUT_REMOTE),
                available(HOME_FEATURE_INPUT_DECK),
                available(HOME_FEATURE_CAMERA_WEBCAM),
            ),
        )
        assertTrue(pc.canControlPc())
        assertTrue(pc.canOpenDeck())
        assertTrue(pc.canOpenWebcam())
        // Offline stays greyed even with negotiated features.
        val away = pc.copy(link = offline())
        assertFalse(away.canControlPc())
        assertFalse(away.canOpenDeck())
        assertFalse(away.canOpenWebcam())
    }

    @Test
    fun `withheld Mac caps grey out Control Deck and Webcam but keep the permission path`() {
        // Mac42 withholds input.inject, deck.actions and every webcam desktop
        // cap, so the matrix reports Locked, never Available.
        val mac = device(
            online(),
            listOf(
                locked(HOME_FEATURE_INPUT_REMOTE, "updateApp", "desktop"),
                locked(HOME_FEATURE_INPUT_DECK, "updateApp", "desktop"),
                locked(HOME_FEATURE_CAMERA_WEBCAM, "updateApp", "desktop"),
            ),
        )
        assertFalse(mac.canControlPc())
        assertFalse(mac.canOpenDeck())
        assertFalse(mac.canOpenWebcam())
        // Locked only on the phone camera permission stays reachable: the
        // Webcam screen grants it and the matrix then flips to Available.
        val noPermission = device(
            online(),
            listOf(locked(HOME_FEATURE_CAMERA_WEBCAM, "grantPermission", "camera")),
        )
        assertTrue(noPermission.canOpenWebcam())
    }
}
