// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.webcam

import android.Manifest
import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.content.pm.ServiceInfo
import android.os.Build
import android.os.Handler
import android.os.Looper
import android.util.Log
import androidx.camera.core.Preview
import androidx.core.app.NotificationCompat
import androidx.core.app.ServiceCompat
import androidx.core.content.ContextCompat
import androidx.lifecycle.LifecycleService
import androidx.lifecycle.lifecycleScope
import app.nectarlink.android.NectarlinkApplication
import app.nectarlink.android.R
import app.nectarlink.android.ui.MainActivity
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch

@androidx.compose.runtime.Immutable
data class WebcamSession(
    val active: Boolean = false,
    val connecting: Boolean = false,
    val pcId: String? = null,
    val entryToken: String? = null,
    val width: Int = 1280,
    val height: Int = 720,
    val fps: Int = 30,
    val camera: String = "back",
    val zoomRatio: Float = 1f,
    val minZoomRatio: Float = 1f,
    val maxZoomRatio: Float = 4f,
    val hasFlash: Boolean = false,
    val torchOn: Boolean = false,
    val supports4k: Boolean = CameraEncoder.supports4kEncoder(),
) {
    /**
     * The resolution streaming (720, 1080 or 2160), from the picture's short
     * side: upright, the phone streams 1080 × 1920 for 1080p.
     */
    val resolution: Int get() = resolutionOf(width, height)
}

/** 720, 1080 or 2160 for a picture of [width] × [height], either way up. */
internal fun resolutionOf(width: Int, height: Int): Int {
    val short = minOf(width, height)
    return when {
        short >= 2000 -> 2160
        short >= 1000 -> 1080
        else -> 720
    }
}

/**
 * Streams the phone's camera as H.264 to a paired PC (`docs/protocol/webcam.md`).
 * Runs as a `camera` foreground service with an ongoing notification and a
 * Stop action so streaming continues if the user switches apps or locks the
 * screen.
 */
class WebcamService : LifecycleService() {
    private val main = Handler(Looper.getMainLooper())
    private var encoder: CameraEncoder? = null
    private var pcId: String? = null
    private var entry: WebcamEntries.Entry? = null
    private var opening: kotlinx.coroutines.Job? = null

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        super.onStartCommand(intent, flags, startId)
        when (intent?.action) {
            ACTION_STOP -> {
                val token = intent.getStringExtra(EXTRA_ENTRY)
                val original = entry
                if (original != null && (original.token == token || (token == null && original.owner == null))) {
                    WebcamEntries.stop(original)
                }
            }
            ACTION_START -> {
                val accepted = WebcamEntries.resolve(intent.getStringExtra(EXTRA_ENTRY)) ?: return START_NOT_STICKY
                if (!hasPermission(this)) { WebcamEntries.stop(accepted); return START_NOT_STICKY }
                if (entry === accepted && encoder != null) { accepted.serviceStartId = startId; configureEntry(accepted); return START_NOT_STICKY }
                if (!WebcamEntries.begin(accepted, this, startId)) { WebcamEntries.stop(accepted); return START_NOT_STICKY }
                startStreamingSession(accepted)
            }
        }
        return START_NOT_STICKY
    }

    private fun startStreamingSession(accepted: WebcamEntries.Entry) {
        if (!WebcamEntries.owns(accepted)) return
        val targetPc = accepted.pcId
        val height = accepted.request.height
        val fps = accepted.request.fps
        val camera = accepted.request.camera
        entry = accepted
        pcId = targetPc
        current = this
        if (accepted.owner == null) WebcamRequests.dismiss(this, targetPc)
        val normCamera = if (camera == "front") "front" else "back"
        val supports4k = CameraEncoder.supports4k(this, normCamera)
        val normHeight = CameraEncoder.normalizeHeight(height, supports4k)
        val normWidth = CameraEncoder.widthForHeight(normHeight)
        val hasFlash = CameraEncoder.cameraHasFlash(this, normCamera)
        val maxZoom = CameraEncoder.cameraMaxZoom(this, normCamera)
        val initialZoom = _session.value.zoomRatio.coerceIn(1f, maxZoom)

        _session.value = WebcamSession(
            active = true,
            connecting = true,
            pcId = targetPc,
            entryToken = accepted.token,
            width = normWidth,
            height = normHeight,
            fps = fps,
            camera = normCamera,
            zoomRatio = initialZoom,
            maxZoomRatio = maxZoom,
            hasFlash = hasFlash,
            supports4k = supports4k,
        )

        createChannel(this)
        val type = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.R) {
            ServiceInfo.FOREGROUND_SERVICE_TYPE_CAMERA
        } else {
            0
        }
        val startedFg = runCatching {
            ServiceCompat.startForeground(
                this,
                NOTIFICATION_ID,
                buildNotification(targetPc, normWidth, normHeight, normCamera, accepted.token),
                type,
            )
        }.onFailure { e ->
            Log.w(TAG, "can't enter camera foreground service", e)
        }.isSuccess
        if (!startedFg) {
            WebcamEntries.stop(accepted)
            return
        }

        val core = (application as NectarlinkApplication).core
        core.clearWebcamRequest(targetPc, accepted.request.ownedEntryToken)

        opening = lifecycleScope.launch(Dispatchers.IO) {
            val stream = if (accepted.owner != null) {
                runCatching { accepted.owner.open() }.getOrNull()
            } else core.webcamOpenOrMessage(targetPc)
            if (stream == null) {
                WebcamEntries.stop(accepted)
                return@launch
            }
            main.post {
                if (entry !== accepted || !WebcamEntries.owns(accepted)) {
                    runCatching { stream.end() }
                    return@post
                }
                val camEncoder = CameraEncoder(
                    context = this@WebcamService,
                    lifecycleOwner = this@WebcamService,
                    stream = stream,
                    initialHeight = normHeight,
                    fps = fps,
                    initialCamera = normCamera,
                    initialZoomRatio = _session.value.zoomRatio,
                    ownedContext = accepted.owner,
                    ownsEntry = { WebcamEntries.owns(accepted) },
                    onStateChanged = { live ->
                        WebcamEntries.withCurrent(accepted) {
                        _session.update { s ->
                            s.copy(
                                active = true,
                                connecting = false,
                                width = live.width,
                                height = live.height,
                                fps = live.fps,
                                camera = live.camera,
                                zoomRatio = live.zoomRatio,
                                minZoomRatio = live.minZoomRatio,
                                maxZoomRatio = live.maxZoomRatio,
                                hasFlash = live.hasFlash,
                                torchOn = live.torchOn,
                                supports4k = live.supports4k,
                            )
                        }
                        updateNotification()
                        }
                    },
                    onEnded = { WebcamEntries.stop(accepted) },
                )
                camEncoder.setUiSurfaceProvider(uiSurfaceProvider)
                encoder = camEncoder
                camEncoder.start()
            }
        }
    }

    private fun updateNotification() {
        val s = _session.value
        val targetPc = s.pcId ?: return
        val nm = getSystemService(NotificationManager::class.java) ?: return
        nm.notify(NOTIFICATION_ID, buildNotification(targetPc, s.width, s.height, s.camera, s.entryToken))
    }

    private fun buildNotification(targetPc: String, width: Int, height: Int, camera: String, entryToken: String?): Notification {
        val core = (application as NectarlinkApplication).core
        val pcName = core.state.value.nameOf(targetPc).orEmpty().ifEmpty { getString(R.string.your_pc) }

        val openIntent = Intent(this, MainActivity::class.java).apply {
            putExtra("webcam_pc", targetPc)
            flags = Intent.FLAG_ACTIVITY_SINGLE_TOP or Intent.FLAG_ACTIVITY_CLEAR_TOP
        }
        val openPending = PendingIntent.getActivity(
            this,
            10,
            openIntent,
            PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
        )
        val stopPending = PendingIntent.getService(
            this,
            11,
            Intent(this, WebcamService::class.java).setAction(ACTION_STOP)
                .setData(android.net.Uri.parse("waypair-webcam://stop/$entryToken"))
                .putExtra(EXTRA_ENTRY, entryToken),
            PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
        )
        val camLabel = getString(
            if (camera == "front") R.string.webcam_camera_front else R.string.webcam_camera_back,
        ).lowercase()

        return NotificationCompat.Builder(this, CHANNEL)
            .setSmallIcon(R.drawable.ic_notification)
            .setContentTitle(getString(R.string.webcam_notification_streaming, pcName))
            .setContentText(getString(R.string.webcam_notification_detail, width, height, camLabel))
            .setContentIntent(openPending)
            .setOngoing(true)
            .setSilent(true)
            .setOnlyAlertOnce(true)
            .setCategory(NotificationCompat.CATEGORY_SERVICE)
            .addAction(0, getString(R.string.action_stop), stopPending)
            .build()
    }

    internal fun stopEntry(original: WebcamEntries.Entry) {
        main.post {
            if (entry !== original) { WebcamEntries.complete(original); return@post }
            opening?.cancel()
            opening = null
            val originalEncoder = encoder
            encoder = null
            // Keep the Entry/global capture slot until both encoder cleanup lanes finish.
            val completed: (Boolean) -> Unit = { success ->
                main.post {
                    if (!success) {
                        // Keep the exact slot reserved; no terminal ACK or replacement.
                        if (entry === original) encoder = originalEncoder
                        return@post
                    }
                    if (entry === original) {
                        entry = null
                        pcId = null
                        if (current === this@WebcamService) current = null
                        _session.update { state ->
                            if (state.entryToken == original.token) state.copy(active = false,
                                connecting = false, pcId = null, entryToken = null, torchOn = false) else state
                        }
                        ServiceCompat.stopForeground(this@WebcamService, ServiceCompat.STOP_FOREGROUND_REMOVE)
                        stopSelfResult(original.serviceStartId)
                    }
                    WebcamEntries.complete(original)
                }
            }
            if (originalEncoder != null) originalEncoder.stop(completed) else completed(true)
        }
    }

    private fun configureEntry(original: WebcamEntries.Entry) {
        WebcamEntries.withCurrent(original) {
            if (entry === original) {
                encoder?.setCamera(original.request.camera)
                encoder?.setResolutionHeight(original.request.height)
            }
        }
    }
    internal fun keyframeEntry(original: WebcamEntries.Entry) {
        main.post { WebcamEntries.withCurrent(original) { if (entry === original) encoder?.requestKeyframe() } }
    }

    override fun onDestroy() {
        entry?.let(WebcamEntries::stop)
        super.onDestroy()
    }

    companion object {
        private const val TAG = "WebcamService"
        private const val CHANNEL = "webcam"
        private const val NOTIFICATION_ID = 8

        private const val ACTION_START = "app.nectarlink.android.webcam.START"
        private const val ACTION_STOP = "app.nectarlink.android.webcam.STOP"
        private const val EXTRA_ENTRY = "entry"
        private const val EXTRA_PC = "pc"
        private const val EXTRA_HEIGHT = "height"
        private const val EXTRA_FPS = "fps"
        private const val EXTRA_CAMERA = "camera"

        @Volatile private var current: WebcamService? = null
        @Volatile private var uiSurfaceProvider: Preview.SurfaceProvider? = null

        private val _session = MutableStateFlow(WebcamSession())
        val session: StateFlow<WebcamSession> = _session.asStateFlow()

        fun hasPermission(context: Context): Boolean =
            ContextCompat.checkSelfPermission(context, Manifest.permission.CAMERA) ==
                PackageManager.PERMISSION_GRANTED &&
                context.packageManager.hasSystemFeature(PackageManager.FEATURE_CAMERA_ANY)

        fun start(
            context: Context,
            pcId: String,
            height: Int = _session.value.height,
            fps: Int = 30,
            camera: String = _session.value.camera,
        ) {
            val core = (context.applicationContext as NectarlinkApplication).core
            if (core.webcamRequiresOwned(pcId)) return
            val accepted = WebcamEntries.legacy(pcId, height, fps, camera) ?: return
            startEntry(context, accepted)
        }

        fun startOwned(context: Context, token: String) {
            val accepted = WebcamEntries.resolve(token)?.takeIf { it.owner != null } ?: return
            startEntry(context, accepted)
        }
        private fun startEntry(context: Context, accepted: WebcamEntries.Entry) {
            runCatching {
                ContextCompat.startForegroundService(context, Intent(context, WebcamService::class.java)
                    .setAction(ACTION_START).putExtra(EXTRA_ENTRY, accepted.token))
            }.onFailure { WebcamEntries.stop(accepted) }
        }
        internal fun configure(original: WebcamEntries.Entry) {
            original.service?.let { service -> service.main.post { service.configureEntry(original) } }
        }

        fun stop(context: Context) {
            val original = current?.entry ?: return
            WebcamEntries.stop(original)
        }
        fun stopForEntry(token: String?) {
            token?.let(WebcamEntries::resolve)?.let(WebcamEntries::stop)
        }
        fun stopForPc(context: Context, pcId: String) {
            // Legacy callback snapshots its actual Entry now, before any post.
            val original = current?.entry?.takeIf { it.pcId == pcId && it.owner == null } ?: return
            WebcamEntries.stop(original)
        }
        fun keyframe(pcId: String) {
            val original = current?.entry?.takeIf { it.pcId == pcId && it.owner == null } ?: return
            original.service?.keyframeEntry(original)
        }

        fun refreshCapabilities(context: Context) {
            val cam = _session.value.camera
            val flash = CameraEncoder.cameraHasFlash(context, cam)
            val maxZoom = CameraEncoder.cameraMaxZoom(context, cam)
            _session.update {
                it.copy(
                    hasFlash = flash,
                    maxZoomRatio = maxZoom,
                    zoomRatio = it.zoomRatio.coerceIn(1f, maxZoom),
                )
            }
        }

        fun setCamera(camera: String, context: Context? = null) {
            val norm = if (camera == "front") "front" else "back"
            val ctx = context ?: current
            val flash = ctx?.let { CameraEncoder.cameraHasFlash(it, norm) } ?: false
            val maxZoom = ctx?.let { CameraEncoder.cameraMaxZoom(it, norm) } ?: 4f
            _session.update {
                it.copy(
                    camera = norm,
                    hasFlash = flash,
                    maxZoomRatio = maxZoom,
                    zoomRatio = it.zoomRatio.coerceIn(1f, maxZoom),
                    torchOn = false,
                )
            }
            current?.encoder?.setCamera(norm)
        }

        fun setResolutionHeight(height: Int) {
            val norm = CameraEncoder.normalizeHeight(height, _session.value.supports4k)
            _session.update {
                it.copy(width = CameraEncoder.widthForHeight(norm), height = norm)
            }
            current?.encoder?.setResolutionHeight(norm)
        }

        fun setZoomRatio(ratio: Float) {
            _session.update { it.copy(zoomRatio = ratio) }
            current?.encoder?.setZoomRatio(ratio)
        }

        fun setTorch(on: Boolean) {
            _session.update { it.copy(torchOn = on) }
            current?.encoder?.setTorch(on)
        }

        fun attachUiPreview(provider: Preview.SurfaceProvider?) {
            uiSurfaceProvider = provider
            current?.encoder?.setUiSurfaceProvider(provider)
        }

        fun createChannel(context: Context) {
            val nm = context.getSystemService(NotificationManager::class.java) ?: return
            val channel = NotificationChannel(
                CHANNEL,
                context.getString(R.string.channel_webcam),
                NotificationManager.IMPORTANCE_LOW,
            ).apply {
                setShowBadge(false)
            }
            nm.createNotificationChannel(channel)
        }
    }
}
