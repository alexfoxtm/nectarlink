// SPDX-License-Identifier: GPL-3.0-or-later
//! Explicit Camera Extension activation and live public-CMIO readiness.
//! Compilation, packaging or a successful activation request alone is not Ready.
#![allow(unsafe_code)]

use crate::i18n::tr;
use crate::{core_host, state::Changes};
use std::{
    ffi::c_void,
    sync::atomic::{AtomicI64, AtomicU8, Ordering},
};

pub const REGISTER_ARG: &str = "--register-vcam";
pub const UNREGISTER_ARG: &str = "--unregister-vcam";
// 0 unknown/startup, 1 submitted, 2 needs approval, 3 activated/check device,
// 4 reboot required, 5 failed, 6 explicitly disabled in this process.
static STATUS: AtomicU8 = AtomicU8::new(0);
static LAST_ERROR: AtomicI64 = AtomicI64::new(0);

unsafe extern "C" {
    fn wp_camera_is_registered() -> i32;
    fn wp_camera_request_activation(
        enable: i32,
        callback: unsafe extern "C" fn(*mut c_void, i32, i64),
        context: *mut c_void,
    );
}

pub fn is_registered() -> bool {
    if matches!(STATUS.load(Ordering::Acquire), 1 | 2 | 4 | 5 | 6) {
        return false;
    }
    // SAFETY: side-effect-free query of the exact public device UID and its
    // one source/one sink. Does not install, start a stream, or access a camera.
    unsafe { wp_camera_is_registered() == 1 }
}
pub fn status_text() -> String {
    if is_registered() {
        return tr!("Webcam", "Waypair Camera is ready · 720p at 30 fps.");
    }
    match STATUS.load(Ordering::Acquire) {
        1 => tr!("Webcam", "Requesting camera extension activation…"),
        2 => tr!("Webcam", "Allow the Waypair camera extension in System Settings, then return here."),
        3 => tr!("Webcam", "Activation completed. Click Check camera to check whether the device is ready."),
        4 => tr!("Webcam", "Restart macOS to finish the camera extension request."),
        5 => tr!(
            "Webcam",
            "Camera extension request failed (OS error %1).",
            LAST_ERROR.load(Ordering::Relaxed)
        ),
        6 => tr!("Webcam", "Camera extension is disabled."),
        _ => tr!("Webcam", "Enable the camera extension to use your phone in video apps."),
    }
}
pub fn setup_label() -> String {
    if STATUS.load(Ordering::Acquire) == 3 {
        tr!("Webcam", "Check camera")
    } else {
        tr!("Webcam", "Enable camera extension")
    }
}
pub fn setup_allowed() -> bool {
    !matches!(STATUS.load(Ordering::Acquire), 1 | 2 | 4)
}

// Preserve the shared Windows CLI surface without executing an elevated helper
// or silently activating a system extension at startup.
pub fn register_hklm() -> Result<(), String> {
    Err("Enable the camera extension in Settings.".into())
}
pub fn unregister_hklm() -> Result<(), String> {
    Err("Remove the camera extension in Settings.".into())
}

struct Activation {
    enabled: bool,
    done: Option<Box<dyn FnOnce(bool) + Send>>,
}
fn announce() {
    core_host::host().hub.changed(Changes::WEBCAM);
    core_host::refresh_capabilities();
}
unsafe extern "C" fn activation_event(context: *mut c_void, status: i32, error: i64) {
    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        if context.is_null() {
            return;
        }
        // SAFETY: the native delegate returns the exact Box pointer and emits
        // one terminal callback. Pending callbacks borrow, terminal takes it.
        let activation = unsafe { &mut *context.cast::<Activation>() };
        match status {
            0 => {
                STATUS.store(1, Ordering::Release);
                announce();
                return;
            }
            1 => {
                STATUS.store(2, Ordering::Release);
                announce();
                return;
            }
            2 => STATUS.store(if activation.enabled { 3 } else { 6 }, Ordering::Release),
            3 => STATUS.store(4, Ordering::Release),
            _ => {
                LAST_ERROR.store(error, Ordering::Relaxed);
                STATUS.store(5, Ordering::Release);
            }
        }
        let mut activation = unsafe { Box::from_raw(context.cast::<Activation>()) };
        let ready = is_registered();
        if let Some(done) = activation.done.take() {
            done(ready);
        }
        announce();
    }));
}

/// Shared name retained for callers; this submits only a public SystemExtensions
/// request after an explicit UI action. It launches no privileged subprocess.
pub fn run_elevated(arg: &'static str, on_done: impl FnOnce(bool) + Send + 'static) {
    let enabled = arg == REGISTER_ARG;
    if arg != REGISTER_ARG && arg != UNREGISTER_ARG {
        on_done(false);
        return;
    }
    if enabled && STATUS.load(Ordering::Acquire) == 3 {
        on_done(is_registered());
        announce();
        return;
    }
    if enabled && !setup_allowed() {
        on_done(false);
        return;
    }
    if enabled && is_registered() {
        on_done(true);
        announce();
        return;
    }
    // Stop output synchronously before deactivation; existing consumers get
    // black, and only the owning core stream is notified to close.
    STATUS.store(1, Ordering::Release);
    if !enabled {
        crate::webcam::stop();
    }
    announce();
    let context = Box::into_raw(Box::new(Activation { enabled, done: Some(Box::new(on_done)) }));
    // SAFETY: retained Box survives approval pending; the delegate owns callback
    // lifetime until one terminal result. No image or credential crosses it.
    unsafe { wp_camera_request_activation(i32::from(enabled), activation_event, context.cast()) };
}

#[derive(Debug, Default)]
pub struct VirtualCameraHandle;
impl VirtualCameraHandle {
    pub fn start() -> Result<Self, String> {
        is_registered().then_some(Self).ok_or_else(status_text)
    }
}
