// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.webcam

import org.junit.Assert.assertArrayEquals
import org.junit.Assert.assertEquals
import org.junit.Test

class CameraEncoderTest {
    @Test
    fun `prepends AUD and SPS PPS to keyframes and avoids duplicate AUD`() {
        val spsPps = byteArrayOf(0x00, 0x00, 0x00, 0x01, 0x67, 0x42, 0x00, 0x00, 0x00, 0x01, 0x68, 0x11)
        val idrSlice = byteArrayOf(0x00, 0x00, 0x00, 0x01, 0x65, 0xAA.toByte())

        val keyOut = CameraEncoder.ensureAudAndParameters(idrSlice, spsPps, isKeyframe = true)
        assertArrayEquals(CameraEncoder.AUD_NAL + spsPps + idrSlice, keyOut)

        // When the encoder already emitted an AUD at the front of the slice, it is not duplicated.
        val idrWithAud = CameraEncoder.AUD_NAL + idrSlice
        val keyDedup = CameraEncoder.ensureAudAndParameters(idrWithAud, spsPps, isKeyframe = true)
        assertArrayEquals(CameraEncoder.AUD_NAL + spsPps + idrSlice, keyDedup)

        // Non-keyframe gets AUD prepended without SPS/PPS.
        val pSlice = byteArrayOf(0x00, 0x00, 0x00, 0x01, 0x41, 0x55)
        val pOut = CameraEncoder.ensureAudAndParameters(pSlice, spsPps, isKeyframe = false)
        assertArrayEquals(CameraEncoder.AUD_NAL + pSlice, pOut)
    }

    @Test
    fun `normalizes resolution height and selects matching width and bitrate`() {
        assertEquals(720, CameraEncoder.normalizeHeight(720, supports4k = false))
        assertEquals(1080, CameraEncoder.normalizeHeight(1080, supports4k = false))
        assertEquals(1080, CameraEncoder.normalizeHeight(2160, supports4k = false))
        assertEquals(2160, CameraEncoder.normalizeHeight(2160, supports4k = true))

        assertEquals(1280, CameraEncoder.widthForHeight(720))
        assertEquals(1920, CameraEncoder.widthForHeight(1080))
        assertEquals(3840, CameraEncoder.widthForHeight(2160))

        assertEquals(4_000_000, CameraEncoder.bitrateForHeight(720))
        assertEquals(8_000_000, CameraEncoder.bitrateForHeight(1080))
        assertEquals(20_000_000, CameraEncoder.bitrateForHeight(2160))
    }
    @Test
    fun `actual camera source supports UHD in either orientation without upscaling`() {
        assertEquals(2160, CameraEncoder.supportedCaptureHeight(3840, 2160, 2160))
        assertEquals(2160, CameraEncoder.supportedCaptureHeight(2160, 3840, 2160))
        assertEquals(2160, CameraEncoder.supportedCaptureHeight(4096, 2160, 2160))
        assertEquals(1080, CameraEncoder.supportedCaptureHeight(3840, 2160, 1080))
    }

    @Test
    fun `lower camera source downgrades to genuine preset or refuses capture`() {
        assertEquals(1080, CameraEncoder.supportedCaptureHeight(1920, 1080, 2160))
        assertEquals(1080, CameraEncoder.supportedCaptureHeight(3264, 2448, 2160))
        assertEquals(720, CameraEncoder.supportedCaptureHeight(1280, 720, 2160))
        assertEquals(720, CameraEncoder.supportedCaptureHeight(720, 1280, 1080))
        assertEquals(null, CameraEncoder.supportedCaptureHeight(640, 480, 2160))
    }
}
