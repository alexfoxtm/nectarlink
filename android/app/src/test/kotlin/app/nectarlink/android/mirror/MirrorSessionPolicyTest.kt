// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.mirror

import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class MirrorSessionPolicyTest {
    @Test
    fun aNewerGenerationFromTheSamePcReplacesTheLiveCapture() {
        // A Mac reconnect mints a new generation; the stale capture retires.
        assertTrue(MirrorSessionPolicy.shouldPreempt("pc", 7u, "pc", 8u))
        // A correlated request supersedes a legacy capture its Mac refuses.
        assertTrue(MirrorSessionPolicy.shouldPreempt("pc", null, "pc", 8u))
    }

    @Test
    fun staleDuplicateAndForeignRequestsKeepTheLiveCapture() {
        // The same consent delivered twice (notification tap + foreground open).
        assertFalse(MirrorSessionPolicy.shouldPreempt("pc", 7u, "pc", 7u))
        // A reordered older generation must not reopen its capture.
        assertFalse(MirrorSessionPolicy.shouldPreempt("pc", 7u, "pc", 6u))
        // Another PC must not kill a consented share.
        assertFalse(MirrorSessionPolicy.shouldPreempt("pc", 7u, "other", 9u))
        // Legacy duplicates keep legacy behavior; an unstamped request never
        // displaces a correlated session it cannot be ordered against.
        assertFalse(MirrorSessionPolicy.shouldPreempt("pc", null, "pc", null))
        assertFalse(MirrorSessionPolicy.shouldPreempt("pc", 7u, "pc", null))
    }
}
