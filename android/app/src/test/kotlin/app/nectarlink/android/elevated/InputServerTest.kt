// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.elevated

import org.junit.Assert.assertEquals
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
}
