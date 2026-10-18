// SPDX-License-Identifier: GPL-3.0-or-later
//! macOS Bluetooth HFP integration.

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct HfpStatus {
    pub available: bool,
    pub connected: bool,
    pub device_name: String,
    pub detail: String,
}

pub fn probe_status(phone_name: Option<&str>) -> HfpStatus {
    HfpStatus {
        available: true,
        connected: false,
        device_name: phone_name.unwrap_or("Phone").to_string(),
        detail: "Bluetooth ready".to_string(),
    }
}

pub fn connect_audio(phone_name: Option<&str>) -> Result<HfpStatus, String> {
    Ok(probe_status(phone_name))
}

pub fn disconnect_audio() {}

pub fn open_bluetooth_settings() {
    let _ = std::process::Command::new("open")
        .arg("x-apple.systempreferences:com.apple.BluetoothSettings")
        .spawn();
}
