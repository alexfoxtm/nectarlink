// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.elevated

import android.content.Context
import android.content.pm.PackageManager
import android.media.AudioFormat
import android.media.AudioManager
import android.os.Build
import android.os.Process
import java.lang.reflect.Method
import org.json.JSONObject

/** Read-only shell probe. It never constructs or starts a call audio endpoint. */
object CallAudioCapabilityProbe {
    @JvmStatic
    fun main(args: Array<String>) {
        val uid = Process.myUid()
        val snapshot = when {
            uid != Process.SHELL_UID -> CallAudioProbeSnapshot(refusal = CallAudioProbeReason.REFUSED_UID)
            args.isNotEmpty() -> CallAudioProbeSnapshot(refusal = CallAudioProbeReason.REFUSED_ARGUMENTS)
            else -> inspect()
        }
        val result = classifyCallAudioProbe(snapshot)
        val permissions = JSONObject()
        PERMISSIONS.zip(snapshot.permissions).forEach { (name, granted) ->
            permissions.put(name, granted ?: JSONObject.NULL)
        }
        val methods = JSONObject()
            .put("isPstnCallAudioInterceptable", snapshot.methods[0] ?: JSONObject.NULL)
            .put("getCallDownlinkExtractionAudioRecord", snapshot.methods[1] ?: JSONObject.NULL)
            .put("getCallUplinkInjectionAudioTrack", snapshot.methods[2] ?: JSONObject.NULL)
        println(
            JSONObject()
                .put("schema", 1)
                .put("sdk", Build.VERSION.SDK_INT)
                .put("uid", uid)
                .put("permissions", permissions)
                .put("methods", methods)
                .put("mode", snapshot.mode ?: JSONObject.NULL)
                .put("pstnInterceptable", snapshot.pstnInterceptable ?: JSONObject.NULL)
                .put("reason", result.reason.name)
                .put("duplexQualified", result.duplexQualified)
                .toString(),
        )
    }

    private fun inspect(): CallAudioProbeSnapshot {
        val context = readOrUnknown { ShellContext.get() } ?: return CallAudioProbeSnapshot()
        val permissions = PERMISSIONS.map { name ->
            readOrUnknown {
                context.checkPermission(name, Process.myPid(), Process.myUid()) == PackageManager.PERMISSION_GRANTED
            }
        }
        val predicate = lookup("isPstnCallAudioInterceptable")
        val downlink = lookup("getCallDownlinkExtractionAudioRecord", AudioFormat::class.java)
        val uplink = lookup("getCallUplinkInjectionAudioTrack", AudioFormat::class.java)
        val manager = readOrUnknown { context.getSystemService(Context.AUDIO_SERVICE) as? AudioManager }
        val mode = manager?.let { readOrUnknown { it.mode } }
        // A manifest declaration or a method name never authorizes invocation.
        val interceptable = if (permissions[0] == true && predicate.method != null && manager != null) {
            readOrUnknown { predicate.method.invoke(manager) as? Boolean }
        } else {
            null
        }
        return CallAudioProbeSnapshot(
            permissions = permissions,
            methods = listOf(predicate.exists, downlink.exists, uplink.exists),
            mode = mode,
            pstnInterceptable = interceptable,
        )
    }

    private data class Lookup(val exists: Boolean?, val method: Method? = null)

    private fun lookup(name: String, vararg parameters: Class<*>): Lookup = try {
        Lookup(true, AudioManager::class.java.getMethod(name, *parameters))
    } catch (_: NoSuchMethodException) {
        // Non-SDK filtering can also hide a method; false means lookup unavailable.
        Lookup(false)
    } catch (_: Exception) {
        Lookup(null)
    } catch (_: LinkageError) {
        Lookup(null)
    }

    private inline fun <T> readOrUnknown(read: () -> T): T? = try {
        read()
    } catch (_: Exception) {
        null
    } catch (_: LinkageError) {
        null
    }

    private val PERMISSIONS = listOf(
        "android.permission.CALL_AUDIO_INTERCEPTION",
        "android.permission.RECORD_AUDIO",
        "android.permission.CAPTURE_AUDIO_OUTPUT",
        "android.permission.MODIFY_AUDIO_ROUTING",
        "android.permission.MODIFY_PHONE_STATE",
    )
}

internal enum class CallAudioProbeReason {
    REFUSED_UID,
    REFUSED_ARGUMENTS,
    PERMISSION_DENIED,
    METHOD_UNAVAILABLE,
    PSTN_UNAVAILABLE,
    UNKNOWN,
    NO_ACTIVE_PSTN_MODE,
    READ_ONLY_CONTRACT_AVAILABLE,
}

internal data class CallAudioProbeSnapshot(
    val permissions: List<Boolean?> = List(5) { null },
    val methods: List<Boolean?> = List(3) { null },
    val mode: Int? = null,
    val pstnInterceptable: Boolean? = null,
    val refusal: CallAudioProbeReason? = null,
)

internal data class CallAudioProbeResult(val reason: CallAudioProbeReason) {
    // Source capability inspection can never qualify physical audio acceptance.
    val duplexQualified: Boolean get() = false
}

internal fun classifyCallAudioProbe(snapshot: CallAudioProbeSnapshot): CallAudioProbeResult {
    val reason = when {
        snapshot.refusal != null -> snapshot.refusal
        snapshot.permissions.any { it == false } -> CallAudioProbeReason.PERMISSION_DENIED
        snapshot.methods.any { it == false } -> CallAudioProbeReason.METHOD_UNAVAILABLE
        snapshot.pstnInterceptable == false -> CallAudioProbeReason.PSTN_UNAVAILABLE
        snapshot.permissions.size != 5 || snapshot.methods.size != 3 ||
            snapshot.permissions.any { it == null } || snapshot.methods.any { it == null } ||
            snapshot.pstnInterceptable == null || snapshot.mode == null -> CallAudioProbeReason.UNKNOWN
        // AOSP: MODE_IN_CALL=2, MODE_CALL_SCREENING=4, MODE_CALL_REDIRECT=5.
        snapshot.mode !in setOf(2, 4, 5) -> CallAudioProbeReason.NO_ACTIVE_PSTN_MODE
        else -> CallAudioProbeReason.READ_ONLY_CONTRACT_AVAILABLE
    }
    return CallAudioProbeResult(reason)
}
