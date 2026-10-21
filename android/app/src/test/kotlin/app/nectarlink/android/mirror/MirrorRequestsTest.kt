// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.mirror

import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class MirrorRequestsTest {
    private fun request(pc: String, generation: ULong?, connection: ULong? = 1u) =
        MirrorRequest(pc, 1920, 60, 8_000_000, audio = false, request = generation, connection = connection)

    @Test
    fun theConsentSharesOnlyTheLatestRequestAndOnlyOnce() {
        val older = request("consent-latest", 5u, connection = 1u)
        val newer = request("consent-latest", 6u, connection = 2u)
        MirrorRequests.ask(older)
        // A Mac reconnect while the prompt for the older one is up.
        MirrorRequests.ask(newer)
        assertFalse(MirrorRequests.take(older))
        // The newer request with the older connection stamped on isn't the one asked.
        assertFalse(MirrorRequests.take(newer.copy(connection = 1u)))
        assertTrue(MirrorRequests.take(newer))
        // The same consent delivered twice (notification tap + foreground open).
        assertFalse(MirrorRequests.take(newer))
    }

    @Test
    fun aRequestThePcStoppedSharesNothing() {
        val asked = request("consent-stopped", 7u)
        MirrorRequests.ask(asked)
        MirrorRequests.forget(asked.pcId)
        assertFalse(MirrorRequests.isPending(asked))
        assertFalse(MirrorRequests.take(asked))
    }

    private fun canControl(
        granted: Boolean = true,
        inputRunning: Boolean = false,
        elevatedRunning: Boolean = false,
        typingStopped: Boolean = false,
        correlated: Boolean = true,
    ) = MirrorRequests.canControl(granted, inputRunning, elevatedRunning, typingStopped, correlated)

    @Test
    fun aPcTheOwnerAllowedCanControlThroughTheAccessibilityServiceOrTheHelper() {
        assertTrue(canControl(inputRunning = true))
        assertTrue(canControl(elevatedRunning = true))
    }

    @Test
    fun aPcOnlyWatchesWithoutTheOwnersGrantInputOrACorrelatedRequestOrAfterStop() {
        // The accessibility service and the helper running is no grant.
        assertFalse(canControl(granted = false, inputRunning = true, elevatedRunning = true))
        assertFalse(canControl())
        assertFalse(canControl(inputRunning = true, typingStopped = true))
        assertFalse(canControl(inputRunning = true, correlated = false))
    }
}
