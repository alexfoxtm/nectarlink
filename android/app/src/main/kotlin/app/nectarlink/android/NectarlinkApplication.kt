// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android

import android.app.Activity
import android.app.Application
import android.content.SharedPreferences
import android.os.Bundle
import app.nectarlink.android.banking.BankingPause
import app.nectarlink.android.core.Core
import app.nectarlink.android.elevated.Elevated
import app.nectarlink.android.ui.Preferences
import app.nectarlink.android.update.AppUpdater
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.launch

/**
 * Starts the core with the process and keeps it for the app's lifetime.
 *
 * When "Run in the background" is off, the core connects only while the app
 * is open: entering the foreground starts it, and leaving the screen stops it
 * after [STOP_DELAY_MS], once no transfer, mirror, webcam or mic session is
 * active. Defaults to on, preserving the always-connected behavior.
 *
 * Foreground tracking uses activity callbacks rather than
 * `ProcessLifecycleOwner` so no extra lifecycle artifact is needed.
 */
class NectarlinkApplication : Application() {
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Default)

    val core: Core by lazy { Core(this, scope) }

    /** Updates from GitHub releases (release builds only). */
    val updater: AppUpdater by lazy { AppUpdater(this) }

    private var pendingStopJob: Job? = null

    /** Started activities; above zero while the app is on screen. */
    @Volatile private var startedActivities = 0

    private val prefListener =
        SharedPreferences.OnSharedPreferenceChangeListener { _, key ->
            if (key == Preferences.BACKGROUND_SERVICE_ENABLED) {
                if (Preferences.isBackgroundServiceEnabled(this)) {
                    pendingStopJob?.cancel()
                    pendingStopJob = null
                } else {
                    if (!isInForeground()) {
                        scheduleStop()
                    }
                }
            }
        }

    private val activityCallbacks = object : ActivityLifecycleCallbacks {
        override fun onActivityStarted(activity: Activity) {
            val wasForeground = startedActivities > 0
            startedActivities += 1
            if (!wasForeground) {
                pendingStopJob?.cancel()
                pendingStopJob = null
                core.start()
            }
        }

        override fun onActivityStopped(activity: Activity) {
            startedActivities = (startedActivities - 1).coerceAtLeast(0)
            if (!isInForeground() && !Preferences.isBackgroundServiceEnabled(this@NectarlinkApplication)) {
                scheduleStop()
            }
        }

        override fun onActivityCreated(activity: Activity, savedInstanceState: Bundle?) = Unit
        override fun onActivityResumed(activity: Activity) = Unit
        override fun onActivityPaused(activity: Activity) = Unit
        override fun onActivitySaveInstanceState(activity: Activity, outState: Bundle) = Unit
        override fun onActivityDestroyed(activity: Activity) = Unit
    }

    override fun onCreate() {
        super.onCreate()
        // Before the core starts: its network code reads the phone's DNS servers.
        app.nectarlink.android.core.AndroidContext.install(this)
        // Before Elevated: it waits while paused for a banking app.
        BankingPause.init(this)
        BankingPause.restoreIfStranded(this)
        Elevated.init(this, startInBackground = Preferences.isBackgroundServiceEnabled(this))
        // What the phone offers PCs follows Elevated starting and stopping.
        Elevated.onChange = { core.refreshNotificationAccess() }
        Elevated.onClipboardChanged = { text, html -> core.onElevatedClipboardChanged(text, html) }
        Elevated.onLinkCopied = { url, copiedAt -> core.onElevatedLinkCopied(url, copiedAt) }

        registerActivityLifecycleCallbacks(activityCallbacks)
        getSharedPreferences("appearance", MODE_PRIVATE)
            .registerOnSharedPreferenceChangeListener(prefListener)

        if (Preferences.isBackgroundServiceEnabled(this)) {
            android.view.Choreographer.getInstance().postFrameCallback {
                scope.launch { core.start() }
            }
            scope.launch {
                kotlinx.coroutines.delay(120)
                core.start()
            }
        }
    }

    fun onBackgroundServiceDisabled() {
        if (!isInForeground()) {
            scheduleStop()
        }
    }

    private fun isInForeground(): Boolean = startedActivities > 0

    private fun isStopEligible(): Boolean =
        !isInForeground() && !Preferences.isBackgroundServiceEnabled(this)

    private fun scheduleStop() {
        pendingStopJob?.cancel()
        pendingStopJob = scope.launch(Dispatchers.IO) {
            delay(STOP_DELAY_MS)
            if (!isStopEligible()) return@launch

            while (core.busy.value) {
                core.busy.first { !it }
                delay(STOP_IDLE_DEBOUNCE_MS)
                if (!isStopEligible()) return@launch
            }

            if (isStopEligible()) {
                core.stop { isStopEligible() }
            }
        }
    }

    companion object {
        const val STOP_DELAY_MS = 5_000L
        const val STOP_IDLE_DEBOUNCE_MS = 2_000L
    }
}
