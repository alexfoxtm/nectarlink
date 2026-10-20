// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.mirror

import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class MirrorSessionPolicyTest {
    @Test
    fun aDistinctGenerationFromTheSamePcReplacesTheLiveCapture() {
        // A Mac reconnect mints a new generation; the stale capture retires.
        assertTrue(MirrorSessionPolicy.shouldPreempt("pc", 7u, "pc", 8u))
        // The core counter resets on process restart: a smaller-but-distinct
        // admitted token is still a replacement, not a reordered stale one.
        assertTrue(MirrorSessionPolicy.shouldPreempt("pc", 7u, "pc", 6u))
        // A correlated request supersedes a legacy capture its Mac refuses.
        assertTrue(MirrorSessionPolicy.shouldPreempt("pc", null, "pc", 8u))
    }

    @Test
    fun staleDuplicateAndForeignRequestsKeepTheLiveCapture() {
        // The same consent delivered twice (notification tap + foreground open).
        assertFalse(MirrorSessionPolicy.shouldPreempt("pc", 7u, "pc", 7u))
        // Another PC must not kill a consented share.
        assertFalse(MirrorSessionPolicy.shouldPreempt("pc", 7u, "other", 9u))
        // Legacy duplicates keep legacy behavior; an unstamped request never
        // displaces a correlated session it cannot be ordered against.
        assertFalse(MirrorSessionPolicy.shouldPreempt("pc", null, "pc", null))
        assertFalse(MirrorSessionPolicy.shouldPreempt("pc", 7u, "pc", null))
    }

    @Test
    fun liveJobWithMatchingEpochPcAndRequestOwnsTheCapture() {
        assertTrue(MirrorSessionPolicy.isLive(1L, "pc", 8u, 1L, "pc", 8u))
    }

    @Test
    fun staleEpochIsFencedEvenWhenPcAndRequestMatch() {
        // Stop/restart reuses the same peer and token (or legacy nulls);
        // the bumped service-local epoch alone retires the old job.
        assertFalse(MirrorSessionPolicy.isLive(1L, "pc", 8u, 0L, "pc", 8u))
        assertFalse(MirrorSessionPolicy.isLive(1L, "pc", null, 0L, "pc", null))
        assertFalse(MirrorSessionPolicy.isLive(1L, "pc", 8u, 1L, "pc", 9u))
        assertFalse(MirrorSessionPolicy.isLive(1L, "pc", 8u, 1L, "other", 8u))
    }
}
