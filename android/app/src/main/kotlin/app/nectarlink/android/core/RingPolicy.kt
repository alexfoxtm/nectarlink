// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.core

/**
 * Pure ring-override policy for "Find my phone" (no Android types, so
 * it is unit-testable without a device): how far the alarm volume is
 * temporarily raised, how the user's original volume is captured and
 * restored exactly once, and when Do Not Disturb can still keep the
 * ring from being heard.
 */
object RingPolicy {

    /** The volume a ring temporarily uses: full alarm volume. */
    fun raisedAlarmVolume(current: Int, max: Int): Int =
        if (max > 0) max else current.coerceAtLeast(0)

    /** Mirrors `NotificationManager.INTERRUPTION_FILTER_*` without depending on it. */
    enum class ZenFilter { UNKNOWN, ALL, PRIORITY, NONE, ALARMS }

    /**
     * Whether alarm-category audio can still be heard under [filter].
     * It passes ALL and ALARMS; under PRIORITY it passes only when the
     * user's DND policy allows alarms, which this cannot see, so
     * PRIORITY is treated as possibly audible. Total silence (NONE)
     * suppresses it at the system level, and no app-side attribute
     * bypasses that.
     */
    fun alarmReachesUser(filter: ZenFilter): Boolean = filter != ZenFilter.NONE

    /**
     * Captures the user's original alarm volume once and hands it
     * back exactly once for restoration. Overlapping rings keep the
     * first captured value (a re-capture would record the raised
     * volume as the "original"); a second restore attempt gets
     * nothing, so stop/timeout/cancel paths can all call it safely.
     */
    class OverrideTracker {
        private var saved: Int? = null

        val active: Boolean get() = saved != null

        /** True when this call captured [current] (no override was active). */
        fun begin(current: Int): Boolean {
            if (saved != null) return false
            saved = current
            return true
        }

        /** The captured volume on the first restore attempt, then nothing. */
        fun consumeRestore(): Int? {
            val value = saved
            saved = null
            return value
        }
    }
}
