// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.mirror

import android.app.PendingIntent
import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.graphics.Bitmap
import android.graphics.Canvas
import android.os.Build
import android.util.Log
import android.view.WindowManager
import androidx.core.app.NotificationCompat
import androidx.core.app.NotificationManagerCompat
import app.nectarlink.android.R
import app.nectarlink.android.elevated.AppDisplay
import app.nectarlink.android.elevated.Elevated
import app.nectarlink.core.MirrorInputEvent
import app.nectarlink.core.MirrorOptions
import app.nectarlink.core.MirrorSendResult
import app.nectarlink.core.MirrorStream
import app.nectarlink.core.NectarlinkNode
import app.nectarlink.core.PhoneControlContext
import app.nectarlink.core.PhoneApp
import app.nectarlink.core.VideoPacketKind
import app.nectarlink.core.mirrorConfig
import java.io.ByteArrayOutputStream
import java.net.Socket
import java.util.concurrent.ConcurrentHashMap
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import java.util.concurrent.CompletableFuture
import java.util.concurrent.CountDownLatch
import java.util.concurrent.ExecutorService
import java.util.concurrent.Executors
import kotlinx.coroutines.launch
import kotlin.math.roundToInt

/** Binds callback-entry state, then completes all admission work off the native callback stack. */
internal fun <T, R> appWindowAdmission(bind: () -> T, work: (T) -> R): R {
    val bound = bind()
    if (Thread.currentThread().name.startsWith(ADMISSION_THREAD)) return work(bound)
    val onWorker = CountDownLatch(1)
    val completed = CompletableFuture<R>()
    var interrupted = Thread.interrupted()
    admissionThreads().execute {
        try {
            completed.complete(work(bound))
        } catch (failure: Throwable) {
            completed.completeExceptionally(failure)
        } finally {
            onWorker.countDown()
        }
    }
    // Completion is mandatory: never abandon a worker that may act after Core's return.
    while (true) {
        try {
            onWorker.await()
            break
        } catch (_: InterruptedException) {
            interrupted = true
        }
    }
    if (interrupted) Thread.currentThread().interrupt()
    return try {
        completed.get()
    } catch (wrapped: java.util.concurrent.ExecutionException) {
        throw wrapped.cause ?: wrapped
    }
}

private const val ADMISSION_THREAD = "app-window-admission"

private fun admissionThreads(): ExecutorService = AdmissionThreads.INSTANCE

private enum class AdmissionThreads {
    ;
    companion object {
        val INSTANCE: ExecutorService = Executors.newSingleThreadExecutor { runnable ->
            Thread(runnable, ADMISSION_THREAD).apply { isDaemon = true }
        }
    }
}

/**
 * Phone apps in windows of their own on a PC (Elevated): each runs on a
 * display the shell-side helper makes for it, whose video goes to the PC
 * as a mirroring session of its own. A notification says so while any are
 * open, with a button to close them.
 */
class AppWindows(
    context: Context,
    private val scope: CoroutineScope,
    /** Pure callback-entry snapshot; reading the Core field does not enter native code. */
    private val ownerSnapshot: () -> NectarlinkNode?,
    /** Resolves the accepted tuple and uses only this callback's bound Node. */
    private val owner: (NectarlinkNode?, String, UInt, ULong, ULong, ULong) -> Pair<NectarlinkNode, PhoneControlContext>?,
    private val currentNode: (NectarlinkNode) -> Boolean,
    /** A PC's name. */
    private val nameOf: (String) -> String,
    /** Retires only local UI state after this exact qualification offer ends. */
    private val onSettingsQualificationEnded: (String, ULong) -> Unit = { _, _ -> },
    /** Rechecks the exact live Node, capture owner, and phone-owned offer. */
    private val settingsQualificationCurrent: (NectarlinkNode, String, ULong, PhoneControlContext) -> Boolean = { _, _, _, _ -> false },
    /** Persistent owner allowance for ordinary owned apps on this exact Node and peer. */
    private val wayModeCurrent: (NectarlinkNode, String, PhoneControlContext) -> Boolean = { _, _, _ -> false },
) {
    private val context = context.applicationContext

    private class Window(
        val pc: String,
        val session: UInt,
        val socket: Socket,
        @Volatile var width: Int,
        @Volatile var height: Int,
        val dpi: Int,
        val maxSize: Int,
        val node: NectarlinkNode,
        val owner: PhoneControlContext,
        val settingsOffer: ULong?,
        val current: () -> Boolean,
    ) {
        @Volatile var display: Int = -1
        var resizeRevision: Long = 0
        lateinit var pump: AppWindowPacketPump
    }

    private val windows = ConcurrentHashMap<Pair<String, UInt>, Window>()

    /** What each PC asks of the phone while its windows are open: (stay awake, screen off). */
    private val power = ConcurrentHashMap<String, Pair<Boolean, Boolean>>()

    init {
        instance = this
    }

    /** Runs the PC app on a display of its own and streams it; false if it cannot. */
    fun open(pc: String, options: MirrorOptions, connection: ULong, capture: ULong): Boolean =
        appWindowAdmission(ownerSnapshot) { receivingNode ->
            openInternal(receivingNode, pc, options, connection, capture, settingsOffer = null)
        }

    /** The only qualified provider is the existing Elevated virtual-display app path. */
    fun settingsWindowQualificationSupported(): Boolean =
        Build.VERSION.SDK_INT >= Build.VERSION_CODES.R && Elevated.running

    /** Fixed, phone-owned Settings launch; no caller-selected package or power/audio flags. */
    fun openSettingsQualification(
        pc: String,
        options: MirrorOptions,
        connection: ULong,
        capture: ULong,
        offer: ULong,
    ): Boolean {
        val valid = settingsWindowQualificationSupported() && offer != 0uL &&
            connection != 0uL && capture != 0uL &&
            options.app == SETTINGS_PACKAGE && options.session != 0u && options.request != null && options.request != 0uL &&
            options.maxSize == 1920u && options.fps == 60u && options.bitrate == 8_000_000u &&
            !options.audio && !options.stayAwake && !options.screenOff
        // A Rust Platform callback may still be on its native call stack. Bind
        // Core's Node before scheduling, then do every owner/Node check and the
        // launch on one managed worker. Return only its completed result.
        val opened = valid && appWindowAdmission(ownerSnapshot) { receivingNode ->
            openInternal(receivingNode, pc, options, connection, capture, settingsOffer = offer)
        }
        if (!opened) onSettingsQualificationEnded(pc, offer)
        return opened
    }

    private fun openInternal(
        receivingNode: NectarlinkNode?,
        pc: String,
        options: MirrorOptions,
        connection: ULong,
        capture: ULong,
        settingsOffer: ULong?,
    ): Boolean {
        val pkg = options.app ?: return false
        val request = options.request ?: return false
        if (pkg == context.packageName || options.session == 0u || (settingsOffer != null && !settingsWindowQualificationSupported())) return false
        val (node, accepted) = owner(receivingNode, pc, options.session, request, connection, capture) ?: return false
        val current = {
            runCatching {
                currentNode(node) && node.mirrorCaptureCurrent(pc, accepted) &&
                    (settingsOffer?.let { settingsWindowQualificationSupported() && settingsQualificationCurrent(node, pc, it, accepted) }
                        ?: wayModeCurrent(node, pc, accepted))
            }.getOrDefault(false)
        }
        if (!current()) return false
        val maxSize = options.maxSize.toInt().coerceIn(320, 1920)
        val (width, height, dpi) = size(maxSize)
        var registered: Window? = null
        val socket = Elevated.openApp(
            pkg, width, height, dpi, options.bitrate.toInt(), options.fps.toInt(),
            stillCurrent = current,
            register = { socket ->
                val window = Window(pc, options.session, socket, width, height, dpi, maxSize, node, accepted, settingsOffer, current)
                window.pump = AppWindowPacketPump(socket, { owns(window) }) {
                    try {
                        window.node.mirrorAppCaptureEnded(window.pc, window.owner)
                    } finally {
                        removeWindow(window)
                    }
                }
                val key = pc to options.session
                var displaced: Window? = null
                var installed = false
                var rejected = false
                // Never hold a shared map monitor while waiting for a window:
                // resize and close serialize through the window monitor, while
                // ConcurrentHashMap compare/replace preserves exact ownership.
                while (!installed && !rejected) {
                    val previous = windows[key]
                    if (previous == null) {
                        if (!current()) {
                            rejected = true
                        } else if (windows.putIfAbsent(key, window) == null) {
                            registered = window
                            installed = true
                        }
                    } else {
                        synchronized(previous) {
                            if (windows[key] === previous) {
                                if (previous.current.invoke() || !current()) {
                                    rejected = true
                                } else if (windows.replace(key, previous, window)) {
                                    registered = window
                                    installed = true
                                    // Revocation and socket close are ordered
                                    // against the old window's final resize write.
                                    runCatching { previous.socket.close() }
                                    displaced = previous
                                }
                            }
                        }
                    }
                }
                displaced?.pump?.finish()
                if (!installed) return@openApp false
                window.pump.start()
                true
            },
        )
        if (socket == null) {
            registered?.let(::close)
            return false
        }
        val window = registered ?: run { socket.close(); return false }
        if (!owns(window)) {
            close(window)
            return false
        }
        power[pc] = if (settingsOffer == null) options.stayAwake to options.screenOff else false to false
        changed()
        scope.launch(Dispatchers.IO) {
            if (!owns(window)) {
                close(window)
                return@launch
            }
            val stream = runCatching { window.node.mirrorOpenOwned(pc, window.owner) }.getOrElse {
                Log.w(TAG, "can't stream an app window", it)
                close(window)
                return@launch
            }
            if (!owns(window)) {
                stream.end()
                close(window)
                return@launch
            }
            Thread({ forward(window, stream) }, "app-window").start()
        }
        return true
    }

    private fun owns(window: Window): Boolean =
        windows[window.pc to window.session] === window && window.current()

    /** The helper's packets to the PC, until either side closes. */
    private fun forward(window: Window, stream: MirrorStream) {
        window.pump.forward(send = sendPacket@ { packet ->
            if (!owns(window)) return@sendPacket false
            val result = when (packet.kind) {
                AppDisplay.KIND_CONFIG -> {
                    val fields = java.nio.ByteBuffer.wrap(packet.data)
                    val (w, h) = fields.int to fields.int
                    window.width = w
                    window.height = h
                    window.display = fields.int
                    stream.send(VideoPacketKind.CONFIG, 0u, mirrorConfig(w.toUInt(), h.toUInt(), window.session, window.owner.request))
                }
                AppDisplay.KIND_KEYFRAME -> stream.send(VideoPacketKind.KEYFRAME, packet.time, packet.data)
                else -> stream.send(VideoPacketKind.FRAME, packet.time, packet.data)
            }
            when (result) {
                MirrorSendResult.CLOSED -> false
                MirrorSendResult.NEED_KEYFRAME, MirrorSendResult.DROPPED -> { keyframe(window); true }
                MirrorSendResult.QUEUED -> true
            }
        }, end = { stream.end() })
    }

    /** The PC closed a window. */
    fun close(pc: String, session: UInt, request: ULong, connection: ULong, capture: ULong) {
        val window = windows[pc to session] ?: return
        appWindowAdmission({ window }) { selected ->
            if (matches(selected, request, connection, capture)) close(selected)
        }
    }

    private fun close(window: Window) {
        window.pump.finish()
    }

    private fun removeWindow(window: Window) {
        val removed = synchronized(window) {
            // Exact-owner revocation and socket closure share the monitor used
            // by resize's final ownership check and command write.
            val removed = windows.remove(window.pc to window.session, window)
            runCatching { window.socket.close() }
            removed
        }
        if (removed) {
            changed()
            window.settingsOffer?.let { onSettingsQualificationEnded(window.pc, it) }
        }
    }

    fun closeAll() {
        windows.values.toList().forEach(::close)
    }

    /** An explicit Way Mode OFF closes only ordinary owned app windows for this PC. */
    fun closeForPc(pc: String) {
        windows.values.filter { it.pc == pc && it.settingsOffer == null }.forEach(::close)
    }

    /** Owned keyframes require the exact accepted capture context. */
    fun keyframeOwned(pc: String, owner: PhoneControlContext) {
        val window = windows[pc to owner.session] ?: return
        appWindowAdmission({ window }) { selected ->
            if (matches(selected, owner) && owns(selected)) keyframe(selected)
        }
    }

    private fun matches(window: Window, owner: PhoneControlContext): Boolean =
        matches(window, owner.request, owner.connection, owner.capture)

    private fun matches(window: Window, request: ULong, connection: ULong, capture: ULong): Boolean =
        window.owner.request == request && window.owner.connection == connection && window.owner.capture == capture

    private fun keyframe(window: Window) {
        runCatching {
            synchronized(window) {
                if (!owns(window) || window.socket.isClosed) return@synchronized
                window.socket.getOutputStream().apply {
                    write("K\n".toByteArray())
                    flush()
                }
            }
        }
    }

    /** The PC resized an app window: fit to encoder limits and tell the helper. */
    fun resize(pc: String, owner: PhoneControlContext, width: Int, height: Int) {
        val window = windows[pc to owner.session] ?: return
        appWindowAdmission({ window }) { selected -> resizeOwned(selected, owner, width, height) }
    }

    private fun resizeOwned(window: Window, owner: PhoneControlContext, width: Int, height: Int) {
        if (window.owner.request != owner.request || window.owner.connection != owner.connection || window.owner.capture != owner.capture) return
        val (phoneW, phoneH) = screen()
        val (newW, newH) = WindowShape.fit(width, height, window.maxSize, phoneW, phoneH)
        val revision = synchronized(window) {
            if (!owns(window) || (newW == window.width && newH == window.height)) return
            window.width = newW
            window.height = newH
            ++window.resizeRevision
        }
        runCatching {
            synchronized(window) {
                // A newer resize may have arrived while this one waited for the
                // socket lock. Only the latest dimensions for this exact window
                // may be emitted, and replacement/close revokes ownership first.
                if (window.resizeRevision != revision || !owns(window) || window.socket.isClosed) return@synchronized
                window.socket.getOutputStream().apply {
                    write("R $newW $newH ${window.dpi}\n".toByteArray())
                    flush()
                }
            }
        }
    }

    /** The PC's mouse and keyboard on a window. */
    fun input(pc: String, owner: PhoneControlContext, event: MirrorInputEvent, admitted: () -> Boolean) {
        val window = windows[pc to owner.session] ?: return
        appWindowAdmission({ window }) { selected -> inputOwned(selected, owner, event, admitted) }
    }

    private fun inputOwned(
        window: Window,
        owner: PhoneControlContext,
        event: MirrorInputEvent,
        admitted: () -> Boolean,
    ) {
        if (window.owner.request != owner.request || window.owner.connection != owner.connection || window.owner.capture != owner.capture) return
        // Every owned app callback runs on the managed admission worker. The
        // legacy Settings qualification keeps its scroll-only restriction;
        // ordinary Way Mode permits tap/scroll/text on this owned display.
        if (window.settingsOffer != null) {
            if (event !is MirrorInputEvent.Scroll) return
        } else {
            // Ordinary display-scoped app input cannot navigate the physical phone or use gamepad.
            if (event is MirrorInputEvent.Gamepad) return
            if (event is MirrorInputEvent.Key && event.key in listOf("home", "recents", "notifications", "keyboard_on", "keyboard_off")) return
        }
        val current = { admitted() && owns(window) && window.display > 0 }
        if (!current()) return
        Elevated.handleOn(window.display, window.width.toFloat(), window.height.toFloat(), event, current)
    }

    /** The phone's screen, in pixels. */
    private fun screen(): Pair<Int, Int> {
        val windows = context.getSystemService(WindowManager::class.java)
        return if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.R) {
            windows.maximumWindowMetrics.bounds.let { it.width() to it.height() }
        } else {
            val metrics = android.util.DisplayMetrics()
            @Suppress("DEPRECATION")
            windows.defaultDisplay.getRealMetrics(metrics)
            metrics.widthPixels to metrics.heightPixels
        }
    }

    /** The phone's screen shape scaled to fit [maxSize] (see [WindowShape]), upright, and its density likewise. */
    private fun size(maxSize: Int): Triple<Int, Int, Int> {
        val (w, h) = screen()
        val (shortSide, longSide) = minOf(w, h) to maxOf(w, h)
        val (fitW, fitH) = WindowShape.fit(shortSide, longSide, maxSize.coerceAtLeast(320), w, h)
        val scale = fitH.toFloat() / longSide
        val dpi = (context.resources.displayMetrics.densityDpi * scale).roundToInt().coerceIn(120, 640)
        return Triple(fitW, fitH, dpi)
    }

    /** The PC turned keeping the phone awake, or its screen off, on or off. */
    fun hasSettingsQualification(pc: String): Boolean =
        windows.values.any { it.pc == pc && it.settingsOffer != null }

    fun setPower(pc: String, stayAwake: Boolean, screenOff: Boolean) {
        if (windows.values.none { it.pc == pc } || hasSettingsQualification(pc)) return
        power[pc] = stayAwake to screenOff
        applyPower()
    }

    private fun applyPower() {
        val open = windows.values.map { it.pc }.toSet()
        power.keys.retainAll(open)
        val wants = power.values
        StreamPower.set(context, StreamPower.APPS, open.isNotEmpty(), wants.any { it.first }, wants.any { it.second })
    }

    private fun changed() {
        applyPower()
        val manager = NotificationManagerCompat.from(context)
        if (windows.isEmpty()) {
            manager.cancel(NOTIFICATION_ID)
            return
        }
        val allowed = Build.VERSION.SDK_INT < Build.VERSION_CODES.TIRAMISU ||
            context.checkSelfPermission(android.Manifest.permission.POST_NOTIFICATIONS) == PackageManager.PERMISSION_GRANTED
        if (!allowed) return
        MirrorService.createChannel(context)
        val pcs = windows.values.map { it.pc }.distinct().map(nameOf).filter { it.isNotEmpty() }
        val closeAll = PendingIntent.getBroadcast(
            context, 0, Intent(context, CloseReceiver::class.java),
            PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
        )
        val notification = NotificationCompat.Builder(context, MirrorService.CHANNEL)
            .setSmallIcon(R.drawable.ic_notification)
            .setContentTitle(
                context.resources.getQuantityString(R.plurals.app_windows_title, windows.size, windows.size),
            )
            .setContentText(
                context.getString(R.string.app_windows_text, pcs.joinToString().ifEmpty { context.getString(R.string.your_pc) }),
            )
            .setOngoing(true)
            .setSilent(true)
            .setCategory(NotificationCompat.CATEGORY_SERVICE)
            .addAction(0, context.getString(R.string.action_close_all), closeAll)
            .build()
        try {
            manager.notify(NOTIFICATION_ID, notification)
        } catch (e: SecurityException) {
            Log.i(TAG, "can't say the app windows are open", e)
        }
    }

    /** The notification's Close all. */
    class CloseReceiver : BroadcastReceiver() {
        override fun onReceive(context: Context, intent: Intent) {
            instance?.closeAll()
        }
    }

    companion object {
        private const val TAG = "AppWindows"
        private const val NOTIFICATION_ID = 7
        private const val SETTINGS_PACKAGE = "com.android.settings"

        @Volatile private var instance: AppWindows? = null

        /** The apps a PC may open in windows: launchable ones, by name, with small icons. */
        fun list(context: Context): List<PhoneApp> {
            val manager = context.packageManager
            val launcher = Intent(Intent.ACTION_MAIN).addCategory(Intent.CATEGORY_LAUNCHER)
            val found = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
                manager.queryIntentActivities(launcher, PackageManager.ResolveInfoFlags.of(0))
            } else {
                @Suppress("DEPRECATION")
                manager.queryIntentActivities(launcher, 0)
            }
            val used = HashSet<String>()
            val apps = found
                .map { it.activityInfo }
                .filter { it.packageName != context.packageName }
                .distinctBy { it.packageName }
                .map { info ->
                    PhoneApp(
                        pkg = info.packageName,
                        label = info.loadLabel(manager).toString().trim().ifEmpty { info.packageName },
                        icon = cachedIcon(info, manager, used),
                    )
                }
                .sortedBy { it.label.lowercase() }
            // Forget uninstalled and updated apps' icons.
            icons.keys.retainAll(used)
            return apps
        }

        /**
         * Icons drawn for an earlier list, by package and its installed APK
         * (a new path after every update) and icon: drawing and compressing
         * every icon again made every list slow to arrive.
         */
        private val icons = java.util.concurrent.ConcurrentHashMap<String, ByteArray>()
        private val NO_ICON = ByteArray(0)

        private fun cachedIcon(
            info: android.content.pm.ActivityInfo,
            manager: PackageManager,
            used: MutableSet<String>,
        ): ByteArray? {
            val key = "${info.packageName}|${info.applicationInfo?.sourceDir}|${info.iconResource}"
            used += key
            val png = icons.getOrPut(key) { runCatching { icon(info.loadIcon(manager)) }.getOrNull() ?: NO_ICON }
            return png.takeIf { it.isNotEmpty() }
        }

        /** A PNG of the icon: 64 pixels, or 48 when that's too big. */
        private fun icon(drawable: android.graphics.drawable.Drawable): ByteArray? {
            for (size in intArrayOf(64, 48)) {
                val bitmap = Bitmap.createBitmap(size, size, Bitmap.Config.ARGB_8888)
                drawable.setBounds(0, 0, size, size)
                drawable.draw(Canvas(bitmap))
                val png = ByteArrayOutputStream().also { bitmap.compress(Bitmap.CompressFormat.PNG, 100, it) }.toByteArray()
                bitmap.recycle()
                if (png.size <= MAX_ICON_BYTES) return png
            }
            return null
        }

        private const val MAX_ICON_BYTES = 8 * 1024
    }
}
