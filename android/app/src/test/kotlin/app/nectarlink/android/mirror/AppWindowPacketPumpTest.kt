// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.mirror

import java.io.DataOutputStream
import java.net.InetAddress
import java.net.ServerSocket
import java.net.Socket
import java.util.Collections
import java.util.concurrent.CountDownLatch
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicInteger
import org.junit.Assert.*
import org.junit.Test

class AppWindowPacketPumpTest {
    private fun pair(): Pair<Socket, Socket> = ServerSocket(0, 1, InetAddress.getLoopbackAddress()).use { server ->
        val client = Socket(InetAddress.getLoopbackAddress(), server.localPort)
        client to server.accept()
    }

    private fun packet(out: DataOutputStream, kind: Int, bytes: ByteArray) {
        out.writeInt(9 + bytes.size)
        out.writeByte(kind)
        out.writeLong(17)
        out.write(bytes)
        out.flush()
    }

    @Test fun configAndKeyframeAreDeliveredInOrderAndEofCleansUp() {
        val (client, helper) = pair()
        val received = Collections.synchronizedList(mutableListOf<Int>())
        val delivered = CountDownLatch(2)
        val retired = CountDownLatch(1)
        val ended = CountDownLatch(1)
        val retireCount = AtomicInteger()
        val pump = AppWindowPacketPump(client, { true }) {
            retireCount.incrementAndGet()
            retired.countDown()
        }
        val consumer = Thread {
            pump.forward({ received.add(it.kind); delivered.countDown(); true }, { ended.countDown() })
        }
        try {
            pump.start()
            consumer.start()
            val output = DataOutputStream(helper.getOutputStream())
            packet(output, 0, java.nio.ByteBuffer.allocate(12).putInt(640).putInt(480).putInt(8).array())
            packet(output, 2, byteArrayOf(1, 2, 3))
            assertTrue(delivered.await(2, TimeUnit.SECONDS))
            assertEquals(listOf(0, 2), received.toList())
            helper.close()
            assertTrue(retired.await(2, TimeUnit.SECONDS))
            assertTrue(ended.await(2, TimeUnit.SECONDS))
            pump.finish()
            assertEquals(1, retireCount.get())
        } finally {
            pump.finish()
            helper.close()
            client.close()
            consumer.join(2_000)
        }
    }

    @Test fun eofRetiresWhileOpeningIsHeldAndLateStreamEndsWithoutTouchingReplacement() {
        val (client, helper) = pair()
        val gate = CountDownLatch(1)
        val retired = CountDownLatch(1)
        val ended = CountDownLatch(1)
        val retireCount = AtomicInteger()
        val sends = AtomicInteger()
        // Adapter for exact tuple comparison: owner 1 cannot retire replacement 2.
        val retainedOwner = 1
        val activeOwner = AtomicInteger(2)
        val pump = AppWindowPacketPump(client, { true }) {
            retireCount.incrementAndGet()
            activeOwner.compareAndSet(retainedOwner, 0)
            retired.countDown()
        }
        val opening = Thread {
            gate.await()
            pump.forward({ sends.incrementAndGet(); true }, { ended.countDown() })
        }
        try {
            pump.start()
            opening.start()
            packet(DataOutputStream(helper.getOutputStream()), 0,
                java.nio.ByteBuffer.allocate(12).putInt(640).putInt(480).putInt(8).array())
            helper.close()
            assertTrue(retired.await(2, TimeUnit.SECONDS))
            assertEquals(1L, gate.count)
            assertEquals(0, sends.get())
            assertEquals(2, activeOwner.get())
            gate.countDown()
            assertTrue(ended.await(2, TimeUnit.SECONDS))
            pump.finish()
            assertEquals(1, retireCount.get())
            assertEquals(0, sends.get())
            assertEquals(2, activeOwner.get())
        } finally {
            gate.countDown()
            pump.finish()
            helper.close()
            client.close()
            opening.join(2_000)
        }
    }
}
