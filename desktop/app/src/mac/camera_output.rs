// SPDX-License-Identifier: GPL-3.0-or-later
//! Serialized macOS CMIO sink ownership. No implicit start or OS installation.
//! `Lease` belongs to exactly one core WebcamSink, never just a peer ID.
#![allow(unsafe_code)]

use nectarlink_core::DeviceId;
use std::{
    ffi::c_void,
    ptr::NonNull,
    sync::{Arc, Mutex},
};

unsafe extern "C" {
    fn wp_camera_open(out: *mut *mut c_void) -> i32;
    fn wp_obs_camera_is_registered() -> i32;
    fn wp_obs_camera_open(out: *mut *mut c_void) -> i32;
    fn wp_liveloop_camera_is_present() -> i32;
    fn wp_liveloop_camera_is_registered() -> i32;
    fn wp_liveloop_camera_prepare_name() -> i32;
    fn wp_liveloop_camera_open(out: *mut *mut c_void) -> i32;
    fn wp_camera_push_bgrx(
        producer: *mut c_void,
        bytes: *const u8,
        length: usize,
        width: u32,
        height: u32,
    ) -> i32;
    fn wp_camera_close(producer: *mut c_void);
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Lease {
    device: DeviceId,
    generation: u64,
}

struct Producer(NonNull<c_void>);
// The opaque native handle has no thread affinity. All operations, including
// destruction, run under OUTPUT. Only this lease's decoder enqueues samples.
unsafe impl Send for Producer {}
impl Drop for Producer {
    fn drop(&mut self) {
        // SAFETY: exclusive ownership; OUTPUT prevents concurrent push/close.
        unsafe { wp_camera_close(self.0.as_ptr()) };
    }
}
struct State {
    generation: u64,
    active: Option<(Lease, Producer, Arc<tokio::sync::Notify>)>,
}
static OUTPUT: Mutex<State> = Mutex::new(State { generation: 0, active: None });

/// Call only after core webcam capability, toggle, and paired-session admission.
/// A second stream is refused, including from the same phone, until teardown.
pub fn claim(device: DeviceId, stopped: Arc<tokio::sync::Notify>) -> Result<Lease, String> {
    claim_output(device, stopped, wp_camera_open)
}

/// Metadata-only query; never starts OBS or a camera stream.
pub fn obs_ready() -> bool {
    // SAFETY: native implementation only inspects public CMIO device metadata.
    unsafe { wp_obs_camera_is_registered() == 1 }
}

pub fn claim_obs(device: DeviceId, stopped: Arc<tokio::sync::Notify>) -> Result<Lease, String> {
    claim_output(device, stopped, wp_obs_camera_open)
}

/// Setup only: confirmed absence may open helper setup; errors must not.
/// Presence does not admit Start or frames.
pub fn liveloop_present() -> Result<bool, String> {
    // SAFETY: native helper enumerates device UIDs only, with no side effects.
    match unsafe { wp_liveloop_camera_is_present() } {
        0 => Ok(false),
        1 => Ok(true),
        status => Err(format!("Camera presence could not be determined ({status}).")),
    }
}

/// Metadata-only; readiness includes confirmed Waypair Camera display name.
pub fn liveloop_ready() -> bool {
    // SAFETY: native helper only reads public device and stream metadata.
    unsafe { wp_liveloop_camera_is_registered() == 1 }
}

/// Explicit setup action only. Never invoke from refresh or camera start.
/// Ok(false) requires a later readiness observation after device republish.
pub fn prepare_liveloop_name() -> Result<bool, String> {
    let state = OUTPUT.lock().unwrap_or_else(|e| e.into_inner());
    if state.active.is_some() {
        return Err("Stop the active camera output before changing its name.".into());
    }
    // SAFETY: OUTPUT excludes our producers; native helper checks external use
    // and only writes LiveLoop's public name property. It never opens a stream.
    match unsafe { wp_liveloop_camera_prepare_name() } {
        0 => Ok(true),
        2 => Ok(false),
        -4 => Err("Close apps using the camera before changing its name.".into()),
        status => Err(format!("Camera name setup unavailable ({status}).")),
    }
}

pub fn claim_liveloop(device: DeviceId, stopped: Arc<tokio::sync::Notify>) -> Result<Lease, String> {
    claim_output(device, stopped, wp_liveloop_camera_open)
}

fn claim_output(
    device: DeviceId,
    stopped: Arc<tokio::sync::Notify>,
    open: unsafe extern "C" fn(*mut *mut c_void) -> i32,
) -> Result<Lease, String> {
    let mut state = OUTPUT.lock().unwrap_or_else(|e| e.into_inner());
    if state.active.is_some() {
        return Err("Another phone camera stream owns the output.".into());
    }
    let generation = state.generation.checked_add(1).ok_or("Camera generation exhausted.")?;
    let mut raw = std::ptr::null_mut();
    // SAFETY: initialized out pointer, native helper stores a unique handle.
    let status = unsafe { open(&mut raw) };
    let pointer = NonNull::new(raw)
        .filter(|_| status == 0)
        .ok_or_else(|| format!("Camera output unavailable ({status})."))?;
    let lease = Lease { device, generation };
    state.generation = generation;
    state.active = Some((lease, Producer(pointer), stopped));
    Ok(lease)
}

/// Tight BGRX after webcam.rs applies the user's mirror preference once.
/// A revoked decoder can never publish into a successor's native stream.
pub fn publish(lease: Lease, width: u32, height: u32, pixels: &[u8]) -> Result<(), String> {
    let state = OUTPUT.lock().unwrap_or_else(|e| e.into_inner());
    let Some((_, producer, _)) = state.active.as_ref().filter(|(owner, _, _)| *owner == lease) else {
        return Err("Camera stream no longer owns the output.".into());
    };
    // SAFETY: helper copies the bounded slice synchronously, and OUTPUT holds
    // the native handle alive through the call; C rechecks dimensions/length.
    match unsafe { wp_camera_push_bgrx(producer.0.as_ptr(), pixels.as_ptr(), pixels.len(), width, height) } {
        0 | 1 => Ok(()), // an intentionally dropped frame is not a new stream
        status => Err(format!("Camera frame output failed ({status}).")),
    }
}

/// Sink end/drop must carry its own lease. A late end cannot close a new owner.
pub fn release(lease: Lease) {
    let mut state = OUTPUT.lock().unwrap_or_else(|e| e.into_inner());
    if state.active.as_ref().is_some_and(|(owner, _, _)| *owner == lease)
        && let Some((_, producer, stopped)) = state.active.take()
    {
        drop(producer);
        stopped.notify_one();
    }
}
pub fn shutdown() {
    let mut state = OUTPUT.lock().unwrap_or_else(|e| e.into_inner());
    if let Some((_, producer, stopped)) = state.active.take() {
        drop(producer);
        stopped.notify_one();
    }
}
