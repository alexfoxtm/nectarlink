// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.webcam

import android.content.Context
import android.os.Looper
import app.nectarlink.core.NectarlinkNode
import app.nectarlink.core.WebcamCaptureContext
import java.util.UUID
import java.util.concurrent.CountDownLatch
import java.util.concurrent.TimeUnit

/** In-memory original admissions. Tokens select Entries, never current pcId authority. */
internal object WebcamEntries {
    private val lock = Any()
    private val entries = mutableMapOf<String, Entry>()
    private var capture: Entry? = null

    class Entry(
        val token: String,
        val pcId: String,
        var request: WebcamRequest,
        val node: NectarlinkNode?,
        val owner: WebcamCaptureContext?,
        val nodeCurrent: () -> Boolean,
        val hide: (String) -> Unit,
    ) {
        var retired = false
        var terminal = false
        var service: WebcamService? = null
        var serviceStartId: Int = 0
        val completed = CountDownLatch(1)
    }

    fun offer(context: Context, pcId: String, request: WebcamRequest,
              node: NectarlinkNode, owner: WebcamCaptureContext, nodeCurrent: () -> Boolean,
              pcName: String, show: (WebcamRequest) -> Unit, hide: (String) -> Unit): Boolean = synchronized(lock) {
        if (!nodeCurrent() || !owner.belongsToNode(node) || !owner.isCurrent()) return false
        val previous = entries.values.firstOrNull { it.owner?.sameOwner(owner) == true }
        if (previous != null) {
            if (!ownsLocked(previous)) return false
            previous.request = request.copy(ownedEntryToken = previous.token)
            if (capture === previous) WebcamService.configure(previous)
            else { show(previous.request); WebcamRequests.show(context, previous.request, pcName) }
            return true
        }
        // One physical capture remains reserved through both encoder cleanup lanes.
        if (capture != null || entries.values.any { it.pcId == pcId && !it.terminal }) return false
        val token = UUID.randomUUID().toString()
        val entry = Entry(token, pcId, request.copy(ownedEntryToken = token), node, owner, nodeCurrent, hide)
        entries[token] = entry
        if (!ownsLocked(entry)) { entries.remove(token); return false }
        show(entry.request)
        WebcamRequests.show(context, entry.request, pcName)
        true
    }

    fun legacy(pcId: String, height: Int, fps: Int, camera: String): Entry? = synchronized(lock) {
        capture?.let { active ->
            if (active.owner != null || active.pcId != pcId || !ownsLocked(active)) return null
            active.request = active.request.copy(height = height, fps = fps, camera = camera)
            return active
        }
        val token = UUID.randomUUID().toString()
        Entry(token, pcId, WebcamRequest(pcId, height = height, fps = fps, camera = camera), null, null, { true }, {}).also {
            entries[token] = it
        }
    }

    fun ownedFor(pcId: String): Boolean = synchronized(lock) { entries.values.any { it.pcId == pcId && it.owner != null && !it.terminal } }

    fun resolve(token: String?): Entry? = synchronized(lock) { entries[token]?.takeIf(::ownsLocked) }
    fun owns(entry: Entry): Boolean = synchronized(lock) { ownsLocked(entry) }
    private fun ownsLocked(entry: Entry): Boolean = entries[entry.token] === entry && !entry.retired &&
        entry.nodeCurrent() && runCatching { entry.owner?.isCurrent() ?: true }.getOrDefault(false)

    fun withCurrent(entry: Entry, action: () -> Unit): Boolean = synchronized(lock) {
        if (!ownsLocked(entry)) return false
        action()
        true
    }

    fun begin(entry: Entry, service: WebcamService, startId: Int): Boolean = synchronized(lock) {
        if (!ownsLocked(entry) || (capture != null && capture !== entry)) return false
        capture = entry
        entry.service = service
        entry.serviceStartId = startId
        entry.hide(entry.token)
        WebcamRequests.dismissOwned(entry.token)
        true
    }

    fun stop(entry: Entry) = synchronized(lock) {
        if (entries[entry.token] !== entry || entry.retired) return
        entry.retired = true
        entry.hide(entry.token)
        WebcamRequests.dismissOwned(entry.token)
        // Revokes the actual original Core owner and its stream; cannot select S2.
        entry.owner?.finish()
        val service = entry.service
        if (service == null) completeLocked(entry) else service.stopEntry(entry)
    }

    fun complete(entry: Entry) = synchronized(lock) { completeLocked(entry) }
    private fun completeLocked(entry: Entry) {
        if (entries[entry.token] !== entry) return
        entry.terminal = true
        if (capture === entry) capture = null
        entries.remove(entry.token)
        entry.completed.countDown()
    }

    /** Called off-main by the Core blocking platform worker, after Core retirement. */
    fun stopOwned(owner: WebcamCaptureContext): Boolean {
        val entry = synchronized(lock) { entries.values.firstOrNull { it.owner?.sameOwner(owner) == true } }
            ?: return !owner.isCurrent() // Core retirement prevents any later admission.
        stop(entry)
        if (Looper.myLooper() == Looper.getMainLooper()) return entry.terminal
        return entry.completed.await(5, TimeUnit.SECONDS)
    }

    fun stopForNode(original: NectarlinkNode) {
        val old = synchronized(lock) { entries.values.filter { it.node === original } }
        old.forEach(::stop)
        if (Looper.myLooper() != Looper.getMainLooper()) old.forEach { it.completed.await(5, TimeUnit.SECONDS) }
    }

    fun keyframeOwned(owner: WebcamCaptureContext) {
        val entry = synchronized(lock) { entries.values.firstOrNull { it.owner?.sameOwner(owner) == true } } ?: return
        withCurrent(entry) { entry.service?.keyframeEntry(entry) }
    }
}
