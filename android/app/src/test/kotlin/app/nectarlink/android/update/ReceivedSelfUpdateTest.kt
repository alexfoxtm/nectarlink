// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.update

import app.nectarlink.android.core.StartGuard
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.runBlocking
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class ReceivedSelfUpdateTest {
    @Test
    fun verifiedSelfUpdateRetiresBeforeInstallerHandoff() = runBlocking {
        assertTrue(isVerifiedSelfPackage("app", "app", setOf("same"), setOf("same")))
        val calls = ArrayList<String>()
        assertTrue(launchVerifiedReceivedUpdate(
            "attempt-a",
            retire = { calls += "retire:" + it; true },
            launch = { calls += "installer" },
            resume = { calls += "resume:" + it },
        ))
        assertEquals(listOf("retire:attempt-a", "installer"), calls)
    }

    @Test
    fun otherOrUnverifiedPackagesAndFailedHandoffsDoNotAcquireSuccessorAuthority() = runBlocking {
        assertFalse(isVerifiedSelfPackage("app", "other", setOf("same"), setOf("same")))
        assertFalse(isVerifiedSelfPackage("app", "app", setOf("same"), setOf("different")))
        assertFalse(isVerifiedSelfPackage("app", "app", emptySet(), emptySet()))
        val failedShutdown = ArrayList<String>()
        assertFalse(launchVerifiedReceivedUpdate(
            "attempt-old",
            retire = { failedShutdown += "retire:" + it; false },
            launch = { error("must not launch") },
            resume = { failedShutdown += "resume:" + it },
        ))
        assertEquals(listOf("retire:attempt-old", "resume:attempt-old"), failedShutdown)
        val failedLaunch = ArrayList<String>()
        try {
            launchVerifiedReceivedUpdate(
                "attempt-old",
                retire = { true },
                launch = { throw IllegalStateException("synthetic installer failure") },
                resume = { failedLaunch += it },
            )
            error("launch must fail")
        } catch (_: IllegalStateException) {
            assertEquals(listOf("attempt-old"), failedLaunch)
        }
        val cancelled = ArrayList<String>()
        try {
            launchVerifiedReceivedUpdate(
                "attempt-cancelled",
                retire = { throw CancellationException("synthetic lifecycle cancellation") },
                launch = { error("must not launch") },
                resume = { cancelled += it },
            )
            error("retirement must cancel")
        } catch (_: CancellationException) {
            assertEquals(listOf("attempt-cancelled"), cancelled)
        }

        // A resume ticket captured at owner release cannot admit a start after
        // B retirement uses the same stop invalidation as shutdownActiveNodeLocked.
        val successor = StartGuard()
        val queuedA = successor.beginStart()
        successor.stop()
        assertNull(successor.beginStartIfCurrent(queuedA))
        val currentB = successor.beginStartIfCurrent(null)!!
        assertFalse(successor.isCancelled(currentB))

        // A newer Stop, including Stop while held, retires queued recovery too.
        val stopped = StartGuard()
        val queuedBeforeStop = stopped.beginStart()
        stopped.stop()
        assertNull(stopped.beginStartIfCurrent(queuedBeforeStop))
        assertNotNull(stopped.beginStartIfCurrent(null))
    }
}
