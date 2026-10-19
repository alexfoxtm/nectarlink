// SPDX-License-Identifier: GPL-3.0-or-later
//! macOS Wake-on-LAN integration.
//!
//! Adapters and their wake settings aren't read on macOS yet, so the status
//! lists no adapters and no hardware addresses; phones then get an empty
//! `pc.wake_info` (the same as a PC with wake turned off) instead of
//! addresses that were never checked.

use nectarlink_core::PcWakeInfo;

use crate::i18n::tr;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WakeSupport {
    Enabled,
    Disabled,
    Unknown,
}

impl WakeSupport {
    pub fn as_str(self) -> &'static str {
        match self {
            WakeSupport::Enabled => "enabled",
            WakeSupport::Disabled => "disabled",
            WakeSupport::Unknown => "unknown",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WakeAdapter {
    pub label: String,
    pub wired: bool,
    pub mac: String,
    pub broadcasts: Vec<String>,
    pub support: WakeSupport,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct WakeStatus {
    pub info: PcWakeInfo,
    pub adapters: Vec<WakeAdapter>,
}

impl WakeStatus {
    pub fn primary(&self) -> Option<&WakeAdapter> {
        self.adapters.first()
    }

    pub fn state_str(&self) -> &'static str {
        match self.primary() {
            Some(a) => a.support.as_str(),
            None => "none",
        }
    }
}

/// No adapters are enumerated yet: empty status, `state_str() == "none"`.
pub fn query_wake_status() -> WakeStatus {
    WakeStatus::default()
}

/// Nothing is sent; callers get an error instead of a silent "woke it".
pub fn wake(_mac: &[u8]) -> Result<(), String> {
    Err(tr!("Wake", "Waking other PCs isn't available on macOS yet."))
}
