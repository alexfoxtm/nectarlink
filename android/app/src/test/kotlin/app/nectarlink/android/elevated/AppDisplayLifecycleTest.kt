// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.elevated

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class AppDisplayLifecycleTest {
    @Test
    fun anOwnedTaskHiddenByKeyguardKeepsItsCurrentWindow() {
        val state = classifyAppDisplayTask(identityMatches = true, childCount = 1, visible = false)
        assertEquals(AppDisplayTaskState.HIDDEN, state)
        assertTrue(shouldKeepAppDisplay(state, keyguardLocked = true))
    }

    @Test
    fun anEmptyOrMissingOwnedTaskIsRetiredEvenWhileKeyguardIsLocked() {
        val empty = classifyAppDisplayTask(identityMatches = true, childCount = 0, visible = null)
        val missing = classifyAppDisplayTask(identityMatches = false, childCount = null, visible = null)
        assertEquals(AppDisplayTaskState.ABSENT, empty)
        assertEquals(AppDisplayTaskState.ABSENT, missing)
        assertFalse(shouldKeepAppDisplay(empty, keyguardLocked = true))
        assertFalse(shouldKeepAppDisplay(missing, keyguardLocked = true))
    }
}
