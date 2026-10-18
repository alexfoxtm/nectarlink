// SPDX-License-Identifier: GPL-3.0-or-later
//! macOS / non-Windows PC audio stream stubs.

pub fn target() -> Option<nectarlink_core::DeviceId> {
    None
}

pub fn set_target(_phone: Option<nectarlink_core::DeviceId>) {}

pub fn is_enabled() -> bool {
    false
}

pub fn set_enabled(_enabled: bool) {}

pub fn f32_stereo_to_i16le(_samples: &[f32]) -> Vec<u8> {
    Vec::new()
}
