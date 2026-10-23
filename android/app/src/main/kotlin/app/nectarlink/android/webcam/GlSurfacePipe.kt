// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.webcam

import android.graphics.SurfaceTexture
import android.opengl.EGL14
import android.opengl.EGLConfig
import android.opengl.EGLContext
import android.opengl.EGLDisplay
import android.opengl.EGLExt
import android.opengl.EGLSurface
import android.opengl.GLES11Ext
import android.opengl.GLES20
import android.os.Handler
import android.os.HandlerThread
import android.view.Surface
import java.nio.ByteBuffer
import java.nio.ByteOrder
import java.nio.FloatBuffer
import java.util.concurrent.CountDownLatch
import java.util.concurrent.TimeUnit

/**
 * Renders CameraX frames from an input [SurfaceTexture] into a [MediaCodec]
 * encoder input [Surface], accounting for the sensor rotation already present
 * in `SurfaceTexture.getTransformMatrix(stMatrix)` and applying an
 * aspect-preserving `FIT_CENTER` quad so the encoded H.264 stream is always
 * upright and never stretched in either portrait or landscape phone orientations.
 */
internal class GlSurfacePipe(
    private val srcWidth: Int,
    private val srcHeight: Int,
    private val dstWidth: Int,
    private val dstHeight: Int,
    private val encoderInputSurface: Surface,
    initialRotationDegrees: Int,
    initialMirror: Boolean,
) : SurfaceTexture.OnFrameAvailableListener {

    private val thread = HandlerThread("webcam-gl-${dstWidth}x${dstHeight}").apply { start() }
    private val handler = Handler(thread.looper)

    @Volatile private var sensorRotationDegrees: Int = ((initialRotationDegrees % 360) + 360) % 360
    @Volatile private var displayRotationDegrees: Int = 0
    @Volatile private var mirrorHorizontally: Boolean = initialMirror
    @Volatile private var released = false
    @Volatile private var releaseResult: Boolean? = null

    private var eglDisplay: EGLDisplay = EGL14.EGL_NO_DISPLAY
    private var eglContext: EGLContext = EGL14.EGL_NO_CONTEXT
    private var eglSurface: EGLSurface = EGL14.EGL_NO_SURFACE
    private var oesTexId: Int = 0
    private var program: Int = 0
    private var aPositionLoc: Int = 0
    private var aTexCoordLoc: Int = 0
    private var uStMatrixLoc: Int = 0

    private var surfaceTexture: SurfaceTexture? = null
    var inputSurface: Surface? = null
        private set

    private val stMatrix = FloatArray(16)
    private val quadBuffer: FloatBuffer = ByteBuffer
        .allocateDirect(4 * 4 * 4)
        .order(ByteOrder.nativeOrder())
        .asFloatBuffer()

    init {
        val ready = CountDownLatch(1)
        handler.post {
            try {
                initGl()
            } catch (_: Exception) {
                // Return the partially initialized pipe so its caller can roll it back.
            } finally {
                ready.countDown()
            }
        }
        try {
            ready.await(3, TimeUnit.SECONDS)
        } catch (_: InterruptedException) {
            // Do not throw away this partially allocated object's cleanup handle.
            Thread.currentThread().interrupt()
        }
    }

    fun setTransform(sensorRotationDegrees: Int, displayRotationDegrees: Int) {
        this.sensorRotationDegrees = ((sensorRotationDegrees % 360) + 360) % 360
        this.displayRotationDegrees = ((displayRotationDegrees % 360) + 360) % 360
    }

    fun setRotation(degrees: Int) {
        setTransform(degrees, 0)
    }

    fun updateTransform(rotationDeg: Int, mirror: Boolean) {
        sensorRotationDegrees = ((rotationDeg % 360) + 360) % 360
        mirrorHorizontally = mirror
    }

    override fun onFrameAvailable(st: SurfaceTexture?) {
        if (released) return
        handler.post { drawFrame() }
    }

    fun release(): Boolean {
        if (released) return releaseResult == true
        released = true
        val done = CountDownLatch(1)
        var cleaned = false
        val posted = handler.post {
            try {
                cleaned = releaseGl()
            } finally {
                done.countDown()
            }
        }
        val completed = try {
            posted && done.await(1, TimeUnit.SECONDS)
        } catch (_: InterruptedException) {
            Thread.currentThread().interrupt()
            false
        }
        val quitting = thread.quitSafely()
        val joined = try {
            thread.join(1_000)
            !thread.isAlive
        } catch (_: InterruptedException) {
            Thread.currentThread().interrupt()
            false
        }
        return (completed && cleaned && quitting && joined).also { releaseResult = it }
    }

    private fun initGl() {
        eglDisplay = EGL14.eglGetDisplay(EGL14.EGL_DEFAULT_DISPLAY)
        val version = IntArray(2)
        EGL14.eglInitialize(eglDisplay, version, 0, version, 1)

        val attribList = intArrayOf(
            EGL14.EGL_RED_SIZE, 8,
            EGL14.EGL_GREEN_SIZE, 8,
            EGL14.EGL_BLUE_SIZE, 8,
            EGL14.EGL_ALPHA_SIZE, 8,
            EGL14.EGL_RENDERABLE_TYPE, EGL14.EGL_OPENGL_ES2_BIT,
            EGLExt.EGL_RECORDABLE_ANDROID, 1,
            EGL14.EGL_NONE,
        )
        val configs = arrayOfNulls<EGLConfig>(1)
        val numConfigs = IntArray(1)
        EGL14.eglChooseConfig(eglDisplay, attribList, 0, configs, 0, 1, numConfigs, 0)
        val config = configs[0] ?: return

        val ctxAttribs = intArrayOf(EGL14.EGL_CONTEXT_CLIENT_VERSION, 2, EGL14.EGL_NONE)
        eglContext = EGL14.eglCreateContext(eglDisplay, config, EGL14.EGL_NO_CONTEXT, ctxAttribs, 0)

        val surfaceAttribs = intArrayOf(EGL14.EGL_NONE)
        eglSurface = EGL14.eglCreateWindowSurface(eglDisplay, config, encoderInputSurface, surfaceAttribs, 0)
        EGL14.eglMakeCurrent(eglDisplay, eglSurface, eglSurface, eglContext)

        program = createProgram(VERTEX_SHADER, FRAGMENT_SHADER)
        aPositionLoc = GLES20.glGetAttribLocation(program, "aPosition")
        aTexCoordLoc = GLES20.glGetAttribLocation(program, "aTexCoord")
        uStMatrixLoc = GLES20.glGetUniformLocation(program, "uStMatrix")

        val texIds = IntArray(1)
        GLES20.glGenTextures(1, texIds, 0)
        oesTexId = texIds[0]
        GLES20.glBindTexture(GLES11Ext.GL_TEXTURE_EXTERNAL_OES, oesTexId)
        GLES20.glTexParameteri(GLES11Ext.GL_TEXTURE_EXTERNAL_OES, GLES20.GL_TEXTURE_MIN_FILTER, GLES20.GL_LINEAR)
        GLES20.glTexParameteri(GLES11Ext.GL_TEXTURE_EXTERNAL_OES, GLES20.GL_TEXTURE_MAG_FILTER, GLES20.GL_LINEAR)
        GLES20.glTexParameteri(GLES11Ext.GL_TEXTURE_EXTERNAL_OES, GLES20.GL_TEXTURE_WRAP_S, GLES20.GL_CLAMP_TO_EDGE)
        GLES20.glTexParameteri(GLES11Ext.GL_TEXTURE_EXTERNAL_OES, GLES20.GL_TEXTURE_WRAP_T, GLES20.GL_CLAMP_TO_EDGE)

        val st = SurfaceTexture(oesTexId)
        st.setDefaultBufferSize(srcWidth, srcHeight)
        st.setOnFrameAvailableListener(this, handler)
        surfaceTexture = st
        inputSurface = Surface(st)
    }

    private fun drawFrame() {
        val st = surfaceTexture ?: return
        if (eglDisplay === EGL14.EGL_NO_DISPLAY || eglSurface === EGL14.EGL_NO_SURFACE) return

        runCatching {
            st.updateTexImage()
            st.getTransformMatrix(stMatrix)
        }.onFailure { return }

        // stMatrix is column-major 4x4: stMatrix[0] is dU/dX, stMatrix[1] is dV/dX.
        val stSwapsAxes = kotlin.math.abs(stMatrix[1]) > kotlin.math.abs(stMatrix[0])
        // Dimensions of the image in UV space AFTER stMatrix is applied:
        val uvWidth = if (stSwapsAxes) srcHeight.toFloat() else srcWidth.toFloat()
        val uvHeight = if (stSwapsAxes) srcWidth.toFloat() else srcHeight.toFloat()
        // If stMatrix already rotated the sensor to Surface.ROTATION_0, we only need
        // to compensate for displayRotationDegrees (0 in portrait, 90 in landscape-left,
        // 270 in landscape-right, 180 upside-down). Otherwise apply sensorRotationDegrees.
        val effectiveRot = if (stSwapsAxes) {
            ((displayRotationDegrees % 360) + 360) % 360
        } else {
            ((sensorRotationDegrees % 360) + 360) % 360
        }

        val uprightWidth = if (effectiveRot % 180 != 0) uvHeight else uvWidth
        val uprightHeight = if (effectiveRot % 180 != 0) uvWidth else uvHeight
        val srcAspect = (uprightWidth / uprightHeight.coerceAtLeast(1f)).coerceAtLeast(0.01f)
        val dstAspect = (dstWidth.toFloat() / dstHeight.coerceAtLeast(1).toFloat()).coerceAtLeast(0.01f)

        // Aspect-preserving FIT_CENTER vertex positions so the camera frame is never stretched:
        val posScaleX: Float
        val posScaleY: Float
        if (srcAspect > dstAspect) {
            posScaleX = 1f
            posScaleY = dstAspect / srcAspect
        } else {
            posScaleX = srcAspect / dstAspect
            posScaleY = 1f
        }

        updateQuadVertices(effectiveRot, posScaleX, posScaleY, mirrorHorizontally && !stSwapsAxes)

        GLES20.glViewport(0, 0, dstWidth, dstHeight)
        GLES20.glClearColor(0.07f, 0.075f, 0.09f, 1f)
        GLES20.glClear(GLES20.GL_COLOR_BUFFER_BIT)

        GLES20.glUseProgram(program)
        GLES20.glActiveTexture(GLES20.GL_TEXTURE0)
        GLES20.glBindTexture(GLES11Ext.GL_TEXTURE_EXTERNAL_OES, oesTexId)
        GLES20.glUniformMatrix4fv(uStMatrixLoc, 1, false, stMatrix, 0)

        quadBuffer.position(0)
        GLES20.glVertexAttribPointer(aPositionLoc, 2, GLES20.GL_FLOAT, false, 16, quadBuffer)
        GLES20.glEnableVertexAttribArray(aPositionLoc)

        quadBuffer.position(2)
        GLES20.glVertexAttribPointer(aTexCoordLoc, 2, GLES20.GL_FLOAT, false, 16, quadBuffer)
        GLES20.glEnableVertexAttribArray(aTexCoordLoc)

        GLES20.glDrawArrays(GLES20.GL_TRIANGLE_STRIP, 0, 4)

        val tsNs = st.timestamp
        if (tsNs > 0L) {
            EGLExt.eglPresentationTimeANDROID(eglDisplay, eglSurface, tsNs)
        }
        EGL14.eglSwapBuffers(eglDisplay, eglSurface)
    }

    private fun updateQuadVertices(
        effectiveRot: Int,
        posScaleX: Float,
        posScaleY: Float,
        mirror: Boolean,
    ) {
        val sx = if (mirror) -posScaleX else posScaleX
        val sy = posScaleY
        val posX = floatArrayOf(-sx, sx, -sx, sx)
        val posY = floatArrayOf(-sy, -sy, sy, sy)

        // Base UV corners: bl = (0f, 0f), br = (1f, 0f), tl = (0f, 1f), tr = (1f, 1f)
        // Surface.ROTATION_90 (standard landscape, phone rotated 90° CCW) puts phone's
        // top edge on the left and right edge on the top: bottom-left is tl, bottom-right
        // is bl, top-left is tr, top-right is br.
        val uvs = when (((effectiveRot % 360) + 360) % 360) {
            90 -> floatArrayOf(
                0f, 1f, // uv0 = tl
                0f, 0f, // uv1 = bl
                1f, 1f, // uv2 = tr
                1f, 0f, // uv3 = br
            )
            180 -> floatArrayOf(
                1f, 1f, // uv0 = tr
                0f, 1f, // uv1 = tl
                1f, 0f, // uv2 = br
                0f, 0f, // uv3 = bl
            )
            270 -> floatArrayOf(
                1f, 0f, // uv0 = br
                1f, 1f, // uv1 = tr
                0f, 0f, // uv2 = bl
                0f, 1f, // uv3 = tl
            )
            else -> floatArrayOf(
                0f, 0f, // uv0 = bl
                1f, 0f, // uv1 = br
                0f, 1f, // uv2 = tl
                1f, 1f, // uv3 = tr
            )
        }

        quadBuffer.position(0)
        for (i in 0 until 4) {
            quadBuffer.put(posX[i])
            quadBuffer.put(posY[i])
            quadBuffer.put(uvs[i * 2])
            quadBuffer.put(uvs[i * 2 + 1])
        }
        quadBuffer.position(0)
    }

    private fun releaseGl(): Boolean {
        var success = runCatching { inputSurface?.release() }.isSuccess
        inputSurface = null
        if (runCatching {
            surfaceTexture?.setOnFrameAvailableListener(null)
            surfaceTexture?.release()
        }.isFailure) success = false
        surfaceTexture = null

        if (eglDisplay !== EGL14.EGL_NO_DISPLAY) {
            if (oesTexId != 0) {
                GLES20.glDeleteTextures(1, intArrayOf(oesTexId), 0)
                oesTexId = 0
            }
            if (program != 0) {
                GLES20.glDeleteProgram(program)
                program = 0
            }
            if (GLES20.glGetError() != GLES20.GL_NO_ERROR) success = false
            if (!EGL14.eglMakeCurrent(eglDisplay, EGL14.EGL_NO_SURFACE, EGL14.EGL_NO_SURFACE, EGL14.EGL_NO_CONTEXT)) success = false
            if (eglSurface !== EGL14.EGL_NO_SURFACE) {
                if (!EGL14.eglDestroySurface(eglDisplay, eglSurface)) success = false
                eglSurface = EGL14.EGL_NO_SURFACE
            }
            if (eglContext !== EGL14.EGL_NO_CONTEXT) {
                if (!EGL14.eglDestroyContext(eglDisplay, eglContext)) success = false
                eglContext = EGL14.EGL_NO_CONTEXT
            }
            if (!EGL14.eglTerminate(eglDisplay)) success = false
            eglDisplay = EGL14.EGL_NO_DISPLAY
        }
        return success
    }

    companion object {
        private const val VERTEX_SHADER = """
            uniform mat4 uStMatrix;
            attribute vec4 aPosition;
            attribute vec4 aTexCoord;
            varying vec2 vTexCoord;
            void main() {
                gl_Position = aPosition;
                vTexCoord = (uStMatrix * aTexCoord).xy;
            }
        """

        private const val FRAGMENT_SHADER = """
            #extension GL_OES_EGL_image_external : require
            precision mediump float;
            varying vec2 vTexCoord;
            uniform samplerExternalOES sTexture;
            void main() {
                gl_FragColor = texture2D(sTexture, vTexCoord);
            }
        """

        private fun compileShader(type: Int, src: String): Int {
            val shader = GLES20.glCreateShader(type)
            GLES20.glShaderSource(shader, src)
            GLES20.glCompileShader(shader)
            return shader
        }

        private fun createProgram(vsSource: String, fsSource: String): Int {
            val vs = compileShader(GLES20.GL_VERTEX_SHADER, vsSource)
            val fs = compileShader(GLES20.GL_FRAGMENT_SHADER, fsSource)
            val prog = GLES20.glCreateProgram()
            GLES20.glAttachShader(prog, vs)
            GLES20.glAttachShader(prog, fs)
            GLES20.glLinkProgram(prog)
            return prog
        }
    }
}
