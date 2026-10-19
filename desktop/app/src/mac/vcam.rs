// SPDX-License-Identifier: GPL-3.0-or-later
//! macOS virtual camera.
//!
//! A virtual camera on macOS needs a Camera Extension (a signed system
//! extension), which this app doesn't ship. Nothing is registered, so every
//! registration call fails rather than reporting a camera that other apps
//! will never see; `webcam.virtual` isn't advertised to phones.

use crate::i18n::tr;

pub const REGISTER_ARG: &str = "--register-vcam";
pub const UNREGISTER_ARG: &str = "--unregister-vcam";

pub fn is_registered() -> bool {
    false
}

fn unsupported() -> String {
    tr!("Webcam", "Phone as webcam isn't available on macOS yet.")
}

pub fn register_hklm() -> Result<(), String> {
    Err(unsupported())
}

pub fn unregister_hklm() -> Result<(), String> {
    Err(unsupported())
}

pub fn run_elevated(_arg: &'static str, on_done: impl FnOnce(bool) + Send + 'static) {
    on_done(false);
}

#[derive(Debug, Default)]
pub struct VirtualCameraHandle;

impl VirtualCameraHandle {
    pub fn start() -> Result<Self, String> {
        Err(unsupported())
    }
}
