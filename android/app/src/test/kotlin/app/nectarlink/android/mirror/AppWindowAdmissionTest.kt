// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.mirror

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotSame
import org.junit.Assert.assertSame
import org.junit.Assert.assertTrue
import org.junit.Assert.fail
import org.junit.Test
import java.util.concurrent.atomic.AtomicReference

class AppWindowAdmissionTest {
    @Test
    fun bindsOnCallbackEntryAndCompletesFalseOnWorkerThread() {
        val callbackThread = Thread.currentThread()
        val bound = Any()
        val bindThread = AtomicReference<Thread>()
        val observedBinding = AtomicReference<Any>()
        val workerThread = AtomicReference<Thread>()
        var completed = false

        val admitted = appWindowAdmission({
            bindThread.set(Thread.currentThread())
            bound
        }) { selected ->
            observedBinding.set(selected)
            workerThread.set(Thread.currentThread())
            completed = true
            false
        }

        assertSame(callbackThread, bindThread.get())
        assertSame(bound, observedBinding.get())
        assertNotSame(callbackThread, workerThread.get())
        assertTrue(completed)
        assertFalse(admitted)
    }

    @Test
    fun interruptedCallbackStillWaitsForWorkerTerminalResult() {
        val workerStarted = java.util.concurrent.CountDownLatch(1)
        val allowFinish = java.util.concurrent.CountDownLatch(1)
        var finished = false
        var workerFailure: Throwable? = null

        val caller = Thread {
            try {
                appWindowAdmission({ Unit }) {
                    workerStarted.countDown()
                    allowFinish.await()
                    finished = true
                    false
                }
            } catch (error: Throwable) {
                workerFailure = error
            }
        }
        caller.start()
        try {
            assertTrue(workerStarted.await(2, java.util.concurrent.TimeUnit.SECONDS))
            caller.interrupt()
            Thread.sleep(100)
            assertTrue("callback must wait for the worker", caller.isAlive)
            assertFalse("worker must finish before callback returns", finished)
        } finally {
            allowFinish.countDown()
            caller.join(2_000)
        }
        assertTrue(finished)
        assertTrue("caller interrupt state is restored", caller.isInterrupted)
        assertFalse("interrupt-to-cancel must not strand an admission", caller.isAlive)
        assertEquals(null, workerFailure)
    }
}
