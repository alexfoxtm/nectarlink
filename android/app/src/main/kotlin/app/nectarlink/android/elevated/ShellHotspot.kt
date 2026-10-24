// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.elevated

import android.content.Context
import android.content.pm.PackageManager
import android.net.TetheringInterface
import android.net.TetheringManager
import android.net.wifi.SoftApConfiguration
import android.net.wifi.WifiManager
import android.os.Build
import android.os.Parcel
import android.os.Process
import androidx.annotation.RequiresApi
import java.security.MessageDigest
import java.security.SecureRandom
import java.util.concurrent.Executors
import java.util.concurrent.TimeUnit
import javax.crypto.Mac
import javax.crypto.spec.SecretKeySpec

/** Independently implemented using Android APIs. Runs only in Waypair's
 * existing, explicitly approved UID-2000 helper. No new bootstrap/grant/root
 * route, credential command line, configuration setter, or raw diagnostics.
 */
internal object ShellHotspot {
    enum class Result { CAPABILITY_VERIFIED, ACTIVE, ACTIVATION_SUBMITTED, UNSUPPORTED, DENIED, CONFIG_CHANGED, CANCELLED, FAILED }
    sealed interface RequestOwner
    data class Owner(val peer: String, val connection: ULong, val request: ULong) : RequestOwner {
        fun valid(): Boolean = peer.matches(Regex("[a-z0-9]{52}")) && connection != 0uL && request != 0uL
    }

    data class LocalOwner(val activity: String, val request: Long, val deadline: Long) : RequestOwner {
        fun valid(now: Long = android.os.SystemClock.elapsedRealtime()): Boolean = activity.matches(Regex("[a-f0-9]{64}")) && request > 0 &&
            now >= 0 && deadline > now && deadline - now <= 40000
    }
    private fun current(owner: RequestOwner): Boolean = owner !is LocalOwner || owner.valid()
    fun localProbe(): Result {
        val capability = probe()
        if (capability != Result.CAPABILITY_VERIFIED) return capability
        return autoProbe()
    }
    fun localTicket(owner: LocalOwner): Ticket? = if (Build.VERSION.SDK_INT >= 36 && owner.valid()) service()?.ticket(owner, null) else null
    fun localCancel(owner: LocalOwner): Ticket? = if (Build.VERSION.SDK_INT >= 36 && owner.activity.matches(Regex("[a-f0-9]{64}")) && owner.request > 0 && Process.myUid() == 2000) service?.cancellation(owner) else null

    @Volatile private var service: Service? = null

    private fun permitted(context: Context): Boolean {
        fun granted(permission: String) = context.checkPermission(permission, Process.myPid(), Process.myUid()) == PackageManager.PERMISSION_GRANTED
        return Process.myUid() == 2000 && granted("android.permission.TETHER_PRIVILEGED") &&
            (granted("android.permission.NETWORK_SETTINGS") || granted("android.permission.OVERRIDE_WIFI_CONFIG")) &&
            granted("android.permission.ACCESS_NETWORK_STATE")
    }

    private fun service(): Service? {
        if (Build.VERSION.SDK_INT < 36 || Process.myUid() != 2000) return null
        val context = runCatching { ShellContext.get() }.getOrNull() ?: return null
        if (!permitted(context)) return null
        return service ?: synchronized(this) {
            service ?: runCatching { Service(context) }.getOrNull()?.also { service = it }
        }
    }

    fun probe(): Result {
        if (Build.VERSION.SDK_INT < 36 || Process.myUid() != 2000) return Result.UNSUPPORTED
        val context = runCatching { ShellContext.get() }.getOrNull() ?: return Result.UNSUPPORTED
        if (!permitted(context)) return Result.DENIED
        return service()?.probe() ?: Result.UNSUPPORTED
    }
    fun ticket(owner: Owner, network: String): Ticket? {
        if (Build.VERSION.SDK_INT < 36) return null
        if (!owner.valid() || network.toByteArray(Charsets.UTF_8).size !in 1..32 || network.contains('\u0000')) return null
        return service()?.ticket(owner, network)
    }
    internal class Network(val ssid: String, val passphrase: String, val security: String) {
        override fun toString(): String = "HotspotNetwork([redacted])"
    }
    fun autoProbe(): Result = if (Build.VERSION.SDK_INT >= 36) service()?.autoProbe() ?: Result.UNSUPPORTED else Result.UNSUPPORTED
    fun autoTicket(owner: Owner): Ticket? = if (owner.valid() && Build.VERSION.SDK_INT >= 36) service()?.ticket(owner, null) else null
    fun cancel(owner: Owner): Ticket? = if (owner.valid() && Process.myUid() == 2000 && Build.VERSION.SDK_INT >= 36) service?.cancellation(owner) else null

    /** A ticket owns only this preparation workflow, never Android's live
     * hotspot type. Its textual protocol returns fixed result words only.
     */
    internal class Ticket internal constructor(
        internal val run: () -> Result,
        internal val abandon: () -> Unit,
        internal val network: () -> Network? = { null },
    )

    @RequiresApi(36)
    @android.annotation.SuppressLint("MissingPermission") // dynamic shell UID/grants are checked before acquisition and each operation
    private class Service(private val context: Context) {
        private val monitor = Object()
        private val executor = Executors.newSingleThreadExecutor { task -> Thread(task, "waypair-hotspot-events").apply { isDaemon = true } }
        private val manager = context.getSystemService(TetheringManager::class.java)
        private val wifi = context.getSystemService(WifiManager::class.java)
        private val fingerprintKey = ByteArray(32).also(SecureRandom()::nextBytes)
        private var registered = false
        private var known = false
        private var interfaces = emptySet<String>()
        private var interfaceGeneration = 0L
        private var interfaceFingerprints = emptyMap<String, ByteArray?>()
        private var pending: Operation? = null
        private val cancelled = LinkedHashSet<RequestOwner>()

        private class Operation(
            val owner: RequestOwner,
            val fingerprint: ByteArray,
            val initiallyActive: Boolean,
            val initialInterfaces: Set<String>,
            val initialGeneration: Long,
            val request: TetheringManager.TetheringRequest,
        ) {
            var cancelled = false
            var submitted = false
            var started = false
            var failed = false
        }

        private fun permitted(): Boolean = ShellHotspot.permitted(context)

        /** Root explicitly authorized this one read-only SystemApi getter.
         * It is public at the JVM level, omitted from ordinary SDK stubs, and
         * independently permission-checked by the real Wi-Fi service. No
         * accessibility override, binder transaction, setter or bootstrap is
         * added. Exceptions and returned objects are never logged/stringified.
         */
        @android.annotation.SuppressLint("PrivateApi") // the single Root-authorized, real-permission-enforced read-only getter
        private fun configuration(): SoftApConfiguration? {
            if (!permitted()) return null
            val wifi = wifi ?: return null
            return runCatching {
                WifiManager::class.java.getMethod("getSoftApConfiguration").invoke(wifi) as? SoftApConfiguration
            }.getOrNull()
        }

        private fun fingerprint(configuration: SoftApConfiguration): ByteArray {
            val parcel = Parcel.obtain()
            var bytes: ByteArray? = null
            return try {
                configuration.writeToParcel(parcel, 0)
                bytes = parcel.marshall()
                Mac.getInstance("HmacSHA256").apply { init(SecretKeySpec(fingerprintKey, "HmacSHA256")) }.doFinal(bytes)
            } finally { bytes?.fill(0); parcel.recycle() }
        }

        private fun unchanged(operation: Operation): Boolean {
            val current = configuration() ?: return false
            return MessageDigest.isEqual(operation.fingerprint, fingerprint(current))
        }

        private val events = object : TetheringManager.TetheringEventCallback {
            override fun onTetheredInterfacesChanged(values: MutableSet<TetheringInterface>) {
                val wifiValues = values.filter { it.type == TetheringManager.TETHERING_WIFI }
                val actual = wifiValues.map { it.`interface` }.toSet()
                val fingerprints = wifiValues.associate { value -> value.`interface` to runCatching { value.softApConfiguration?.let(::fingerprint) }.getOrNull() }
                synchronized(monitor) {
                    val sameConfigurations = interfaceFingerprints.keys == fingerprints.keys && fingerprints.all { (name, value) ->
                        val old = interfaceFingerprints[name]
                        if (value == null) old == null else old != null && MessageDigest.isEqual(old, value)
                    }
                    if (!known || interfaces != actual || !sameConfigurations) interfaceGeneration++
                    known = true
                    interfaces = actual
                    interfaceFingerprints = fingerprints
                    monitor.notifyAll()
                }
            }
        }

        private fun observe(): Boolean {
            if (!permitted() || manager == null || wifi == null) return false
            synchronized(monitor) {
                if (!registered) {
                    registered = true
                    try { manager.registerTetheringEventCallback(executor, events) }
                    catch (_: Exception) { registered = false; return false }
                }
                val until = System.nanoTime() + TimeUnit.SECONDS.toNanos(2)
                while (!known) {
                    val left = until - System.nanoTime()
                    if (left <= 0) return false
                    try { TimeUnit.NANOSECONDS.timedWait(monitor, left) }
                    catch (_: InterruptedException) { Thread.currentThread().interrupt(); return false }
                }
                return true
            }
        }

        private fun readNetwork(configuration: SoftApConfiguration): Network? {
            val bytes = configuration.wifiSsid?.bytes ?: return null
            val ssid = runCatching {
                Charsets.UTF_8.newDecoder().decode(java.nio.ByteBuffer.wrap(bytes)).toString()
            }.getOrNull() ?: return null
            if (bytes.size !in 1..32 || ssid.any { it == '\u0000' || it == '\r' || it == '\n' }) return null
            val security = when (configuration.securityType) {
                SoftApConfiguration.SECURITY_TYPE_WPA2_PSK -> "WPA2"
                SoftApConfiguration.SECURITY_TYPE_WPA3_SAE -> "WPA3"
                else -> return null
            }
            val passphrase = configuration.passphrase ?: return null
            if (passphrase.length !in 8..63 || passphrase.any { it.code !in 32..126 }) return null
            return Network(ssid, passphrase, security)
        }

        fun autoProbe(): Result = if (probe() == Result.CAPABILITY_VERIFIED && configuration()?.let(::readNetwork) != null) Result.CAPABILITY_VERIFIED else Result.UNSUPPORTED

        fun probe(): Result {
            if (!permitted()) return Result.DENIED
            return if (observe() && configuration() != null) Result.CAPABILITY_VERIFIED else Result.UNSUPPORTED
        }

        fun ticket(owner: RequestOwner, network: String?): Ticket? {
            if (!current(owner)) return null
            if (probe() != Result.CAPABILITY_VERIFIED) return null
            val configuration = configuration() ?: return null
            if (network == null && readNetwork(configuration) == null) return null
            if (network != null && configuration.wifiSsid?.bytes?.contentEquals(network.toByteArray(Charsets.UTF_8)) != true) return null
            val fingerprint = fingerprint(configuration)
            val operation = synchronized(monitor) {
                if (!current(owner) || owner in cancelled || pending != null || !known) return null
                // Actual TETHER_PRIVILEGED authority permits the persistent
                // configuration request. The framework reads its own current
                // configuration atomically; we never supply/restore a stale
                // snapshot or change SSID/passphrase merely to enable it.
                val request = TetheringManager.TetheringRequest.Builder(TetheringManager.TETHERING_WIFI).build()
                Operation(owner, fingerprint, interfaces.isNotEmpty(), interfaces, interfaceGeneration, request)
                    .also { pending = it }
            }
            return Ticket(run = { start(operation) }, abandon = { retire(operation) }, network = {
                synchronized(monitor) {
                    if (operation.cancelled || operation.owner in cancelled || !permitted() || !known || !matchingTetherReadback(operation)) null
                    else configuration()?.takeIf { MessageDigest.isEqual(fingerprint(it), operation.fingerprint) }?.let(::readNetwork)
                }
            })
        }

        private fun retire(operation: Operation) {
            synchronized(monitor) { if (pending === operation) pending = null; monitor.notifyAll() }
        }

        // Matching readback permits joining only. It is not an OS-owned
        // handle and never grants authority to turn a live hotspot off.
        private fun matchingTetherReadback(operation: Operation): Boolean = interfaces.isNotEmpty() && interfaces.all { name ->
            // A default persistent-config request may expose no configuration
            // in TetheringInterface. Reject a known mismatch; otherwise require
            // the typed start callback, actual off-to-on interface transition,
            // and a fresh permission-checked configuration HMAC before success.
            interfaceFingerprints[name]?.let { MessageDigest.isEqual(it, operation.fingerprint) } != false
        }

        private fun start(operation: Operation): Result {
            try {
                if (!permitted() || !unchanged(operation)) return Result.CONFIG_CHANGED
                synchronized(monitor) {
                    if (!current(operation.owner) || pending !== operation || operation.cancelled || operation.owner in cancelled) return Result.CANCELLED
                    if (!known || interfaces != operation.initialInterfaces || interfaceGeneration != operation.initialGeneration) return Result.CONFIG_CHANGED
                    if (operation.initiallyActive) {
                        if (interfaceFingerprints.values.filterNotNull().any { !MessageDigest.isEqual(it, operation.fingerprint) }) return Result.CONFIG_CHANGED
                        return Result.ACTIVE // verified reuse confers no authority to turn the hotspot off
                    }
                    operation.submitted = true
                    try {
                        manager!!.startTethering(operation.request, executor, object : TetheringManager.StartTetheringCallback {
                            override fun onTetheringStarted() { synchronized(monitor) { operation.started = true; monitor.notifyAll() } }
                            override fun onTetheringFailed(error: Int) { synchronized(monitor) { operation.failed = true; monitor.notifyAll() } }
                        })
                    } catch (_: Exception) { operation.failed = true }
                }
                val until = System.nanoTime() + TimeUnit.SECONDS.toNanos(35)
                synchronized(monitor) {
                    while (current(operation.owner) && pending === operation && !operation.cancelled && !operation.failed && !(operation.started && matchingTetherReadback(operation))) {
                        val left = until - System.nanoTime()
                        if (left <= 0) return Result.FAILED
                        try { TimeUnit.NANOSECONDS.timedWait(monitor, minOf(left, TimeUnit.MILLISECONDS.toNanos(50))) }
                        catch (_: InterruptedException) { Thread.currentThread().interrupt(); return Result.CANCELLED }
                    }
                    if (!current(operation.owner) || operation.cancelled || operation.owner in cancelled) return if (operation.submitted) Result.ACTIVATION_SUBMITTED else Result.CANCELLED
                    if (operation.failed || !operation.started || !matchingTetherReadback(operation)) return Result.FAILED
                    // Callback alone is insufficient: configuration may have
                    // changed while start/provisioning was asynchronous.
                    if (!permitted() || !unchanged(operation)) return Result.CONFIG_CHANGED
                    return Result.ACTIVE
                }
            } finally { retire(operation) }
        }

        fun cancellation(owner: RequestOwner): Ticket? {
            val submitted = synchronized(monitor) {
                // Retire this logical preparation only. Configuration and
                // interface observations cannot prove OS ownership against
                // another privileged caller, so cancellation never invokes an
                // Android tether stop after a start has been submitted.
                cancelled += owner
                while (cancelled.size > 256) cancelled.remove(cancelled.first())
                val operation = pending?.takeIf { it.owner == owner }
                operation?.cancelled = true
                monitor.notifyAll()
                operation?.submitted == true
            }
            return Ticket(
                run = { if (submitted) Result.ACTIVATION_SUBMITTED else Result.CANCELLED },
                abandon = {},
            )
        }
    }
}
