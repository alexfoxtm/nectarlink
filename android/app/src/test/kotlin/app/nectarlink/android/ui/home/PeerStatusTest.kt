// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.ui.home

import org.junit.Assert.assertEquals
import org.junit.Test

class PeerStatusTest {
    @Test
    fun `proven cable renders USB, unknown medium does not`() {
        assertEquals(PeerOnlineLabel.USB, resolvePeerOnlineLabel("usb", "physical", null))
        // A selected tunnel without origin proof (old peer, direct TCP) stays honest.
        assertEquals(PeerOnlineLabel.CONNECTED, resolvePeerOnlineLabel("usb", "unknown", null))
    }

    @Test
    fun `wireless ADB never claims USB`() {
        // Galaxy107 regression: selected network tunnel, no proven route.
        assertEquals(PeerOnlineLabel.CONNECTED, resolvePeerOnlineLabel("usb", "network", null))
        assertEquals(PeerOnlineLabel.CONNECTED, resolvePeerOnlineLabel("lan", "unknown", null))
        // Invalid hints never render a transport either.
        for (raw in listOf("", "WiFi", "WIFI", "lte", "vpn", "usb-tether", "bluetooth")) {
            assertEquals("raw=$raw", PeerOnlineLabel.CONNECTED, resolvePeerOnlineLabel("usb", "network", raw))
        }
        assertEquals(null, normalizePeerTransport("vpn"))
        assertEquals("wifi", normalizePeerTransport("wifi"))
    }

    @Test
    fun `relay stays Away even with proven hints`() {
        assertEquals(PeerOnlineLabel.AWAY, resolvePeerOnlineLabel("relay", "unknown", "wifi"))
        assertEquals(PeerOnlineLabel.AWAY, resolvePeerOnlineLabel("relay", "physical", "wifi"))
    }

    @Test
    fun `proven wifi renders Wi-Fi`() {
        assertEquals(PeerOnlineLabel.WIFI, resolvePeerOnlineLabel("lan", "unknown", "wifi"))
        // A network tunnel currently carrying the session reports its network.
        assertEquals(PeerOnlineLabel.WIFI, resolvePeerOnlineLabel("usb", "network", "wifi"))
    }

    @Test
    fun `proven cellular resolves without guessing a radio subtype`() {
        assertEquals(PeerOnlineLabel.CELLULAR, resolvePeerOnlineLabel("lan", "unknown", "cellular"))
    }

    @Test
    fun `proven ethernet and USB tethering render their transports`() {
        assertEquals(PeerOnlineLabel.ETHERNET, resolvePeerOnlineLabel("lan", "unknown", "ethernet"))
        // linkTransport usb means a proven USB-tethering route, the only other honest USB.
        assertEquals(PeerOnlineLabel.USB, resolvePeerOnlineLabel("lan", "unknown", "usb"))
    }
}
