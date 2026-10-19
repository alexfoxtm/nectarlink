// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.banking

/**
 * How the banking poll waits, and which foreground package a poll may act on.
 *
 * The screen-off path uses the idle cadence. A screen-on or user-present
 * signal ends that wait immediately, but it is not itself a privacy check:
 * a package cached from before the screen went off stays hidden until a read
 * taken while the screen is interactive observes one. Callers must run that
 * check before turning debugging back on.
 */
internal class BankingPollCadence(
    private val pollMs: Long,
    private val idleMs: Long,
) {
    private var wakePending = false
    /** One short poll after a signal that still saw the screen off, so a late flag is not an idle gap. */
    private var recheckSoon = false
    /** True after a screen return until the caller drops any resume countdown. */
    private var returnPending = false
    private var trusted = false
    private var trustedPackage: String? = null
    private var stamp = 0L

    /** Screen on or the user is present. Wakes the idle wait and drops the cached package. */
    @Synchronized
    fun onScreenReturned() {
        wakePending = true
        returnPending = true
        trusted = false
        stamp += 1
    }

    /**
     * True once after [onScreenReturned]. The caller drops its resume countdown
     * so time spent with the screen off cannot finish it.
     */
    @Synchronized
    fun consumeScreenReturn(): Boolean {
        if (!returnPending) return false
        returnPending = false
        return true
    }

    /** Captured before a usage read. A screen return during the read changes it. */
    @Synchronized
    fun stamp(): Long = stamp

    /**
     * How long to sleep after a poll. Zero when a screen return is already
     * pending, so the loop checks again before sleeping. The pending wake is
     * consumed, so a quiet screen cannot spin.
     */
    @Synchronized
    fun waitMs(screenInteractive: Boolean): Long {
        if (wakePending) {
            wakePending = false
            // The broadcast can land before isInteractive flips. One short poll
            // covers that. Leaving the flag set would busy-loop while the screen stays off.
            recheckSoon = !screenInteractive
            return 0L
        }
        if (recheckSoon) {
            recheckSoon = false
            return pollMs
        }
        return if (screenInteractive) pollMs else idleMs
    }

    /**
     * The package [PauseDecider] may see.
     *
     * [observedPackage] is the newest foreground event in this read, or null
     * when the read saw none. [readStamp] is [stamp] from before the read.
     * A screen-off read, a failed read, or a read that raced a screen return
     * is not a current check: the result is null and the cache is not kept.
     */
    @Synchronized
    fun checkedForeground(
        screenInteractive: Boolean,
        observedPackage: String?,
        readStamp: Long,
        readOk: Boolean,
    ): String? {
        if (!screenInteractive || readStamp != stamp || !readOk) {
            trusted = false
            return null
        }
        if (observedPackage != null) {
            trustedPackage = observedPackage
            trusted = true
        }
        return if (trusted) trustedPackage else null
    }
}
