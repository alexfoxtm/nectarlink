// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.elevated

import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class LocalHotspotOwnerTest {
    @Test
    fun `bounded local admission has a separate owner domain`() {
        val now = 1000L
        val local = ShellHotspot.LocalOwner("a".repeat(64), 1, now + 40000)
        assertTrue(local.valid(now))
        val remote: ShellHotspot.RequestOwner = ShellHotspot.Owner("a".repeat(52), 1uL, 1uL)
        assertFalse((local as ShellHotspot.RequestOwner) == remote)
    }

    @Test
    fun `stale malformed and excessive local requests fail closed`() {
        val now = 1000L
        val good = ShellHotspot.LocalOwner("a".repeat(64), 1, now + 1)
        for (owner in listOf(
            good.copy(deadline = now), good.copy(deadline = now - 1),
            good.copy(deadline = now + 40001), good.copy(activity = "invalid"),
            good.copy(request = 0), good.copy(request = -1),
        )) assertFalse(owner.valid(now))
    }
}
