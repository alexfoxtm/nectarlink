// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.banking

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class BankingPollCadenceTest {
    private val pollMs = 800L
    private val idleMs = 15_000L
    private val bank = "com.uco.bank"
    private val launcher = "com.launcher"

    @Test
    fun screenOffUsesTheIdleCadenceAndAScreenReturnRechecksImmediately() {
        val cadence = BankingPollCadence(pollMs, idleMs)
        assertEquals(pollMs, cadence.waitMs(screenInteractive = true))
        assertEquals(idleMs, cadence.waitMs(screenInteractive = false))

        // An off-screen observation is not a check, so it cannot lift a pause.
        assertNull(cadence.checkedForeground(false, bank, cadence.stamp(), readOk = true))
        cadence.onScreenReturned()
        // The signal arrived before the sleep: the next poll runs at once.
        assertEquals(0L, cadence.waitMs(screenInteractive = true))
        val stamp = cadence.stamp()
        assertEquals(bank, cadence.checkedForeground(true, bank, stamp, readOk = true))
        // The same app stays in front. A later read with no new event still counts.
        assertEquals(bank, cadence.checkedForeground(true, null, cadence.stamp(), readOk = true))
        assertEquals(pollMs, cadence.waitMs(screenInteractive = true))
    }

    @Test
    fun screenReturnDoesNotTrustAStalePackageOrSpinTheWait() {
        val cadence = BankingPollCadence(pollMs, idleMs)
        val before = cadence.stamp()
        assertEquals(launcher, cadence.checkedForeground(true, launcher, before, readOk = true))
        assertNull(cadence.checkedForeground(false, launcher, cadence.stamp(), readOk = true))

        assertFalse(cadence.consumeScreenReturn())
        cadence.onScreenReturned()
        // One signal, one chance to drop a resume countdown. A second read must not keep resetting it.
        assertTrue(cadence.consumeScreenReturn())
        assertFalse(cadence.consumeScreenReturn())
        // The read started before the signal, and a failed read, are not checks.
        assertNull(cadence.checkedForeground(true, launcher, before, readOk = true))
        assertNull(cadence.checkedForeground(true, launcher, cadence.stamp(), readOk = false))
        // No observation since the screen returned: the cached launcher stays hidden.
        assertNull(cadence.checkedForeground(true, null, cadence.stamp(), readOk = true))

        // Signal while the screen is still off: one immediate poll, one short
        // recheck, then the idle cadence. It must not busy-loop or stay at 800ms.
        assertEquals(0L, cadence.waitMs(screenInteractive = false))
        assertEquals(pollMs, cadence.waitMs(screenInteractive = false))
        assertEquals(idleMs, cadence.waitMs(screenInteractive = false))
    }
}
