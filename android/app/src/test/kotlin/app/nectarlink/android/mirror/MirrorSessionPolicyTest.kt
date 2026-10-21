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
        // Policy accepts distinct authenticated tokens; the core now allocates
        // them durably, while older peers may have used a resetting counter.
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

    @Test
    fun aStartSentAfterThePhonesStopMayCapture() {
        assertTrue(MirrorSessionPolicy.admitsStart(issue = 5L, stoppedThrough = 4L))
    }

    @Test
    fun aStartSentBeforeThePhonesStopNeverCaptures() {
        // Already on its way when the user tapped Stop (or revoked the capture).
        assertFalse(MirrorSessionPolicy.admitsStart(issue = 4L, stoppedThrough = 4L))
        assertFalse(MirrorSessionPolicy.admitsStart(issue = 3L, stoppedThrough = 4L))
        // An unstamped start can't be ordered against a Stop.
        assertFalse(MirrorSessionPolicy.admitsStart(issue = 0L, stoppedThrough = 0L))
    }
    @Test
    fun onlyTheLiveCaptureOnTheOriginalRequestAndConnectionAcceptsControl() {
        assertTrue(MirrorSessionPolicy.admitsCaptureOwner(8uL, 4uL, 4uL, true))
        assertTrue(MirrorSessionPolicy.admitsCaptureOwner(null, 4uL, null, true)) // legacy viewing, original live connection
        assertTrue(MirrorSessionPolicy.canControl(true, "pc", 8uL, 4uL, "pc", 8uL, 4uL))
    }

    @Test
    fun chooserStopRevokeAndOldOrForeignCaptureOwnersRefuseControl() {
        assertFalse(MirrorSessionPolicy.admitsCaptureOwner(8uL, null, 4uL, true))
        assertFalse(MirrorSessionPolicy.admitsCaptureOwner(8uL, 4uL, null, true))
        assertFalse(MirrorSessionPolicy.admitsCaptureOwner(8uL, 4uL, 3uL, true))
        assertFalse(MirrorSessionPolicy.admitsCaptureOwner(null, 4uL, null, false))
        assertFalse(MirrorSessionPolicy.canControl(false, "pc", 8uL, 4uL, "pc", 8uL, 4uL))
        assertFalse(MirrorSessionPolicy.canControl(true, "pc", null, null, "pc", 8uL, 4uL))
        assertFalse(MirrorSessionPolicy.canControl(true, "pc", 8uL, 4uL, "other", 8uL, 4uL))
        assertFalse(MirrorSessionPolicy.canControl(true, "pc", 8uL, 4uL, "pc", 7uL, 4uL))
        assertFalse(MirrorSessionPolicy.canControl(true, "pc", 8uL, 4uL, "pc", 8uL, 3uL))
        assertFalse(MirrorSessionPolicy.canControl(true, "pc", 8uL, 0uL, "pc", 8uL, 0uL))
    }
}
