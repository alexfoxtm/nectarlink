// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.update

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Assert.fail
import org.junit.Test

class AppUpdaterTest {
    @Test
    fun versionsCompare() {
        assertTrue(AppUpdater.isNewer("0.1.0", "0.0.1"))
        assertTrue(AppUpdater.isNewer("1.0", "0.9.9"))
        assertTrue(AppUpdater.isNewer("0.2.0", "0.2.0-beta.1"))
        assertTrue(AppUpdater.isNewer("0.2.0-beta.2", "0.2.0-beta.1"))
        assertFalse(AppUpdater.isNewer("0.0.1", "0.0.1"))
        assertFalse(AppUpdater.isNewer("0.0.1", "0.1.0"))
        assertFalse(AppUpdater.isNewer("garbage", "0.1.0"))
    }

    @Test
    fun findsTheApkHash() {
        val hash = "a".repeat(64)
        val sums = "$hash  Nectarlink-9.0.0-android.apk\n${"b".repeat(64)}  Nectarlink-9.0.0-x64-setup.exe\n"
        assertEquals(hash, AppUpdater.listedHash(sums, "Nectarlink-9.0.0-android.apk"))
        assertNull(AppUpdater.listedHash(sums, "missing.apk"))
        assertEquals("c".repeat(64), AppUpdater.listedHash("${"C".repeat(64)} *x.apk", "x.apk"))
    }

    @Test
    fun readsTheFullSizeFromContentRange() {
        assertEquals(49_362_916L, AppUpdater.rangeTotal("bytes 1048576-49362915/49362916"))
        assertNull(AppUpdater.rangeTotal("bytes 0-99/*"))
        assertNull(AppUpdater.rangeTotal(null))
        assertNull(AppUpdater.rangeTotal("garbage"))
    }

    @Test
    fun parsesTheR2StableManifest() {
        val origin = "https://pub-a57457f4da43415ea6335550ee9e2983.r2.dev/waypair/nectar/android/stable"
        val apk = "$origin/Nectarlink-9.0.0-android.apk"
        val sums = "$origin/SHA256SUMS.txt"
        val body = """
            {
              "tag_name": "v9.0.0",
              "html_url": "https://github.com/thtbee/nectarlink/releases/tag/v9.0.0",
              "draft": false,
              "prerelease": false,
              "assets": [
                {"name": "Nectarlink-9.0.0-x64-setup.exe", "browser_download_url": "https://example.invalid/ignore.exe"},
                {"name": "Nectarlink-9.0.0-android.apk", "browser_download_url": "$apk"},
                {"name": "SHA256SUMS.txt", "browser_download_url": "$sums"}
              ]
            }
        """.trimIndent()
        assertTrue(AppUpdater.allowedUpdateUrl(AppUpdater.MANIFEST_URL))
        val update = AppUpdater.manifestUpdate(200, body, "0.1.0")
        assertEquals("9.0.0", update!!.version)
        assertEquals("https://github.com/thtbee/nectarlink/releases/tag/v9.0.0", update.notesUrl)
        assertEquals("Nectarlink-9.0.0-android.apk", update.apkName)
        assertEquals(apk, update.apkUrl)
        assertEquals(sums, update.sumsUrl)
        assertEquals(sums, AppUpdater.nextHop(AppUpdater.MANIFEST_URL, "SHA256SUMS.txt"))
    }

    @Test
    fun refusesAReleaseOffTheR2Origin() {
        assertNull(AppUpdater.manifestUpdate(404, """{"tag_name":"v9.0.0"}""", "0.1.0"))
        try {
            AppUpdater.manifestUpdate(503, "{}", "0.1.0")
            fail("an HTTP error is not an empty release")
        } catch (e: AppUpdater.HttpStatus) {
            assertEquals(503, e.code)
        }
        val host = "pub-a57457f4da43415ea6335550ee9e2983.r2.dev"
        val prefix = "/waypair/nectar/android/stable/Nectarlink-9.0.0-android.apk"
        assertFalse(AppUpdater.allowedUpdateUrl("http://$host$prefix"))
        assertFalse(AppUpdater.allowedUpdateUrl("https://user@$host$prefix"))
        assertFalse(AppUpdater.allowedUpdateUrl("https://$host/waypair/android/stable/Nectarlink-9.0.0-android.apk"))
        assertFalse(AppUpdater.allowedUpdateUrl("https://$host/waypair/cloud/android/stable/Nectarlink-9.0.0-android.apk"))
        assertFalse(AppUpdater.allowedUpdateUrl("https://$host/waypair/nectar/android/../cloud/Nectarlink-9.0.0-android.apk"))
        assertFalse(AppUpdater.allowedUpdateUrl("https://$host$prefix?dl=1"))
        assertFalse(AppUpdater.allowedUpdateUrl("https://evil.example$prefix"))
        assertNull(AppUpdater.nextHop(AppUpdater.MANIFEST_URL, "https://evil.example$prefix"))
        assertNull(AppUpdater.manifestUpdate(200, manifest("https://evil.example$prefix", "https://$host/waypair/nectar/android/stable/SHA256SUMS.txt"), "0.1.0"))
        assertNull(AppUpdater.manifestUpdate(200, manifest("https://$host$prefix", "https://$host/waypair/nectar/android/stable/SHA256SUMS.txt", draft = true), "0.1.0"))
        assertNull(AppUpdater.manifestUpdate(200, manifest("https://$host$prefix", "https://$host/waypair/nectar/android/stable/SHA256SUMS.txt", prerelease = true), "0.1.0"))
        assertNull(AppUpdater.manifestUpdate(200, manifest("https://$host/waypair/nectar/android/stable/other-android.apk", "https://$host/waypair/nectar/android/stable/SHA256SUMS.txt"), "0.1.0"))
        assertNull(AppUpdater.manifestUpdate(200, manifest("https://$host$prefix", "https://$host/waypair/nectar/android/stable/SHA256SUMS.txt"), "9.0.0"))
    }

    private fun manifest(apkUrl: String, sumsUrl: String, draft: Boolean = false, prerelease: Boolean = false) = """
        {
          "tag_name": "v9.0.0",
          "html_url": "https://github.com/thtbee/nectarlink/releases/tag/v9.0.0",
          "draft": $draft,
          "prerelease": $prerelease,
          "assets": [
            {"name": "Nectarlink-9.0.0-android.apk", "browser_download_url": "$apkUrl"},
            {"name": "SHA256SUMS.txt", "browser_download_url": "$sumsUrl"}
          ]
        }
    """.trimIndent()
}
