// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.ui.home

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class TimelinePageOwnerTest {
    @Test fun nextPageHasOneOwnerAndCanContinueAfterCompletion() {
        val owner = TimelinePageOwner()
        val generation = owner.reset()
        assertNull(owner.beginMore(0u))
        assertTrue(owner.markReady(generation))
        val first = requireNotNull(owner.beginMore(50u))
        assertEquals(50u, first.offset)
        assertTrue(owner.isCurrent(generation))
        assertTrue(owner.isCurrent(first))
        assertNull(owner.beginMore(50u))
        assertTrue(owner.completeMore(first))
        val next = requireNotNull(owner.beginMore(100u))
        assertTrue(owner.isCurrent(next))
        assertFalse(owner.completeMore(first))
    }

    @Test fun changedQueryRejectsOldPageAndCannotReleaseNewOwner() {
        val owner = TimelinePageOwner()
        val oldGeneration = owner.reset()
        assertTrue(owner.markReady(oldGeneration))
        val oldPage = requireNotNull(owner.beginMore(50u))
        val currentGeneration = owner.reset()
        assertFalse(owner.markReady(oldGeneration))
        assertNull(owner.beginMore(0u))
        assertTrue(owner.markReady(currentGeneration))
        val currentPage = requireNotNull(owner.beginMore(0u))
        assertFalse(owner.isCurrent(oldGeneration))
        assertFalse(owner.isCurrent(oldPage))
        assertFalse(owner.completeMore(oldPage))
        assertTrue(owner.isCurrent(currentGeneration))
        assertTrue(owner.isCurrent(currentPage))
        assertNull(owner.beginMore(0u))
    }
}
