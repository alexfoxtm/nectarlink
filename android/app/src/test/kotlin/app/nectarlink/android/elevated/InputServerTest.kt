// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.elevated

import app.nectarlink.android.BuildConfig
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test
import org.w3c.dom.Element
import java.io.File
import javax.xml.parsers.DocumentBuilderFactory

class InputServerTest {
    @Test
    fun theHelperGoesByTheAppLabel() {
        val res = listOf(File("src/main/res"), File("app/src/main/res"), File("android/app/src/main/res"))
            .first { it.isDirectory }
        for (values in listOf("values")) {
            val strings = DocumentBuilderFactory.newInstance().newDocumentBuilder()
                .parse(File(res, "$values/strings.xml")).getElementsByTagName("string")
            val appName = (0 until strings.length).map { strings.item(it) as Element }
                .first { it.getAttribute("name") == "app_name" }.textContent
            assertEquals(values, appName, InputServer.LABEL)
        }
    }

    @Test
    fun aFreshCopyIsTimedByTheClipboardStamp() {
        assertEquals(1_500L, InputServer.linkCopyAgeMs(stampMs = 10_000L, nowMs = 11_500L, lastStampMs = 9_000L))
    }

    @Test
    fun repeatedFutureUnstampedOrOldCopiesAreNotOffered() {
        assertEquals(null, InputServer.linkCopyAgeMs(stampMs = 10_000L, nowMs = 11_500L, lastStampMs = 10_000L))
        assertEquals(null, InputServer.linkCopyAgeMs(stampMs = 12_000L, nowMs = 11_500L, lastStampMs = 0L))
        assertEquals(null, InputServer.linkCopyAgeMs(stampMs = 0L, nowMs = 11_500L, lastStampMs = 0L))
        assertEquals(null, InputServer.linkCopyAgeMs(stampMs = 10_000L, nowMs = 70_000L, lastStampMs = 0L))
    }

    @Test
    fun usageAccessIsAllowedForThisAppOnly() {
        assertTrue(InputServer.allowedCommand("appops set ${BuildConfig.APPLICATION_ID} GET_USAGE_STATS allow"))
    }

    @Test
    fun usageAccessForAnythingElseIsRefused() {
        val id = BuildConfig.APPLICATION_ID
        for (cmd in listOf(
            "appops set com.example GET_USAGE_STATS allow",
            "appops set $id.other GET_USAGE_STATS allow",
            "appops set $id GET_USAGE_STATS allow; reboot",
            "appops set $id GET_USAGE_STATS allow && reboot",
            "appops set $id  GET_USAGE_STATS allow",
            "appops set $id SYSTEM_ALERT_WINDOW allow",
        )) {
            assertFalse(cmd, InputServer.allowedCommand(cmd))
        }
    }
    @Test
    fun aBrowserUrlMayHaveAdditionalTextRepresentations() {
        assertTrue(InputServer.linkMimeTypesAllowed(listOf("text/plain", "text/html")))
        assertTrue(InputServer.linkMimeTypesAllowed(listOf("text/x-moz-url", "text/plain")))
        assertTrue(InputServer.linkMimeTypesAllowed(listOf("text/uri-list")))
    }

    @Test
    fun htmlWithoutPlainTextAndNonTextRepresentationsStayOut() {
        assertFalse(InputServer.linkMimeTypesAllowed(emptyList()))
        assertFalse(InputServer.linkMimeTypesAllowed(listOf("text/html")))
        assertFalse(InputServer.linkMimeTypesAllowed(listOf("text/x-moz-url")))
        assertFalse(InputServer.linkMimeTypesAllowed(listOf("text/plain", "image/png")))
        assertFalse(InputServer.linkMimeTypesAllowed(listOf("text/plain", "application/octet-stream")))
    }


    @Test
    fun aBufferedSameDisplayWheelFoldsOnceAndStopsBeforeTouch() {
        val ready = mutableListOf(
            "@7 S 4.0 5.0 0.0 1.5",
            "@7 D 1 2",
            "@7 S 4 5 0.0 4.0",
        )
        val folded = InputServer.coalesceReadyScrolls("@7 S 4 5 0.0 1.0") {
            if (ready.isEmpty()) null else ready.removeAt(0)
        }
        assertEquals("@7 S 4 5 0.0 2.5", folded.line)
        assertEquals("@7 D 1 2", folded.held)
        assertEquals(listOf("@7 S 4 5 0.0 4.0"), ready)

        val more = MutableList(InputServer.SCROLL_MERGE_LIMIT) { "@7 S 1 2 0.0 1.0" }
        val capped = InputServer.coalesceReadyScrolls("@7 S 1 2 0.0 1.0") {
            if (more.isEmpty()) null else more.removeAt(0)
        }
        assertEquals("@7 S 1 2 0.0 ${InputServer.SCROLL_MERGE_LIMIT.toFloat()}", capped.line)
        assertEquals(null, capped.held)
        assertEquals(1, more.size)
    }

    @Test
    fun aDifferentDisplayOrAnOutOfBoundsWheelIsNotFolded() {
        val moved = mutableListOf("@7 S 9 8 0.0 1.5", "@7 S 9 8 0.0 1.0")
        val barrier = InputServer.coalesceReadyScrolls("@7 S 4 5 0.0 1.0") {
            if (moved.isEmpty()) null else moved.removeAt(0)
        }
        assertEquals("@7 S 4 5 0.0 1.0", barrier.line)
        assertEquals("@7 S 9 8 0.0 1.5", barrier.held)
        assertEquals(listOf("@7 S 9 8 0.0 1.0"), moved)

        val replaced = mutableListOf("@8 S 4 5 0.0 1.0", "@7 S 4 5 0.0 1.0")
        val other = InputServer.coalesceReadyScrolls("@7 S 4 5 0.0 2.0") {
            if (replaced.isEmpty()) null else replaced.removeAt(0)
        }
        assertEquals("@7 S 4 5 0.0 2.0", other.line)
        assertEquals("@8 S 4 5 0.0 1.0", other.held)
        assertEquals(listOf("@7 S 4 5 0.0 1.0"), replaced)

        val over = mutableListOf("@7 S 4 5 0.0 40.0", "@7 S 4 5 0.0 NaN")
        val bounded = InputServer.coalesceReadyScrolls("@7 S 4 5 0.0 80.0") {
            if (over.isEmpty()) null else over.removeAt(0)
        }
        assertEquals("@7 S 4 5 0.0 80.0", bounded.line)
        assertEquals("@7 S 4 5 0.0 40.0", bounded.held)
        val nanHeld = InputServer.coalesceReadyScrolls(checkNotNull(bounded.held)) {
            if (over.isEmpty()) null else over.removeAt(0)
        }
        assertEquals("@7 S 4 5 0.0 40.0", nanHeld.line)
        assertEquals("@7 S 4 5 0.0 NaN", nanHeld.held)

        var pulled = false
        val typed = InputServer.coalesceReadyScrolls("D 1 2") {
            pulled = true
            null
        }
        assertEquals("D 1 2", typed.line)
        assertEquals(null, typed.held)
        assertFalse(pulled)

        val screen = mutableListOf("S 4 5 0.0 1.0")
        val windowOnly = InputServer.coalesceReadyScrolls("@7 S 4 5 0.0 1.0") {
            if (screen.isEmpty()) null else screen.removeAt(0)
        }
        assertEquals("@7 S 4 5 0.0 1.0", windowOnly.line)
        assertEquals("S 4 5 0.0 1.0", windowOnly.held)
    }

}
