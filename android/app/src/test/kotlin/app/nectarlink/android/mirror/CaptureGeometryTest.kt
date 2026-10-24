// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.mirror

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class CaptureGeometryTest {
    @Test
    fun orientedAndFoldedContentUsesTheLatestCapturedShapeWithoutTransposing() {
        assertEquals(864 to 1920, fitCapturedContent(1080, 2400, 1920, 420)?.let { it.first to it.second })
        assertEquals(1920 to 864, fitCapturedContent(2400, 1080, 1920, 420)?.let { it.first to it.second })
        assertEquals(1760 to 1920, fitCapturedContent(2208, 2400, 1920, 420)?.let { it.first to it.second })
        assertEquals(420, fitCapturedContent(2208, 2400, 1920, 420)?.third)
        val gate = CaptureGeometryGate()
        val portrait = gate.submit(2256, 2504, 480)
        val landscape = gate.submit(2504, 2256, 480)
        val written = mutableListOf<Triple<Int, Int, Int>>()
        assertFalse(gate.publish(portrait, Any()) { written += Triple(it.width, it.height, it.dpi) })
        assertTrue(gate.publish(landscape, Any()) { written += Triple(it.width, it.height, it.dpi) })
        assertEquals(listOf(Triple(2504, 2256, 480)), written)
    }

    @Test
    fun invalidResizeAndRetiredCaptureEpochCannotSupplyNewGeometry() {
        assertNull(fitCapturedContent(0, 2400, 1920, 420))
        assertNull(fitCapturedContent(1080, -1, 1920, 420))
        assertNull(fitCapturedContent(1080, 2400, 0, 420))
        val gate = CaptureGeometryGate()
        val older = gate.submit(2256, 2504, 480)
        gate.submit(2504, 2256, 480)
        assertFalse(gate.publish(older, Any()) {})
        // MirrorService uses this actual production admission predicate before
        // forwarding a callback, even when a restarted capture reuses its IDs.
        assertFalse(MirrorSessionPolicy.isLive(12L, "pc", 8uL, 11L, "pc", 8uL))
    }
}
