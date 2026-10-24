// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.elevated

import android.annotation.SuppressLint
import android.content.Context
import android.content.ContextWrapper
import android.hardware.display.DisplayManager
import android.hardware.display.VirtualDisplay
import android.media.MediaCodec
import android.media.MediaCodecInfo
import android.media.MediaFormat
import android.os.Build
import android.os.Looper
import android.view.Surface
import java.io.DataOutputStream
import java.io.OutputStream

internal class PendingAppDisplaySize {
    private var pending: Triple<Int, Int, Int>? = null

    @Synchronized
    fun submit(size: Triple<Int, Int, Int>) {
        pending = size
    }

    @Synchronized
    fun take(): Triple<Int, Int, Int>? {
        val latest = pending
        pending = null
        return latest
    }

    @Synchronized
    fun isPending(): Boolean = pending != null
}

/**
 * Task visibility can change because Android places the keyguard over a still-owned app display.
 * Only that exact hidden task may be retained; an absent task still ends the request.
 */
internal enum class AppDisplayTaskState { VISIBLE, HIDDEN, ABSENT, UNKNOWN }

internal fun classifyAppDisplayTask(identityMatches: Boolean?, childCount: Int?, visible: Boolean?): AppDisplayTaskState = when {
    identityMatches == false -> AppDisplayTaskState.ABSENT
    identityMatches != true || childCount == null -> AppDisplayTaskState.UNKNOWN
    childCount <= 0 -> AppDisplayTaskState.ABSENT
    visible == null -> AppDisplayTaskState.UNKNOWN
    visible -> AppDisplayTaskState.VISIBLE
    else -> AppDisplayTaskState.HIDDEN
}

internal fun shouldKeepAppDisplay(state: AppDisplayTaskState, keyguardLocked: Boolean): Boolean = when (state) {
    AppDisplayTaskState.VISIBLE, AppDisplayTaskState.UNKNOWN -> true
    AppDisplayTaskState.HIDDEN -> keyguardLocked
    AppDisplayTaskState.ABSENT -> false
}

/**
 * An app in a window of its own (Elevated): a virtual display of its own,
 * made by the shell-side helper (apps may not put other apps on their
 * displays), with an H.264 encoder drawing from it, and the app started on
 * it. The encoded video goes to [out] as packets:
 * `u32 length | u8 kind | u64 time µs | data` (kind 0: config, as
 * `u32 width | u32 height | u32 display id`; 1: frame; 2: keyframe).
 */
@SuppressLint("PrivateApi", "DiscouragedPrivateApi", "BlockedPrivateApi")
class AppDisplay(
    private val width: Int,
    private val height: Int,
    private val dpi: Int,
    private val bitrate: Int,
    private val fps: Int,
    private val codecName: String = "h264",
) {
    @Volatile private var running = true
    private val pendingSize = PendingAppDisplaySize()
    private var display: VirtualDisplay? = null
    /** The exact package-resolver or launcher child for this display's request. */
    private var commandProcess: Process? = null

    /** The display's ID, once made. */
    var displayId: Int = -1
        private set

    /** Streams until [stop] or until [out] fails; starts [pkg] on the display first (or mirrors the main screen when `pkg == "-"`). */
    fun run(pkg: String, out: OutputStream) {
        val fullScreen = pkg == "-"
        var curW = width
        var curH = height
        var curDpi = dpi
        var codec: MediaCodec? = null
        var surface: Surface? = null
        val data = DataOutputStream(out.buffered(1 shl 16))
        try {
            val made = synchronized(this) {
                // EOF/Stop may win before this helper thread starts running.
                if (!running) return
                val (firstCodec, firstSurface) = createEncoder(curW, curH)
                codec = firstCodec
                surface = firstSurface
                if (fullScreen) {
                    createMirrorDisplay(curW, curH, curDpi, firstSurface)
                } else {
                    createDisplay(curW, curH, curDpi, firstSurface)
                }
            }
            val firstCodec = checkNotNull(codec)
            display = made
            displayId = if (fullScreen) 0 else made.display.displayId
            if (!fullScreen) {
                hideIme(displayId)
            }
            firstCodec.start()
            codecForKeyframes = firstCodec
            write(data, KIND_CONFIG, 0, ByteArray(12).also {
                java.nio.ByteBuffer.wrap(it).putInt(curW).putInt(curH).putInt(displayId)
            })
            data.flush()
            if (!fullScreen) {
                launch(pkg, displayId)
                if (!running) return
                watchActivities(displayId, pkg)
                keepActive(displayId)
            }
            while (running) {
                val activeCodec = codec ?: break
                drain(activeCodec, data)
                if (!running) break
                val (newW, newH, newDpi) = pendingSize.take() ?: continue
                if (newW == curW && newH == curH && newDpi == curDpi) continue
                codecForKeyframes = null
                runCatching { activeCodec.stop() }
                runCatching { activeCodec.release() }
                codec = null
                runCatching { surface?.release() }
                surface = null
                val (nextCodec, nextSurface) = createEncoder(newW, newH)
                codec = nextCodec
                surface = nextSurface
                nextCodec.start()
                codecForKeyframes = nextCodec
                made.resize(newW, newH, newDpi)
                made.surface = nextSurface
                curW = newW
                curH = newH
                curDpi = newDpi
                write(data, KIND_CONFIG, 0, ByteArray(12).also {
                    java.nio.ByteBuffer.wrap(it).putInt(curW).putInt(curH).putInt(displayId)
                })
                data.flush()
            }
        } finally {
            running = false
            codecForKeyframes = null
            runCatching { codec?.stop() }
            runCatching { codec?.release() }
            runCatching { surface?.release() }
            display?.release()
            display = null
        }
    }

    @Synchronized
    fun stop() {
        running = false
        // InputServer calls stop on this exact AppDisplay when its owner socket ends.
        // Do not leave a shell `am start` running past that request's lifetime.
        runCatching { commandProcess?.destroyForcibly() }
    }

    fun resize(w: Int, h: Int, newDpi: Int) {
        pendingSize.submit(Triple(w, h, newDpi))
    }

    @Volatile private var codecForKeyframes: MediaCodec? = null

    /** The PC's decoder lost its place: a keyframe next. */
    fun requestKeyframe() {
        val codec = codecForKeyframes ?: return
        runCatching {
            codec.setParameters(android.os.Bundle().apply { putInt(MediaCodec.PARAMETER_KEY_REQUEST_SYNC_FRAME, 0) })
        }
    }

    private fun mimeForCodec(name: String): String = when (name.lowercase()) {
        "h265", "hevc" -> MediaFormat.MIMETYPE_VIDEO_HEVC
        "av1" -> if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) "video/av01" else MediaFormat.MIMETYPE_VIDEO_AVC
        else -> MediaFormat.MIMETYPE_VIDEO_AVC
    }

    private fun createEncoderForMime(mime: String, w: Int, h: Int): Pair<MediaCodec, Surface> {
        val format = MediaFormat.createVideoFormat(mime, w, h).apply {
            setInteger(MediaFormat.KEY_COLOR_FORMAT, MediaCodecInfo.CodecCapabilities.COLOR_FormatSurface)
            setInteger(MediaFormat.KEY_BIT_RATE, bitrate)
            setInteger(MediaFormat.KEY_FRAME_RATE, fps)
            setInteger(MediaFormat.KEY_I_FRAME_INTERVAL, 10)
            // Frames only when the picture changes; repeat now and then so a
            // still app still reaches a PC that just joined.
            setLong(MediaFormat.KEY_REPEAT_PREVIOUS_FRAME_AFTER, 100_000)
            setInteger(MediaFormat.KEY_COLOR_STANDARD, MediaFormat.COLOR_STANDARD_BT709)
            setInteger(MediaFormat.KEY_COLOR_RANGE, MediaFormat.COLOR_RANGE_LIMITED)
            setInteger(MediaFormat.KEY_PRIORITY, 0)
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {
                setInteger(MediaFormat.KEY_MAX_B_FRAMES, 0)
            }
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.R) {
                setInteger(MediaFormat.KEY_LATENCY, 1)
            }
        }
        val codec = MediaCodec.createEncoderByType(mime)
        codec.configure(format, null, null, MediaCodec.CONFIGURE_FLAG_ENCODE)
        return codec to codec.createInputSurface()
    }

    private fun createEncoder(w: Int, h: Int): Pair<MediaCodec, Surface> {
        val preferredMime = mimeForCodec(codecName)
        if (preferredMime != MediaFormat.MIMETYPE_VIDEO_AVC) {
            runCatching { return createEncoderForMime(preferredMime, w, h) }
        }
        return createEncoderForMime(MediaFormat.MIMETYPE_VIDEO_AVC, w, h)
    }

    private fun drain(codec: MediaCodec, out: DataOutputStream) {
        val info = MediaCodec.BufferInfo()
        var parameters = ByteArray(0)
        while (running && !pendingSize.isPending()) {
            val index = codec.dequeueOutputBuffer(info, 50_000)
            if (index < 0) continue
            val buffer = codec.getOutputBuffer(index)
            if (buffer != null && info.size > 0) {
                val bytes = ByteArray(info.size)
                buffer.position(info.offset)
                buffer.get(bytes)
                when {
                    info.flags and MediaCodec.BUFFER_FLAG_CODEC_CONFIG != 0 -> parameters = bytes
                    info.flags and MediaCodec.BUFFER_FLAG_KEY_FRAME != 0 -> {
                        write(out, KIND_KEYFRAME, info.presentationTimeUs, parameters + bytes)
                        out.flush()
                    }
                    else -> {
                        write(out, KIND_FRAME, info.presentationTimeUs, bytes)
                        out.flush()
                    }
                }
            }
            codec.releaseOutputBuffer(index, false)
            if (info.flags and MediaCodec.BUFFER_FLAG_END_OF_STREAM != 0) {
                running = false
                return
            }
        }
    }

    private fun write(out: DataOutputStream, kind: Int, timeUs: Long, bytes: ByteArray) {
        out.writeInt(1 + 8 + bytes.size)
        out.writeByte(kind)
        out.writeLong(timeUs)
        out.write(bytes)
    }

    private fun createMirrorDisplay(w: Int, h: Int, d: Int, surface: Surface): VirtualDisplay {
        // 1. Android 11+ hidden static helper DisplayManager.createVirtualDisplay(name, width, height, displayIdToMirror, surface)
        runCatching {
            val m = DisplayManager::class.java.getMethod(
                "createVirtualDisplay",
                String::class.java,
                Int::class.javaPrimitiveType,
                Int::class.javaPrimitiveType,
                Int::class.javaPrimitiveType,
                Surface::class.java,
            )
            val vd = m.invoke(null, "NectarlinkScreen", w, h, 0, surface) as? VirtualDisplay
            if (vd != null) return vd
        }
        // 2. Shell DisplayManager instance with VIRTUAL_DISPLAY_FLAG_AUTO_MIRROR
        val constructor = DisplayManager::class.java.getDeclaredConstructor(Context::class.java)
        constructor.isAccessible = true
        val manager = constructor.newInstance(ShellContext.get())
        @SuppressLint("WrongConstant")
        return runCatching {
            manager.createVirtualDisplay(
                "NectarlinkScreen",
                w,
                h,
                d,
                surface,
                PUBLIC or DisplayManager.VIRTUAL_DISPLAY_FLAG_AUTO_MIRROR or DisplayManager.VIRTUAL_DISPLAY_FLAG_PRESENTATION,
            )
        }.getOrElse {
            manager.createVirtualDisplay(
                "NectarlinkScreen",
                w,
                h,
                d,
                surface,
                DisplayManager.VIRTUAL_DISPLAY_FLAG_AUTO_MIRROR,
            )
        }
    }

    private fun createDisplay(w: Int, h: Int, d: Int, surface: Surface): VirtualDisplay {
        var flags = PUBLIC or OWN_CONTENT_ONLY or SUPPORTS_TOUCH or ROTATES_WITH_CONTENT or DESTROY_CONTENT_ON_REMOVAL
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
            flags = flags or TRUSTED or OWN_DISPLAY_GROUP or ALWAYS_UNLOCKED or TOUCH_FEEDBACK_DISABLED
        }
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.UPSIDE_DOWN_CAKE) {
            flags = flags or OWN_FOCUS or DEVICE_DISPLAY_GROUP
        }
        val constructor = DisplayManager::class.java.getDeclaredConstructor(Context::class.java)
        constructor.isAccessible = true
        val manager = constructor.newInstance(ShellContext.get())
        // Some of the flags are hidden ones (TRUSTED, OWN_FOCUS...), which
        // the shell may use.
        @SuppressLint("WrongConstant")
        return manager.createVirtualDisplay("Nectarlink", w, h, d, surface, flags)
    }

    /** Hides the soft keyboard on this virtual display so typing from the PC never pops it up on the phone. */
    private fun hideIme(display: Int) {
        val hidden = runCatching {
            val wmg = Class.forName("android.view.WindowManagerGlobal")
            val wm = wmg.getMethod("getWindowManagerService").invoke(null) ?: return@runCatching false
            wm.javaClass
                .getMethod("setDisplayImePolicy", Int::class.javaPrimitiveType, Int::class.javaPrimitiveType)
                .invoke(wm, display, DISPLAY_IME_POLICY_HIDE)
            true
        }.getOrDefault(false)
        if (!hidden) {
            runCatching {
                Runtime.getRuntime().exec(arrayOf("wm", "set-display-ime-policy", display.toString(), "hide")).waitFor()
            }
        }
    }

    private fun launch(pkg: String, display: Int) {
        // By its launcher activity's name: a package-only launcher intent isn't resolved on some
        // phones (Samsung's One UI answers "intent not resolved"), and nothing would open.
        val resolved = launcherActivity(pkg)
        // The resolver may have been canceled with this window; don't run a fallback launch.
        if (!running || Thread.currentThread().isInterrupted) return
        val target = resolved?.let { arrayOf("-n", it) }
            ?: arrayOf("-a", "android.intent.action.MAIN", "-c", "android.intent.category.LAUNCHER", "-p", pkg)
        val command = arrayOf("am", "start", "--user", "0", "--display", display.toString(), "--windowingMode", "1") + target +
            arrayOf("-f", (FLAG_NEW_TASK or FLAG_MULTIPLE_TASK).toString())
        val said = runCommand(command) ?: return
        if (said.contains("Error", ignoreCase = true) || said.contains("Warning", ignoreCase = true)) {
            System.err.println("launching $pkg: ${said.trim().lines().joinToString(" | ")}")
        }
    }

    /** Run one shell child owned by this display request; stop() cancels its exact process. */
    private fun runCommand(command: Array<String>): String? {
        val process = synchronized(this) {
            if (!running) return null
            ProcessBuilder(*command).redirectErrorStream(true).start().also { commandProcess = it }
        }
        var interrupted = false
        try {
            val output = readCommandOutputBounded(process)
            process.waitFor()
            return output.takeIf { running }
        } catch (e: InterruptedException) {
            interrupted = true
            throw e
        } finally {
            try {
                // Keep this exact process owned until its termination is observed.
                ensureCommandExited(process)
            } finally {
                synchronized(this) {
                    if (commandProcess === process) commandProcess = null
                }
                if (interrupted) Thread.currentThread().interrupt()
            }
        }
    }

    /** Drain merged output fully but retain only a bounded tail for resolver parsing and diagnostics. */
    private fun readCommandOutputBounded(process: Process): String {
        val retained = StringBuilder()
        val buffer = CharArray(1024)
        process.inputStream.bufferedReader().use { reader ->
            while (true) {
                val read = reader.read(buffer)
                if (read < 0) break
                val overflow = retained.length + read - MAX_COMMAND_OUTPUT_CHARS
                if (overflow > 0) retained.delete(0, overflow)
                retained.append(buffer, 0, read)
            }
        }
        return retained.toString()
    }

    /** Destroy on failure/stop and wait uninterruptibly before dropping this process owner. */
    private fun ensureCommandExited(process: Process) {
        var interrupted = Thread.interrupted()
        try {
            if (process.isAlive) runCatching { process.destroyForcibly() }
            while (process.isAlive) {
                try {
                    process.waitFor()
                } catch (_: InterruptedException) {
                    interrupted = true
                    runCatching { process.destroyForcibly() }
                }
            }
        } finally {
            if (interrupted) Thread.currentThread().interrupt()
        }
    }

    /** `pkg/.Activity` for the package's launcher activity, or null. */
    private fun launcherActivity(pkg: String): String? = runCatching {
        val output = runCommand(
            arrayOf(
                "cmd", "package", "resolve-activity", "--brief", "--user", "0",
                "-a", "android.intent.action.MAIN", "-c", "android.intent.category.LAUNCHER", pkg,
            ),
        ) ?: return@runCatching null
        output.lineSequence().lastOrNull { it.isNotBlank() }?.trim()?.takeIf { it.startsWith("$pkg/") }
    }.getOrNull()

    /**
     * Tells Android there is activity on [display] every few seconds while
     * the window is open. Otherwise the phone sleeps from inactivity, which
     * stops the app and freezes its window on the PC. The phone's own screen
     * still dims as usual (scrcpy's `--keep-active` does the same).
     */
    private fun keepActive(display: Int) {
        val signal = UserActivity.forDisplay(display) ?: return
        Thread({
            try {
                while (running && signal()) {
                    Thread.sleep(KEEP_ACTIVE_MS)
                }
            } catch (_: InterruptedException) {
            }
        }, "app-display-active").apply {
            isDaemon = true
            start()
        }
    }

    /** Stops only after the exact app task has disappeared; keyguard visibility alone is not removal. */
    private fun watchActivities(display: Int, pkg: String) {
        Thread({
            try {
                Thread.sleep(1_500)
                var ownedTaskId: Int? = null
                var seen = false
                var emptyCount = 0
                var ticks = 0
                while (running) {
                    val observation = activityOnDisplay(display, pkg, ownedTaskId)
                    val locked = observation.state == AppDisplayTaskState.HIDDEN && keyguardIsLocked()
                    if (observation.state == AppDisplayTaskState.VISIBLE) {
                        ownedTaskId = observation.taskId
                        seen = true
                        emptyCount = 0
                    } else if (shouldKeepAppDisplay(observation.state, locked)) {
                        emptyCount = 0
                    } else if (seen || ticks >= 15) {
                        emptyCount++
                        if (emptyCount >= 2) {
                            System.err.println("the app on display $display closed")
                            running = false
                            break
                        }
                    }
                    ticks++
                    Thread.sleep(1_000)
                }
            } catch (_: InterruptedException) {
            }
        }, "app-display-watch").apply {
            isDaemon = true
            start()
        }
    }

    private data class ActivityTaskObservation(val state: AppDisplayTaskState, val taskId: Int? = null)

    /** The task is tied to this app-display request, not merely any visible task on the display. */
    @SuppressLint("PrivateApi", "DiscouragedPrivateApi")
    private fun activityOnDisplay(display: Int, pkg: String, ownedTaskId: Int?): ActivityTaskObservation = runCatching {
        val service = Class.forName("android.app.ActivityTaskManager").getMethod("getService").invoke(null)
            ?: return@runCatching ActivityTaskObservation(AppDisplayTaskState.UNKNOWN)
        val tasks = service.javaClass
            .getMethod("getAllRootTaskInfosOnDisplay", Int::class.javaPrimitiveType)
            .invoke(service, display) as? List<*>
            ?: return@runCatching ActivityTaskObservation(AppDisplayTaskState.UNKNOWN)

        fun taskId(task: Any): Int? = runCatching { task.javaClass.getField("taskId").getInt(task) }.getOrNull()
        fun childCount(task: Any): Int? = runCatching {
            (task.javaClass.getField("childTaskIds").get(task) as IntArray).size
        }.getOrNull()
        fun visible(task: Any): Boolean? = runCatching { task.javaClass.getField("visible").getBoolean(task) }.getOrNull()
        fun packages(task: Any): Set<String>? {
            val names = listOf("topActivity", "baseActivity", "rootActivity").mapNotNull { field ->
                runCatching { (task.javaClass.getField(field).get(task) as? android.content.ComponentName)?.packageName }.getOrNull()
            }.toSet()
            return names.takeIf { it.isNotEmpty() }
        }

        if (ownedTaskId != null) {
            for (task in tasks) {
                if (task == null) return@runCatching ActivityTaskObservation(AppDisplayTaskState.UNKNOWN)
                val id = taskId(task) ?: return@runCatching ActivityTaskObservation(AppDisplayTaskState.UNKNOWN)
                if (id == ownedTaskId) {
                    val children = childCount(task) ?: return@runCatching ActivityTaskObservation(AppDisplayTaskState.UNKNOWN)
                    val noChildren = classifyAppDisplayTask(identityMatches = true, childCount = children, visible = null)
                    if (noChildren == AppDisplayTaskState.ABSENT) return@runCatching ActivityTaskObservation(noChildren)
                    val isVisible = visible(task) ?: return@runCatching ActivityTaskObservation(AppDisplayTaskState.UNKNOWN)
                    val state = classifyAppDisplayTask(identityMatches = true, childCount = children, visible = isVisible)
                    if (state == AppDisplayTaskState.UNKNOWN) return@runCatching ActivityTaskObservation(state)
                    val packageNames = packages(task) ?: return@runCatching ActivityTaskObservation(AppDisplayTaskState.UNKNOWN)
                    if (pkg !in packageNames) return@runCatching ActivityTaskObservation(AppDisplayTaskState.ABSENT)
                    return@runCatching ActivityTaskObservation(state, id)
                }
            }
            return@runCatching ActivityTaskObservation(
                classifyAppDisplayTask(identityMatches = false, childCount = null, visible = null),
            )
        }

        for (task in tasks) {
            if (task == null) return@runCatching ActivityTaskObservation(AppDisplayTaskState.UNKNOWN)
            val id = taskId(task) ?: return@runCatching ActivityTaskObservation(AppDisplayTaskState.UNKNOWN)
            val children = childCount(task) ?: return@runCatching ActivityTaskObservation(AppDisplayTaskState.UNKNOWN)
            if (children == 0) continue
            val isVisible = visible(task) ?: return@runCatching ActivityTaskObservation(AppDisplayTaskState.UNKNOWN)
            val packageNames = packages(task) ?: return@runCatching ActivityTaskObservation(AppDisplayTaskState.UNKNOWN)
            val state = classifyAppDisplayTask(identityMatches = pkg in packageNames, childCount = children, visible = isVisible)
            if (state == AppDisplayTaskState.VISIBLE) return@runCatching ActivityTaskObservation(state, id)
        }
        ActivityTaskObservation(AppDisplayTaskState.ABSENT)
    }.getOrDefault(ActivityTaskObservation(AppDisplayTaskState.UNKNOWN))

    private fun keyguardIsLocked(): Boolean = runCatching {
        val keyguard = ShellContext.get().getSystemService(android.app.KeyguardManager::class.java) ?: return false
        keyguard.isKeyguardLocked || keyguard.isDeviceLocked
    }.getOrDefault(false)



    companion object {
        const val KIND_CONFIG = 0
        const val KIND_FRAME = 1
        const val KIND_KEYFRAME = 2

        /** How often an open window counts as activity; well under any screen timeout. */
        private const val KEEP_ACTIVE_MS = 4_000L

        /** WindowManager.DISPLAY_IME_POLICY_HIDE. */
        private const val DISPLAY_IME_POLICY_HIDE = 2

        // DisplayManager.VIRTUAL_DISPLAY_FLAG_* (some hidden).
        private const val PUBLIC = 1
        private const val OWN_CONTENT_ONLY = 1 shl 3
        private const val SUPPORTS_TOUCH = 1 shl 6
        private const val ROTATES_WITH_CONTENT = 1 shl 7
        private const val DESTROY_CONTENT_ON_REMOVAL = 1 shl 8
        private const val TRUSTED = 1 shl 10
        private const val OWN_DISPLAY_GROUP = 1 shl 11
        private const val ALWAYS_UNLOCKED = 1 shl 12
        private const val TOUCH_FEEDBACK_DISABLED = 1 shl 13
        private const val OWN_FOCUS = 1 shl 14
        private const val DEVICE_DISPLAY_GROUP = 1 shl 15

        private const val FLAG_NEW_TASK = 0x10000000
        private const val FLAG_MULTIPLE_TASK = 0x08000000
        private const val MAX_COMMAND_OUTPUT_CHARS = 8 * 1024
    }
}

/**
 * A context for system services in the shell-side helper, which has no app
 * of its own: the system context, speaking as the shell package (scrcpy
 * does the same).
 */
@SuppressLint("PrivateApi", "DiscouragedPrivateApi", "SoonBlockedPrivateApi", "BlockedPrivateApi")
internal class ShellContext private constructor(base: Context) : ContextWrapper(base) {
    override fun getPackageName(): String = SHELL
    override fun getOpPackageName(): String = SHELL
    override fun getApplicationContext(): Context = this

    @Suppress("NewApi")
    override fun getAttributionSource(): android.content.AttributionSource =
        android.content.AttributionSource.Builder(android.os.Process.SHELL_UID).setPackageName(SHELL).build()

    companion object {
        private const val SHELL = "com.android.shell"
        @Volatile private var instance: ShellContext? = null

        @Synchronized
        fun get(): ShellContext = instance ?: ShellContext(systemContext()).also { instance = it }

        private fun systemContext(): Context {
            if (Looper.getMainLooper() == null) {
                @Suppress("DEPRECATION")
                Looper.prepareMainLooper()
            }
            val type = Class.forName("android.app.ActivityThread")
            val constructor = type.getDeclaredConstructor().apply { isAccessible = true }
            val thread = constructor.newInstance()
            type.getDeclaredField("sCurrentActivityThread").apply { isAccessible = true }.set(null, thread)
            // Hidden-API rules are for apps; this runs in the shell's own process.
            runCatching { type.getDeclaredField("mSystemThread").apply { isAccessible = true }.setBoolean(thread, true) }
            fillConfigurationController(type, thread)
            return type.getDeclaredMethod("getSystemContext").invoke(thread) as Context
        }

        /**
         * A bare ActivityThread has no configuration controller (Android 13+). Samsung's
         * display sandboxing asks the thread for its configuration while the system context is
         * made, which then fails. Gives it a controller holding the default configuration.
         */
        private fun fillConfigurationController(type: Class<*>, thread: Any) {
            if (Build.VERSION.SDK_INT < Build.VERSION_CODES.TIRAMISU) return
            runCatching {
                val controllerType = Class.forName("android.app.ConfigurationController")
                val internal = Class.forName("android.app.ActivityThreadInternal")
                val controller = controllerType.getDeclaredConstructor(internal)
                    .apply { isAccessible = true }
                    .newInstance(thread)
                runCatching {
                    controllerType.getDeclaredField("mConfiguration").apply { isAccessible = true }
                        .set(controller, android.content.res.Configuration().apply { setToDefaults() })
                }
                type.getDeclaredField("mConfigurationController").apply { isAccessible = true }.set(thread, controller)
            }.onFailure { System.err.println("no configuration controller: $it") }
        }
    }
}

/** `IPowerManager.userActivity`, which the shell may call (DEVICE_POWER). */
@SuppressLint("PrivateApi", "DiscouragedPrivateApi")
private object UserActivity {
    private val power: Any? by lazy {
        runCatching {
            val binder = Class.forName("android.os.ServiceManager")
                .getMethod("getService", String::class.java)
                .invoke(null, "power") as android.os.IBinder
            Class.forName("android.os.IPowerManager\$Stub")
                .getMethod("asInterface", android.os.IBinder::class.java)
                .invoke(null, binder)
        }.onFailure { System.err.println("no power manager: $it") }.getOrNull()
    }

    /**
     * A call that counts as activity on [display] (false once Android refuses
     * it), or null if there is none. Before Android 12 activity is for the
     * whole device.
     */
    fun forDisplay(display: Int): (() -> Boolean)? {
        val manager = power ?: return null
        val perDisplay = Build.VERSION.SDK_INT >= Build.VERSION_CODES.S
        val int = Int::class.javaPrimitiveType
        val long = Long::class.javaPrimitiveType
        val method = runCatching {
            if (perDisplay) {
                manager.javaClass.getMethod("userActivity", int, long, int, int)
            } else {
                manager.javaClass.getMethod("userActivity", long, int, int)
            }
        }.onFailure { System.err.println("no userActivity: $it") }.getOrNull() ?: return null
        return {
            val now = android.os.SystemClock.uptimeMillis()
            runCatching {
                if (perDisplay) {
                    method.invoke(manager, display, now, EVENT_OTHER, NO_CHANGE_LIGHTS)
                } else {
                    method.invoke(manager, now, EVENT_OTHER, NO_CHANGE_LIGHTS)
                }
            }.onFailure { System.err.println("user activity: $it") }.isSuccess
        }
    }

    /** PowerManager.USER_ACTIVITY_EVENT_OTHER. */
    private const val EVENT_OTHER = 0

    /** PowerManager.USER_ACTIVITY_FLAG_NO_CHANGE_LIGHTS: the screen may still dim. */
    private const val NO_CHANGE_LIGHTS = 1
}
