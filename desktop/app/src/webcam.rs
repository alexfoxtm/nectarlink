// SPDX-License-Identifier: GPL-3.0-or-later
//! Phone as a webcam (`docs/protocol/webcam.md`): decodes a phone's H.264
//! camera stream on a dedicated thread, publishes BGRX frames into the
//! shared-memory mapping read by `nectarlink_vcam.dll` (the global one the
//! camera creates inside Windows' Frame Server, or this session's while it
//! isn't there), and feeds the live preview `VideoView` (`stream:
//! "webcam"`) in Settings.
//!
//! ## 4K (3840×2160) Support & Why 10-bit HDR Is Omitted
//!
//! End-to-end 4K UHD (`3840×2160` @ 30 fps) is supported across the phone
//! camera encoder, the desktop H.264 decoder, the shared-memory frame
//! transport (`nectarlink-vcam` v2), and the Media Foundation virtual camera
//! source (`RGB32` and `NV12`).
//!
//! 10-bit HDR (`P010` / BT.2020 PQ or HLG) is intentionally omitted: Windows'
//! inbox H.264 Media Foundation Transform (`CLSID_CMSH264DecoderMFT`) decodes
//! 8-bit 4:2:0 profiles into 8-bit `NV12` (`MFVideoFormat_NV12`), and the
//! shared-memory virtual camera pipeline operates in 8-bit-per-channel SDR
//! Rec.709 (`BGRX` / `MFVideoFormat_RGB32` and `MFVideoFormat_NV12` with
//! `MFVideoTransferMatrix_BT709`) for universal compatibility with conferencing
//! apps.

use std::sync::{
    Arc, Mutex, OnceLock,
    atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering},
    mpsc::{Receiver, SyncSender, TrySendError, sync_channel},
};

#[cfg(target_os = "macos")]
use crate::win::camera_output;
use nectarlink_core::{DeviceId, Error, NodeEvent, WebcamConfig, WebcamSink, WebcamStart};
#[cfg(not(target_os = "macos"))]
use nectarlink_vcam::SharedFrameMapping;
use nectarlink_vcam::render_placeholder_bgrx;
#[cfg(not(target_os = "macos"))]
use nectarlink_vcam::{DEFAULT_MAPPING_NAME, LOCAL_MAPPING_NAME};
#[cfg(not(target_os = "macos"))]
use std::time::{Duration, Instant};

use crate::{
    bridge::{app::describe, native::ffi},
    core_host,
    settings::Settings,
    state::Changes,
    win::{
        h264::{Decoder, to_bgrx},
        vcam::{self, VirtualCameraHandle},
    },
};

/// Stream key used by `VideoView` in `SettingsPage.qml`.
pub const PREVIEW_STREAM: &str = "webcam";

/// Packets waiting for the decoder, at most (~2 seconds at 30 fps).
const BACKLOG: usize = 60;

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum Phase {
    #[default]
    Idle,
    Asking {
        device: DeviceId,
    },
    Streaming {
        device: DeviceId,
        width: u32,
        height: u32,
        fps: u32,
    },
    Ended {
        reason: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum OutputTarget {
    #[default]
    NativeCamera,
    PreviewOnly,
}

#[derive(Debug, Default)]
struct State {
    output_target: OutputTarget,
    #[cfg(target_os = "macos")]
    sink_stopped: Option<Arc<tokio::sync::Notify>>,
    phase: Phase,
    preferred_phone: Option<DeviceId>,
    addon_registered: bool,
    addon_busy: bool,
    vcam_handle: Option<VirtualCameraHandle>,
    #[cfg(target_os = "macos")]
    next_camera_generation: u64,
    #[cfg(target_os = "macos")]
    request: Option<CameraRequest>,
    #[cfg(target_os = "macos")]
    sink_generation: Option<u64>,
    #[cfg(target_os = "macos")]
    capture_view: Option<nectarlink_core::WebcamView>,
    #[cfg(target_os = "macos")]
    shutting_down: bool,
    #[cfg(not(target_os = "macos"))]
    mapping: Option<Arc<SharedFrameMapping>>,
    /// Whether `mapping` is the camera's global one (else this session's).
    #[cfg(not(target_os = "macos"))]
    mapping_global: bool,
    /// When joining the global one was last tried.
    #[cfg(not(target_os = "macos"))]
    global_tried: Option<std::time::Instant>,
}

#[cfg(target_os = "macos")]
#[derive(Debug)]
struct CameraRequest {
    // Application ownership only; never a guessed Core/wire request ID.
    generation: u64,
    device: DeviceId,
    cancelled: bool,
    sink_admitted: bool,
    view: Option<nectarlink_core::WebcamView>,
    actor_running: bool,
    cancel_attempt: u64,
    stop_error: Option<String>,
    wake: Arc<tokio::sync::Notify>,
}

#[cfg(target_os = "macos")]
impl State {
    fn select_output_target(&mut self, target: OutputTarget) -> bool {
        if self.shutting_down
            || self.request.is_some()
            || self.sink_generation.is_some()
            || self.capture_view.is_some()
        {
            return false;
        }
        self.output_target = target;
        true
    }

    fn notify_sink_stopped(&mut self) {
        if let Some(stopped) = self.sink_stopped.take() {
            stopped.notify_one();
        }
    }
    fn next_camera_generation(&mut self) -> Option<u64> {
        self.next_camera_generation = self.next_camera_generation.checked_add(1)?;
        Some(self.next_camera_generation)
    }

    fn begin_camera_request(&mut self, device: DeviceId, reconfigure: bool) -> Option<u64> {
        if self.shutting_down
            || self.request.is_some()
            || (!reconfigure && (self.sink_generation.is_some() || self.capture_view.is_some()))
            || (reconfigure && self.sink_generation.is_none())
        {
            return None;
        }
        let generation = self.next_camera_generation()?;
        let wake = Arc::new(tokio::sync::Notify::new());
        self.request = Some(CameraRequest {
            generation,
            device,
            cancelled: false,
            sink_admitted: reconfigure,
            view: if reconfigure { self.capture_view.clone() } else { None },
            actor_running: true,
            cancel_attempt: 0,
            stop_error: None,
            wake,
        });
        if !reconfigure {
            self.preferred_phone = Some(device);
            self.phase = Phase::Asking { device };
        }
        Some(generation)
    }

    fn cancel_camera_request(&mut self) -> bool {
        let Some(request) = self.request.as_mut() else {
            return false;
        };
        request.cancelled = true;
        request.cancel_attempt = request.cancel_attempt.saturating_add(1);
        request.stop_error = None;
        if !matches!(self.phase, Phase::Ended { .. }) {
            self.phase = Phase::Asking { device: request.device };
        }
        request.wake.notify_one();
        true
    }

    fn retire_camera_request(&mut self, generation: u64) -> bool {
        if !self.request.as_ref().is_some_and(|request| request.generation == generation) {
            return false;
        }
        let request = self.request.take().expect("matched request");
        if request.cancelled
            && self.sink_generation.is_none()
            && request
                .view
                .as_ref()
                .is_some_and(|view| self.capture_view.as_ref().is_some_and(|live| live.same_owner(view)))
        {
            self.capture_view = None;
        }
        if request.cancelled && self.sink_generation.is_none() && !matches!(self.phase, Phase::Ended { .. }) {
            self.phase = Phase::Idle;
        }
        true
    }

    fn ensure_camera_cleanup(&mut self) -> Option<u64> {
        if let Some(request) = &self.request {
            return Some(request.generation);
        }
        let view = self.capture_view.clone()?;
        let generation = self.next_camera_generation()?;
        self.request = Some(CameraRequest {
            generation,
            device: view.peer(),
            cancelled: true,
            sink_admitted: true,
            view: Some(view),
            actor_running: false,
            cancel_attempt: 1,
            stop_error: None,
            wake: Arc::default(),
        });
        Some(generation)
    }
    fn take_cleanup_actor(&mut self) -> Option<u64> {
        let request = self.request.as_mut()?;
        if !request.cancelled || request.actor_running {
            return None;
        }
        request.actor_running = true;
        Some(request.generation)
    }

    // Controller writes and native/preview cleanup are one guarded commit
    // under STATE. An old decoder cannot pass a check and write after B starts.
    fn finish_camera_sink(
        &mut self,
        generation: u64,
        reason: Option<String>,
        close_and_clear: impl FnOnce(),
    ) -> bool {
        if self.sink_generation != Some(generation) {
            return false;
        }
        self.sink_generation = None;
        self.cancel_camera_request();
        self.ensure_camera_cleanup();
        self.phase = match reason {
            Some(reason) => Phase::Ended { reason },
            None if self.request.is_some() && !self.shutting_down => {
                Phase::Asking { device: self.request.as_ref().expect("pending request").device }
            }
            None => Phase::Idle,
        };
        self.notify_sink_stopped();
        close_and_clear();
        true
    }
}

static STATE: Mutex<Option<State>> = Mutex::new(None);
static MIRROR: AtomicBool = AtomicBool::new(false);
static CAMERA_FRONT: AtomicBool = AtomicBool::new(false);
static HEIGHT: AtomicU32 = AtomicU32::new(720);
/// Whether the phone's camera in use can stream 4K, as its last stream said
/// (offered until a phone says it can't).
static UHD: AtomicBool = AtomicBool::new(true);
static PREVIEW_ACTIVE: AtomicBool = AtomicBool::new(false);
static PREVIEW_WINDOW_OPEN: AtomicBool = AtomicBool::new(false);

fn preview_active() -> bool {
    PREVIEW_ACTIVE.load(Ordering::Acquire) || PREVIEW_WINDOW_OPEN.load(Ordering::Acquire)
}

pub fn preview_window_open() -> bool {
    PREVIEW_WINDOW_OPEN.load(Ordering::Acquire)
}

/// The app-owned window observes the existing stream; it never starts another request.
pub fn set_preview_window_open(open: bool) {
    if open && !preview_only() {
        return;
    }
    let previous = PREVIEW_WINDOW_OPEN.swap(open, Ordering::AcqRel);
    if previous == open {
        return;
    }
    if open {
        refresh_idle_frame();
    } else {
        stop();
        set_preview_active(PREVIEW_ACTIVE.load(Ordering::Acquire));
    }
    core_host::host().hub.changed(Changes::WEBCAM);
}

fn state<T>(f: impl FnOnce(&mut State) -> T) -> T {
    f(STATE.lock().unwrap_or_else(|e| e.into_inner()).get_or_insert_default())
}

fn normalize_height(h: u32) -> u32 {
    match h {
        2160 => 2160,
        1080 => 1080,
        _ => 720,
    }
}

/// 720, 1080 or 2160 for a picture of `width` × `height`, either way up (an
/// upright phone streams 1080 × 1920 for 1080p).
fn resolution_of(width: u32, height: u32) -> u32 {
    match width.min(height) {
        2000.. => 2160,
        1000.. => 1080,
        _ => 720,
    }
}

/// What the phone is streaming now, chosen here or on the phone (or 1080p
/// when it can't do 4K): the resolution buttons follow it, and the next
/// stream starts with it. Doesn't ask the phone again.
fn follow_stream(width: u32, height: u32) {
    let h = resolution_of(width, height);
    let prev = HEIGHT.swap(h, Ordering::Relaxed);
    #[cfg(not(test))]
    if prev != h {
        save_settings(|s| s.webcam_height = h);
    }
    #[cfg(test)]
    let _ = prev;
}

/// Maps a resolution label (`"720"`, `"1080"`, `"2160"`, `"4k"`, `"4K"`) to
/// `(width, height)` in pixels.
pub fn dimensions(res: &str) -> (u16, u16) {
    match res.trim() {
        "2160" | "4k" | "4K" => (3840, 2160),
        "1080" => (1920, 1080),
        _ => (1280, 720),
    }
}

/// Returns the currently selected vertical resolution as `"720"`, `"1080"`, or `"2160"`.
pub fn resolution() -> &'static str {
    match height() {
        2160 => "2160",
        1080 => "1080",
        _ => "720",
    }
}

/// Sets the webcam resolution from a string token (`"720"`, `"1080"`, or `"2160"`).
/// Unrecognized values are ignored.
#[cfg(test)]
pub fn set_resolution(res: &str) {
    let h = match res.trim() {
        "2160" => 2160,
        "1080" => 1080,
        "720" => 720,
        _ => return,
    };
    set_height(h);
}

/// Initializes webcam preferences and (if registered) the Windows virtual
/// camera session.
pub fn init(settings: &Settings) {
    MIRROR.store(settings.webcam_mirror, Ordering::Relaxed);
    CAMERA_FRONT.store(settings.webcam_camera.trim().eq_ignore_ascii_case("front"), Ordering::Relaxed);
    let h = normalize_height(settings.webcam_height);
    HEIGHT.store(h, Ordering::Relaxed);
    let preferred = settings.webcam_phone.as_deref().and_then(|s| s.parse().ok());
    let registered = vcam::is_registered();
    let vcam_handle = if registered {
        match VirtualCameraHandle::start() {
            Ok(handle) => Some(handle),
            Err(e) => {
                tracing::warn!(error = %e, "can't start IMFVirtualCamera session");
                None
            }
        }
    } else {
        None
    };
    state(|s| {
        s.preferred_phone = preferred;
        s.addon_registered = registered;
        s.vcam_handle = vcam_handle;
    });
    if registered {
        refresh_idle_frame();
    }
}

/// Stops the virtual camera session on app exit.
pub fn shutdown() {
    #[cfg(target_os = "macos")]
    {
        state(|s| {
            s.shutting_down = true;
            s.cancel_camera_request();
            s.sink_generation = None;
            s.notify_sink_stopped();
            camera_output::shutdown();
            s.phase = Phase::Idle;
            s.vcam_handle = None;
            ffi::video_clear(PREVIEW_STREAM);
        });
    }
    #[cfg(not(target_os = "macos"))]
    state(|s| {
        if let Some(mapping) = &s.mapping {
            mapping.set_idle("");
        }
        s.mapping = None;
        s.vcam_handle = None;
    });
}

fn save_settings(update: impl FnOnce(&mut Settings)) {
    let dir = &core_host::host().data_dir;
    let mut s = Settings::load(dir);
    update(&mut s);
    if let Err(e) = s.save(dir) {
        tracing::warn!(error = %e, "can't save webcam settings");
    }
}

/// Currently selected phone (`preferred_phone` if online, otherwise the first
/// online paired phone, falling back to `preferred_phone` if paired or the
/// first paired phone).
pub fn selected_phone() -> Option<DeviceId> {
    let preferred = state(|s| s.preferred_phone);
    core_host::host().hub.read(|app| {
        if let Some(id) = preferred
            && app
                .devices
                .iter()
                .any(|d| d.id == id && matches!(d.link, nectarlink_core::LinkState::Online { .. }))
        {
            return Some(id);
        }
        if let Some(online_id) = app
            .devices
            .iter()
            .find(|d| matches!(d.link, nectarlink_core::LinkState::Online { .. }))
            .map(|d| d.id)
        {
            return Some(online_id);
        }
        if let Some(id) = preferred
            && app.devices.iter().any(|d| d.id == id)
        {
            return Some(id);
        }
        app.devices.first().map(|d| d.id)
    })
}

pub fn set_selected_phone(device: DeviceId) {
    state(|s| s.preferred_phone = Some(device));
    save_settings(|s| s.webcam_phone = Some(device.to_string()));
    refresh_idle_frame();
    core_host::host().hub.changed(Changes::WEBCAM);
}

pub fn height() -> u32 {
    HEIGHT.load(Ordering::Relaxed)
}

/// Whether to offer 4K: the phone's camera in use can stream it, as far as
/// the PC knows.
pub fn uhd() -> bool {
    UHD.load(Ordering::Relaxed)
}

pub fn set_height(h: u32) {
    let h = normalize_height(h);
    let prev = HEIGHT.swap(h, Ordering::Relaxed);
    #[cfg(not(test))]
    {
        save_settings(|s| s.webcam_height = h);
        core_host::host().hub.changed(Changes::WEBCAM);
        if prev != h {
            reconfigure_active_stream();
        }
    }
    #[cfg(test)]
    let _ = prev;
}

pub fn camera() -> &'static str {
    if CAMERA_FRONT.load(Ordering::Relaxed) { "front" } else { "back" }
}

pub fn set_camera(cam: &str) {
    let front = cam.trim().eq_ignore_ascii_case("front");
    let prev = CAMERA_FRONT.swap(front, Ordering::Relaxed);
    #[cfg(not(test))]
    {
        save_settings(|s| s.webcam_camera = camera().to_owned());
        core_host::host().hub.changed(Changes::WEBCAM);
        if prev != front {
            reconfigure_active_stream();
        }
    }
    #[cfg(test)]
    let _ = prev;
}

pub fn mirror() -> bool {
    MIRROR.load(Ordering::Relaxed)
}

pub fn set_mirror(on: bool) {
    MIRROR.store(on, Ordering::Relaxed);
    #[cfg(not(test))]
    {
        save_settings(|s| s.webcam_mirror = on);
        core_host::host().hub.changed(Changes::WEBCAM);
    }
}

pub fn phase() -> Phase {
    state(|s| s.phase.clone())
}
pub fn cancellation_error() -> Option<String> {
    #[cfg(target_os = "macos")]
    {
        state(|s| s.request.as_ref().and_then(|request| request.stop_error.clone()))
    }
    #[cfg(not(target_os = "macos"))]
    {
        None
    }
}
pub fn cancelling() -> bool {
    #[cfg(target_os = "macos")]
    {
        state(|s| s.request.as_ref().is_some_and(|request| request.cancelled))
    }
    #[cfg(not(target_os = "macos"))]
    {
        false
    }
}

pub fn addon_registered() -> bool {
    #[cfg(target_os = "macos")]
    {
        state(|s| {
            s.addon_registered = vcam::is_registered();
            s.addon_registered
        })
    }
    #[cfg(not(target_os = "macos"))]
    {
        state(|s| s.addon_registered)
    }
}
pub fn addon_status_text() -> String {
    #[cfg(target_os = "macos")]
    {
        vcam::status_text()
    }
    #[cfg(not(target_os = "macos"))]
    {
        String::new()
    }
}
pub fn addon_action_text() -> String {
    #[cfg(target_os = "macos")]
    {
        vcam::setup_label()
    }
    #[cfg(not(target_os = "macos"))]
    {
        "Set up".into()
    }
}
pub fn addon_setup_allowed() -> bool {
    #[cfg(target_os = "macos")]
    {
        vcam::setup_allowed()
    }
    #[cfg(not(target_os = "macos"))]
    {
        true
    }
}

pub fn addon_busy() -> bool {
    state(|s| s.addon_busy)
}

/// Launches the elevated helper (`--register-vcam`) when the user clicks "Set up".
pub fn setup_addon() {
    let already_busy = state(|s| {
        if s.addon_busy {
            return true;
        }
        s.addon_busy = true;
        false
    });
    if already_busy {
        return;
    }
    core_host::host().hub.changed(Changes::WEBCAM);
    vcam::run_elevated(vcam::REGISTER_ARG, |_ok| {
        let registered = vcam::is_registered();
        let handle = if registered { VirtualCameraHandle::start().ok() } else { None };
        state(|s| {
            s.addon_busy = false;
            s.addon_registered = registered;
            if handle.is_some() {
                s.vcam_handle = handle;
            }
        });
        core_host::host().hub.changed(Changes::WEBCAM);
    });
}

/// Launches the elevated helper (`--unregister-vcam`) when the user clicks "Remove".
pub fn remove_addon() {
    let already_busy = state(|s| {
        if s.addon_busy {
            return true;
        }
        s.addon_busy = true;
        s.vcam_handle = None;
        false
    });
    if already_busy {
        return;
    }
    core_host::host().hub.changed(Changes::WEBCAM);
    vcam::run_elevated(vcam::UNREGISTER_ARG, |_ok| {
        let registered = vcam::is_registered();
        state(|s| {
            s.addon_busy = false;
            s.addon_registered = registered;
        });
        core_host::host().hub.changed(Changes::WEBCAM);
    });
}

fn phone_name_for(device: Option<DeviceId>) -> String {
    let Some(id) = device else { return String::new() };
    core_host::host().hub.read(|s| s.name_of(&id).unwrap_or_default())
}

/// The shared mapping to publish frames into: the camera's global one as soon
/// as it exists (the camera creates it when an app first opens it, so look
/// again every second until then), else this session's.
#[cfg(not(target_os = "macos"))]
fn mapping() -> Option<Arc<SharedFrameMapping>> {
    state(|s| {
        let due = s.global_tried.is_none_or(|at| at.elapsed() >= std::time::Duration::from_secs(1));
        if !s.mapping_global && due {
            s.global_tried = Some(std::time::Instant::now());
            if let Ok(global) = SharedFrameMapping::open_existing(DEFAULT_MAPPING_NAME) {
                tracing::debug!("joined the virtual camera's frame mapping");
                s.mapping = Some(Arc::new(global));
                s.mapping_global = true;
            }
        }
        if s.mapping.is_none() {
            s.mapping = SharedFrameMapping::open_or_create(LOCAL_MAPPING_NAME).ok().map(Arc::new);
        }
        s.mapping.clone()
    })
}

/// Updates the camera's global mapping (if a camera client has opened it)
/// without allocating the 33 MB local fallback mapping while idle.
#[cfg(not(target_os = "macos"))]
fn idle_mapping() -> Option<Arc<SharedFrameMapping>> {
    state(|s| {
        let due = s.global_tried.is_none_or(|at| at.elapsed() >= std::time::Duration::from_secs(1));
        if !s.mapping_global && due {
            s.global_tried = Some(std::time::Instant::now());
            if let Ok(global) = SharedFrameMapping::open_existing(DEFAULT_MAPPING_NAME) {
                tracing::debug!("joined the virtual camera's frame mapping");
                s.mapping = Some(Arc::new(global));
                s.mapping_global = true;
            }
        }
        s.mapping.clone()
    })
}

/// Updates the shared-memory header and (when Settings is visible) the QML
/// preview `VideoView` with the idle placeholder frame when no stream is active.
#[cfg(not(target_os = "macos"))]
fn refresh_idle_frame() {
    let is_streaming = state(|s| matches!(s.phase, Phase::Streaming { .. }));
    if is_streaming {
        return;
    }
    let phone = selected_phone();
    let name = phone_name_for(phone);
    #[cfg(not(target_os = "macos"))]
    if let Some(mapping) = idle_mapping() {
        mapping.set_idle(&name);
    }
    if preview_active() {
        let (w, h) = (640u32, 360u32);
        let mut bgrx = vec![0u8; (w * h * 4) as usize];
        render_placeholder_bgrx(&mut bgrx, w, h, &name);
        ffi::video_frame(PREVIEW_STREAM, w, h, &bgrx);
    }
}

#[cfg(target_os = "macos")]
fn refresh_idle_frame() {
    if !preview_active() {
        return;
    }
    let name = phone_name_for(selected_phone());
    let (width, height) = (640u32, 360u32);
    let mut pixels = vec![0; (width * height * 4) as usize];
    render_placeholder_bgrx(&mut pixels, width, height, &name);
    state(|s| {
        if !s.shutting_down
            && s.sink_generation.is_none()
            && !matches!(s.phase, Phase::Streaming { .. })
            && preview_active()
        {
            ffi::video_frame(PREVIEW_STREAM, width, height, &pixels);
        }
    });
}

/// Marks whether the Settings webcam preview is on screen so the idle
/// placeholder image is only held while needed.
pub fn set_preview_active(active: bool) {
    let prev = PREVIEW_ACTIVE.swap(active, Ordering::AcqRel);
    if active && !prev {
        refresh_idle_frame();
    } else if !active && !preview_window_open() {
        #[cfg(target_os = "macos")]
        state(|s| {
            if s.sink_generation.is_none() {
                ffi::video_clear(PREVIEW_STREAM);
            }
        });
        #[cfg(not(target_os = "macos"))]
        {
            let is_streaming = state(|s| matches!(s.phase, Phase::Streaming { .. }));
            if !is_streaming {
                ffi::video_clear(PREVIEW_STREAM);
            }
        }
    }
}

/// Releases the idle preview image and any local-only shared mapping when the
/// main window closes to the tray.
pub fn release_idle_resources() {
    PREVIEW_ACTIVE.store(false, Ordering::Release);
    if preview_window_open() {
        return;
    }
    #[cfg(target_os = "macos")]
    {
        state(|s| {
            if s.sink_generation.is_none() {
                ffi::video_clear(PREVIEW_STREAM);
            }
        });
    }
    #[cfg(not(target_os = "macos"))]
    {
        let is_streaming = state(|s| {
            let streaming = matches!(s.phase, Phase::Streaming { .. });
            if !streaming && !s.mapping_global {
                s.mapping = None;
            }
            streaming
        });
        if !is_streaming {
            ffi::video_clear(PREVIEW_STREAM);
        }
    }
}

fn current_webcam_start() -> WebcamStart {
    let (w_u16, h_u16) = dimensions(resolution());
    let (w, h) = (u32::from(w_u16), u32::from(h_u16));
    let bitrate = match h {
        2160 => 20_000_000,
        1080 => 8_000_000,
        _ => 4_000_000,
    };
    WebcamStart { width: w, height: h, fps: 30, bitrate, camera: camera().to_owned() }
}

#[cfg(all(not(test), not(target_os = "macos")))]
fn reconfigure_active_stream() {
    let active_device = state(|s| match s.phase {
        Phase::Streaming { device, .. } | Phase::Asking { device } => Some(device),
        Phase::Idle | Phase::Ended { .. } => None,
    });
    if let (Some(device), Some(node)) = (active_device, core_host::node()) {
        let options = current_webcam_start();
        core_host::spawn(async move {
            let _ = node.webcam_start(device, options).await;
        });
    }
}

#[cfg(all(not(test), target_os = "macos"))]
fn reconfigure_active_stream() {
    let request = state(|s| {
        let device = match s.phase {
            Phase::Streaming { device, .. } => device,
            _ => return None,
        };
        let generation = s.begin_camera_request(device, true)?;
        let view = s.request.as_ref()?.view.clone()?;
        Some((generation, view))
    });
    if let Some((generation, view)) = request {
        let options = current_webcam_start();
        core_host::spawn(async move {
            let result = view.configure(options).await;
            camera_request_completed(generation, result);
            complete_camera_request(generation).await;
        });
    }
}

#[cfg(target_os = "macos")]
fn camera_start_error(error: Error) -> String {
    match error {
        Error::Denied => "The camera request was refused or its previous owner is still closing.".into(),
        Error::Unsupported => {
            "This phone needs owned webcam support. Update Waypair Link on the phone.".into()
        }
        Error::Offline | Error::NotPaired => "The original phone connection ended.".into(),
        error => describe(&error),
    }
}

#[cfg(target_os = "macos")]
fn camera_request_completed(generation: u64, result: nectarlink_core::Result<()>) {
    if let Err(error) = result {
        let reason = camera_start_error(error);
        state(|s| {
            let Some(request) = s.request.as_mut().filter(|request| request.generation == generation) else {
                return;
            };
            request.cancelled = true;
            request.cancel_attempt = request.cancel_attempt.saturating_add(1);
            request.wake.notify_one();
            s.phase = Phase::Ended { reason };
        });
        core_host::host().hub.changed(Changes::WEBCAM);
    }
}

#[cfg(target_os = "macos")]
async fn camera_request(
    node: nectarlink_core::Node,
    device: DeviceId,
    generation: u64,
    options: WebcamStart,
) {
    let send_start = state(|s| {
        s.request.as_ref().is_some_and(|request| request.generation == generation && !request.cancelled)
    });
    if send_start {
        let result = node
            .webcam_start_owned(device, options, move |view| {
                state(|s| {
                    if s.shutting_down {
                        return false;
                    }
                    let Some(request) = s
                        .request
                        .as_mut()
                        .filter(|request| request.generation == generation && !request.cancelled)
                    else {
                        return false;
                    };
                    request.view = Some(view.clone());
                    s.capture_view = Some(view);
                    true
                })
            })
            .await;
        camera_request_completed(generation, result);
    }
    complete_camera_request(generation).await;
}

#[cfg(target_os = "macos")]
async fn complete_camera_request(generation: u64) {
    enum Next {
        Wait(Arc<tokio::sync::Notify>, Option<nectarlink_core::WebcamView>, bool),
        Stop(Option<nectarlink_core::WebcamView>, u64),
        Done,
    }
    let mut failed_attempt = None;
    loop {
        let next = state(|s| {
            let Some(request) = s.request.as_mut().filter(|request| request.generation == generation) else {
                return Next::Done;
            };
            if !request.cancelled && request.view.as_ref().is_some_and(|view| !view.is_current()) {
                request.cancelled = true;
                request.cancel_attempt = request.cancel_attempt.saturating_add(1);
            }
            if request.cancelled {
                let gone = request.view.as_ref().is_some_and(|view| !view.original_session_current());
                if failed_attempt != Some(request.cancel_attempt) || gone {
                    return Next::Stop(request.view.clone(), request.cancel_attempt);
                }
                return Next::Wait(request.wake.clone(), request.view.clone(), true);
            }
            if request.sink_admitted {
                s.retire_camera_request(generation);
                return Next::Done;
            }
            Next::Wait(request.wake.clone(), request.view.clone(), false)
        });
        match next {
            Next::Wait(wake, Some(view), cancelled) => {
                if cancelled {
                    tokio::select! { _ = wake.notified() => {}, _ = view.session_ended() => {} }
                } else {
                    tokio::select! { _ = wake.notified() => {}, _ = view.closed() => {} }
                }
            }
            Next::Wait(wake, None, _) => wake.notified().await,
            Next::Stop(view, attempt) => {
                let stopped = match view {
                    Some(view) => view.stop_if_current().await.map(|_| ()),
                    None => Ok(()),
                };
                if let Err(error) = stopped {
                    failed_attempt = Some(attempt);
                    let reason = describe(&error);
                    state(|s| {
                        if let Some(request) =
                            s.request.as_mut().filter(|request| request.generation == generation)
                        {
                            request.stop_error = Some(format!(
                                "Original camera cleanup is pending: {reason}. Press Stop to retry."
                            ));
                            s.phase = Phase::Asking { device: request.device };
                        }
                    });
                    core_host::host().hub.changed(Changes::WEBCAM);
                    continue;
                }
                state(|s| {
                    s.retire_camera_request(generation);
                });
                core_host::host().hub.changed(Changes::WEBCAM);
                return;
            }
            Next::Done => {
                core_host::host().hub.changed(Changes::WEBCAM);
                return;
            }
        }
    }
}

#[cfg(target_os = "macos")]
pub fn start(device: Option<DeviceId>) {
    let Some(device) = device.or_else(selected_phone) else {
        return;
    };
    let Some(node) = core_host::node() else {
        return;
    };
    let generation = state(|s| {
        if s.shutting_down || s.request.is_some() || s.sink_generation.is_some() {
            return None;
        }
        if s.output_target == OutputTarget::NativeCamera && !vcam::is_registered() {
            s.phase = Phase::Ended { reason: vcam::status_text() };
            return None;
        }
        s.begin_camera_request(device, false)
    });
    let Some(generation) = generation else {
        core_host::host().hub.changed(Changes::WEBCAM);
        return;
    };
    save_settings(|settings| settings.webcam_phone = Some(device.to_string()));
    core_host::host().hub.changed(Changes::WEBCAM);
    core_host::spawn(camera_request(node, device, generation, current_webcam_start()));
}

#[cfg(target_os = "macos")]
pub fn stop() {
    let cleanup = state(|s| {
        s.cancel_camera_request();
        s.ensure_camera_cleanup();
        s.sink_generation = None;
        s.notify_sink_stopped();
        camera_output::shutdown();
        if s.request.is_none() {
            s.phase = Phase::Idle;
        }
        ffi::video_clear(PREVIEW_STREAM);
        s.take_cleanup_actor()
    });
    if let Some(generation) = cleanup {
        core_host::spawn(complete_camera_request(generation));
    }
    // The retained request actor owns pending cancellation. Active output uses
    // only its exact Lease/Notify. This function spawns no peer-wide Stop.
    core_host::host().hub.changed(Changes::WEBCAM);
}

#[cfg(target_os = "macos")]
fn finish_camera_sink(lease: Option<camera_output::Lease>, generation: u64, reason: Option<String>) {
    let (changed, cleanup) = state(|s| {
        let changed = s.finish_camera_sink(generation, reason, || {
            if let Some(lease) = lease {
                camera_output::release(lease);
            }
            ffi::video_clear(PREVIEW_STREAM);
        });
        (changed, s.take_cleanup_actor())
    });
    if let Some(generation) = cleanup {
        core_host::spawn(complete_camera_request(generation));
    }
    if changed {
        core_host::host().hub.changed(Changes::WEBCAM);
    }
}

#[cfg(target_os = "macos")]
fn publish_camera_sink(
    lease: Option<camera_output::Lease>,
    generation: u64,
    width: u32,
    height: u32,
    pixels: &[u8],
) -> bool {
    let mut ended = false;
    let published = state(|s| {
        if s.sink_generation != Some(generation) {
            return false;
        }
        if !s.capture_view.as_ref().is_some_and(|view| view.is_current()) {
            ended = s.finish_camera_sink(generation, None, || {
                if let Some(lease) = lease {
                    camera_output::release(lease);
                }
                ffi::video_clear(PREVIEW_STREAM);
            });
            return false;
        }
        let result = if width == 0
            || height == 0
            || width > 8192
            || height > 8192
            || pixels.len() != width as usize * height as usize * 4
        {
            Err("Invalid camera preview frame.".into())
        } else if let Some(lease) = lease {
            camera_output::publish(lease, width, height, pixels)
        } else {
            Ok(())
        };
        match result {
            Ok(()) => {
                // video_frame copies and only queues Qt wakeups; it does not
                // synchronously call Rust controller code while STATE is held.
                ffi::video_frame(PREVIEW_STREAM, width, height, pixels);
                true
            }
            Err(reason) => {
                ended = s.finish_camera_sink(generation, Some(reason), || {
                    if let Some(lease) = lease {
                        camera_output::release(lease);
                    }
                    ffi::video_clear(PREVIEW_STREAM);
                });
                false
            }
        }
    });
    let cleanup = state(|s| s.take_cleanup_actor());
    if let Some(generation) = cleanup {
        core_host::spawn(complete_camera_request(generation));
    }
    if ended {
        core_host::host().hub.changed(Changes::WEBCAM);
    }
    published
}

#[cfg(target_os = "macos")]
pub fn on_event(event: &NodeEvent) {
    // Stream state is owned by Sink::config/ended/Drop and request callbacks,
    // never by device-only events that may belong to an older same-peer stream.
    if matches!(
        event,
        NodeEvent::DeviceAdded(_) | NodeEvent::DeviceRemoved(_) | NodeEvent::PeerInfoChanged { .. }
    ) {
        refresh_idle_frame();
        core_host::host().hub.changed(Changes::WEBCAM);
    }
}

pub fn preview_only() -> bool {
    state(|s| s.output_target == OutputTarget::PreviewOnly)
}

pub fn set_preview_only(on: bool) {
    if preview_window_open() {
        return;
    }
    #[cfg(target_os = "macos")]
    if state(|s| {
        s.select_output_target(if on { OutputTarget::PreviewOnly } else { OutputTarget::NativeCamera })
    }) {
        core_host::host().hub.changed(Changes::WEBCAM);
    }
    #[cfg(not(target_os = "macos"))]
    let _ = on;
}

/// Requests a webcam stream from `device` (or the selected phone).
#[cfg(not(target_os = "macos"))]
pub fn start(device: Option<DeviceId>) {
    let Some(device) = device.or_else(selected_phone) else { return };
    set_selected_phone(device);
    state(|s| s.phase = Phase::Asking { device });
    core_host::host().hub.changed(Changes::WEBCAM);

    let Some(node) = core_host::node() else { return };
    let options = current_webcam_start();
    core_host::spawn(async move {
        if let Err(e) = node.webcam_start(device, options).await {
            let reason = match e {
                Error::Denied => "Webcam is turned off for this phone.".to_owned(),
                Error::Unsupported => {
                    "This phone's app doesn't support Webcam yet. Update Waypair Link on the phone.".into()
                }
                Error::Offline | Error::NotPaired => "The phone isn't connected.".into(),
                e => describe(&e),
            };
            state(|s| {
                if matches!(s.phase, Phase::Asking { device: d } if d == device) {
                    s.phase = Phase::Ended { reason };
                }
            });
            refresh_idle_frame();
            core_host::host().hub.changed(Changes::WEBCAM);
        }
    });
}

/// Stops the active or pending webcam stream.
#[cfg(not(target_os = "macos"))]
pub fn stop() {
    let device = state(|s| {
        let device = match s.phase {
            Phase::Asking { device } | Phase::Streaming { device, .. } => Some(device),
            Phase::Idle | Phase::Ended { .. } => None,
        };
        s.phase = Phase::Idle;
        device
    });
    refresh_idle_frame();
    core_host::host().hub.changed(Changes::WEBCAM);
    if let (Some(device), Some(node)) = (device, core_host::node()) {
        core_host::spawn(async move {
            node.webcam_stop(device).await;
        });
    }
}

#[cfg(not(target_os = "macos"))]
pub fn on_event(event: &NodeEvent) {
    match event {
        NodeEvent::Webcam { device, on: true } => {
            state(|s| {
                s.preferred_phone = Some(*device);
                if !matches!(s.phase, Phase::Streaming { device: d, .. } if d == *device) {
                    s.phase = Phase::Streaming { device: *device, width: 1280, height: 720, fps: 30 };
                }
            });
            core_host::host().hub.changed(Changes::WEBCAM);
        }
        NodeEvent::Webcam { device, on: false }
        | NodeEvent::LinkChanged { device, link: nectarlink_core::LinkState::Offline { .. } } => {
            // Sink::ended/Drop closes its exact lease. A late device-only event
            // may update UI, but must never revoke a new sink from that phone.
            let changed = state(|s| {
                if matches!(s.phase, Phase::Asking { device: d } | Phase::Streaming { device: d, .. } if d == *device)
                {
                    s.phase = Phase::Idle;
                    true
                } else {
                    false
                }
            });
            if changed {
                refresh_idle_frame();
                core_host::host().hub.changed(Changes::WEBCAM);
            }
        }
        NodeEvent::DeviceAdded(_) | NodeEvent::DeviceRemoved(_) | NodeEvent::PeerInfoChanged { .. } => {
            refresh_idle_frame();
            core_host::host().hub.changed(Changes::WEBCAM);
        }
        _ => {}
    }
}

/// Returns a [`WebcamSink`] for an incoming `"webcam"` stream from `device`.
#[cfg(target_os = "macos")]
pub fn sink(_device: &DeviceId) -> Option<Arc<dyn WebcamSink>> {
    None
}
#[cfg(not(target_os = "macos"))]
pub fn sink(device: &DeviceId) -> Option<Arc<dyn WebcamSink>> {
    Some(Arc::new(Sink::new(*device)))
}
#[cfg(target_os = "macos")]
pub fn sink_owned(device: &DeviceId, view: &nectarlink_core::WebcamView) -> Option<Arc<dyn WebcamSink>> {
    state(|s| {
        if !view.is_current() || !s.capture_view.as_ref().is_some_and(|owner| owner.same_owner(view)) {
            return None;
        }
        if s.shutting_down
            || s.sink_generation.is_some()
            || (s.output_target == OutputTarget::NativeCamera && !vcam::is_registered())
        {
            return None;
        }
        if let Some(request) = &s.request
            && (request.cancelled || request.device != *device)
        {
            return None;
        }
        let stopped = Arc::new(tokio::sync::Notify::new());
        let lease = if s.output_target == OutputTarget::PreviewOnly {
            None
        } else {
            match camera_output::claim(*device, stopped.clone()) {
                Ok(lease) => Some(lease),
                Err(reason) => {
                    s.cancel_camera_request();
                    s.phase = Phase::Ended { reason };
                    return None;
                }
            }
        };
        let Some(generation) = s.next_camera_generation() else {
            if let Some(lease) = lease {
                camera_output::release(lease);
            }
            return None;
        };
        s.sink_stopped = Some(stopped.clone());
        s.sink_generation = Some(generation);
        if let Some(request) = s.request.as_mut() {
            request.sink_admitted = true;
            request.wake.notify_one();
        }
        Some(Arc::new(Sink { device: *device, video: OnceLock::new(), lease, generation, stopped })
            as Arc<dyn WebcamSink>)
    })
}

enum Item {
    Config(WebcamConfig),
    Packet { keyframe: bool, time_us: u64, data: Vec<u8> },
    Ended,
}

struct Video {
    queue: SyncSender<Item>,
    waiting: Arc<AtomicUsize>,
}

struct Sink {
    device: DeviceId,
    video: OnceLock<Video>,
    #[cfg(target_os = "macos")]
    lease: Option<camera_output::Lease>,
    #[cfg(target_os = "macos")]
    generation: u64,
    #[cfg(target_os = "macos")]
    stopped: Arc<tokio::sync::Notify>,
}

impl Sink {
    #[cfg(not(target_os = "macos"))]
    fn new(device: DeviceId) -> Self {
        Self { device, video: OnceLock::new() }
    }

    fn keyframe_please(&self) {
        #[cfg(target_os = "macos")]
        {
            let view =
                state(
                    |s| {
                        if s.sink_generation == Some(self.generation) { s.capture_view.clone() } else { None }
                    },
                );
            if let Some(view) = view {
                core_host::spawn(async move {
                    let _ = view.keyframe().await;
                });
            }
        }
        #[cfg(not(target_os = "macos"))]
        let device = self.device;
        #[cfg(not(target_os = "macos"))]
        if let Some(node) = core_host::node() {
            core_host::spawn(async move { node.webcam_keyframe(device).await });
        }
    }
}

impl Video {
    fn start(
        device: DeviceId,
        #[cfg(target_os = "macos")] lease: Option<camera_output::Lease>,
        #[cfg(target_os = "macos")] generation: u64,
    ) -> Self {
        let (queue, items) = sync_channel(BACKLOG);
        let waiting = Arc::new(AtomicUsize::new(0));
        let counter = waiting.clone();
        let started = std::thread::Builder::new().name("webcam-decoder".into()).spawn(move || {
            #[cfg(target_os = "macos")]
            decode(device, &items, &counter, lease, generation);
            #[cfg(not(target_os = "macos"))]
            decode(device, &items, &counter);
        });
        if let Err(e) = started {
            tracing::error!(error = %e, "can't start the webcam decoder");
            #[cfg(target_os = "macos")]
            finish_camera_sink(lease, generation, Some("The camera decoder could not start.".into()));
        }
        Self { queue, waiting }
    }
}

impl WebcamSink for Sink {
    fn config(&self, config: WebcamConfig) {
        #[cfg(target_os = "macos")]
        {
            let accepted = state(|s| {
                if s.sink_generation != Some(self.generation) {
                    return false;
                }
                follow_stream(config.width, config.height);
                UHD.store(config.max_height >= 2160, Ordering::Relaxed);
                s.preferred_phone = Some(self.device);
                s.phase = Phase::Streaming {
                    device: self.device,
                    width: config.width,
                    height: config.height,
                    fps: config.fps,
                };
                true
            });
            if !accepted {
                return;
            }
        }
        #[cfg(not(target_os = "macos"))]
        {
            follow_stream(config.width, config.height);
            UHD.store(config.max_height >= 2160, Ordering::Relaxed);
            state(|s| {
                s.preferred_phone = Some(self.device);
                s.phase = Phase::Streaming {
                    device: self.device,
                    width: config.width,
                    height: config.height,
                    fps: config.fps,
                };
            });
        }
        core_host::host().hub.changed(Changes::WEBCAM);
        let video = self.video.get_or_init(|| {
            #[cfg(target_os = "macos")]
            {
                Video::start(self.device, self.lease, self.generation)
            }
            #[cfg(not(target_os = "macos"))]
            {
                Video::start(self.device)
            }
        });
        let _ = video.queue.send(Item::Config(config));
    }

    fn packet(&self, keyframe: bool, time_us: u64, data: Vec<u8>) {
        #[cfg(target_os = "macos")]
        if !state(|s| s.sink_generation == Some(self.generation)) {
            return;
        }
        let Some(video) = self.video.get() else { return };
        match video.queue.try_send(Item::Packet { keyframe, time_us, data }) {
            Ok(()) => {
                video.waiting.fetch_add(1, Ordering::AcqRel);
            }
            Err(TrySendError::Disconnected(_)) => {}
            Err(TrySendError::Full(_)) => self.keyframe_please(),
        }
    }

    fn ended(&self) {
        #[cfg(target_os = "macos")]
        finish_camera_sink(self.lease, self.generation, None);
        if let Some(video) = self.video.get() {
            let _ = video.queue.send(Item::Ended);
        }
    }
    #[cfg(target_os = "macos")]
    fn stop_signal(&self) -> Option<Arc<tokio::sync::Notify>> {
        Some(self.stopped.clone())
    }
}

#[cfg(target_os = "macos")]
impl Drop for Sink {
    fn drop(&mut self) {
        finish_camera_sink(self.lease, self.generation, None);
    }
}

fn mirror_bgrx_in_place(bgrx: &mut [u8], width: u32, height: u32) {
    let row_bytes = (width as usize) * 4;
    for y in 0..height as usize {
        let row = &mut bgrx[y * row_bytes..(y + 1) * row_bytes];
        for x in 0..(width as usize) / 2 {
            let l = x * 4;
            let r = (width as usize - 1 - x) * 4;
            for c in 0..4 {
                row.swap(l + c, r + c);
            }
        }
    }
}

fn decode(
    #[cfg(target_os = "macos")] _device: DeviceId,
    #[cfg(not(target_os = "macos"))] device: DeviceId,
    items: &Receiver<Item>,
    waiting: &AtomicUsize,
    #[cfg(target_os = "macos")] lease: Option<camera_output::Lease>,
    #[cfg(target_os = "macos")] generation: u64,
) {
    // On the GPU when there is one: 4K takes about 24 ms a frame in software
    // and 11 ms on a GPU, against 33 ms per frame at 30 fps.
    let mut decoder = match Decoder::new_gpu("h264") {
        Ok(decoder) => decoder,
        Err(e) => {
            tracing::error!(error = %e, "no H.264 decoder for webcam");
            #[cfg(target_os = "macos")]
            finish_camera_sink(
                lease,
                generation,
                Some("This PC can't decode the phone's camera video.".into()),
            );
            #[cfg(not(target_os = "macos"))]
            {
                state(|s| {
                    s.phase =
                        Phase::Ended { reason: "This PC can't decode the phone's camera video.".into() };
                });
                refresh_idle_frame();
                core_host::host().hub.changed(Changes::WEBCAM);
            }
            return;
        }
    };
    tracing::info!(gpu = decoder.on_gpu(), "webcam decoder ready");
    let keyframe_please = || {
        #[cfg(target_os = "macos")]
        {
            let view =
                state(|s| if s.sink_generation == Some(generation) { s.capture_view.clone() } else { None });
            if let Some(view) = view {
                core_host::spawn(async move {
                    let _ = view.keyframe().await;
                });
            }
        }
        #[cfg(not(target_os = "macos"))]
        if let Some(node) = core_host::node() {
            core_host::spawn(async move { node.webcam_keyframe(device).await });
        }
    };
    #[cfg(not(target_os = "macos"))]
    let phone_name = phone_name_for(Some(device));
    let mut size = (0u32, 0u32);
    let mut need_keyframe = true;
    #[cfg(not(target_os = "macos"))]
    let mut local_mapping = mapping();
    #[cfg(not(target_os = "macos"))]
    let mut last_mapping_check = Instant::now();

    while let Ok(item) = items.recv() {
        match item {
            Item::Config(config) => {
                if size != (0, 0) && size != (config.width, config.height) {
                    decoder.flush();
                }
                size = (config.width, config.height);
                need_keyframe = true;
            }
            Item::Packet { keyframe, time_us, data } => {
                let behind = waiting.fetch_sub(1, Ordering::AcqRel) > 1;
                if need_keyframe && !keyframe {
                    continue;
                }
                need_keyframe = false;
                let pictures = match decoder.decode(&data, time_us) {
                    Ok(pictures) => pictures,
                    Err(e) => {
                        tracing::debug!(error = %e, "webcam packet didn't decode");
                        decoder.flush();
                        need_keyframe = true;
                        keyframe_please();
                        continue;
                    }
                };
                if let (Some(picture), false) = (pictures.last(), behind) {
                    let (w, h) = if size.0 > 0 { size } else { (picture.width, picture.height) };
                    let (src_w, src_h) = (w.min(picture.width), h.min(picture.height));
                    if src_w == 0 || src_h == 0 {
                        continue;
                    }
                    let mut bgrx = to_bgrx(picture, w, h);
                    let flip = mirror();
                    if flip {
                        mirror_bgrx_in_place(&mut bgrx, src_w, src_h);
                    }
                    #[cfg(target_os = "macos")]
                    if !publish_camera_sink(lease, generation, src_w, src_h, &bgrx) {
                        break;
                    }
                    #[cfg(not(target_os = "macos"))]
                    if local_mapping.is_none() && last_mapping_check.elapsed() >= Duration::from_secs(1) {
                        local_mapping = mapping();
                        last_mapping_check = Instant::now();
                    }
                    #[cfg(not(target_os = "macos"))]
                    if let Some(ref map) = local_mapping {
                        // Already flipped in place if `flip` is true, so pass `false` here.
                        map.write_frame(&phone_name, src_w, src_h, time_us, &bgrx, false);
                    }
                    #[cfg(not(target_os = "macos"))]
                    ffi::video_frame(PREVIEW_STREAM, src_w, src_h, &bgrx);
                }
            }
            Item::Ended => break,
        }
    }

    #[cfg(target_os = "macos")]
    finish_camera_sink(lease, generation, None);
    #[cfg(not(target_os = "macos"))]
    {
        state(|s| {
            if matches!(s.phase, Phase::Streaming { device: d, .. } if d == device) {
                s.phase = Phase::Idle;
            }
        });
        refresh_idle_frame();
        core_host::host().hub.changed(Changes::WEBCAM);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(target_os = "macos")]
    #[tokio::test]
    async fn preview_target_finishes_and_notifies_its_owner() {
        let mut s = State::default();
        assert!(s.select_output_target(OutputTarget::PreviewOnly));
        let stopped = Arc::new(tokio::sync::Notify::new());
        s.sink_stopped = Some(stopped.clone());
        s.sink_generation = Some(7);
        let mut cleared = false;
        assert!(s.finish_camera_sink(7, None, || cleared = true));
        assert!(cleared);
        tokio::time::timeout(std::time::Duration::from_millis(100), stopped.notified()).await.unwrap();
        assert!(s.sink_stopped.is_none());
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn active_target_and_successor_are_protected() {
        let mut s = State::default();
        assert_eq!(s.output_target, OutputTarget::NativeCamera);
        s.sink_generation = Some(8);
        assert!(!s.select_output_target(OutputTarget::PreviewOnly));
        assert!(!s.finish_camera_sink(7, None, || panic!("stale sink cleared successor")));
        assert_eq!(s.sink_generation, Some(8));
        assert_eq!(s.output_target, OutputTarget::NativeCamera);
    }

    #[test]
    fn maps_720_1080_and_2160_4k_dimensions() {
        assert_eq!(dimensions("720"), (1280, 720));
        assert_eq!(dimensions("1080"), (1920, 1080));
        assert_eq!(dimensions("2160"), (3840, 2160));
        assert_eq!(dimensions("4k"), (3840, 2160));
        assert_eq!(dimensions("4K"), (3840, 2160));
        assert_eq!(dimensions("unknown"), (1280, 720));
    }

    #[test]
    fn the_resolution_comes_from_the_short_side() {
        assert_eq!(resolution_of(1280, 720), 720);
        assert_eq!(resolution_of(1920, 1080), 1080);
        assert_eq!(resolution_of(1080, 1920), 1080);
        assert_eq!(resolution_of(3840, 2160), 2160);
        assert_eq!(resolution_of(2160, 3840), 2160);
        // Encoders round to their block size.
        assert_eq!(resolution_of(1920, 1088), 1080);
    }

    #[test]
    fn selects_and_validates_720_1080_and_2160_resolutions() {
        set_resolution("720");
        assert_eq!(height(), 720);
        assert_eq!(resolution(), "720");

        set_resolution("1080");
        assert_eq!(height(), 1080);
        assert_eq!(resolution(), "1080");

        set_resolution("2160");
        assert_eq!(height(), 2160);
        assert_eq!(resolution(), "2160");

        // Invalid resolution string leaves selection unchanged.
        set_resolution("9999");
        assert_eq!(height(), 2160);
        assert_eq!(resolution(), "2160");

        set_height(720);
        assert_eq!(height(), 720);
    }
}
