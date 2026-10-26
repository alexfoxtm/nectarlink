// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android

import app.nectarlink.android.companion.PcCompanion
import app.nectarlink.android.ui.Preferences
import app.nectarlink.android.ui.theme.Appearance
import app.nectarlink.android.ui.theme.Tokens
import org.junit.Assert.assertArrayEquals
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class CompanionAndMigrationTest {

    @Test
    fun `PcCompanion decodes protocol device IDs and matches the PC beacon`() {
        assertArrayEquals(
            byteArrayOf(0xF0.toByte(), 0xBF.toByte(), 0xC7.toByte()),
            PcCompanion.decodeZBase32("6n9hq"),
        )
        assertArrayEquals(
            ByteArray(32) { it.toByte() },
            PcCompanion.decodeZBase32("yyyoryarywdyqnyjbefoadeqbhebnrounoktcfaadrpbs8y7daxo"),
        )
        // The PC broadcasts the magic and its first eight raw identity bytes.
        val id = "ww6yb9a1gtm8tgw4ukpjigw4ukpjigw4ukpjigw4ukpjigw4ukpy"
        val broadcast = PcCompanion.BEACON_MAGIC + byteArrayOf(
            0xA5.toByte(), 0x3C, 0x00, 0xFF.toByte(), 0x12, 0x34, 0x56, 0x78,
        )
        assertArrayEquals(broadcast, PcCompanion.buildBeaconPayload(id))
        assertArrayEquals(broadcast, PcCompanion.buildBeaconPayload(" ${id.uppercase()} "))

        val maskWithId = PcCompanion.buildBeaconMask(hasDeviceId = true)
        assertEquals(13, maskWithId.size)
        assertTrue(maskWithId.all { it == 0xFF.toByte() })
    }

    @Test
    fun `PcCompanion malformed device IDs fall back to discovery magic only`() {
        val id = "ww6yb9a1gtm8tgw4ukpjigw4ukpjigw4ukpjigw4ukpjigw4ukpy"
        val nonCanonicalPadding = id.dropLast(1) + "b"
        assertEquals(null, PcCompanion.decodeZBase32(nonCanonicalPadding))
        val invalidIds = listOf(
            null,
            "",
            "abcd",
            "not-an-id",
            "0123456789abcdef-fedcba9876543210",
            id.dropLast(1),
            id + "y",
            nonCanonicalPadding,
            id.replaceFirst('k', '\u212A'),
        )
        for (invalid in invalidIds) {
            assertArrayEquals(PcCompanion.BEACON_MAGIC, PcCompanion.buildBeaconPayload(invalid))
        }
        val maskWithoutId = PcCompanion.buildBeaconMask(hasDeviceId = false)
        assertEquals(5, maskWithoutId.size)
        assertTrue(maskWithoutId.all { it == 0xFF.toByte() })
    }

    @Test
    fun `Preferences sanitizeAppearance falls back on unknown legacy theme, mode, and seed values`() {
        val legacy = Preferences.sanitizeAppearance(
            theme = "sepia",
            mode = "neon",
            seed = "cyberpunk",
            dynamicColor = false,
        )
        assertEquals("graphite", legacy.theme)
        assertEquals("system", legacy.mode)
        assertEquals(Tokens.DEFAULT_SEED, legacy.seed)
        assertFalse(legacy.dynamicColor)

        val empty = Preferences.sanitizeAppearance(
            Appearance(theme = "", mode = "", seed = "", dynamicColor = true),
        )
        assertEquals("graphite", empty.theme)
        assertEquals("system", empty.mode)
        assertEquals(Tokens.DEFAULT_SEED, empty.seed)
        assertTrue(empty.dynamicColor)

        val valid = Preferences.sanitizeAppearance(
            theme = "graphite",
            mode = "dark",
            seed = "ocean",
            dynamicColor = false,
        )
        assertEquals("graphite", valid.theme)
        assertEquals("dark", valid.mode)
        assertEquals("ocean", valid.seed)
        assertFalse(valid.dynamicColor)
    }
}
