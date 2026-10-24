// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.core

import java.util.concurrent.atomic.AtomicInteger

/**
 * Tracks stops so that in-flight starts can detect if a stop occurred
 * after they began. Multiple start calls do not cancel each other.
 */
internal class StartGuard {
    private val stopCount = AtomicInteger(0)

    class Token internal constructor(internal val stopAtStart: Int)

    fun beginStart(): Token = Token(stopCount.get())

    /** Core calls this under its lifecycle lock, before any queued-start side effect. */
    fun beginStartIfCurrent(expected: Token?): Token? {
        val stop = stopCount.get()
        if (expected != null && expected.stopAtStart != stop) return null
        return Token(stop)
    }

    fun stop() {
        stopCount.incrementAndGet()
    }

    fun isCancelled(token: Token): Boolean = stopCount.get() != token.stopAtStart
}
