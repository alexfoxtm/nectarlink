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
     * Whether an incoming start replaces the live capture. Only a newer
     * generation from the same PC preempts: duplicates, reordered older
     * generations, and other PCs keep the live capture, and an unstamped
     * (legacy) request never displaces a correlated one.
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
        return incomingRequest > activeRequest
    }
}
