// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.mirror

import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class InputServiceInterruptTest {
    @Test
    fun interruptionRejectsPreviouslyQueuedInputAndAllowsNewGeneration() {
        val fence = InputInterruptFence()
        val queuedBeforeInterrupt = fence.snapshot()

        fence.interrupt()

        assertFalse(fence.isCurrent(queuedBeforeInterrupt))
        assertTrue(fence.isCurrent(fence.snapshot()))
    }
}
