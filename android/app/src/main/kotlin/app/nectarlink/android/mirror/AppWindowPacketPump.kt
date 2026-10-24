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
    private val onEnded: () -> Unit,
) {
    internal data class Packet(val kind: Int, val time: ULong, val data: ByteArray)
    private val lock = Object()
    private val packets = ArrayDeque<Packet>()
    private var queuedBytes = 0
    private var closed = false
    private var attached = false

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
                if (kind == 0 && data.size != 12) return
                if (!current()) return
                synchronized(lock) {
                    if (closed || packets.size >= MAX_QUEUED_PACKETS || queuedBytes + data.size > MAX_QUEUED_BYTES) return
                    packets.addLast(Packet(kind, time, data))
                    queuedBytes += data.size
                    lock.notifyAll()
                }
            }
        } catch (_: IOException) {
        } finally {
            finish()
        }
    }

    /** Called once after the real stream opens; a late stream still runs its own cleanup. */
    fun forward(send: (Packet) -> Boolean, end: () -> Unit) {
        try {
            synchronized(lock) {
                check(!attached)
                attached = true
            }
            while (current()) {
                val packet = synchronized(lock) {
                    while (!closed && packets.isEmpty()) lock.wait()
                    if (closed) return
                    packets.removeFirst().also { queuedBytes -= it.data.size }
                }
                if (!current() || !send(packet)) return
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
                lock.notifyAll()
                true
            }
        }
        if (first) {
            try { onEnded() } finally { runCatching { socket.close() } }
        }
    }

    companion object {
        private const val MAX_PACKET = 8 shl 20
        private const val MAX_QUEUED_PACKETS = 4
        private const val MAX_QUEUED_BYTES = 16 shl 20
    }
}
