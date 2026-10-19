// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.core

import android.app.NotificationManager
import android.content.Context
import android.media.AudioAttributes
import android.media.AudioManager
import android.media.MediaPlayer
import android.media.RingtoneManager
import android.os.Build
import android.os.Handler
import android.os.Looper
import android.os.VibrationEffect
import android.os.Vibrator
import android.os.VibratorManager
import android.util.Log

/**
 * "Find my phone": plays the alarm sound on the alarm stream and
 * vibrates, until stopped or for a minute.
 *
 * The alarm-stream volume is temporarily raised to full for the ring
 * and the user's original volume is restored exactly once on every
 * exit path (stop, the one-minute timeout, overlap handling, and
 * player failure). The saved volume is also kept in preferences
 * while a ring is active, so if the process dies mid-ring the next
 * launch restores it instead of leaving the user's alarm volume
 * raised forever. No user setting is permanently changed.
 *
 * Do Not Disturb: alarm-category audio (`USAGE_ALARM`) passes the
 * ALL, PRIORITY and ALARMS interruption filters, so raising the
 * volume is enough there. Total silence
 * (`INTERRUPTION_FILTER_NONE`) suppresses alarms at the system level
 * and cannot be bypassed by any audio attribute; this class
 * deliberately does not call `setInterruptionFilter` (whose
 * semantics for third-party callers are restricted on current
 * target SDKs) and never grants itself policy access. In that one
 * mode the ring degrades to vibration plus the raised volume, and
 * the truth is logged — a real bypass would need the user to grant
 * notification-policy access and, on Android 15+/targetSdk 37 Zen
 * modes, to allow this app's rule in system settings.
 */
class Ringer(context: Context) {
    private val context = context.applicationContext
    private val main = Handler(Looper.getMainLooper())
    private var player: MediaPlayer? = null
    private val timeout = Runnable { stopRingingNow() }
    private val tracker = RingPolicy.OverrideTracker()

    private val audio: AudioManager? get() = context.getSystemService(AudioManager::class.java)
    private val notifications: NotificationManager?
        get() = context.getSystemService(NotificationManager::class.java)

    init {
        restoreStaleOverride()
    }

    fun startRinging() {
        main.post { startRingingNow() }
    }

    fun stopRinging() {
        main.post { stopRingingNow() }
    }

    private fun startRingingNow() {
        if (player != null) {
            // Overlapping ring request: keep the original saved
            // volume and the player, just extend the bound.
            main.removeCallbacks(timeout)
            main.postDelayed(timeout, MAX_RING_MS)
            return
        }
        beginOverride()
        val uri = RingtoneManager.getDefaultUri(RingtoneManager.TYPE_ALARM)
            ?: RingtoneManager.getDefaultUri(RingtoneManager.TYPE_RINGTONE)
        player = runCatching {
            MediaPlayer().apply {
                setAudioAttributes(
                    AudioAttributes.Builder()
                        .setUsage(AudioAttributes.USAGE_ALARM)
                        .setContentType(AudioAttributes.CONTENT_TYPE_SONIFICATION)
                        .build(),
                )
                setDataSource(context, uri)
                isLooping = true
                prepare()
                start()
            }
        }.getOrNull()
        vibrator()?.vibrate(VibrationEffect.createWaveform(longArrayOf(0, 600, 400), 0))
        if (player != null) {
            main.postDelayed(timeout, MAX_RING_MS)
        } else {
            // Nothing is sounding: never leave the volume raised.
            Log.w(TAG, "no alarm player could start; restoring the alarm volume")
            restoreOverride()
        }
    }

    private fun stopRingingNow() {
        main.removeCallbacks(timeout)
        player?.runCatching { stop(); release() }
        player = null
        vibrator()?.cancel()
        restoreOverride()
    }

    /**
     * Raises the alarm volume to full, capturing the user's volume
     * first (once — an overlapping ring keeps the first capture), and
     * reports the one DND mode volume cannot fix.
     */
    private fun beginOverride() {
        val am = audio ?: return
        val current = runCatching { am.getStreamVolume(AudioManager.STREAM_ALARM) }.getOrNull() ?: return
        if (tracker.begin(current)) {
            persistSaved(current)
        }
        val max = runCatching { am.getStreamMaxVolume(AudioManager.STREAM_ALARM) }.getOrDefault(current)
        runCatching {
            am.setStreamVolume(AudioManager.STREAM_ALARM, RingPolicy.raisedAlarmVolume(current, max), 0)
        }.onFailure { Log.w(TAG, "can't raise the alarm volume", it) }
        val filter = when (notifications?.currentInterruptionFilter) {
            NotificationManager.INTERRUPTION_FILTER_NONE -> RingPolicy.ZenFilter.NONE
            NotificationManager.INTERRUPTION_FILTER_PRIORITY -> RingPolicy.ZenFilter.PRIORITY
            NotificationManager.INTERRUPTION_FILTER_ALARMS -> RingPolicy.ZenFilter.ALARMS
            NotificationManager.INTERRUPTION_FILTER_ALL -> RingPolicy.ZenFilter.ALL
            else -> RingPolicy.ZenFilter.UNKNOWN
        }
        if (!RingPolicy.alarmReachesUser(filter)) {
            // Truthful degraded state: no self-grant, no filter change.
            Log.w(TAG, "Do Not Disturb total silence suppresses alarm audio; ringing by vibration only")
        }
    }

    /** Restores the captured alarm volume, exactly once. */
    private fun restoreOverride() {
        val saved = tracker.consumeRestore() ?: return
        val restored = runCatching {
            audio?.setStreamVolume(AudioManager.STREAM_ALARM, saved, 0)
        }.isSuccess
        if (restored) {
            clearPersisted()
        }
        // If setting it failed, the persisted copy stays, so the next
        // launch's stale-override restore retries instead of losing it.
    }

    /**
     * If a previous process died while a ring had the volume raised,
     * its persisted saved volume is restored now and forgotten. This
     * never runs while this instance has an active override.
     */
    private fun restoreStaleOverride() {
        if (tracker.active) return
        val prefs = context.getSharedPreferences(PREFS, Context.MODE_PRIVATE)
        if (!prefs.contains(KEY_SAVED_ALARM_VOLUME)) return
        val saved = prefs.getInt(KEY_SAVED_ALARM_VOLUME, -1)
        if (saved >= 0) {
            runCatching { audio?.setStreamVolume(AudioManager.STREAM_ALARM, saved, 0) }
                .onSuccess { clearPersisted() }
        } else {
            clearPersisted()
        }
    }

    private fun persistSaved(volume: Int) {
        context.getSharedPreferences(PREFS, Context.MODE_PRIVATE)
            .edit().putInt(KEY_SAVED_ALARM_VOLUME, volume).apply()
    }

    private fun clearPersisted() {
        context.getSharedPreferences(PREFS, Context.MODE_PRIVATE)
            .edit().remove(KEY_SAVED_ALARM_VOLUME).apply()
    }

    private fun vibrator(): Vibrator? =
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.S) {
            context.getSystemService(VibratorManager::class.java)?.defaultVibrator
        } else {
            @Suppress("DEPRECATION")
            context.getSystemService(Vibrator::class.java)
        }

    private companion object {
        const val TAG = "Ringer"
        const val MAX_RING_MS = 60_000L
        const val PREFS = "ringer"
        const val KEY_SAVED_ALARM_VOLUME = "saved_alarm_volume"
    }
}
