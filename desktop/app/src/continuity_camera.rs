// SPDX-License-Identifier: GPL-3.0-or-later
//! Continuity Camera coordinator for Windows: initiates "Take photo with phone"
//! and "Scan document with phone" from the tray menu, global hotkeys, or the
//! NectarLink UI, remembers the external foreground window (`HWND`) that was
//! active when the request started, places the captured image onto the Windows
//! clipboard on arrival, and either restores focus + synthesizes `Ctrl+V` into
//! the original target window or prompts the user if that window was closed.

use crate::i18n::tr;
use std::{
    collections::VecDeque,
    sync::{
        Mutex,
        atomic::{AtomicU64, Ordering},
        mpsc,
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use nectarlink_core::{CameraCaptureMode, DeviceId, Error, LinkState, NodeEvent};

use crate::{core_host, state::Changes};

static REQUEST_SEQ: AtomicU64 = AtomicU64::new(1);

#[derive(Debug, Clone)]
struct ActiveRequest {
    request_id: String,
    device: DeviceId,
    device_name: String,
    mode: CameraCaptureMode,
    target_hwnd: Option<isize>,
    target_title: String,
    queued_offline: bool,
}

#[derive(Debug, Clone)]
struct PendingPastePrompt {
    title: String,
    body: String,
    target_name: String,
}

/// One in-flight macOS Service capture (see `crate::win::services`).
/// Owned here so core events can be intercepted before the ordinary
/// clipboard/paste delivery below ever sees them.
#[derive(Debug)]
struct ServiceWaiter {
    device: DeviceId,
    device_name: String,
    request_id: String,
    mode: CameraCaptureMode,
    sender: mpsc::Sender<ServiceEvent>,
}

/// Events delivered to a waiting Service invocation, matched strictly by
/// `(device, request_id)` — never by device alone.
#[derive(Debug)]
pub enum ServiceEvent {
    /// The phone accepted (`camera.capture.ok`); the capture is still open.
    Accepted,
    /// The phone sent the image; the core already validated and saved it.
    Received { mime: String, data: Vec<u8> },
    /// The phone cancelled (reason is peer free text; sanitize before showing).
    Cancelled { reason: Option<String> },
    /// Cancelled from this app (window Cancel, timeout, or shutdown).
    CancelledLocally,
    /// The link dropped or the device was unpaired mid-capture.
    Disconnected,
    /// The live request itself failed before acceptance.
    Failed(String),
}

/// Why a Service capture could not be started. Checked locally, before
/// anything is sent; a Service never queues.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ServiceRefusal {
    /// Another capture (window or Service) is already in progress.
    Busy,
    NoPairedDevice,
    Offline {
        device_name: String,
    },
    PhotosOff {
        device_name: String,
    },
    Unsupported {
        device_name: String,
    },
    CoreNotReady,
}

/// Handle for one started Service capture. The receiver is fed by
/// [`on_event`] (and by the spawned send task on failure) until exactly
/// one terminal [`ServiceEvent`] arrives.
pub struct ServiceTicket {
    pub device: DeviceId,
    pub device_name: String,
    pub request_id: String,
    pub mode: CameraCaptureMode,
    pub receiver: mpsc::Receiver<ServiceEvent>,
}

/// How many finished Service request ids are remembered. A result that
/// arrives for one of these (after timeout, cancel, or delivery) is
/// dropped here and never reaches the general clipboard or a later,
/// unrelated request.
const MAX_CLOSED_SERVICE_IDS: usize = 32;

#[derive(Debug, Default)]
struct ContinuityState {
    active: Option<ActiveRequest>,
    prompt: Option<PendingPastePrompt>,
    service: Option<ServiceWaiter>,
    closed_service_ids: VecDeque<String>,
}

static STATE: Mutex<ContinuityState> = Mutex::new(ContinuityState {
    active: None,
    prompt: None,
    service: None,
    closed_service_ids: VecDeque::new(),
});

/// Snapshot of Continuity Camera state exposed to QML via `AppController`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ContinuityView {
    pub busy: bool,
    pub mode: String,
    pub status: String,
    pub paste_prompt_visible: bool,
    pub paste_prompt_title: String,
    pub paste_prompt_body: String,
    pub paste_target_name: String,
}

/// Pure decision for what to do after placing a captured image on the clipboard.
#[derive(Debug, Clone, PartialEq, Eq)]
enum DeliveryDecision {
    /// The remembered target window is still alive and visible: focus it and paste.
    AutoPaste { hwnd: isize, toast: String },
    /// A target window was remembered when the request started, but is no longer
    /// alive/visible when the capture arrived. Do not blindly paste elsewhere;
    /// ask the user in NectarLink.
    PromptMissingTarget { title: String, body: String, target_name: String },
    /// No external target window was remembered when the request started.
    ClipboardOnly { toast: String },
}

fn decide_delivery_action(
    mode: CameraCaptureMode,
    device_name: &str,
    target: Option<(isize, &str)>,
    target_still_alive: bool,
) -> DeliveryDecision {
    let (item_cap, item_lower) = match mode {
        CameraCaptureMode::Photo => ("Photo", "photo"),
        CameraCaptureMode::Scan => ("Scanned document", "scanned document"),
    };
    if let Some((hwnd, raw_title)) = target
        && hwnd != 0
    {
        let trimmed = raw_title.trim();
        let target_name =
            if trimmed.is_empty() { "your previous window".to_owned() } else { trimmed.to_owned() };
        if target_still_alive {
            return DeliveryDecision::AutoPaste {
                hwnd,
                toast: format!("{item_cap} from {device_name} pasted into {target_name}"),
            };
        }
        return DeliveryDecision::PromptMissingTarget {
            title: format!("{item_cap} ready on clipboard"),
            body: format!(
                "“{target_name}” is no longer open, so Waypair Link didn’t paste automatically. Switch to the window where you want your {item_lower} from {device_name} and click Paste into active window, or press Ctrl+V anywhere."
            ),
            target_name,
        };
    }
    DeliveryDecision::ClipboardOnly { toast: format!("{item_cap} from {device_name} copied to clipboard") }
}

fn next_request_id() -> String {
    let ms = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0);
    let seq = REQUEST_SEQ.fetch_add(1, Ordering::Relaxed);
    format!("cc-{ms}-{seq}")
}

fn notify_continuity() {
    core_host::host().hub.update(|_| Changes::CONTINUITY);
}

/// Returns a snapshot of the current Continuity Camera UI state.
pub fn view() -> ContinuityView {
    let guard = STATE.lock().unwrap_or_else(|e| e.into_inner());
    let (busy, mode, status) = match &guard.service {
        Some(service) => {
            let status_str = match service.mode {
                CameraCaptureMode::Photo => {
                    tr!("ContinuityCamera", "Take a photo on %1…", service.device_name)
                }
                CameraCaptureMode::Scan => {
                    tr!("ContinuityCamera", "Scan a document on %1…", service.device_name)
                }
            };
            (true, service.mode.as_str().to_owned(), status_str)
        }
        _ => match &guard.active {
            Some(active) if !active.queued_offline => {
                let mode_str = active.mode.as_str().to_owned();
                let status_str = match active.mode {
                    CameraCaptureMode::Photo => {
                        tr!("ContinuityCamera", "Take a photo on %1…", active.device_name)
                    }
                    CameraCaptureMode::Scan => {
                        tr!("ContinuityCamera", "Scan a document on %1…", active.device_name)
                    }
                };
                (true, mode_str, status_str)
            }
            _ => (false, String::new(), String::new()),
        },
    };
    let (paste_prompt_visible, paste_prompt_title, paste_prompt_body, paste_target_name) =
        if let Some(prompt) = &guard.prompt {
            (true, prompt.title.clone(), prompt.body.clone(), prompt.target_name.clone())
        } else {
            (false, String::new(), String::new(), String::new())
        };
    ContinuityView {
        busy,
        mode,
        status,
        paste_prompt_visible,
        paste_prompt_title,
        paste_prompt_body,
        paste_target_name,
    }
}

/// Starts a Continuity Camera capture (`"photo"` or `"scan"`) on the active
/// paired phone, remembering the current external foreground window.
pub fn start(mode_raw: &str) {
    start_for_index(0, mode_raw);
}

/// Starts a Continuity Camera capture preferring the paired device at
/// `preferred_index` if it is online, or the first online phone, or the
/// preferred offline paired phone (queueing a 2-minute expiring request).
pub fn start_for_index(preferred_index: usize, mode_raw: &str) {
    // One capture at a time across entry points: while a Service request
    // is open, the window can cancel it (see `cancel`) but not start another.
    if STATE.lock().unwrap_or_else(|e| e.into_inner()).service.is_some() {
        crate::bridge::app::show_message(tr!(
            "ContinuityCamera",
            "A photo or scan requested by another app is still in progress."
        ));
        return;
    }
    let mode = CameraCaptureMode::parse(mode_raw).unwrap_or(CameraCaptureMode::Photo);
    let target = crate::win::tray::snapshot_target_window();

    let chosen = core_host::host().hub.read(|s| {
        s.devices
            .get(preferred_index)
            .filter(|d| matches!(d.link, LinkState::Online { .. }))
            .or_else(|| s.devices.iter().find(|d| matches!(d.link, LinkState::Online { .. })))
            .or_else(|| s.devices.get(preferred_index))
            .or_else(|| s.devices.first())
            .map(|d| (d.id, d.info.name.clone(), matches!(d.link, LinkState::Online { .. })))
    });
    let Some((device_id, device_name, online)) = chosen else {
        crate::bridge::app::show_message(tr!(
            "ContinuityCamera",
            "Pair your phone first to take a photo or scan a document."
        ));
        return;
    };

    let request_id = next_request_id();
    let (target_hwnd, target_title) = match target {
        Some((hwnd, title)) => (Some(hwnd), title),
        None => (None, String::new()),
    };

    let previous = {
        let mut guard = STATE.lock().unwrap_or_else(|e| e.into_inner());
        guard.prompt = None;
        guard.active.replace(ActiveRequest {
            request_id: request_id.clone(),
            device: device_id,
            device_name: device_name.clone(),
            mode,
            target_hwnd,
            target_title,
            queued_offline: !online,
        })
    };
    notify_continuity();

    if online {
        let open_msg = match mode {
            CameraCaptureMode::Photo => tr!("ContinuityCamera", "Opening camera on %1…", device_name),
            CameraCaptureMode::Scan => {
                tr!("ContinuityCamera", "Opening document scanner on %1…", device_name)
            }
        };
        crate::bridge::app::show_message(open_msg);
    }

    let Some(node) = core_host::node() else { return };
    core_host::spawn(async move {
        if let Some(prev) = previous {
            let _ = node.cancel_camera_capture(prev.device, prev.request_id, Some("replaced".into())).await;
        }
        match node.request_camera_capture(device_id, request_id.clone(), mode).await {
            Ok(()) => {
                if !online {
                    let _ = crate::bridge::app::show_offline_queued_toast(device_id, None);
                }
            }
            Err(err) => {
                let should_report = {
                    let mut guard = STATE.lock().unwrap_or_else(|e| e.into_inner());
                    if guard.active.as_ref().is_some_and(|a| a.request_id == request_id) {
                        guard.active = None;
                        true
                    } else {
                        false
                    }
                };
                if should_report {
                    notify_continuity();
                    let msg = match err {
                        Error::Denied => tr!(
                            "ContinuityCamera",
                            "Allow Photos access for %1 in Settings first.",
                            device_name
                        ),
                        Error::Unsupported => format!("{device_name} doesn’t support Continuity Camera yet."),
                        Error::Offline => format!("{device_name} is not connected."),
                        other => {
                            tr!("ContinuityCamera", "Couldn’t open camera on %1: %2", device_name, other)
                        }
                    };
                    crate::bridge::app::show_message(msg);
                }
            }
        }
    });
}

/// Records a Service request id as closed, so a late result for it is
/// dropped instead of reaching the general clipboard or another request.
fn tombstone_locked(state: &mut ContinuityState, request_id: &str) {
    if state.closed_service_ids.iter().any(|id| id == request_id) {
        return;
    }
    state.closed_service_ids.push_back(request_id.to_owned());
    while state.closed_service_ids.len() > MAX_CLOSED_SERVICE_IDS {
        state.closed_service_ids.pop_front();
    }
}

/// Takes the Service waiter when `(device, request_id)` matches it
/// strictly, tombstoning the id. A different device or a different id
/// (e.g. a late result from an older request) never matches.
fn take_service_locked(
    state: &mut ContinuityState,
    device: &DeviceId,
    request_id: &str,
) -> Option<ServiceWaiter> {
    let matches = state.service.as_ref().is_some_and(|w| w.device == *device && w.request_id == request_id);
    if !matches {
        return None;
    }
    let waiter = state.service.take()?;
    tombstone_locked(state, &waiter.request_id);
    Some(waiter)
}

fn next_service_request_id() -> String {
    let ms = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0);
    let seq = REQUEST_SEQ.fetch_add(1, Ordering::Relaxed);
    format!("mac-svc-{ms}-{seq}")
}

/// Starts a capture for a macOS Service invocation. All readiness checks
/// are local and happen before anything is sent: one capture at a time,
/// a paired device that is online, the `photos` toggle on, and — when the
/// capability matrix is known — a phone that isn't `Unsupported` for
/// Continuity Camera. The request itself uses the core's live-only
/// `request_camera_capture_now`, so `Ok` from the send is never a queue
/// receipt; acceptance arrives as [`ServiceEvent::Accepted`].
pub fn begin_service(mode: CameraCaptureMode) -> Result<ServiceTicket, ServiceRefusal> {
    {
        let guard = STATE.lock().unwrap_or_else(|e| e.into_inner());
        if guard.service.is_some() || guard.active.is_some() {
            return Err(ServiceRefusal::Busy);
        }
    }
    let chosen: Option<Result<(DeviceId, String), String>> = core_host::host().hub.read(|s| {
        let first_name = s.devices.first().map(|d| d.info.name.clone());
        match s.devices.iter().find(|d| matches!(d.link, LinkState::Online { .. })) {
            Some(d) => Some(Ok((d.id, d.info.name.clone()))),
            None => first_name.map(Err),
        }
    });
    let (device_id, device_name) = match chosen {
        None => return Err(ServiceRefusal::NoPairedDevice),
        Some(Err(name)) => return Err(ServiceRefusal::Offline { device_name: name }),
        Some(Ok(found)) => found,
    };
    let Some(node) = core_host::node() else { return Err(ServiceRefusal::CoreNotReady) };
    match node.device_toggles(device_id) {
        Ok(toggles) => {
            if toggles.iter().any(|(name, on)| *name == "photos" && !*on) {
                return Err(ServiceRefusal::PhotosOff { device_name });
            }
        }
        Err(Error::NotPaired) => return Err(ServiceRefusal::NoPairedDevice),
        Err(_) => {} // `request_camera_capture_now` re-checks the same gates.
    }
    if let Ok(matrix) = node.capabilities(device_id)
        && matches!(
            matrix.state("camera.continuity"),
            Some(nectarlink_core::FeatureState::Unsupported { .. })
        )
    {
        return Err(ServiceRefusal::Unsupported { device_name });
    }

    let request_id = next_service_request_id();
    let (sender, receiver) = mpsc::channel();
    {
        let mut guard = STATE.lock().unwrap_or_else(|e| e.into_inner());
        // Rechecked under the lock that installs the waiter, so two
        // invocations racing the preflight can't both start a capture.
        if guard.service.is_some() || guard.active.is_some() {
            return Err(ServiceRefusal::Busy);
        }
        guard.service = Some(ServiceWaiter {
            device: device_id,
            device_name: device_name.clone(),
            request_id: request_id.clone(),
            mode,
            sender,
        });
    }
    notify_continuity();

    let ticket = ServiceTicket {
        device: device_id,
        device_name: device_name.clone(),
        request_id: request_id.clone(),
        mode,
        receiver,
    };
    core_host::spawn(async move {
        if let Err(err) = node.request_camera_capture_now(device_id, request_id.clone(), mode).await {
            let message = match err {
                Error::Denied => {
                    tr!("ContinuityCamera", "Allow Photos access for %1 in Settings first.", device_name)
                }
                Error::Offline => format!("{device_name} is not connected."),
                Error::NotPaired => tr!("ContinuityCamera", "Pair your phone first."),
                other => tr!("ContinuityCamera", "Couldn’t open camera on %1: %2", device_name, other),
            };
            fail_service(&request_id, message);
        }
    });
    Ok(ticket)
}

/// Reports a failed live send to the waiter for `request_id`, if it is
/// still the current Service request, and closes it.
fn fail_service(request_id: &str, message: String) {
    let waiter = {
        let mut guard = STATE.lock().unwrap_or_else(|e| e.into_inner());
        if guard.service.as_ref().is_some_and(|w| w.request_id == request_id) {
            let waiter = guard.service.take();
            if let Some(w) = &waiter {
                tombstone_locked(&mut guard, &w.request_id);
            }
            waiter
        } else {
            None
        }
    };
    if let Some(waiter) = waiter {
        let _ = waiter.sender.send(ServiceEvent::Failed(message));
        notify_continuity();
    }
}

/// Cancels the in-flight Service capture, if any: clears the waiter,
/// wakes it as locally cancelled, tombstones the id so a late result is
/// dropped, and tells the phone. Returns whether one was in flight.
pub fn cancel_service(phone_reason: &str) -> bool {
    let waiter = {
        let mut guard = STATE.lock().unwrap_or_else(|e| e.into_inner());
        let waiter = guard.service.take();
        if let Some(w) = &waiter {
            tombstone_locked(&mut guard, &w.request_id);
        }
        waiter
    };
    let Some(waiter) = waiter else { return false };
    let _ = waiter.sender.send(ServiceEvent::CancelledLocally);
    notify_continuity();
    if let Some(node) = core_host::node() {
        let reason = phone_reason.to_owned();
        core_host::spawn(async move {
            let _ = node.cancel_camera_capture(waiter.device, waiter.request_id, Some(reason)).await;
        });
    }
    true
}

/// Cancels any in-flight Service capture at core shutdown.
pub fn shutdown_service() {
    cancel_service("cancelled");
}

/// Cancels an in-flight Continuity Camera request from the PC side.
pub fn cancel() {
    // While a Service request is open, Cancel belongs to it.
    if cancel_service("cancelled") {
        return;
    }
    let active = {
        let mut guard = STATE.lock().unwrap_or_else(|e| e.into_inner());
        guard.active.take()
    };
    let Some(active) = active else { return };
    notify_continuity();
    if let Some(node) = core_host::node() {
        core_host::spawn(async move {
            let _ =
                node.cancel_camera_capture(active.device, active.request_id, Some("cancelled".into())).await;
        });
    }
}

/// Confirms pasting the clipboard image when the original target window had closed.
pub fn confirm_paste() {
    {
        let mut guard = STATE.lock().unwrap_or_else(|e| e.into_inner());
        guard.prompt = None;
    }
    notify_continuity();

    if let Some((hwnd, title)) = crate::win::tray::snapshot_target_window() {
        spawn_focus_and_paste(hwnd);
        let label = if title.is_empty() { "active window".to_owned() } else { title };
        crate::bridge::app::show_message(tr!("ContinuityCamera", "Pasted into %1", label));
    } else {
        crate::bridge::app::show_message(tr!(
            "ContinuityCamera",
            "Image is on your clipboard. Press Ctrl+V in your target app."
        ));
    }
}

/// Dismisses the missing-window paste confirmation prompt, keeping the image on the clipboard.
pub fn dismiss_paste() {
    {
        let mut guard = STATE.lock().unwrap_or_else(|e| e.into_inner());
        guard.prompt = None;
    }
    notify_continuity();
}

fn spawn_focus_and_paste(hwnd: isize) {
    let _ = std::thread::Builder::new().name("continuity-paste".into()).spawn(move || {
        if crate::win::tray::focus_window(hwnd) {
            std::thread::sleep(Duration::from_millis(65));
            if crate::win::tray::is_window_alive_and_visible(hwnd) {
                crate::win::input::press_key("paste", &[]);
                crate::win::tray::send_paste_fallback_if_not_foreground(hwnd);
            }
        }
    });
}

/// Routes one event to the Service waiter when it belongs to it.
/// Returns `true` when the event was consumed (or dropped as a late
/// Service response) and ordinary delivery must not see it. Matching is
/// strict: the same device *and* the waiter's current request id.
fn intercept_service_event(event: &NodeEvent) -> bool {
    match event {
        NodeEvent::CameraCaptureAccepted { device, request_id } => {
            let sender = {
                let guard = STATE.lock().unwrap_or_else(|e| e.into_inner());
                guard
                    .service
                    .as_ref()
                    .filter(|w| w.device == *device && w.request_id == *request_id)
                    .map(|w| w.sender.clone())
            };
            match sender {
                Some(sender) => {
                    let _ = sender.send(ServiceEvent::Accepted);
                    true
                }
                None => false,
            }
        }
        NodeEvent::CameraCaptureReceived { device, request_id, mime, data, .. } => {
            let waiter = {
                let mut guard = STATE.lock().unwrap_or_else(|e| e.into_inner());
                if guard.closed_service_ids.iter().any(|id| id == request_id) {
                    // Late Service response: the core already saved it,
                    // but it must not reach the general clipboard.
                    return true;
                }
                take_service_locked(&mut guard, device, request_id)
            };
            match waiter {
                Some(waiter) => {
                    let _ =
                        waiter.sender.send(ServiceEvent::Received { mime: mime.clone(), data: data.clone() });
                    notify_continuity();
                    true
                }
                None => false,
            }
        }
        NodeEvent::CameraCaptureCancelled { device, request_id, reason } => {
            let waiter = {
                let mut guard = STATE.lock().unwrap_or_else(|e| e.into_inner());
                if guard.closed_service_ids.iter().any(|id| id == request_id) {
                    return true;
                }
                take_service_locked(&mut guard, device, request_id)
            };
            match waiter {
                Some(waiter) => {
                    let _ = waiter.sender.send(ServiceEvent::Cancelled { reason: reason.clone() });
                    notify_continuity();
                    true
                }
                None => false,
            }
        }
        NodeEvent::LinkChanged { device, link } if !matches!(link, LinkState::Online { .. }) => {
            let waiter = {
                let mut guard = STATE.lock().unwrap_or_else(|e| e.into_inner());
                let request_id =
                    guard.service.as_ref().filter(|w| w.device == *device).map(|w| w.request_id.clone());
                match request_id {
                    Some(id) => take_service_locked(&mut guard, device, &id),
                    None => None,
                }
            };
            match waiter {
                Some(waiter) => {
                    let _ = waiter.sender.send(ServiceEvent::Disconnected);
                    notify_continuity();
                    true
                }
                None => false,
            }
        }
        NodeEvent::DeviceRemoved(device) => {
            let waiter = {
                let mut guard = STATE.lock().unwrap_or_else(|e| e.into_inner());
                let request_id =
                    guard.service.as_ref().filter(|w| w.device == *device).map(|w| w.request_id.clone());
                match request_id {
                    Some(id) => take_service_locked(&mut guard, device, &id),
                    None => None,
                }
            };
            match waiter {
                Some(waiter) => {
                    let _ = waiter.sender.send(ServiceEvent::Disconnected);
                    notify_continuity();
                    true
                }
                None => false,
            }
        }
        _ => false,
    }
}

/// Handles Continuity Camera events from `nectarlink_core`.
pub fn on_event(event: &NodeEvent) {
    // Service-bound responses are intercepted before ordinary delivery,
    // so they never reach the general clipboard or paste path.
    if intercept_service_event(event) {
        return;
    }
    match event {
        NodeEvent::CameraCaptureReceived { device, request_id, mode, mime, data, .. } => {
            let matched = {
                let mut guard = STATE.lock().unwrap_or_else(|e| e.into_inner());
                if guard.active.as_ref().is_some_and(|a| a.request_id == *request_id || a.device == *device) {
                    guard.active.take()
                } else {
                    None
                }
            };

            let device_name = matched.as_ref().map(|a| a.device_name.clone()).unwrap_or_else(|| {
                core_host::host().hub.read(|s| s.name_of(device).unwrap_or_else(|| device.short()))
            });

            if let Err(e) = crate::win::clipboard::write_image(mime, data) {
                tracing::warn!(error = %e, "can't copy continuity camera image to clipboard");
                notify_continuity();
                crate::bridge::app::show_message(tr!(
                    "ContinuityCamera",
                    "Couldn’t copy image to clipboard: %1",
                    e
                ));
                return;
            }

            let target_ref =
                matched.as_ref().and_then(|a| a.target_hwnd.map(|hwnd| (hwnd, a.target_title.as_str())));
            let alive =
                target_ref.is_some_and(|(hwnd, _)| crate::win::tray::is_window_alive_and_visible(hwnd));

            match decide_delivery_action(*mode, &device_name, target_ref, alive) {
                DeliveryDecision::AutoPaste { hwnd, toast } => {
                    {
                        let mut guard = STATE.lock().unwrap_or_else(|e| e.into_inner());
                        guard.prompt = None;
                    }
                    notify_continuity();
                    crate::bridge::app::show_message(toast);
                    spawn_focus_and_paste(hwnd);
                }
                DeliveryDecision::PromptMissingTarget { title, body, target_name } => {
                    {
                        let mut guard = STATE.lock().unwrap_or_else(|e| e.into_inner());
                        guard.prompt = Some(PendingPastePrompt { title, body, target_name });
                    }
                    notify_continuity();
                    crate::bridge::app::request_activation();
                }
                DeliveryDecision::ClipboardOnly { toast } => {
                    {
                        let mut guard = STATE.lock().unwrap_or_else(|e| e.into_inner());
                        guard.prompt = None;
                    }
                    notify_continuity();
                    crate::bridge::app::show_message(toast);
                }
            }
        }
        NodeEvent::CameraCaptureCancelled { device, request_id, reason } => {
            let removed = {
                let mut guard = STATE.lock().unwrap_or_else(|e| e.into_inner());
                if guard.active.as_ref().is_some_and(|a| {
                    a.request_id == *request_id || (request_id.is_empty() && a.device == *device)
                }) {
                    guard.active.take()
                } else {
                    None
                }
            };
            if let Some(active) = removed {
                notify_continuity();
                let msg = match reason.as_deref() {
                    Some("busy") => tr!("ContinuityCamera", "Camera is busy on %1.", active.device_name),
                    Some("permission_denied") => {
                        tr!("ContinuityCamera", "Camera permission was denied on %1.", active.device_name)
                    }
                    _ => tr!("ContinuityCamera", "Camera capture cancelled on %1.", active.device_name),
                };
                crate::bridge::app::show_message(msg);
            }
        }
        NodeEvent::LinkChanged { device, link } if !matches!(link, LinkState::Online { .. }) => {
            let cleared = {
                let mut guard = STATE.lock().unwrap_or_else(|e| e.into_inner());
                if guard.active.as_ref().is_some_and(|a| a.device == *device && !a.queued_offline) {
                    guard.active = None;
                    true
                } else {
                    false
                }
            };
            if cleared {
                notify_continuity();
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn delivery_decision_auto_pastes_when_target_window_is_alive() {
        let decision = decide_delivery_action(
            CameraCaptureMode::Photo,
            "Pixel 9",
            Some((0x1234, "Document.docx - Word")),
            true,
        );
        assert_eq!(
            decision,
            DeliveryDecision::AutoPaste {
                hwnd: 0x1234,
                toast: "Photo from Pixel 9 pasted into Document.docx - Word".into(),
            }
        );
    }

    #[test]
    fn delivery_decision_prompts_when_target_window_disappeared() {
        let decision = decide_delivery_action(
            CameraCaptureMode::Scan,
            "Pixel 9",
            Some((0x1234, "Notes - Notion")),
            false,
        );
        match decision {
            DeliveryDecision::PromptMissingTarget { title, body, target_name } => {
                assert_eq!(title, "Scanned document ready on clipboard");
                assert_eq!(target_name, "Notes - Notion");
                assert!(body.contains("Notes - Notion"));
                assert!(body.contains("no longer open"));
            }
            other => panic!("expected PromptMissingTarget, got {other:?}"),
        }
    }

    #[test]
    fn service_waiter_matches_strictly_and_late_ids_are_tombstoned() {
        let phone = DeviceId([7; 32]);
        let other = DeviceId([8; 32]);
        let (sender, _rx) = mpsc::channel();
        let mut state = ContinuityState {
            active: None,
            prompt: None,
            service: Some(ServiceWaiter {
                device: phone,
                device_name: "Pixel".into(),
                request_id: "mac-svc-1-1".into(),
                mode: CameraCaptureMode::Photo,
                sender,
            }),
            closed_service_ids: VecDeque::new(),
        };
        // A late result from another request, or from another device,
        // never resolves this waiter.
        assert!(take_service_locked(&mut state, &phone, "mac-svc-old-9").is_none());
        assert!(take_service_locked(&mut state, &other, "mac-svc-1-1").is_none());
        assert!(state.service.is_some());
        // The exact (device, request_id) pair does, and closes the id.
        assert!(take_service_locked(&mut state, &phone, "mac-svc-1-1").is_some());
        assert!(state.service.is_none());
        assert!(state.closed_service_ids.iter().any(|id| id == "mac-svc-1-1"));
        // Closing is idempotent and bounded.
        tombstone_locked(&mut state, "mac-svc-1-1");
        for n in 0..(MAX_CLOSED_SERVICE_IDS + 5) {
            tombstone_locked(&mut state, &format!("mac-svc-x-{n}"));
        }
        assert_eq!(state.closed_service_ids.len(), MAX_CLOSED_SERVICE_IDS);
    }

    #[test]
    fn delivery_decision_copies_only_when_no_external_target_was_recorded() {
        let decision = decide_delivery_action(CameraCaptureMode::Photo, "Pixel 9", None, false);
        assert_eq!(
            decision,
            DeliveryDecision::ClipboardOnly { toast: "Photo from Pixel 9 copied to clipboard".into() }
        );
    }
}
