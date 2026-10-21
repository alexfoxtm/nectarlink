// SPDX-License-Identifier: GPL-3.0-or-later
//! macOS Bluetooth HFP integration.
//!
//! macOS has no public Audio Gateway role, so the name-only seam
//! (`probe_status`, `connect_audio`) stays unavailable: a phone's display
//! name can't authorize anything. What macOS does have is the public
//! HF-unit role (`bt_hfp_provider.m`), where the Mac serves a phone that is
//! already paired with it. That is driven here only for an exact binding: a
//! trusted Waypair phone, the Bluetooth device the user picked for it, the
//! phone's one live call, and this Mac's microphone and speaker as chosen,
//! and only after the user starts it during that call. Each native lease
//! check reads all of that again; anything else stopping the attempt hands
//! the audio back. Native snapshots never claim the route is available,
//! connected or heard both ways: SLC and SCO are observations, not proof.

use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};

use dispatch2::DispatchQueue;
use nectarlink_core::{CallState, DeviceId, LinkState};
use objc2::MainThreadMarker;
use serde::{Deserialize, Serialize};

use crate::bridge::native::ffi;
use crate::{
    calls::{self, MacCallAudioBinding, PcAudioView},
    core_host,
    i18n::tr,
    state::{Changes, CoreStatus},
};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct HfpStatus {
    pub available: bool,
    pub connected: bool,
    pub device_name: String,
    pub detail: String,
}

/// The legacy name-only seam has no qualified, identity-bound provider.
/// Keep the status unavailable until native integration and runtime acceptance.
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

/// A display name cannot authorize a native HF session. The error carries
/// the same unavailable reason shown by the Calls page.
pub fn connect_audio(phone_name: Option<&str>) -> Result<HfpStatus, String> {
    Err(probe_status(phone_name).detail)
}

/// The phones' calls are all over: any owned attempt hands the audio back.
pub fn disconnect_audio() {
    hand_back();
}

pub fn open_bluetooth_settings() {
    if let Err(e) = std::process::Command::new("open")
        .arg("x-apple.systempreferences:com.apple.BluetoothSettings")
        .spawn()
    {
        tracing::warn!(error = %e, "can't open Bluetooth settings");
    }
}

// ---- Exact-binding HF-unit attempts ----

/// How long a listed Bluetooth device can still be chosen.
const LIST_FRESH: Duration = Duration::from_secs(300);
/// How long quitting waits for the owned handback to be observed.
const QUIT_HANDBACK_WAIT: Duration = Duration::from_secs(3);

/// What the provider last reported (`WPHfpEvent`). Its `available` and
/// `connected` fields are always false and aren't read.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct Snapshot {
    phase: String,
    reason: String,
    slc_observed: bool,
    sco_observed: bool,
    duplex_qualified: bool,
    handset_audible_verified: bool,
    retired: bool,
}

#[derive(Debug, Clone)]
struct Session {
    /// Ours, handed to the native callbacks, never reused.
    token: u64,
    /// The provider's, once `WPHfpStart` returned it; 0 before.
    handle: u64,
    device: DeviceId,
    call_id: String,
    number: String,
    incoming: bool,
    /// `calls::call_generation` when the user started it.
    generation: u64,
    binding: MacCallAudioBinding,
    binding_json: String,
    /// The lease is refused from here on; set before any native handback.
    stopped: bool,
    snapshot: Snapshot,
}

/// One attempt at a time, kept until the provider frees its handle,
/// including while the handback is unconfirmed.
static SESSION: Mutex<Option<Session>> = Mutex::new(None);
static NEXT_TOKEN: AtomicU64 = AtomicU64::new(1);
static SHUTTING_DOWN: AtomicBool = AtomicBool::new(false);
/// How the last attempt ended, for its phone and call only.
static LAST: Mutex<Option<(DeviceId, String, Snapshot)>> = Mutex::new(None);
static PREREQUISITES: Mutex<Option<Prerequisites>> = Mutex::new(None);
static LISTED: Mutex<Option<Listed>> = Mutex::new(None);

fn session<T>(f: impl FnOnce(&mut Option<Session>) -> T) -> T {
    f(&mut SESSION.lock().unwrap_or_else(|e| e.into_inner()))
}

fn changed() {
    if core_host::try_data_dir().is_some() {
        core_host::host().hub.changed(Changes::CALLS);
    }
}

fn on_main(work: impl FnOnce() + Send + 'static) {
    if MainThreadMarker::new().is_some() {
        work();
    } else {
        DispatchQueue::main().exec_async(work);
    }
}

/// Usage descriptions and permission status, as last read (reads request nothing).
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct Prerequisites {
    bluetooth_usage_description: bool,
    microphone_usage_description: bool,
    bluetooth: String,
    microphone: String,
}

/// Reads permission status again (when Calls opens or reloads, and before an action).
pub fn refresh_prerequisites() {
    let read = serde_json::from_str(&ffi::mac_hfp_prerequisites()).ok();
    *PREREQUISITES.lock().unwrap_or_else(|e| e.into_inner()) = read;
}

fn prerequisites() -> Option<Prerequisites> {
    PREREQUISITES.lock().unwrap_or_else(|e| e.into_inner()).clone()
}

/// Paired hands-free gateways the user listed for a phone.
#[derive(Debug, Clone)]
struct Listed {
    device: DeviceId,
    at: Instant,
    gateways: Vec<Gateway>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Gateway {
    address: String,
    name: String,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
struct Endpoints {
    input: Option<Endpoint>,
    output: Option<Endpoint>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
struct Endpoint {
    uid: String,
    local: bool,
}

/// Where setting up stands, before any attempt.
fn setup_state(p: Option<&Prerequisites>, bound: bool) -> &'static str {
    let Some(p) = p else { return "unknown" };
    let refused = |s: &str| matches!(s, "denied" | "restricted");
    if !p.bluetooth_usage_description || !p.microphone_usage_description {
        "unavailable"
    } else if refused(&p.bluetooth) {
        "bluetooth-denied"
    } else if refused(&p.microphone) {
        "microphone-denied"
    } else if p.bluetooth != "allowed" {
        "bluetooth-undecided"
    } else if !bound {
        "choose-phone"
    } else if p.microphone != "allowed" {
        "microphone-undecided"
    } else {
        "ready"
    }
}

fn setup_detail(setup: &str, name: &str) -> String {
    match setup {
        "unavailable" => tr!("BtHfp", "This version of Waypair Link can't use the Mac for call audio yet."),
        "bluetooth-denied" => tr!(
            "BtHfp",
            "Bluetooth access for Waypair Link is off in System Settings > Privacy & Security > Bluetooth."
        ),
        "microphone-denied" => tr!(
            "BtHfp",
            "Microphone access for Waypair Link is off in System Settings > Privacy & Security > Microphone."
        ),
        "choose-phone" => tr!("BtHfp", "Choose which of this Mac's paired Bluetooth devices is this phone."),
        "microphone-undecided" => tr!("BtHfp", "Allow microphone access to use this Mac for calls."),
        "bluetooth-undecided" => tr!("BtHfp", "Allow Bluetooth access to use this Mac for calls."),
        "ready" => tr!(
            "BtHfp",
            "Can try during a call: %1 over Bluetooth, with this Mac's current microphone and speaker.",
            name
        ),
        _ => String::new(),
    }
}

fn phase_detail(snapshot: &Snapshot, name: &str) -> String {
    match (snapshot.phase.as_str(), snapshot.reason.as_str()) {
        ("rejected", "exact-already-paired-phone-required") => {
            tr!("BtHfp", "The chosen Bluetooth device isn't paired with this Mac anymore.")
        }
        ("rejected", "phone-audio-gateway-service-not-in-cached-sdp") => {
            tr!("BtHfp", "The chosen Bluetooth device doesn't offer call audio.")
        }
        ("rejected", "selected-current-mic-speaker-not-observed") => tr!(
            "BtHfp",
            "This Mac's microphone or speaker changed since setup. Choose the phone's Bluetooth device again to use the current ones."
        ),
        ("rejected", "new-exclusive-hf-owner-required") => {
            tr!("BtHfp", "The phone's Bluetooth call audio is already in use on this Mac.")
        }
        ("rejected", _) => tr!("BtHfp", "Mac call audio didn't start; the call's audio stays on your phone."),
        ("created" | "starting" | "connecting-service", _) => {
            tr!("BtHfp", "Connecting to %1 over Bluetooth…", name)
        }
        ("checking-exact-call", _) => tr!("BtHfp", "Checking that Bluetooth sees this same call…"),
        ("connecting-audio", _) => tr!("BtHfp", "Asking the phone to send this call's audio to the Mac…"),
        ("sco-observed-unqualified", _) => tr!(
            "BtHfp",
            "The phone opened a Bluetooth audio link to this Mac. Whether both sides hear each other isn't verified; you can return the audio to the phone at any time."
        ),
        ("returning-to-handset", _) => tr!("BtHfp", "Returning the call's audio to the phone…"),
        ("handback-unconfirmed", _) => tr!(
            "BtHfp",
            "Returning the call's audio to the phone isn't confirmed yet. If you can't hear the call, use the audio button on the phone."
        ),
        ("service-and-sco-disconnected", _) => tr!("BtHfp", "This Mac let go of the call's Bluetooth audio."),
        _ => String::new(),
    }
}

/// What's true right now, read again for every lease check.
#[derive(Debug, Clone)]
struct Live {
    node_ready: bool,
    shutting_down: bool,
    /// Paired (its Device ID is its authenticated key) and online.
    peer_online: bool,
    binding: Option<MacCallAudioBinding>,
    generation: u64,
    /// The only call on any phone, answered, with nothing ringing or held.
    call: Option<(DeviceId, CallState)>,
}

fn live(device: DeviceId) -> Live {
    let (node_ready, peer_online) = if core_host::try_data_dir().is_some() {
        core_host::host().hub.read(|s| {
            (
                matches!(s.status, Some(CoreStatus::Ready { .. })),
                s.devices.iter().any(|d| d.id == device && matches!(d.link, LinkState::Online { .. })),
            )
        })
    } else {
        (false, false)
    };
    Live {
        node_ready: node_ready && core_host::node().is_some(),
        shutting_down: SHUTTING_DOWN.load(Ordering::SeqCst),
        peer_online,
        binding: calls::pc_audio_binding(device),
        generation: calls::call_generation(),
        call: calls::sole_active_call(),
    }
}

/// Whether an attempt may go on: the exact binding it started with still
/// names the current phone, key, Bluetooth device, endpoints and the same
/// one live call, and nothing has stopped it.
fn lease_holds(session: &Session, binding_json: &[u8], live: &Live) -> bool {
    !session.stopped
        && !live.shutting_down
        && live.node_ready
        && live.peer_online
        && binding_json == session.binding_json.as_bytes()
        && live.binding.as_ref() == Some(&session.binding)
        && session.binding.phone_id == session.device.to_string()
        && session.binding.trusted_key_sha256 == calls::trusted_key_sha256(session.device)
        && live.generation == session.generation
        && live.call.as_ref().is_some_and(|(device, call)| {
            *device == session.device
                && call.id == session.call_id
                && call.state == "active"
                && call.number.as_deref() == Some(session.number.as_str())
                && call.incoming == session.incoming
                && !call.controls.is_some_and(|c| c.held)
        })
}

/// The provider's input (FFI-CONTRACT.md); every field is fixed for the attempt.
fn binding_json(binding: &MacCallAudioBinding, call: &CallState, number: &str, generation: &str) -> String {
    serde_json::json!({
        "phoneId": binding.phone_id,
        "trustedKeySHA256": binding.trusted_key_sha256,
        "bluetoothAddress": binding.bluetooth_address,
        "callId": call.id,
        "generation": generation,
        "remoteNumber": number,
        "incoming": call.incoming,
        "inputUID": binding.input_uid,
        "outputUID": binding.output_uid,
    })
    .to_string()
}

fn lease_callback(token: u64, binding_json: &[u8]) -> bool {
    let Some(current) = session(|s| s.as_ref().filter(|s| s.token == token).cloned()) else {
        return false;
    };
    lease_holds(&current, binding_json, &live(current.device))
}

fn event_callback(token: u64, snapshot_json: &[u8]) {
    let snapshot: Snapshot = serde_json::from_slice(snapshot_json)
        .unwrap_or_else(|_| Snapshot { phase: "unreadable".into(), ..Snapshot::default() });
    let retired = snapshot.retired;
    let ours = session(|s| match s {
        Some(s) if s.token == token => {
            s.snapshot = snapshot;
            true
        }
        _ => false,
    });
    if ours {
        if retired {
            // Not from inside the callback: the provider refuses that.
            DispatchQueue::main().exec_async(move || release(token));
        }
        changed();
    }
}

/// Frees a retired attempt; an unconfirmed one stays owned.
fn release(token: u64) {
    let Some(handle) = session(|s| {
        s.as_ref().filter(|s| s.token == token && s.handle != 0 && s.snapshot.retired).map(|s| s.handle)
    }) else {
        return;
    };
    if !ffi::mac_hfp_release(handle) {
        return;
    }
    if let Some(ended) =
        session(|s| if s.as_ref().is_some_and(|s| s.token == token) { s.take() } else { None })
    {
        *LAST.lock().unwrap_or_else(|e| e.into_inner()) = Some((ended.device, ended.call_id, ended.snapshot));
    }
    changed();
}

/// Latches the current attempt stopped (its lease fails at once) and asks
/// the provider to return the audio to the phone. Any thread.
pub fn hand_back() {
    let Some(token) = session(|s| {
        s.as_mut().map(|s| {
            s.stopped = true;
            s.token
        })
    }) else {
        return;
    };
    on_main(move || hand_back_on_main(token));
}

fn hand_back_on_main(token: u64) {
    if let Some(handle) = session(|s| s.as_ref().filter(|s| s.token == token).map(|s| s.handle))
        && handle != 0
    {
        ffi::mac_hfp_handback(handle);
    }
    changed();
}

/// Before an explicit quit: hands back, then runs `quit` once the attempt
/// is freed or `QUIT_HANDBACK_WAIT` has passed, while the app's event loop
/// still delivers the provider's callbacks. At once without an attempt.
pub fn hand_back_then(quit: impl FnOnce() + Send + 'static) {
    if session(|s| s.is_none()) {
        quit();
        return;
    }
    SHUTTING_DOWN.store(true, Ordering::SeqCst);
    hand_back();
    let quit = Arc::new(Mutex::new(Some(quit)));
    let waiter = Arc::clone(&quit);
    let spawned = std::thread::Builder::new().name("pc-audio-handback".into()).spawn(move || {
        let deadline = Instant::now() + QUIT_HANDBACK_WAIT;
        while Instant::now() < deadline && session(|s| s.is_some()) {
            std::thread::sleep(Duration::from_millis(100));
        }
        if session(|s| s.is_some()) {
            tracing::warn!("Mac call audio handback wasn't confirmed before quitting");
        }
        run_once(&waiter);
    });
    if let Err(e) = spawned {
        tracing::warn!(error = %e, "can't wait for the Mac call audio handback");
        run_once(&quit);
    }
}

fn run_once<F: FnOnce()>(work: &Mutex<Option<F>>) {
    if let Some(work) = work.lock().unwrap_or_else(|e| e.into_inner()).take() {
        work();
    }
}

/// After the event loop: hands back whatever is still owned and runs the
/// main run loop briefly to see it happen. Main thread.
pub fn shutdown() {
    SHUTTING_DOWN.store(true, Ordering::SeqCst);
    let Some(token) = session(|s| {
        s.as_mut().map(|s| {
            s.stopped = true;
            s.token
        })
    }) else {
        return;
    };
    if MainThreadMarker::new().is_none() {
        return;
    }
    hand_back_on_main(token);
    let deadline = Instant::now() + QUIT_HANDBACK_WAIT;
    while Instant::now() < deadline {
        ffi::mac_hfp_pump_main_run_loop(0.1);
        release(token);
        if session(|s| s.is_none()) {
            return;
        }
    }
    tracing::warn!("Mac call audio handback wasn't confirmed before quitting");
}

fn trusted(device: DeviceId) -> bool {
    core_host::try_data_dir().is_some()
        && core_host::host().hub.read(|s| s.devices.iter().any(|d| d.id == device))
}

fn not_main() -> String {
    tr!("BtHfp", "Mac call audio can only be set up from the Waypair Link window.")
}

/// Lists this Mac's already-paired phones that offer call audio, for the
/// user to say which one `device` is. User action only.
pub fn find_phones(device: DeviceId) -> Result<(), String> {
    if MainThreadMarker::new().is_none() {
        return Err(not_main());
    }
    if !trusted(device) {
        return Err(tr!("BtHfp", "Pair the phone with Waypair Link first."));
    }
    refresh_prerequisites();
    let setup = setup_state(prerequisites().as_ref(), false);
    if setup != "choose-phone" {
        return Err(setup_detail(setup, ""));
    }
    let mut ok = false;
    let json = ffi::mac_hfp_paired_gateways(&mut ok);
    if !ok {
        return Err(tr!("BtHfp", "Couldn't list this Mac's Bluetooth devices."));
    }
    let gateways: Vec<Gateway> = serde_json::from_str(&json).unwrap_or_default();
    *LISTED.lock().unwrap_or_else(|e| e.into_inner()) = Some(Listed { device, at: Instant::now(), gateways });
    changed();
    Ok(())
}

/// Records the user's pick from the last list as `device`'s Bluetooth
/// device, with this Mac's current microphone and speaker. User action only.
pub fn bind(device: DeviceId, address: &str) -> Result<(), String> {
    if MainThreadMarker::new().is_none() {
        return Err(not_main());
    }
    if !trusted(device) {
        return Err(tr!("BtHfp", "Pair the phone with Waypair Link first."));
    }
    let picked = LISTED.lock().unwrap_or_else(|e| e.into_inner()).as_ref().and_then(|listed| {
        (listed.device == device && listed.at.elapsed() < LIST_FRESH)
            .then(|| listed.gateways.iter().find(|g| g.address == address).cloned())
            .flatten()
    });
    let Some(gateway) = picked else {
        return Err(tr!("BtHfp", "Choose the phone from the list again."));
    };
    let mut ok = false;
    let json = ffi::mac_hfp_default_endpoints(&mut ok);
    let Some(endpoints) = serde_json::from_str::<Endpoints>(&json).ok().filter(|_| ok) else {
        return Err(tr!("BtHfp", "Couldn't read this Mac's microphone and speaker."));
    };
    let local = |e: &Endpoint| e.local && !e.uid.is_empty();
    let (Some(input), Some(output)) = (endpoints.input.filter(local), endpoints.output.filter(local)) else {
        return Err(tr!(
            "BtHfp",
            "Pick this Mac's built-in or wired microphone and speaker in System Settings > Sound, then choose again."
        ));
    };
    let binding = MacCallAudioBinding {
        phone_id: device.to_string(),
        trusted_key_sha256: calls::trusted_key_sha256(device),
        bluetooth_address: gateway.address,
        bluetooth_name: gateway.name,
        input_uid: input.uid,
        output_uid: output.uid,
    };
    calls::put_pc_audio_binding(binding)?;
    *LISTED.lock().unwrap_or_else(|e| e.into_inner()) = None;
    changed();
    Ok(())
}

/// Forgets `device`'s Bluetooth pick; an attempt for it hands back.
pub fn forget(device: DeviceId) {
    if session(|s| s.as_ref().is_some_and(|s| s.device == device)) {
        hand_back();
    }
    calls::remove_pc_audio_binding(device);
    changed();
}

// A process-lifetime fn item: native callbacks carry no UI, phone or call
// references. Completion only refreshes setup; it never starts/retargets audio.
fn prerequisites_changed() {
    if MainThreadMarker::new().is_none() || SHUTTING_DOWN.load(Ordering::SeqCst) {
        return;
    }
    refresh_prerequisites();
    changed();
}

/// Asks macOS for Bluetooth access, while undecided. User action only.
pub fn allow_bluetooth() {
    if MainThreadMarker::new().is_some() && !SHUTTING_DOWN.load(Ordering::SeqCst) {
        ffi::mac_hfp_request_bluetooth(prerequisites_changed);
        prerequisites_changed();
    }
}

/// Asks macOS for microphone access, while undecided. User action only.
pub fn allow_microphone() {
    if MainThreadMarker::new().is_some() && !SHUTTING_DOWN.load(Ordering::SeqCst) {
        ffi::mac_hfp_request_microphone(prerequisites_changed);
        prerequisites_changed();
    }
}

/// Starts one attempt for `device`'s only live call. User action only.
pub fn start(device: DeviceId) -> Result<(), String> {
    if MainThreadMarker::new().is_none() {
        return Err(not_main());
    }
    if SHUTTING_DOWN.load(Ordering::SeqCst) {
        return Err(String::new());
    }
    if session(|s| s.is_some()) {
        return Err(tr!("BtHfp", "The last Mac call audio attempt is still being closed."));
    }
    refresh_prerequisites();
    let binding = calls::pc_audio_binding(device);
    let setup = setup_state(prerequisites().as_ref(), binding.is_some());
    let Some(binding) = binding.filter(|_| setup == "ready") else {
        return Err(setup_detail(setup, ""));
    };
    let Some((_, call)) = calls::sole_active_call().filter(|(d, _)| *d == device) else {
        return Err(tr!("BtHfp", "Start this during the phone's only call, once it's answered."));
    };
    let Some(number) = call.number.clone().filter(|n| !n.trim().is_empty()) else {
        return Err(tr!(
            "BtHfp",
            "The phone didn't share this call's number, so Bluetooth can't be matched to it."
        ));
    };
    let token = NEXT_TOKEN.fetch_add(1, Ordering::SeqCst);
    let generation = calls::call_generation();
    let json = binding_json(&binding, &call, &number, &format!("{token}.{generation}"));
    let attempt = Session {
        token,
        handle: 0,
        device,
        call_id: call.id.clone(),
        number,
        incoming: call.incoming,
        generation,
        binding,
        binding_json: json.clone(),
        stopped: false,
        snapshot: Snapshot { phase: "starting".into(), ..Snapshot::default() },
    };
    if !lease_holds(&attempt, json.as_bytes(), &live(device)) {
        return Err(tr!("BtHfp", "The phone or its call changed; try again."));
    }
    session(|s| *s = Some(attempt));
    *LAST.lock().unwrap_or_else(|e| e.into_inner()) = None;
    // No lock is held: the provider calls back synchronously from here.
    let handle = ffi::mac_hfp_start(&json, token, lease_callback, event_callback);
    let started = session(|s| {
        if !s.as_ref().is_some_and(|s| s.token == token) {
            return false;
        }
        if handle == 0 {
            *s = None;
            return false;
        }
        if let Some(s) = s.as_mut() {
            s.handle = handle;
        }
        true
    });
    changed();
    if started {
        Ok(())
    } else {
        Err(tr!("BtHfp", "Mac call audio didn't start; the call's audio stays on your phone."))
    }
}

/// The Calls page's view of `device`, whose shown call is `call`.
pub fn view(device: DeviceId, call: Option<&str>) -> PcAudioView {
    let binding = calls::pc_audio_binding(device);
    let name = binding.as_ref().map(|b| b.bluetooth_name.clone()).unwrap_or_default();
    let setup = setup_state(prerequisites().as_ref(), binding.is_some());
    let current = session(|s| s.clone());
    let mut view = PcAudioView {
        setup: setup.to_owned(),
        bluetooth_name: name.clone(),
        gateways: LISTED
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_ref()
            .filter(|l| l.device == device && l.at.elapsed() < LIST_FRESH)
            .map(|l| serde_json::to_value(&l.gateways).unwrap_or_default())
            .unwrap_or_else(|| serde_json::json!([])),
        ..PcAudioView::default()
    };
    match current {
        // Its own call, or a stopped attempt still being closed for this phone.
        Some(s) if s.device == device && (call == Some(s.call_id.as_str()) || s.stopped) => {
            view.phase = s.snapshot.phase.clone();
            view.slc_observed = s.snapshot.slc_observed;
            view.sco_observed = s.snapshot.sco_observed;
            view.verified = s.snapshot.duplex_qualified && s.snapshot.handset_audible_verified;
            view.can_stop = !s.stopped && !s.snapshot.retired;
            view.detail = phase_detail(&s.snapshot, &name);
        }
        Some(_) => {
            view.detail = tr!("BtHfp", "Another Mac call audio attempt is in progress.");
        }
        None => {
            let last = LAST.lock().unwrap_or_else(|e| e.into_inner()).clone();
            if let Some((d, c, snapshot)) = last
                && d == device
                && call == Some(c.as_str())
            {
                view.phase = snapshot.phase.clone();
                view.detail = phase_detail(&snapshot, &name);
            } else {
                view.detail = setup_detail(setup, &name);
            }
            view.can_start = setup == "ready"
                && call.is_some()
                && calls::sole_active_call()
                    .is_some_and(|(d, c)| d == device && Some(c.id.as_str()) == call && c.number.is_some());
        }
    }
    view
}

#[cfg(test)]
mod tests {
    use super::*;
    use nectarlink_core::CallControls;

    #[test]
    fn call_audio_is_reported_unavailable_and_connecting_fails() {
        let status = probe_status(Some("Pixel"));
        assert!(!status.available);
        assert!(!status.connected);
        assert!(status.device_name.is_empty());
        assert!(!status.detail.is_empty());
        let err = connect_audio(Some("Pixel")).expect_err("name-only HFP seam must stay unavailable");
        assert_eq!(err, status.detail);
    }

    fn fixture() -> (Session, Live) {
        let device = DeviceId([7; 32]);
        let binding = MacCallAudioBinding {
            phone_id: device.to_string(),
            trusted_key_sha256: calls::trusted_key_sha256(device),
            bluetooth_address: "00-11-22-33-44-55".into(),
            bluetooth_name: "Phone".into(),
            input_uid: "mic".into(),
            output_uid: "speaker".into(),
        };
        let call = CallState {
            id: "call-1".into(),
            state: "active".into(),
            incoming: true,
            number: Some("+15550000000".into()),
            name: None,
            photo: None,
            missed: false,
            since: None,
            controls: None,
        };
        let json = binding_json(&binding, &call, "+15550000000", "1.4");
        let session = Session {
            token: 1,
            handle: 9,
            device,
            call_id: call.id.clone(),
            number: "+15550000000".into(),
            incoming: true,
            generation: 4,
            binding: binding.clone(),
            binding_json: json,
            stopped: false,
            snapshot: Snapshot::default(),
        };
        let live = Live {
            node_ready: true,
            shutting_down: false,
            peer_online: true,
            binding: Some(binding),
            generation: 4,
            call: Some((device, call)),
        };
        (session, live)
    }

    #[test]
    fn lease_holds_for_the_exact_binding_and_its_one_live_call() {
        let (session, live) = fixture();
        assert!(lease_holds(&session, session.binding_json.as_bytes(), &live));
    }

    #[test]
    fn lease_fails_for_any_stale_or_replaced_identity() {
        let (session, live) = fixture();
        let json = session.binding_json.clone();
        let refused = |s: &Session, l: &Live, j: &str| !lease_holds(s, j.as_bytes(), l);
        assert!(refused(&session, &live, &json.replace("call-1", "call-2")));
        assert!(refused(&Session { stopped: true, ..session.clone() }, &live, &json));
        assert!(refused(&session, &Live { shutting_down: true, ..live.clone() }, &json));
        assert!(refused(&session, &Live { peer_online: false, ..live.clone() }, &json));
        assert!(refused(&session, &Live { generation: 5, ..live.clone() }, &json));
        assert!(refused(&session, &Live { binding: None, ..live.clone() }, &json));
        let mut other = live.clone();
        other.call.as_mut().unwrap().1.id = "call-2".into();
        assert!(refused(&session, &other, &json));
        let mut renumbered = live.clone();
        renumbered.call.as_mut().unwrap().1.number = Some("15550000000".into());
        assert!(refused(&session, &renumbered, &json));
        let mut held = live.clone();
        held.call.as_mut().unwrap().1.controls = Some(CallControls { held: true, ..CallControls::default() });
        assert!(refused(&session, &held, &json));
        let mut replaced = live;
        replaced.call.as_mut().unwrap().0 = DeviceId([8; 32]);
        assert!(refused(&session, &replaced, &json));
    }
}
