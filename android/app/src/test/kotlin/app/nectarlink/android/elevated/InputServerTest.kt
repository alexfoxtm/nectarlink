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
        for (values in listOf("values", "values-zh-rCN")) {
            val strings = DocumentBuilderFactory.newInstance().newDocumentBuilder()
                .parse(File(res, "$values/strings.xml")).getElementsByTagName("string")
            val appName = (0 until strings.length).map { strings.item(it) as Element }
                .first { it.getAttribute("name") == "app_name" }.textContent
            assertEquals(values, appName, InputServer.LABEL)
        }
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
}
