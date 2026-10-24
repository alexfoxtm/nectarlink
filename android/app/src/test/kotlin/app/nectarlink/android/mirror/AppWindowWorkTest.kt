// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.mirror

import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class AppWindowWorkTest {
    @Test fun successorAdmitStaysBusyWhenOlderMemberRetires() {
        val work = AppWindowWork()
        assertFalse(work.busy.value)
        val older = work.openStarted { true }
        assertTrue(older != null && work.busy.value)
        work.admit(older!!)
        val successor = work.openStarted { true }
        work.admit(successor!!)
        work.retire(older)
        assertTrue(work.busy.value)
        // Stale A cannot restore itself or clear admitted B, so idle must not unpublish.
        var unpublished = false
        work.admit(older)
        assertFalse(work.retireIfIdle { unpublished = true })
        assertFalse(unpublished)
        assertTrue(work.busy.value)
        val failed = work.openStarted { true }
        work.abandon(failed!!)
        assertTrue(work.busy.value)
        assertFalse(work.retireIfIdle { unpublished = true })
        assertFalse(unpublished)
        work.retire(successor)
        assertFalse(work.busy.value)
    }

    @Test fun abandonedOrRetiredMembershipIsNotBusy() {
        val work = AppWindowWork()
        assertNull(work.openStarted { false })
        assertFalse(work.busy.value)
        val denied = work.openStarted { true }
        work.abandon(denied!!)
        assertFalse(work.busy.value)
        work.admit(denied)
        assertFalse(work.busy.value)

        val closing = work.openStarted { true }
        assertTrue(work.busy.value)
        work.retire(closing!!)
        assertFalse(work.busy.value)
        work.admit(closing)
        assertFalse(work.busy.value)

        val opened = work.openStarted { true }
        work.admit(opened!!)
        assertTrue(work.busy.value)
        // Pending B keeps idle from unpublishing after admitted A retires.
        val pending = work.openStarted { true }
        work.retire(opened)
        assertFalse(work.retireIfIdle { error("unpublish") })
        assertTrue(work.busy.value)
        work.retire(pending!!)
        assertFalse(work.busy.value)
        work.admit(opened)
        work.retire(opened)
        var published = true
        assertTrue(work.retireIfIdle { published = false })
        assertFalse(published)
        assertNull(work.openStarted { published })
        assertFalse(work.busy.value)
    }
}
