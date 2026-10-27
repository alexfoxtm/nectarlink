// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.banking

import kotlinx.coroutines.channels.Channel
import kotlinx.coroutines.runBlocking
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * The waits and the cursor live in [BankingPollCadence] and [awaitScreenWake],
 * which is what ConnectionService parks on. These two tests call those, not a copy.
 */
class BankingPollCadenceTest {
    private val pollMs = 800L
    private val idleMs = 15_000L
    private val bank = "com.uco.bank"
    private val launcher = "com.launcher"

    @Test
    fun screenOffIdlesOnlyWhenTheReceiverCanWakeAndTheChannelInterruptsThatWait() = runBlocking {
        val cadence = BankingPollCadence(pollMs, idleMs)
        // registerScreenWake failed: ConnectionService passes wakeAvailable false.
        // A 15s sleep here would be a foreground gap with no broadcast to end it.
        assertEquals(pollMs, cadence.waitMs(screenInteractive = false, wakeAvailable = false))
        assertEquals(pollMs, cadence.waitMs(screenInteractive = true, wakeAvailable = false))

        assertEquals(idleMs, cadence.waitMs(screenInteractive = false, wakeAvailable = true))
        assertEquals(pollMs, cadence.waitMs(screenInteractive = true, wakeAvailable = true))

        val wake = Channel<Unit>(Channel.CONFLATED)
        wake.trySend(Unit)
        val started = System.nanoTime()
        awaitScreenWake(idleMs, wake)
        assertTrue(System.nanoTime() - started < 1_000_000_000L)
        assertTrue(wake.tryReceive().isFailure)

        // The zero-wait branch drains one queued signal so the next sleep is not skipped too.
        wake.trySend(Unit)
        awaitScreenWake(0L, wake)
        assertTrue(wake.tryReceive().isFailure)

        cadence.onScreenReturned()
        assertEquals(0L, cadence.waitMs(screenInteractive = false, wakeAvailable = true))
        assertEquals(pollMs, cadence.waitMs(screenInteractive = false, wakeAvailable = true))
        assertEquals(idleMs, cadence.waitMs(screenInteractive = false, wakeAvailable = true))
    }

    @Test
    fun aRacedReadReplaysTheWindowAndDoesNotTrustAStalePackage() {
        val cadence = BankingPollCadence(pollMs, idleMs, initialCursorMs = 1_000L)
        val opened = cadence.beginRead()
        assertEquals(1_000L, opened.cursor)
        assertEquals(launcher, cadence.finishRead(opened.stamp, true, true, launcher, 2_000L))

        val continued = cadence.beginRead()
        assertEquals(2_000L, continued.cursor)
        // No new event. The cursor moves, but the replay point stays the read that saw the launcher.
        assertEquals(launcher, cadence.finishRead(continued.stamp, true, true, null, 3_000L))

        val inFlight = cadence.beginRead()
        assertEquals(3_000L, inFlight.cursor)
        cadence.onScreenReturned()
        // Same signal the receiver sends: one chance to drop the resume countdown, then the idle wait ends.
        assertTrue(cadence.consumeScreenReturn())
        assertFalse(cadence.consumeScreenReturn())
        assertEquals(0L, cadence.waitMs(screenInteractive = true, wakeAvailable = true))
        assertNull(cadence.finishRead(inFlight.stamp, true, true, launcher, 4_000L))

        val replay = cadence.beginRead()
        assertEquals(1_000L, replay.cursor)
        assertEquals(bank, cadence.finishRead(replay.stamp, true, true, bank, 4_500L))
        assertEquals(4_500L, cadence.beginRead().cursor)

        val failed = cadence.beginRead()
        assertNull(cadence.finishRead(failed.stamp, true, false, launcher, 6_000L))
        assertEquals(failed.cursor, cadence.beginRead().cursor)
    }
}
