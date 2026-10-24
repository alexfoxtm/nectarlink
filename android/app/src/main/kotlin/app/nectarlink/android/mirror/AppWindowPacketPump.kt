// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.mirror

import java.io.DataInputStream
import java.io.IOException
import java.net.Socket
import java.util.ArrayDeque

/** The single helper reader stays live while opening the remote stream is suspended. */
internal class AppWindowPacketPump(
    private val socket: Socket,
    private val current: () -> Boolean,
    private val requestKeyframe: () -> Unit = {},
    private val onEnded: () -> Unit,
) {
    internal data class Packet(val kind: Int, val time: ULong, val data: ByteArray)
    private val lock = Object()
    private val packets = ArrayDeque<Packet>()
    private var queuedBytes = 0
    private var closed = false
    private var attached = false
    private var config: Packet? = null
    private var waitingForKeyframe = true
    private var keyframeRequestPending = false

    fun start() {
        Thread({ read() }, "app-window-reader").apply { isDaemon = true; start() }
    }

    private fun read() {
        try {
            val input = DataInputStream(socket.getInputStream().buffered(1 shl 16))
            while (current()) {
                val length = input.readInt()
                if (length < 9 || length > MAX_PACKET) return
                val kind = input.readUnsignedByte()
                val time = input.readLong().toULong()
                val data = ByteArray(length - 9).also { input.readFully(it) }
                if (kind !in KIND_CONFIG..KIND_KEYFRAME || (kind == KIND_CONFIG && data.size != 12)) return
                if (!current()) return
                val packet = Packet(kind, time, data)
                synchronized(lock) {
                    if (closed) return
                    if (kind == KIND_CONFIG) {
                        // A new encoder configuration invalidates every queued dependency.
                        keyframeRequestPending = config != null
                        config = packet
                        packets.clear()
                        queuedBytes = 0
                        waitingForKeyframe = true
                        enqueue(packet)
                    } else {
                        val latestConfig = config ?: return
                        if (kind == KIND_KEYFRAME || !waitingForKeyframe) {
                            if (packets.size >= MAX_QUEUED_PACKETS || queuedBytes + data.size > MAX_QUEUED_BYTES) {
                                // Dropping any frame breaks the queued chain. Keep its config,
                                // drain the helper, and resume only at a complete keyframe.
                                packets.clear()
                                queuedBytes = 0
                                enqueue(latestConfig)
                                waitingForKeyframe = true
                                keyframeRequestPending = true
                            }
                            if (kind == KIND_KEYFRAME) {
                                waitingForKeyframe = false
                                keyframeRequestPending = false
                            }
                            if (!waitingForKeyframe) enqueue(packet)
                        }
                    }
                    lock.notifyAll()
                }
            }
        } catch (_: IOException) {
        } finally {
            finish()
        }
    }

    /** Queue mutations and accounting are protected by [lock]. */
    private fun enqueue(packet: Packet) {
        packets.addLast(packet)
        queuedBytes += packet.data.size
    }

    /** Called once after the real stream opens; a late stream still runs its own cleanup. */
    fun forward(send: (Packet) -> Boolean, end: () -> Unit) {
        try {
            synchronized(lock) {
                check(!attached)
                attached = true
            }
            while (current()) {
                var askKeyframe = false
                val packet = synchronized(lock) {
                    while (!closed && packets.isEmpty() && !keyframeRequestPending) lock.wait()
                    if (closed) return
                    if (keyframeRequestPending) {
                        keyframeRequestPending = false
                        askKeyframe = true
                        null
                    } else {
                        packets.removeFirst().also { queuedBytes -= it.data.size }
                    }
                }
                if (!current()) return
                // A helper command may block; the reader must remain free to observe EOF.
                if (askKeyframe) requestKeyframe() else if (!send(checkNotNull(packet))) return
            }
        } catch (_: InterruptedException) {
            Thread.currentThread().interrupt()
        } finally {
            try { end() } finally { finish() }
        }
    }

    /** No callback runs under the queue lock or a Window/map monitor. */
    fun finish() {
        val first = synchronized(lock) {
            if (closed) false else {
                closed = true
                packets.clear()
                queuedBytes = 0
                config = null
                keyframeRequestPending = false
                lock.notifyAll()
                true
            }
        }
        if (first) {
            try { onEnded() } finally { runCatching { socket.close() } }
        }
    }

    companion object {
        private const val KIND_CONFIG = 0
        private const val KIND_FRAME = 1
        private const val KIND_KEYFRAME = 2
        private const val MAX_PACKET = 8 shl 20
        private const val MAX_QUEUED_PACKETS = 4
        private const val MAX_QUEUED_BYTES = 16 shl 20
    }
}
