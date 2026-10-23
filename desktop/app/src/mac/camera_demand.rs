// SPDX-License-Identifier: GPL-3.0-or-later
//! Passive source-consumer edges. Never opens a camera or grants phone permission.
#![allow(unsafe_code)]
use std::sync::atomic::{AtomicBool, Ordering};
static RUNNING: AtomicBool = AtomicBool::new(false);
unsafe extern "C" {
    fn wp_camera_demand_start(callback: extern "C" fn(bool));
    fn wp_camera_demand_stop();
}
extern "C" fn changed(active: bool) {
    if RUNNING.load(Ordering::Acquire) { crate::webcam::consumer_demand(active); }
}
pub fn start() {
    if !RUNNING.swap(true, Ordering::AcqRel) {
        // SAFETY: static callback remains valid until process exit; native observer owns no camera.
        unsafe { wp_camera_demand_start(changed); }
    }
}
pub fn stop() {
    RUNNING.store(false, Ordering::Release);
    // SAFETY: removal is serialized with native callbacks.
    unsafe { wp_camera_demand_stop(); }
}
