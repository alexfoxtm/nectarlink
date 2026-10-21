// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.widget

import android.app.PendingIntent
import android.appwidget.AppWidgetManager
import android.appwidget.AppWidgetProvider
import android.content.BroadcastReceiver
import android.content.ComponentName
import android.content.Context
import android.content.Intent
import android.graphics.Color
import android.view.View
import android.widget.RemoteViews
import android.widget.Toast
import app.nectarlink.android.NectarlinkApplication
import app.nectarlink.android.R
import app.nectarlink.android.clipboard.SendActivity
import app.nectarlink.android.core.CoreState
import app.nectarlink.android.core.CoreStatus
import app.nectarlink.android.core.Device
import app.nectarlink.android.core.WakeState
import app.nectarlink.android.ui.MainActivity
import app.nectarlink.android.ui.Preferences
import app.nectarlink.android.ui.home.PeerOnlineLabel
import app.nectarlink.android.ui.home.resolvePeerOnlineLabel
import app.nectarlink.core.Link
import app.nectarlink.core.MediaAction

/**
 * Battery-free glassmorphic home-screen widget showing the primary paired PC's
 * connection status, live PC Now Playing strip with inline playback controls,
 * Wake-on-LAN button when offline, and one-tap quick actions (Send files,
 * Send clipboard, Ring PC, Lock PC). Has `updatePeriodMillis="0"` so it never
 * wakes the device on a timer; [Core] and [PcMedia] refresh it on state changes.
 */
class PcWidgetProvider : AppWidgetProvider() {

    override fun onUpdate(
        context: Context,
        appWidgetManager: AppWidgetManager,
        appWidgetIds: IntArray,
    ) {
        val application = context.applicationContext as? NectarlinkApplication
        val core = application?.core
        startForWidget(
            backgroundEnabled = Preferences.isBackgroundServiceEnabled(context),
            explicitUserAction = false,
            start = { core?.start() },
            stopWhenIdle = { application?.onBackgroundServiceDisabled() },
        )
        val state = core?.state?.value
        // Avoid overwriting a previously rendered widget with "No PC paired" while Core is still starting up.
        if (state != null && (state.devices.isNotEmpty() || state.status != app.nectarlink.android.core.CoreStatus.Starting)) {
            updateWidgets(context, appWidgetManager, appWidgetIds, state)
        } else if (state != null) {
            updateWidgets(context, appWidgetManager, appWidgetIds, state)
        }
    }

    /**
     * Non-exported internal receiver for widget action buttons so third-party apps
     * cannot spoof broadcast intents to lock, ring, or wake a paired PC.
     */
    class ActionReceiver : BroadcastReceiver() {
        override fun onReceive(context: Context, intent: Intent) {
            val action = intent.action ?: return
            val app = context.applicationContext as? NectarlinkApplication ?: return
            startForWidget(
                backgroundEnabled = Preferences.isBackgroundServiceEnabled(context),
                explicitUserAction = true,
                start = { app.core.start() },
                stopWhenIdle = { app.onBackgroundServiceDisabled() },
            )
            val state = app.core.state.value
            val pcId = intent.getStringExtra(EXTRA_PC_ID)?.takeIf { it.isNotBlank() }
                ?: primaryPc(state)?.id
                ?: return
            val pc = state.devices.firstOrNull { it.id == pcId }
            val pcName = pc?.name?.takeIf { it.isNotBlank() }
                ?: context.getString(R.string.your_pc)

            val notConnected = {
                Toast.makeText(context, context.getString(R.string.error_not_connected), Toast.LENGTH_SHORT).show()
            }
            when (action) {
                ACTION_RING_PC -> sendIfConnected(state.status, pc, notConnected = notConnected, send = {
                    app.core.ring(pcId, true)
                    Toast.makeText(
                        context,
                        context.getString(R.string.widget_pc_ringing_toast, pcName),
                        Toast.LENGTH_SHORT,
                    ).show()
                })
                ACTION_LOCK_PC -> sendIfConnected(state.status, pc, notConnected = notConnected, send = {
                    app.core.pcPower(pcId, sleep = false)
                    Toast.makeText(
                        context,
                        context.getString(R.string.pc_locked, pcName),
                        Toast.LENGTH_SHORT,
                    ).show()
                })
                ACTION_WAKE_PC -> {
                    app.core.wake(pcId)
                    Toast.makeText(
                        context,
                        context.getString(R.string.widget_pc_waking_toast, pcName),
                        Toast.LENGTH_SHORT,
                    ).show()
                }
                ACTION_MEDIA_PLAY_PAUSE -> {
                    val playing = app.core.pcMedia.currentSummary?.playing == true
                    app.core.pcMedia.pressed(if (playing) MediaAction.PAUSE else MediaAction.PLAY)
                }
                ACTION_MEDIA_NEXT -> {
                    app.core.pcMedia.pressed(MediaAction.NEXT)
                }
            }
        }
    }

    companion object {
        const val ACTION_RING_PC = "app.nectarlink.widget.ACTION_RING_PC"
        const val ACTION_LOCK_PC = "app.nectarlink.widget.ACTION_LOCK_PC"
        const val ACTION_WAKE_PC = "app.nectarlink.widget.ACTION_WAKE_PC"
        const val ACTION_MEDIA_PLAY_PAUSE = "app.nectarlink.widget.ACTION_MEDIA_PLAY_PAUSE"
        const val ACTION_MEDIA_NEXT = "app.nectarlink.widget.ACTION_MEDIA_NEXT"
        const val EXTRA_PC_ID = "app.nectarlink.widget.EXTRA_PC_ID"

        private const val REQ_OPEN_APP = 100
        private const val REQ_SEND_CLIP = 101
        private const val REQ_RING = 102
        private const val REQ_LOCK = 103
        private const val REQ_WAKE = 104
        private const val REQ_SEND_FILES = 105
        private const val REQ_MEDIA_PLAY_PAUSE = 106
        private const val REQ_MEDIA_NEXT = 107

        private val COLOR_ONLINE_TEXT = Color.parseColor("#F4B841")
        private val COLOR_OFFLINE_TEXT = Color.parseColor("#D5CFC2")
        private val COLOR_BTN_ACTIVE = Color.parseColor("#FAF6EE")
        private val COLOR_BTN_DIM = Color.parseColor("#77FAF6EE")

        private fun primaryPc(state: CoreState?): Device? =
            state?.devices?.firstOrNull { it.online } ?: state?.devices?.firstOrNull()

        fun refreshAll(context: Context, state: CoreState? = null) {
            val manager = AppWidgetManager.getInstance(context) ?: return
            val ids = runCatching {
                manager.getAppWidgetIds(ComponentName(context, PcWidgetProvider::class.java))
            }.getOrNull() ?: return
            if (ids.isEmpty()) return

            val effectiveState = state
                ?: (context.applicationContext as? NectarlinkApplication)?.core?.state?.value
            updateWidgets(context, manager, ids, effectiveState)
        }

        private fun updateWidgets(
            context: Context,
            appWidgetManager: AppWidgetManager,
            appWidgetIds: IntArray,
            state: CoreState?,
        ) {
            if (appWidgetIds.isEmpty()) return
            val pc = primaryPc(state)
            val views = RemoteViews(context.packageName, R.layout.widget_pc)

            val launchIntent = Intent(context, MainActivity::class.java).apply {
                flags = Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_CLEAR_TOP
            }
            val launchPending = PendingIntent.getActivity(
                context,
                REQ_OPEN_APP,
                launchIntent,
                PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE,
            )
            views.setOnClickPendingIntent(R.id.widget_root, launchPending)

            if (pc == null) {
                views.setTextViewText(R.id.widget_pc_name, context.getString(R.string.widget_pc_no_pc))
                views.setTextViewText(R.id.widget_pc_status, context.getString(R.string.widget_pc_offline))
                views.setInt(R.id.widget_pc_status, "setBackgroundResource", R.drawable.bg_widget_pill_offline)
                views.setTextColor(R.id.widget_pc_status, COLOR_OFFLINE_TEXT)
                views.setViewVisibility(R.id.widget_pc_subtitle, View.VISIBLE)
                views.setTextViewText(R.id.widget_pc_subtitle, context.getString(R.string.widget_pc_tap_to_pair))
                views.setViewVisibility(R.id.widget_btn_wake, View.GONE)
                views.setViewVisibility(R.id.widget_media_row, View.GONE)
                views.setViewVisibility(R.id.widget_actions_row, View.GONE)
            } else {
                views.setViewVisibility(R.id.widget_actions_row, View.VISIBLE)
                val name = pc.name.ifBlank { context.getString(R.string.your_pc) }
                views.setTextViewText(R.id.widget_pc_name, name)

                val app = context.applicationContext as? NectarlinkApplication
                val mediaSummary = if (pc.online) {
                    app?.core?.pcMedia?.currentSummary?.takeIf { it.pcId == pc.id }
                        ?: app?.core?.pcMedia?.currentSummary
                } else {
                    null
                }

                when {
                    pc.online -> {
                        views.setTextViewText(R.id.widget_pc_status, context.getString(R.string.widget_pc_connected))
                        views.setInt(R.id.widget_pc_status, "setBackgroundResource", R.drawable.bg_widget_pill_online)
                        views.setTextColor(R.id.widget_pc_status, COLOR_ONLINE_TEXT)
                        val onlineLink = pc.link as? Link.Online
                        // Same rule as Home: only a proven cable renders USB;
                        // a tunnel record alone (including wireless ADB) never
                        // does, and unproven links stay Connected.
                        val transport = when (resolvePeerOnlineLabel(pc.displayPath, pc.usbMedium, pc.routeTransport)) {
                            PeerOnlineLabel.AWAY -> "Relay"
                            PeerOnlineLabel.USB -> "USB"
                            PeerOnlineLabel.WIFI -> "Wi‑Fi"
                            PeerOnlineLabel.CELLULAR -> "Cellular"
                            PeerOnlineLabel.ETHERNET -> "Ethernet"
                            PeerOnlineLabel.CONNECTED -> "Connected"
                        }
                        val rtt = onlineLink?.rttMs?.takeIf { it > 0u }
                        val deck = pc.deckState
                        val details = buildList {
                            add(transport)
                            if (rtt != null) add("$rtt ms")
                            if (deck != null) {
                                add(if (deck.muted) "Muted (${deck.volume}%)" else "Vol ${deck.volume}%")
                            }
                        }.joinToString(" · ")
                        // Hide the secondary subtitle line when the live media row is visible so
                        // all 3 rows fit comfortably inside a 4x2 home-screen widget cell.
                        if (mediaSummary != null) {
                            views.setViewVisibility(R.id.widget_pc_subtitle, View.GONE)
                        } else {
                            views.setViewVisibility(R.id.widget_pc_subtitle, View.VISIBLE)
                            views.setTextViewText(R.id.widget_pc_subtitle, details)
                        }
                    }
                    pc.wakeState == WakeState.Waking -> {
                        views.setTextViewText(R.id.widget_pc_status, context.getString(R.string.widget_pc_waking))
                        views.setInt(R.id.widget_pc_status, "setBackgroundResource", R.drawable.bg_widget_pill_offline)
                        views.setTextColor(R.id.widget_pc_status, COLOR_OFFLINE_TEXT)
                        views.setViewVisibility(R.id.widget_pc_subtitle, View.GONE)
                    }
                    else -> {
                        views.setTextViewText(R.id.widget_pc_status, context.getString(R.string.widget_pc_offline))
                        views.setInt(R.id.widget_pc_status, "setBackgroundResource", R.drawable.bg_widget_pill_offline)
                        views.setTextColor(R.id.widget_pc_status, COLOR_OFFLINE_TEXT)
                        views.setViewVisibility(R.id.widget_pc_subtitle, View.GONE)
                    }
                }

                if (mediaSummary != null) {
                    val label = if (mediaSummary.subtitle.isNotBlank()) {
                        "${mediaSummary.title} · ${mediaSummary.subtitle}"
                    } else {
                        mediaSummary.title
                    }
                    views.setViewVisibility(R.id.widget_media_row, View.VISIBLE)
                    views.setTextViewText(R.id.widget_media_title, label)
                    views.setImageViewResource(
                        R.id.widget_btn_media_play,
                        if (mediaSummary.playing) R.drawable.ic_pause else R.drawable.ic_play,
                    )
                    views.setOnClickPendingIntent(
                        R.id.widget_btn_media_play,
                        actionBroadcast(context, REQ_MEDIA_PLAY_PAUSE, ACTION_MEDIA_PLAY_PAUSE, mediaSummary.pcId),
                    )
                    views.setOnClickPendingIntent(
                        R.id.widget_btn_media_next,
                        actionBroadcast(context, REQ_MEDIA_NEXT, ACTION_MEDIA_NEXT, mediaSummary.pcId),
                    )
                } else {
                    views.setViewVisibility(R.id.widget_media_row, View.GONE)
                }

                if (!pc.online && pc.canWake) {
                    views.setViewVisibility(R.id.widget_btn_wake, View.VISIBLE)
                    views.setOnClickPendingIntent(
                        R.id.widget_btn_wake,
                        actionBroadcast(context, REQ_WAKE, ACTION_WAKE_PC, pc.id),
                    )
                } else {
                    views.setViewVisibility(R.id.widget_btn_wake, View.GONE)
                }

                val btnColor = if (pc.online) COLOR_BTN_ACTIVE else COLOR_BTN_DIM
                views.setTextColor(R.id.widget_btn_send_files, btnColor)
                views.setTextColor(R.id.widget_btn_send_clip, btnColor)
                views.setTextColor(R.id.widget_btn_ring, btnColor)
                views.setTextColor(R.id.widget_btn_lock, btnColor)
            }

            val filesIntent = SendActivity.sendFilesIntent(context, pc?.id)
            val filesPending = PendingIntent.getActivity(
                context,
                REQ_SEND_FILES,
                filesIntent,
                PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE,
            )
            views.setOnClickPendingIntent(R.id.widget_btn_send_files, filesPending)

            val clipIntent = SendActivity.sendClipboardIntent(context, pc?.id)
            val clipPending = PendingIntent.getActivity(
                context,
                REQ_SEND_CLIP,
                clipIntent,
                PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE,
            )
            views.setOnClickPendingIntent(R.id.widget_btn_send_clip, clipPending)

            views.setOnClickPendingIntent(
                R.id.widget_btn_ring,
                actionBroadcast(context, REQ_RING, ACTION_RING_PC, pc?.id),
            )
            views.setOnClickPendingIntent(
                R.id.widget_btn_lock,
                actionBroadcast(context, REQ_LOCK, ACTION_LOCK_PC, pc?.id),
            )

            for (id in appWidgetIds) {
                appWidgetManager.updateAppWidget(id, views)
            }
        }

        private fun actionBroadcast(
            context: Context,
            requestCode: Int,
            action: String,
            pcId: String?,
        ): PendingIntent {
            val intent = Intent(context, ActionReceiver::class.java).apply {
                this.action = action
                if (!pcId.isNullOrEmpty()) {
                    putExtra(EXTRA_PC_ID, pcId)
                } else {
                    removeExtra(EXTRA_PC_ID)
                }
            }
            return PendingIntent.getBroadcast(
                context,
                requestCode,
                intent,
                PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE,
            )
        }
    }
}

/**
 * With Run in the background off, a silent widget refresh never starts the node, and a
 * tap starts it and then asks for the app's usual delayed stop: a broadcast never reaches
 * onActivityStopped, so nothing else would stop that node.
 *
 * Top level rather than in the companion, so unit tests can call it without initializing
 * [PcWidgetProvider], whose companion calls `Color.parseColor`.
 */
internal fun startForWidget(
    backgroundEnabled: Boolean,
    explicitUserAction: Boolean,
    start: () -> Unit,
    stopWhenIdle: () -> Unit,
) {
    if (!backgroundEnabled && !explicitUserAction) return
    start()
    if (!backgroundEnabled) stopWhenIdle()
}

/**
 * Ring and Lock need a live session to [pc]. A node that isn't Ready (a tap that just started
 * it, or one stopped or failed) has no session yet, so the tap says the PC isn't connected
 * instead of claiming success. A Ready node with [pc] offline or unknown stays silent.
 */
internal fun sendIfConnected(status: CoreStatus, pc: Device?, notConnected: () -> Unit, send: () -> Unit) {
    when {
        status !is CoreStatus.Ready -> notConnected()
        pc?.online == true -> send()
    }
}
