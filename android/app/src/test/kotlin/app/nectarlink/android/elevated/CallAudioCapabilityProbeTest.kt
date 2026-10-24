// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.elevated

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Test

class CallAudioCapabilityProbeTest {
    @Test
    fun completeReadOnlyContractStillCannotQualifyDuplex() {
        val result = classifyCallAudioProbe(
            CallAudioProbeSnapshot(List(5) { true }, List(3) { true }, 2, true),
        )
        assertEquals(CallAudioProbeReason.READ_ONLY_CONTRACT_AVAILABLE, result.reason)
        assertFalse(result.duplexQualified)
    }

    @Test
    fun deniedAndUnknownCapabilitiesStayClosed() {
        val denied = classifyCallAudioProbe(
            CallAudioProbeSnapshot(listOf(false, true, true, true, true), List(3) { true }, 2, true),
        )
        val unknown = classifyCallAudioProbe(CallAudioProbeSnapshot())
        assertEquals(CallAudioProbeReason.PERMISSION_DENIED, denied.reason)
        assertEquals(CallAudioProbeReason.UNKNOWN, unknown.reason)
        assertFalse(denied.duplexQualified)
        assertFalse(unknown.duplexQualified)
    }
}
