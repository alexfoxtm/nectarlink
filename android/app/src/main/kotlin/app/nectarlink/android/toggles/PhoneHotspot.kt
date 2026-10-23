// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.toggles

import android.content.Context
import android.net.TetheringInterface
import android.net.TetheringManager
import android.os.Build
import androidx.annotation.RequiresApi
import app.nectarlink.core.PhoneHotspotOutcome
import app.nectarlink.android.elevated.Elevated
import app.nectarlink.android.elevated.ShellHotspot
import java.util.concurrent.TimeUnit

/** Capability and verified Wi-Fi Internet-tether state, distinct from helper
 * liveness and local-only Soft AP state. This app-side adapter reads no
 * credentials; probing never enables tethering. Denied/unavailable APIs fail
 * closed. The shell-side adapter separately checks real system authority.
 * A tethered interface means the Internet-tether service is active; it does not
 * establish that the carrier currently supplies working Internet connectivity.
 */
internal class PhoneHotspot(context: Context, private val changed: () -> Unit) {
    private val context = context.applicationContext
    private val monitor = Object()
    private val probing = java.util.concurrent.atomic.AtomicBoolean(false)
    @Volatile private var registered = false
    @Volatile private var known = false
    @Volatile private var tethered = false
    @Volatile private var helperVerified = false
    @Volatile private var autoVerified = false
    @Volatile private var generation = 0L
    private var callback: Any? = null

    fun start() {
        if (Build.VERSION.SDK_INT < 36 || registered) return
        register()
        refreshCapability()
    }

    fun refreshCapability() {
        if (!registered || !probing.compareAndSet(false, true)) return
        val owner = generation
        Thread {
            try {
                val auto = Elevated.hotspotAutoCapability() == ShellHotspot.Result.CAPABILITY_VERIFIED
                val verified = Elevated.hotspotCapability() == ShellHotspot.Result.CAPABILITY_VERIFIED
                if (generation == owner && registered && (helperVerified != verified || autoVerified != auto)) {
                    helperVerified = verified
                    autoVerified = auto
                    changed()
                }
            } finally { probing.set(false) }
        }.apply { isDaemon = true; start() }
    }

    @RequiresApi(36)
    private fun register() {
        val manager = context.getSystemService(TetheringManager::class.java) ?: return
        val owner = ++generation
        val events = object : TetheringManager.TetheringEventCallback {
            override fun onTetheredInterfacesChanged(interfaces: MutableSet<TetheringInterface>) {
                if (owner != generation || !registered) return
                synchronized(monitor) {
                    tethered = interfaces.any { it.type == TetheringManager.TETHERING_WIFI }
                    known = true
                    monitor.notifyAll()
                }
                changed()
            }
        }
        registered = true
        callback = events
        runCatching { manager.registerTetheringEventCallback(context.mainExecutor, events) }
            .onFailure { registered = false; callback = null; known = false }
    }

    fun stop() {
        ++generation
        registered = false
        if (Build.VERSION.SDK_INT >= 36) {
            (callback as? TetheringManager.TetheringEventCallback)?.let { events ->
                runCatching { context.getSystemService(TetheringManager::class.java)?.unregisterTetheringEventCallback(events) }
            }
        }
        callback = null
        helperVerified = false
        autoVerified = false
        synchronized(monitor) { known = false; monitor.notifyAll() }
    }

    fun canEnableOrJoin(): Boolean = registered && known && helperVerified && Elevated.running

    fun canAutoJoin(): Boolean = canEnableOrJoin() && autoVerified

    fun prepareAuto(peer: String, connection: ULong, request: ULong, stillCurrent: () -> Boolean): app.nectarlink.core.PhoneHotspotAutoOutcome {
        fun outcome(state: PhoneHotspotOutcome) = app.nectarlink.core.PhoneHotspotAutoOutcome(state, null)
        if (!stillCurrent()) return outcome(PhoneHotspotOutcome.CANCELLED)
        if (!canAutoJoin()) return outcome(PhoneHotspotOutcome.NEEDS_SETTINGS)
        val owner = generation
        val reply = Elevated.prepareHotspotAutoOwned(peer, connection, request, stillCurrent)
        if (!stillCurrent() || !registered || generation != owner) return outcome(PhoneHotspotOutcome.CANCELLED)
        if (reply.state != ShellHotspot.Result.ACTIVE) return outcome(when (reply.state) {
            ShellHotspot.Result.CONFIG_CHANGED -> PhoneHotspotOutcome.CONFIG_CHANGED
            ShellHotspot.Result.CANCELLED, ShellHotspot.Result.ACTIVATION_SUBMITTED -> PhoneHotspotOutcome.CANCELLED
            ShellHotspot.Result.UNSUPPORTED, ShellHotspot.Result.DENIED -> PhoneHotspotOutcome.NEEDS_SETTINGS
            else -> PhoneHotspotOutcome.FAILED
        })
        val network = reply.network ?: return outcome(PhoneHotspotOutcome.FAILED)
        if (!stillCurrent()) return outcome(PhoneHotspotOutcome.CANCELLED)
        return app.nectarlink.core.PhoneHotspotAutoOutcome(PhoneHotspotOutcome.ACTIVE,
            app.nectarlink.core.PhoneHotspotNetwork(network.ssid, network.passphrase,
                if (network.security == "WPA2") app.nectarlink.core.PhoneHotspotSecurity.WPA2 else app.nectarlink.core.PhoneHotspotSecurity.WPA3))
    }

    /** Called on the blocking platform executor. An active Internet-tether
     * interface is required; carrier Internet reachability is not claimed.
     * The existing configuration is retained. Cancelling the join never turns
     * the phone hotspot off, including after our start was submitted.
     */
    fun prepare(peer: String, connection: ULong, request: ULong, network: String, stillCurrent: () -> Boolean): app.nectarlink.core.PhoneHotspotOutcome {
        if (!stillCurrent()) return PhoneHotspotOutcome.CANCELLED
        if (!registered || !known) return PhoneHotspotOutcome.NEEDS_SETTINGS
        val owner = generation
        if (!helperVerified) return PhoneHotspotOutcome.NEEDS_SETTINGS
        val submitted = Elevated.prepareHotspotOwned(peer, connection, request, network, stillCurrent)
        if (!stillCurrent()) return PhoneHotspotOutcome.CANCELLED
        if (submitted == ShellHotspot.Result.CONFIG_CHANGED) return PhoneHotspotOutcome.CONFIG_CHANGED
        if (submitted == ShellHotspot.Result.CANCELLED || submitted == ShellHotspot.Result.ACTIVATION_SUBMITTED) return PhoneHotspotOutcome.CANCELLED
        if (submitted != ShellHotspot.Result.ACTIVE) {
            helperVerified = false
            changed()
            return if (submitted == ShellHotspot.Result.UNSUPPORTED || submitted == ShellHotspot.Result.DENIED) PhoneHotspotOutcome.NEEDS_SETTINGS else PhoneHotspotOutcome.FAILED
        }
        val until = System.nanoTime() + TimeUnit.SECONDS.toNanos(2)
        synchronized(monitor) {
            while (registered && known && generation == owner && !tethered && stillCurrent()) {
                val left = until - System.nanoTime()
                if (left <= 0) return PhoneHotspotOutcome.FAILED
                try { TimeUnit.NANOSECONDS.timedWait(monitor, minOf(left, TimeUnit.MILLISECONDS.toNanos(50))) }
                catch (_: InterruptedException) { Thread.currentThread().interrupt(); return PhoneHotspotOutcome.CANCELLED }
            }
            return if (!stillCurrent() || !registered || generation != owner) PhoneHotspotOutcome.CANCELLED
                else if (known && tethered) PhoneHotspotOutcome.ACTIVE else PhoneHotspotOutcome.FAILED
        }
    }
    fun cancel(peer: String, connection: ULong, request: ULong, stillCurrent: () -> Boolean) {
        if (stillCurrent()) Elevated.cancelHotspotOwned(peer, connection, request, stillCurrent)
    }

}
