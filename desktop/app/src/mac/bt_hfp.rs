// SPDX-License-Identifier: GPL-3.0-or-later
//! macOS Bluetooth HFP integration.
//!
//! macOS has no public API for a computer to act as the Hands-Free audio
//! gateway for a phone's calls, and no route has been proven against
//! hardware here. Until one exists this module reports PC call audio as
//! unavailable: call alerts, the log and dialling still work, and the audio
//! stays on the phone. It must never claim a connected or ready route.

use crate::i18n::tr;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct HfpStatus {
    pub available: bool,
    pub connected: bool,
    pub device_name: String,
    pub detail: String,
}

/// Probes whether call audio can be routed through this Mac. It can't yet, so
/// the status is always unavailable with a plain-words reason.
pub fn probe_status(_phone_name: Option<&str>) -> HfpStatus {
    HfpStatus {
        available: false,
        connected: false,
        device_name: String::new(),
        detail: tr!(
            "BtHfp",
            "PC call audio isn't available on macOS yet; the call's audio stays on your phone."
        ),
    }
}

/// There is no audio route to connect; the error carries the same reason the
/// status shows so the Calls page can say why.
pub fn connect_audio(phone_name: Option<&str>) -> Result<HfpStatus, String> {
    Err(probe_status(phone_name).detail)
}

pub fn disconnect_audio() {}

pub fn open_bluetooth_settings() {
    if let Err(e) = std::process::Command::new("open")
        .arg("x-apple.systempreferences:com.apple.BluetoothSettings")
        .spawn()
    {
        tracing::warn!(error = %e, "can't open Bluetooth settings");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn call_audio_is_reported_unavailable_and_connecting_fails() {
        let status = probe_status(Some("Pixel"));
        assert!(!status.available);
        assert!(!status.connected);
        assert!(status.device_name.is_empty());
        assert!(!status.detail.is_empty());
        let err = connect_audio(Some("Pixel")).expect_err("no HFP route exists on macOS");
        assert_eq!(err, status.detail);
    }
}
