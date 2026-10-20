// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.mirror

/**
 * Which full-screen mirroring request owns [MirrorService]'s capture.
 *
 * The core stamps every `mirror.start` with a generation that the phone
 * echoes in the stream's config; a Mac reconnect is a *new* generation from
 * the same PC. The service keeps a single capture, so an incoming start
 * either replaces the live one or is dropped, and every async capture job
 * compares its own generation against the live one before touching shared
 * state (so a stale job can neither outlive nor reopen a replacement).
 */
internal object MirrorSessionPolicy {
    /**
     * Whether an incoming start replaces the live capture. Only a distinct
     * generation from the same PC preempts: duplicates, other PCs, and an
     * unstamped (legacy) request against a correlated session keep the live
     * capture, and a correlated request supersedes a legacy capture.
     *
     * Ordering is by distinctness, not `>`: the core already authenticated
     * and admitted the incoming start, and its counter resets on process
     * restart, so a smaller-but-distinct token is still a replacement.
     */
    fun shouldPreempt(
        activePc: String?,
        activeRequest: ULong?,
        incomingPc: String,
        incomingRequest: ULong?,
    ): Boolean {
        if (activePc == null) return true
        if (activePc != incomingPc) return false
        if (activeRequest == null) return incomingRequest != null
        if (incomingRequest == null) return false
        return incomingRequest != activeRequest
    }

    /**
     * Whether an async capture job still owns the live capture. Compares the
     * service-local [activeEpoch] first: (pc, request) can match again after
     * a stop/restart or a legacy `null` request, so the epoch alone retires
     * those jobs. Call on main with the live fields, so the check is atomic
     * with the guarded mutation.
     */
    fun isLive(
        activeEpoch: Long,
        activePc: String?,
        activeRequest: ULong?,
        myEpoch: Long,
        myPc: String,
        myRequest: ULong?,
    ): Boolean {
        if (activeEpoch != myEpoch) return false
        if (activePc != myPc) return false
        return activeRequest == myRequest
    }
}
