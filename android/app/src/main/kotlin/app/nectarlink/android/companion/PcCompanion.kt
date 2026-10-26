// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.companion

import android.app.Activity
import android.bluetooth.le.ScanFilter
import android.companion.AssociationInfo
import android.companion.AssociationRequest
import android.companion.BluetoothLeDeviceFilter
import android.companion.CompanionDeviceManager
import android.content.Context
import android.content.IntentSender
import android.os.Build
import java.util.concurrent.Executor

/**
 * Companion Device Manager (CDM) integration for waking NectarLink when a
 * paired PC's BLE proximity beacon (`NCTR\x01` + 8-byte device ID prefix) is
 * nearby, and granting OS-level background execution / foreground-service start
 * exemptions while associated.
 */
object PcCompanion {
    const val MANUFACTURER_ID: Int = 0xFFFF

    val BEACON_MAGIC: ByteArray = byteArrayOf(
        'N'.code.toByte(),
        'C'.code.toByte(),
        'T'.code.toByte(),
        'R'.code.toByte(),
        1,
    )

    /** z-base-32, as device IDs are written (`core/nectarlink-protocol/src/id.rs`). */
    private const val ZBASE32 = "ybndrfg8ejkmcpqxot1uwisza345h769"

    /** Decodes canonical z-base-32 bytes, accepting the protocol's trimmed, case-insensitive input. */
    fun decodeZBase32(text: String): ByteArray? {
        val out = java.io.ByteArrayOutputStream()
        var buffer = 0
        var bits = 0
        for (c in text.trim()) {
            val normalized = if (c in 'A'..'Z') c.lowercaseChar() else c
            val value = ZBASE32.indexOf(normalized)
            if (value < 0) return null
            buffer = (buffer shl 5) or value
            bits += 5
            if (bits >= 8) {
                bits -= 8
                out.write((buffer shr bits) and 0xFF)
            }
        }
        // A canonical encoding has fewer than 5 unused bits, all zero.
        if (bits >= 5 || (buffer and ((1 shl bits) - 1)) != 0) return null
        return out.toByteArray()
    }

    /**
     * Builds the manufacturer-specific BLE beacon payload: the 5-byte
     * `NCTR\x01` magic followed by the first 8 bytes of the PC's 32-byte
     * z-base-32 device ID, or just the magic when [deviceIdHex] is invalid.
     * The parameter retains its legacy name for source compatibility.
     * A magic-only fallback broadens discovery; it does not prove identity or grant access.
     */
    fun buildBeaconPayload(deviceIdHex: String?): ByteArray {
        val id = deviceIdHex?.let(::decodeZBase32)
        if (id == null || id.size != 32) {
            return BEACON_MAGIC.copyOf()
        }
        return BEACON_MAGIC + id.copyOfRange(0, 8)
    }

    /**
     * Builds the scan filter mask matching [buildBeaconPayload]: 13 `0xFF`
     * bytes when a specific 8-byte device ID prefix is present, or 5 `0xFF`
     * bytes to match any NectarLink PC beacon.
     */
    fun buildBeaconMask(hasDeviceId: Boolean): ByteArray =
        ByteArray(if (hasDeviceId) 13 else 5) { 0xFF.toByte() }

    fun isAssociated(context: Context): Boolean = runCatching {
        val cdm = context.getSystemService(CompanionDeviceManager::class.java) ?: return false
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
            cdm.myAssociations.isNotEmpty()
        } else {
            @Suppress("DEPRECATION")
            cdm.associations.isNotEmpty()
        }
    }.getOrDefault(false)

    fun startObservingPresence(context: Context) {
        runCatching {
            val cdm = context.getSystemService(CompanionDeviceManager::class.java) ?: return
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
                for (info in cdm.myAssociations) {
                    runCatching {
                        val addr = info.deviceMacAddress?.toString()
                        if (!addr.isNullOrBlank()) {
                            @Suppress("DEPRECATION")
                            cdm.startObservingDevicePresence(addr)
                        }
                    }
                }
            } else if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.S) {
                @Suppress("DEPRECATION")
                for (addr in cdm.associations) {
                    runCatching {
                        cdm.startObservingDevicePresence(addr)
                    }
                }
            }
        }
    }

    fun requestAssociation(
        activity: Activity,
        pcName: String,
        pcDeviceIdHex: String? = null,
        onResult: (Boolean) -> Unit = {},
        onLauncherFallback: (IntentSender) -> Unit,
    ) {
        runCatching {
            val cdm = activity.getSystemService(CompanionDeviceManager::class.java) ?: run {
                onResult(false)
                return
            }
            val payload = buildBeaconPayload(pcDeviceIdHex)
            val mask = buildBeaconMask(payload.size == 13)
            val scanFilter = ScanFilter.Builder()
                .setManufacturerData(MANUFACTURER_ID, payload, mask)
                .build()
            val leFilter = BluetoothLeDeviceFilter.Builder()
                .setScanFilter(scanFilter)
                .build()
            val requestBuilder = AssociationRequest.Builder()
                .addDeviceFilter(leFilter)
                .setSingleDevice(false)
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU && pcName.isNotBlank()) {
                runCatching { requestBuilder.setDisplayName(pcName) }
            }
            val request = requestBuilder.build()
            val mainExecutor = Executor { runnable -> activity.runOnUiThread(runnable) }
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
                cdm.associate(
                    request,
                    mainExecutor,
                    object : CompanionDeviceManager.Callback() {
                        override fun onAssociationPending(intentSender: IntentSender) {
                            onLauncherFallback(intentSender)
                        }

                        override fun onAssociationCreated(associationInfo: AssociationInfo) {
                            startObservingPresence(activity)
                            onResult(true)
                        }

                        override fun onFailure(error: CharSequence?) {
                            onResult(false)
                        }
                    },
                )
            } else {
                cdm.associate(
                    request,
                    object : CompanionDeviceManager.Callback() {
                        @Deprecated("Deprecated in Java")
                        override fun onDeviceFound(chooserLauncher: IntentSender) {
                            onLauncherFallback(chooserLauncher)
                        }

                        override fun onFailure(error: CharSequence?) {
                            onResult(false)
                        }
                    },
                    null,
                )
            }
        }.onFailure {
            onResult(false)
        }
    }
}
