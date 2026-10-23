// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.ui.home

/** Main-thread ownership for one Timeline query and its single pending next page. */
internal class TimelinePageOwner {
    data class Request internal constructor(val generation: Long, val ticket: Long, val offset: UInt)

    private var generation = 0L
    private var ticket = 0L
    private var pending: Request? = null
    private var queryReady = false

    fun reset(): Long {
        generation += 1
        pending = null
        queryReady = false
        return generation
    }

    fun isCurrent(expected: Long): Boolean = generation == expected

    fun markReady(expected: Long): Boolean {
        if (!isCurrent(expected)) return false
        queryReady = true
        return true
    }

    fun beginMore(offset: UInt): Request? {
        if (!queryReady || pending != null) return null
        return Request(generation, ++ticket, offset).also { pending = it }
    }

    fun isCurrent(request: Request): Boolean =
        generation == request.generation && pending == request

    fun completeMore(request: Request): Boolean {
        if (!isCurrent(request)) return false
        pending = null
        return true
    }
}
