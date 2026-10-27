// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.banking

import kotlinx.coroutines.channels.Channel
import kotlinx.coroutines.withTimeoutOrNull

/**
 * How the banking poll waits, and which foreground package a poll may act on.
 *
 * The screen-off path uses the idle cadence only when a screen-on receiver can
 * cut that wait short. Without the receiver, the wait stays on the short poll
 * so a dark screen cannot hide the app in front for the whole idle period.
 *
 * A screen return is not itself a privacy check. It drops the cached package
 * and rewinds the usage cursor to the window that held the last observed
 * foreground event. A read that raced the return is not trusted and does not
 * move the cursor, so the next poll can see that event again.
 *
 * The only lock is this instance. [onScreenReturned], [beginRead], [finishRead],
 * and [waitMs] take it for the field update and release it. None of them runs
 * the usage query, and none calls another, so the screen receiver is not
 * blocked on that query and there is no second lock to order against.
 */
internal class BankingPollCadence(
    private val pollMs: Long,
    private val idleMs: Long,
    initialCursorMs: Long = 0L,
) {
    private var wakePending = false
    /** One short poll after a signal that still saw the screen off, so a late flag is not an idle gap. */
    private var recheckSoon = false
    /** True after a screen return until the caller drops any resume countdown. */
    private var returnPending = false
    private var trusted = false
    private var trustedPackage: String? = null
    private var stamp = 0L
    private var cursor = initialCursorMs
    /** Cursor at the start of the read that last observed a foreground event. */
    private var eventFrom = initialCursorMs

    /** Screen on or the user is present. Wakes the idle wait, drops the cache, and replays from [eventFrom]. */
    @Synchronized
    fun onScreenReturned() {
        wakePending = true
        returnPending = true
        trusted = false
        stamp += 1
        cursor = eventFrom
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

    /** Cursor and stamp for a usage read. Captured together, before the query. */
    @Synchronized
    fun beginRead(): ReadWindow = ReadWindow(cursor, stamp)

    /**
     * How long to sleep after a poll. Without [wakeAvailable], always the short
     * poll: nothing can interrupt an idle sleep. Zero when a screen return is
     * already pending. That wake is consumed, so a quiet screen cannot spin.
     */
    @Synchronized
    fun waitMs(screenInteractive: Boolean, wakeAvailable: Boolean): Long {
        if (!wakeAvailable) return pollMs
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
     * The package [PauseDecider] may see, or null when this read is not a check.
     *
     * [observedPackage] is the newest foreground event in the window that
     * started at [beginRead], or null when the read saw none. The cursor
     * advances to [windowEnd] only when the read is accepted. A screen-off
     * read, a failed read, or a read whose stamp no longer matches is rejected:
     * the cache is cleared and the cursor stays where it was.
     */
    @Synchronized
    fun finishRead(
        readStamp: Long,
        screenInteractive: Boolean,
        readOk: Boolean,
        observedPackage: String?,
        windowEnd: Long,
    ): String? {
        if (!screenInteractive || readStamp != stamp || !readOk) {
            trusted = false
            return null
        }
        if (observedPackage != null) {
            trustedPackage = observedPackage
            trusted = true
            eventFrom = cursor
        }
        if (windowEnd > cursor) cursor = windowEnd
        return if (trusted) trustedPackage else null
    }
}

/** The usage window a poll opened. [stamp] is what [BankingPollCadence.finishRead] must still see. */
internal data class ReadWindow(val cursor: Long, val stamp: Long)

/**
 * The park [app.nectarlink.android.service.ConnectionService] uses after a banking poll.
 * A queued screen return ends the wait immediately. A zero wait drains one queued signal
 * so the next sleep is not skipped as well.
 */
internal suspend fun awaitScreenWake(waitMs: Long, wake: Channel<Unit>) {
    if (waitMs <= 0L) {
        wake.tryReceive()
        return
    }
    withTimeoutOrNull(waitMs) { wake.receive() }
}
