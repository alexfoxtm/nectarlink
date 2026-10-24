// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.mirror

import java.io.DataOutputStream
import java.net.InetAddress
import java.net.ServerSocket
import java.net.Socket
import java.nio.ByteBuffer
import java.util.Collections
import java.util.concurrent.CountDownLatch
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicBoolean
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


    @Test fun preattachOverflowKeepsConfigAndResyncsAtFreshKeyframe() {
        val (client, helper) = pair()
        val readerChecks = AtomicInteger()
        val burstRead = CountDownLatch(1)
        val freshKeyframeQueued = CountDownLatch(1)
        val requestEntered = CountDownLatch(1)
        val requestOnForwardThread = AtomicBoolean()
        val requestOutsideQueueLock = AtomicBoolean()
        val requestCount = AtomicInteger()
        val received = Collections.synchronizedList(mutableListOf<Int>())
        val delivered = CountDownLatch(3)
        val retired = AtomicInteger()
        val retiredAtEof = CountDownLatch(1)
        val streamEnded = CountDownLatch(1)
        val pump = AppWindowPacketPump(
            client,
            current = {
                if (Thread.currentThread().name == "app-window-reader") {
                    when (readerChecks.incrementAndGet()) {
                        13 -> burstRead.countDown()
                        15 -> freshKeyframeQueued.countDown()
                    }
                }
                true
            },
            requestKeyframe = {
                requestCount.incrementAndGet()
                requestOnForwardThread.set(Thread.currentThread().name == "startup-regression-forward")
                requestEntered.countDown()
                // The reader must be able to enqueue the new keyframe while this callback waits.
                requestOutsideQueueLock.set(freshKeyframeQueued.await(2, TimeUnit.SECONDS))
            },
            onEnded = { retired.incrementAndGet(); retiredAtEof.countDown() },
        )
        val output = DataOutputStream(helper.getOutputStream())
        val forwarder = Thread({
            pump.forward({ frame ->
                received.add(frame.kind)
                delivered.countDown()
                true
            }, { streamEnded.countDown() })
        }, "startup-regression-forward")

        try {
            pump.start()
            packet(output, 0, ByteBuffer.allocate(12).putInt(640).putInt(480).putInt(8).array())
            packet(output, 2, byteArrayOf(10)) // Establish a chain that can overflow.
            repeat(4) { packet(output, 1, byteArrayOf(it.toByte())) }

            // 13 is the reader's next-loop predicate after six packets were fully enqueued.
            assertTrue("reader did not consume the preattach burst", burstRead.await(2, TimeUnit.SECONDS))
            assertEquals("a healthy helper burst must not retire the capture", 0, retired.get())

            forwarder.start()
            assertTrue("overflow must request a keyframe once attached", requestEntered.await(2, TimeUnit.SECONDS))
            packet(output, 2, byteArrayOf(20, 21)) // Fresh decodable keyframe after overflow.
            packet(output, 1, byteArrayOf(22)) // Delta is forwarded only after that keyframe.

            assertTrue("configuration, fresh keyframe, and its delta must be delivered", delivered.await(2, TimeUnit.SECONDS))
            assertTrue("request callback must run on the forwarding thread", requestOnForwardThread.get())
            assertTrue("reader must enqueue while the request callback is running", requestOutsideQueueLock.get())
            assertEquals("overflow requests one keyframe", 1, requestCount.get())
            assertEquals(listOf(0, 2, 1), received.toList())
            assertEquals("healthy resynchronization must keep the capture alive", 0, retired.get())

            helper.close()
            assertTrue("reader did not retire after EOF", retiredAtEof.await(2, TimeUnit.SECONDS))
            assertTrue("attached stream must end after EOF", streamEnded.await(2, TimeUnit.SECONDS))
            pump.finish()
            assertEquals(1, retired.get())
        } finally {
            pump.finish()
            helper.close()
            client.close()
            forwarder.join(2_000)
        }
    }

    @Test fun eofRetiresWhileOpeningIsHeldAndLateStreamEndsWithoutTouchingReplacement() {
        val (client, helper) = pair()
        val gate = CountDownLatch(1)
        val retired = CountDownLatch(1)
        val ended = CountDownLatch(1)
        val retireCount = AtomicInteger()
        val sends = AtomicInteger()
        val endCount = AtomicInteger()
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
            pump.forward({ sends.incrementAndGet(); true }, { endCount.incrementAndGet(); ended.countDown() })
        }
        try {
            pump.start()
            opening.start()
            packet(DataOutputStream(helper.getOutputStream()), 0,
                java.nio.ByteBuffer.allocate(12).putInt(640).putInt(480).putInt(8).array())
            helper.close()
            assertTrue(retired.await(2, TimeUnit.SECONDS))
            assertEquals("EOF retirement must be immediate and exactly once", 1, retireCount.get())
            assertEquals(1L, gate.count)
            assertEquals(0, sends.get())
            assertEquals(2, activeOwner.get())
            gate.countDown()
            assertTrue(ended.await(2, TimeUnit.SECONDS))
            pump.finish()
            assertEquals(1, retireCount.get())
            assertEquals(0, sends.get())
            assertEquals("late stream end must run once", 1, endCount.get())
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
