// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.elevated

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

class AppDisplayPendingSizeTest {
    @Test
    fun consumeIsAtomicAndLaterResizeRemainsPending() {
        val pending = PendingAppDisplaySize()
        val first = Triple(800, 600, 240)
        val latestBeforeConsume = Triple(1024, 768, 280)
        val submittedAfterConsume = Triple(1280, 720, 320)

        pending.submit(first)
        pending.submit(latestBeforeConsume)
        assertEquals(latestBeforeConsume, pending.take())
        assertNull(pending.take())

        pending.submit(submittedAfterConsume)
        assertEquals(submittedAfterConsume, pending.take())
    }
}
