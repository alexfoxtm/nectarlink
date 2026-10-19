// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.notifications

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class NotificationFilterTest {
    @Test
    fun defaultSelectionMirrorsEveryTypeUntilTheUserNarrowsIt() {
        val open = NotificationFilter()
        assertEquals(open, NotificationFilterPolicy.decode(null))
        assertEquals(open, NotificationFilterPolicy.decode(NotificationFilterPolicy.encode(open)))
        for (kind in NotificationKind.entries) {
            assertTrue(open.allows("com.whatsapp", kind))
            assertTrue(
                NotificationFilterPolicy.forwardable(
                    ownPackage = "app.nectarlink",
                    sourcePackage = "com.whatsapp",
                    currentUser = true,
                    groupSummary = false,
                    localOnly = false,
                    kind = kind,
                    filter = open,
                ),
            )
        }
        assertEquals(NotificationKind.MESSAGES, NotificationFilterPolicy.notificationKind("msg", null, false))
        assertEquals(NotificationKind.MESSAGES, NotificationFilterPolicy.notificationKind("email", "InboxStyle", true))
        assertEquals(NotificationKind.EMAIL, NotificationFilterPolicy.notificationKind("email", null, false))
        assertEquals(
            NotificationKind.EMAIL,
            NotificationFilterPolicy.notificationKind(null, "android.app.Notification\$InboxStyle", false),
        )
        assertEquals(NotificationKind.OTHER, NotificationFilterPolicy.notificationKind(null, null, false))

        val messagesOnly = open.copy(
            appMode = NotificationAppMode.SELECTED,
            selectedPackages = setOf("com.whatsapp"),
            kinds = setOf(NotificationKind.MESSAGES),
        )
        assertEquals(messagesOnly, NotificationFilterPolicy.decode(NotificationFilterPolicy.encode(messagesOnly)))
        assertTrue(messagesOnly.allows("com.whatsapp", NotificationKind.MESSAGES))
        assertFalse(messagesOnly.allows("com.whatsapp", NotificationKind.EMAIL))
        assertFalse(messagesOnly.allows("com.whatsapp", NotificationKind.OTHER))
        assertFalse(messagesOnly.allows("com.slack", NotificationKind.MESSAGES))
    }

    @Test
    fun corruptRecordAndRejectedSourcesDoNotMirror() {
        assertEquals(NotificationFilter.BLOCKED, NotificationFilterPolicy.decode("nope"))
        assertEquals(NotificationFilter.BLOCKED, NotificationFilterPolicy.decode("0|true|ALL||MESSAGES"))
        assertEquals(NotificationFilter.BLOCKED, NotificationFilterPolicy.decode("1|true|ALL|not a package|MESSAGES"))
        assertFalse(NotificationFilter.BLOCKED.allows("com.whatsapp", NotificationKind.MESSAGES))
        val open = NotificationFilter()
        assertFalse(open.copy(enabled = false).allows("com.whatsapp", NotificationKind.MESSAGES))
        assertFalse(open.copy(appMode = NotificationAppMode.SELECTED).allows("com.whatsapp", NotificationKind.MESSAGES))
        assertFalse(open.copy(kinds = emptySet()).allows("com.whatsapp", NotificationKind.MESSAGES))
        assertFalse(open.allows("not a package", NotificationKind.OTHER))
        assertFalse(open.allows("com.whatsapp", NotificationKind.MESSAGES, currentUser = false))
        assertNull(NotificationFilterPolicy.encode(open.copy(selectedPackages = setOf("not a package"))))
        assertFalse(
            NotificationFilterPolicy.forwardable(
                "app.nectarlink", "app.nectarlink", true, false, false, NotificationKind.OTHER, open,
            ),
        )
        assertFalse(
            NotificationFilterPolicy.forwardable(
                "app.nectarlink", "com.whatsapp", true, true, false, NotificationKind.MESSAGES, open,
            ),
        )
        assertFalse(
            NotificationFilterPolicy.forwardable(
                "app.nectarlink", "com.whatsapp", true, false, true, NotificationKind.EMAIL, open,
            ),
        )
        assertFalse(
            NotificationFilterPolicy.forwardable(
                "app.nectarlink", "com.whatsapp", false, false, false, NotificationKind.OTHER, open,
            ),
        )
    }
}
