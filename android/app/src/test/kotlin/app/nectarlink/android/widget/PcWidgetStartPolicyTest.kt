// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.widget

import app.nectarlink.android.core.CoreStatus
import app.nectarlink.android.core.Device
import app.nectarlink.core.DeviceKind
import app.nectarlink.core.Link
import org.junit.Assert.assertEquals
import org.junit.Test

/**
 * Runs the same [startForWidget] that onUpdate and ActionReceiver call, and
 * records which of the core start and the app's delayed stop it invokes, in order.
 */
class PcWidgetStartPolicyTest {
    private fun calls(backgroundEnabled: Boolean, explicitUserAction: Boolean): List<String> {
        val calls = mutableListOf<String>()
        startForWidget(
            backgroundEnabled = backgroundEnabled,
            explicitUserAction = explicitUserAction,
            start = { calls += "start" },
            stopWhenIdle = { calls += "stopWhenIdle" },
        )
        return calls
    }

    @Test
    fun `background on starts the node for refreshes and taps and never stops it`() {
        assertEquals(listOf("start"), calls(backgroundEnabled = true, explicitUserAction = false))
        assertEquals(listOf("start"), calls(backgroundEnabled = true, explicitUserAction = true))
    }

    @Test
    fun `background off never starts on a silent refresh and stops a tap's node once idle`() {
        assertEquals(emptyList<String>(), calls(backgroundEnabled = false, explicitUserAction = false))
        // Before this change a tap only started the node, which then ran until the app was opened and closed.
        assertEquals(listOf("start", "stopWhenIdle"), calls(backgroundEnabled = false, explicitUserAction = true))
    }

    private fun pc(link: Link) = Device(
        id = "pc",
        name = "Mac",
        kind = DeviceKind.DESKTOP,
        model = null,
        pairedAt = 1L,
        link = link,
    )

    private val online = pc(Link.Online(relayed = false, usb = false, rttMs = 5u))

    /** Runs the same [sendIfConnected] that Ring and Lock call, and records which outcome it picks. */
    private fun tap(status: CoreStatus, pc: Device?): List<String> {
        val calls = mutableListOf<String>()
        sendIfConnected(status, pc, notConnected = { calls += "notConnected" }, send = { calls += "send" })
        return calls
    }

    @Test
    fun `a ready node sends to an online PC and stays silent for an offline one`() {
        assertEquals(listOf("send"), tap(CoreStatus.Ready("phone"), online))
        assertEquals(emptyList<String>(), tap(CoreStatus.Ready("phone"), pc(Link.Offline(null))))
    }

    @Test
    fun `a tap on a node that isn't ready says not connected instead of sending`() {
        // Before this change a tap on a just-started node sent, which failed, and still toasted success.
        assertEquals(listOf("notConnected"), tap(CoreStatus.Starting, null))
        assertEquals(listOf("notConnected"), tap(CoreStatus.Starting, online))
        assertEquals(listOf("notConnected"), tap(CoreStatus.Failed("no keystore"), online))
    }
}
