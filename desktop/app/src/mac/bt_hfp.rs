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
//!
//! The picker lists cached SDP facts only: a paired phone whose hands-free
//! audio-gateway record is not cached yet shows as UNKNOWN, never as granted,
//! connected or qualified. The separate Check action (`check_phone`) may ask
//! the OS for a fresh SDP query of exactly that phone, at most one query
//! outstanding process-wide; it never binds. Errors and unsupported results
//! stay distinct from unknown, and binding/starting still require the strict
//! cached qualification.

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
    #[serde(default, deserialize_with = "diagnostic_field")]
    native_status: Option<i64>,
    slc_observed: bool,
    sco_observed: bool,
    #[serde(default, deserialize_with = "diagnostic_field")]
    stop_latched: Option<bool>,
    duplex_qualified: bool,
    handset_audible_verified: bool,
    retired: bool,
    #[serde(default, deserialize_with = "diagnostic_field")]
    audio_observation: Option<AudioObservation>,
}

// A malformed optional diagnostic must not hide the existing retirement fields.
fn diagnostic_field<'de, D, T>(deserializer: D) -> Result<Option<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: serde::de::DeserializeOwned,
{
    let value = serde_json::Value::deserialize(deserializer)?;
    Ok(serde_json::from_value(value).ok())
}

// Parse only booleans from the last successful endpoint observation. Native
// WPAudioObservation returns both endpoints only after their exact UID checks.
// This sample can predate a Stop; it is never proof of a current audio route.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(default)]
struct AudioObservation {
    input: Option<EndpointStatus>,
    output: Option<EndpointStatus>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(default)]
struct EndpointStatus {
    alive: bool,
    running: bool,
}

const DIAGNOSTIC_LIMIT: u8 = 64;

fn diagnostic_label(value: &str) -> &'static str {
    const LABELS: &[&str] = &[
        "",
        "created",
        "starting",
        "unreadable",
        "rejected",
        "connecting-service",
        "checking-exact-call",
        "connecting-audio",
        "sco-observed-unqualified",
        "returning-to-handset",
        "handback-unconfirmed",
        "service-and-sco-disconnected",
        "operator-prototype-only",
        "permission-or-current-lease-required",
        "selected-current-mic-speaker-not-observed",
        "exact-already-paired-phone-required",
        "phone-audio-gateway-service-not-in-cached-sdp",
        "new-exclusive-hf-owner-required",
        "lease-or-permission-revoked",
        "selected-current-audio-endpoints-changed",
        "native-phase-timeout",
        "service-or-sco-no-longer-connected",
        "service-connect-failed",
        "lease-revoked-before-call-check",
        "hfp-call-does-not-match-current-bound-call",
        "route-or-lease-changed-before-transfer",
        "sco-failed-or-arrived-without-bound-transfer",
        "audio-endpoints-changed-at-sco-open",
        "physical-duplex-and-handset-return-not-qualified",
        "service-disconnected",
        "sco-closed",
        "phone-call-no-longer-active",
        "phone-call-setup-changed",
        "phone-held-call-present",
        "operator-or-root-stop",
        "native-start-refused",
    ];
    LABELS.iter().copied().find(|label| *label == value).unwrap_or("unknown")
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct DiagnosticMetadata {
    phase: &'static str,
    reason: &'static str,
    native_status: Option<i64>,
    slc: bool,
    sco: bool,
    stopped: Option<bool>,
    retired: bool,
    last_input: Option<EndpointStatus>,
    last_output: Option<EndpointStatus>,
}

#[derive(Debug, Clone, Default)]
struct Diagnostics {
    last: Option<DiagnosticMetadata>,
    emitted: u8,
}

fn record_snapshot(
    slot: &mut Option<Session>,
    token: u64,
    snapshot: Snapshot,
) -> (bool, Option<(u8, DiagnosticMetadata)>) {
    let Some(s) = slot.as_mut().filter(|s| s.token == token) else { return (false, None) };
    let metadata = DiagnosticMetadata {
        phase: diagnostic_label(&snapshot.phase),
        reason: diagnostic_label(&snapshot.reason),
        native_status: snapshot.native_status,
        slc: snapshot.slc_observed,
        sco: snapshot.sco_observed,
        stopped: snapshot.stop_latched,
        retired: snapshot.retired,
        last_input: snapshot.audio_observation.as_ref().and_then(|a| a.input.clone()),
        last_output: snapshot.audio_observation.as_ref().and_then(|a| a.output.clone()),
    };
    s.snapshot = snapshot;
    if s.diagnostics.last.as_ref() == Some(&metadata) || s.diagnostics.emitted == DIAGNOSTIC_LIMIT {
        return (true, None);
    }
    s.diagnostics.last = Some(metadata.clone());
    s.diagnostics.emitted += 1;
    (true, Some((s.diagnostics.emitted, metadata)))
}

fn log_transition(token: u64, transition: Option<(u8, DiagnosticMetadata)>) {
    if let Some((sequence, m)) = transition {
        tracing::info!(
            target: "waypair_call_audio",
            session = token, sequence, limit_reached = sequence == DIAGNOSTIC_LIMIT,
            phase = m.phase, reason = m.reason, native_status = ?m.native_status,
            slc_observed = m.slc, sco_observed = m.sco,
            stop_latched = ?m.stopped, retired = m.retired,
            last_selected_input_matched = m.last_input.is_some(),
            last_selected_output_matched = m.last_output.is_some(),
            last_input_alive = ?m.last_input.as_ref().map(|e| e.alive),
            last_input_running = ?m.last_input.as_ref().map(|e| e.running),
            last_output_alive = ?m.last_output.as_ref().map(|e| e.alive),
            last_output_running = ?m.last_output.as_ref().map(|e| e.running),
            "owned Mac call audio transition"
        );
    }
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
    diagnostics: Diagnostics,
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
static SDP_CHECK: Mutex<Option<SdpCheck>> = Mutex::new(None);
static SDP_CLOCK: AtomicU64 = AtomicU64::new(1);

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
    candidates: Vec<Candidate>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Gateway {
    address: String,
    name: String,
}

/// Call-audio support as known for one already-paired phone. Never a grant:
/// `Unknown` (cached record cold or never checked) must not bind, start, or
/// claim availability; only `Supported` (cached AG flag set) may.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
enum SdpSupport {
    Unknown,
    Supported,
    Unsupported,
    Checking,
    CheckFailed,
}

/// One already-OS-paired phone with its cached SDP/AG facts. `address` is the
/// identity; `name` and the class numbers are display hints only. `support`
/// is `Supported` exactly when either cached read reports audio gateway.
#[derive(Debug, Clone)]
struct Candidate {
    address: String,
    name: String,
    device_class_major: u32,
    device_class_minor: u32,
    services_known: bool,
    support: SdpSupport,
}

#[derive(Debug, Deserialize)]
struct CandidateJson {
    address: String,
    #[serde(default)]
    name: String,
    #[serde(default, rename = "agCached")]
    ag_cached: bool,
    #[serde(default, rename = "servicesKnown")]
    services_known: bool,
    #[serde(default, rename = "deviceClassMajor")]
    device_class_major: u32,
    #[serde(default, rename = "deviceClassMinor")]
    device_class_minor: u32,
}

/// The one OS SDP query outstanding process-wide, from its start until its
/// actual completion (or an immediate refusal); nothing else frees it. A
/// cancel only clears `owned`, so the query keeps the slot busy and its
/// result is dropped. Its result may update only the exact list (`listed_at`)
/// of the phone (`device`) it was started from.
#[derive(Debug, Clone)]
struct SdpCheck {
    request: u64,
    device: DeviceId,
    address: String,
    listed_at: Instant,
    owned: bool,
}

/// One native SDP completion (`WPHfpSupportRefreshPhoneSDP`).
#[derive(Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct CheckResult {
    address: String,
    ok: bool,
    ag_supported: bool,
    services_known: bool,
}

/// What a Check press may do.
#[derive(Debug, PartialEq, Eq)]
enum CheckStep {
    /// The slot is now reserved for this exact address.
    Start(String),
    /// A query is still outstanding (wanted or cancelled): nothing starts.
    Busy,
    /// The cache already reports audio gateway, or the row isn't checkable.
    Nothing,
    /// Not on the owner's current list.
    Stale,
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
        // Usage descriptions and Bluetooth/microphone grants only reach this
        // state. The provider keeps duplexQualified false; this is not a route.
        "ready" => tr!(
            "BtHfp",
            "%1 is chosen, and Bluetooth and microphone access are allowed. Those checks do not qualify this Mac's microphone and speaker for the call.",
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
    let (ours, transition) = session(|s| record_snapshot(s, token, snapshot));
    log_transition(token, transition);
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
    cancel_sdp_check();
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
    cancel_sdp_check();
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
    // A new list replaces the old one: an outstanding check belonged to it.
    // Its query keeps the slot busy until the OS completes it; the result is
    // dropped.
    cancel_sdp_check();
    if !relist(device) {
        return Err(tr!("BtHfp", "Couldn't list this Mac's Bluetooth devices."));
    }
    changed();
    Ok(())
}

/// Re-reads the strict gateway list and the side-effect-free candidate list.
/// The strict list still gates binding/starting; candidates only display
/// cached facts. Keeps the old list on a transient read failure. Main thread.
fn relist(device: DeviceId) -> bool {
    let mut ok = false;
    let json = ffi::mac_hfp_paired_gateways(&mut ok);
    if !ok {
        return false;
    }
    let gateways: Vec<Gateway> = serde_json::from_str(&json).unwrap_or_default();
    let mut candidates_ok = false;
    let candidates_json = ffi::mac_hfp_paired_phone_candidates(&mut candidates_ok);
    let fresh: Vec<CandidateJson> =
        if candidates_ok { serde_json::from_str(&candidates_json).unwrap_or_default() } else { Vec::new() };
    let previous: std::collections::HashMap<String, SdpSupport> = LISTED
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .as_ref()
        .filter(|l| l.device == device)
        .map(|l| l.candidates.iter().map(|c| (c.address.clone(), c.support)).collect())
        .unwrap_or_default();
    let candidates: Vec<Candidate> = fresh
        .into_iter()
        .filter_map(|c| {
            let address = normalize_address(&c.address)?;
            let support = listed_support(c.ag_cached, &address, &gateways, previous.get(&address).copied());
            Some(Candidate {
                address,
                name: c.name,
                device_class_major: c.device_class_major,
                device_class_minor: c.device_class_minor,
                services_known: c.services_known,
                support,
            })
        })
        .collect();
    // Degraded but not regressed: when the new candidate facts are unreadable,
    // the strict list still shows exactly what it showed before.
    let candidates = if candidates.is_empty() && !candidates_ok && !gateways.is_empty() {
        gateways
            .iter()
            .map(|g| Candidate {
                address: g.address.clone(),
                name: g.name.clone(),
                device_class_major: 0,
                device_class_minor: 0,
                services_known: false,
                support: SdpSupport::Supported,
            })
            .collect()
    } else {
        candidates
    };
    *LISTED.lock().unwrap_or_else(|e| e.into_inner()) =
        Some(Listed { device, at: Instant::now(), gateways, candidates });
    true
}

/// A re-read row's support. Either cached read reporting audio gateway (the
/// candidate's flag, or the same address on the strict list read just before)
/// makes it `Supported`. Binding still revalidates the strict list, so a
/// changed cache may require choosing again. Otherwise an explicit result
/// survives; anything else falls back to unknown rather than unsupported.
fn listed_support(
    ag_cached: bool,
    address: &str,
    gateways: &[Gateway],
    previous: Option<SdpSupport>,
) -> SdpSupport {
    if ag_cached || gateways.iter().any(|g| g.address == address) {
        return SdpSupport::Supported;
    }
    match previous {
        Some(kept @ (SdpSupport::Unsupported | SdpSupport::CheckFailed)) => kept,
        _ => SdpSupport::Unknown,
    }
}

/// Normalizes a native dash/colon address to canonical `XX-XX-...` form.
fn normalize_address(raw: &str) -> Option<String> {
    let upper = raw.replace(':', "-").to_uppercase();
    let parts: Vec<&str> = upper.split('-').collect();
    if parts.len() != 6 || parts.iter().any(|p| p.len() != 2 || !p.bytes().all(|b| b.is_ascii_hexdigit())) {
        return None;
    }
    Some(upper)
}

/// What a Check press may do, from the owner's current list and the one
/// process-wide query slot; reserves the slot only for `Start`. Never binds:
/// a phone on the strict cached list, or shown as supported, is `Nothing`, so
/// a cache that turns audio gateway between render, press and completion
/// can't make Check bind. An unsupported row may be checked again.
fn plan_check(
    listed: Option<&Listed>,
    slot: &mut Option<SdpCheck>,
    device: DeviceId,
    address: &str,
    request: u64,
) -> CheckStep {
    let Some(listed) = listed.filter(|l| l.device == device && l.at.elapsed() < LIST_FRESH) else {
        return CheckStep::Stale;
    };
    let Some(candidate) =
        normalize_address(address).and_then(|a| listed.candidates.iter().find(|c| c.address == a))
    else {
        return CheckStep::Stale;
    };
    if listed.gateways.iter().any(|g| g.address == candidate.address)
        || !matches!(
            candidate.support,
            SdpSupport::Unknown | SdpSupport::CheckFailed | SdpSupport::Unsupported
        )
    {
        return CheckStep::Nothing;
    }
    if slot.is_some() {
        return CheckStep::Busy;
    }
    *slot = Some(SdpCheck {
        request,
        device,
        address: candidate.address.clone(),
        listed_at: listed.at,
        owned: true,
    });
    CheckStep::Start(candidate.address.clone())
}

/// Frees the slot for exactly `request`, once; any other request leaves it.
fn finish_check(slot: &mut Option<SdpCheck>, request: u64) -> Option<SdpCheck> {
    slot.take_if(|c| c.request == request)
}

/// The owner no longer wants the outstanding result; the slot stays held.
fn disown(slot: &mut Option<SdpCheck>) {
    if let Some(check) = slot.as_mut() {
        check.owned = false;
    }
}

/// Only a successful query of this same phone that refreshed its services
/// record may say unsupported; anything else is a failed check, never a claim.
fn checked_support(result: &CheckResult, address: &str) -> SdpSupport {
    if result.ok && result.address == address && result.services_known && !result.ag_supported {
        SdpSupport::Unsupported
    } else {
        SdpSupport::CheckFailed
    }
}

/// Records an owned check's result on its row of the re-read list. A phone
/// the cache now reports as audio gateway keeps `Supported`, so only the
/// explicit bind button, never Check, can use it.
fn apply_result(listed: &mut Listed, check: &SdpCheck, result: &CheckResult) {
    if listed.device != check.device || listed.gateways.iter().any(|g| g.address == check.address) {
        return;
    }
    if let Some(candidate) = listed
        .candidates
        .iter_mut()
        .find(|c| c.address == check.address && c.support != SdpSupport::Supported)
    {
        candidate.support = checked_support(result, &check.address);
    }
}

/// What a row shows: `Checking` only while this list's owner still wants the
/// outstanding query of that phone; a cancelled one shows the row's own state.
/// Trust isn't consulted: an unpaired phone's row keeps `Checking` until the
/// completion, which the callback's trust gate then drops.
fn shown_support(listed: &Listed, candidate: &Candidate, slot: Option<&SdpCheck>) -> SdpSupport {
    if slot.is_some_and(|s| {
        s.owned && s.device == listed.device && s.listed_at == listed.at && s.address == candidate.address
    }) {
        SdpSupport::Checking
    } else {
        candidate.support
    }
}

/// Logically cancels the outstanding check, if any. There is no public API
/// that cancels an SDP query, so its slot stays busy until the OS completes it.
fn cancel_sdp_check() {
    disown(&mut SDP_CHECK.lock().unwrap_or_else(|e| e.into_inner()));
}

/// Checks one listed phone's call-audio support with one SDP query of that
/// phone only. Never binds, saves settings or grants anything: the result
/// only changes what the list shows. Only while Bluetooth is already allowed,
/// the Waypair phone is trusted and the list is current. User action only.
pub fn check_phone(device: DeviceId, address: &str) -> Result<(), String> {
    if MainThreadMarker::new().is_none() {
        return Err(not_main());
    }
    if SHUTTING_DOWN.load(Ordering::SeqCst) {
        return Err(String::new());
    }
    if !trusted(device) {
        return Err(tr!("BtHfp", "Pair the phone with Waypair Link first."));
    }
    refresh_prerequisites();
    let setup = setup_state(prerequisites().as_ref(), false);
    if setup != "choose-phone" {
        return Err(setup_detail(setup, ""));
    }
    let request = SDP_CLOCK.fetch_add(1, Ordering::SeqCst);
    let step = {
        let listed = LISTED.lock().unwrap_or_else(|e| e.into_inner());
        let mut slot = SDP_CHECK.lock().unwrap_or_else(|e| e.into_inner());
        plan_check(listed.as_ref(), &mut slot, device, address, request)
    };
    let address = match step {
        CheckStep::Start(address) => address,
        CheckStep::Stale => return Err(tr!("BtHfp", "Choose the phone from the list again.")),
        // The list already says why: busy, or the phone's own state.
        CheckStep::Busy | CheckStep::Nothing => {
            changed();
            return Err(String::new());
        }
    };
    changed();
    let started = ffi::mac_hfp_refresh_phone_sdp(&address, request, sdp_event_callback);
    if started == 1 {
        return Ok(());
    }
    // Refused (0), or natively busy (2): nothing of this request is outstanding.
    finish_check(&mut SDP_CHECK.lock().unwrap_or_else(|e| e.into_inner()), request);
    if started == 0
        && let Some(listed) = LISTED.lock().unwrap_or_else(|e| e.into_inner()).as_mut()
        && listed.device == device
        && let Some(candidate) =
            listed.candidates.iter_mut().find(|c| c.address == address && c.support != SdpSupport::Supported)
    {
        candidate.support = SdpSupport::CheckFailed;
    }
    changed();
    Err(String::new())
}

/// Native completion of the outstanding query, once, on Main. Frees the slot
/// for its exact request; the result updates the list only while that owner
/// still wants it, isn't shutting down, the phone is still trusted and the
/// list it was started from is still current. Then re-reads both lists, so a
/// fresh audio gateway is strictly qualified before any explicit bind.
fn sdp_event_callback(request: u64, result_json: &[u8]) {
    if MainThreadMarker::new().is_none() {
        return;
    }
    let Some(check) = finish_check(&mut SDP_CHECK.lock().unwrap_or_else(|e| e.into_inner()), request) else {
        return;
    };
    let wanted = check.owned
        && !SHUTTING_DOWN.load(Ordering::SeqCst)
        && trusted(check.device)
        && LISTED.lock().unwrap_or_else(|e| e.into_inner()).as_ref().is_some_and(|l| {
            l.device == check.device && l.at == check.listed_at && l.at.elapsed() < LIST_FRESH
        });
    if wanted {
        let result: CheckResult = serde_json::from_slice(result_json).unwrap_or_default();
        relist(check.device);
        if let Some(listed) = LISTED.lock().unwrap_or_else(|e| e.into_inner()).as_mut() {
            apply_result(listed, &check, &result);
        }
    }
    changed();
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
    cancel_sdp_check();
    *LISTED.lock().unwrap_or_else(|e| e.into_inner()) = None;
    changed();
    Ok(())
}

/// Forgets `device`'s Bluetooth pick; an attempt for it hands back.
pub fn forget(device: DeviceId) {
    if session(|s| s.as_ref().is_some_and(|s| s.device == device)) {
        hand_back();
    }
    cancel_sdp_check();
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
        diagnostics: Diagnostics::default(),
    };
    if !lease_holds(&attempt, json.as_bytes(), &live(device)) {
        return Err(tr!("BtHfp", "The phone or its call changed; try again."));
    }
    session(|s| *s = Some(attempt));
    *LAST.lock().unwrap_or_else(|e| e.into_inner()) = None;
    let (_, transition) =
        session(|s| record_snapshot(s, token, Snapshot { phase: "starting".into(), ..Snapshot::default() }));
    log_transition(token, transition);
    // No lock is held: the provider calls back synchronously from here.
    let handle = ffi::mac_hfp_start(&json, token, lease_callback, event_callback);
    if handle == 0 {
        let (_, transition) = session(|s| {
            record_snapshot(
                s,
                token,
                Snapshot {
                    phase: "rejected".into(),
                    reason: "native-start-refused".into(),
                    ..Snapshot::default()
                },
            )
        });
        log_transition(token, transition);
    }
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
        // Every listed phone with its cached support: `supported` only when
        // the cached SDP record says audio gateway, `unknown` otherwise (never
        // granted, connected or qualified). `checkBusy` is whether any query
        // is still outstanding, wanted or not; it never means one started.
        // The address is the identity; the name and class numbers are display
        // hints only.
        gateways: LISTED
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_ref()
            .filter(|l| l.device == device && l.at.elapsed() < LIST_FRESH)
            .map(|l| {
                let slot = SDP_CHECK.lock().unwrap_or_else(|e| e.into_inner()).clone();
                serde_json::to_value(
                    l.candidates
                        .iter()
                        .map(|c| {
                            serde_json::json!({
                                "address": c.address,
                                "name": c.name,
                                "support": shown_support(l, c, slot.as_ref()),
                                "checkBusy": slot.is_some(),
                                "deviceClassMajor": c.device_class_major,
                                "deviceClassMinor": c.device_class_minor,
                                "servicesKnown": c.services_known,
                            })
                        })
                        .collect::<Vec<_>>(),
                )
                .unwrap_or_default()
            })
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

    fn grants(bluetooth: &str, microphone: &str) -> Prerequisites {
        Prerequisites {
            bluetooth_usage_description: true,
            microphone_usage_description: true,
            bluetooth: bluetooth.into(),
            microphone: microphone.into(),
        }
    }

    #[test]
    fn ready_grants_do_not_claim_a_qualified_mac_route() {
        let ready = grants("allowed", "allowed");
        assert_eq!(setup_state(Some(&ready), true), "ready");
        let detail = setup_detail("ready", "Pixel");
        assert_eq!(
            detail,
            "Pixel is chosen, and Bluetooth and microphone access are allowed. Those checks do not qualify this Mac's microphone and speaker for the call."
        );
        assert!(!detail.contains("current microphone and speaker"));
    }

    #[test]
    fn missing_or_denied_checks_do_not_claim_a_mac_route() {
        let mut missing = grants("allowed", "allowed");
        missing.microphone_usage_description = false;
        assert_eq!(setup_state(Some(&missing), true), "unavailable");
        let unavailable = setup_detail("unavailable", "Pixel");
        assert!(!unavailable.contains("microphone and speaker"));

        let allowed = grants("allowed", "allowed");
        assert_eq!(setup_state(Some(&allowed), false), "choose-phone");
        assert!(!setup_detail("choose-phone", "Pixel").contains("qualify"));

        assert_eq!(setup_state(Some(&grants("allowed", "denied")), true), "microphone-denied");
        let denied = setup_detail("microphone-denied", "Pixel");
        assert!(denied.contains("Microphone access"));
        assert!(!denied.contains("qualify"));
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
            diagnostics: Diagnostics::default(),
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
    fn diagnostics_preserve_ordered_owned_changes_and_status_only_endpoints() {
        let (owned, _) = fixture();
        let mut slot = Some(owned);
        let snapshot: Snapshot = serde_json::from_str(
            r#"{
            "phase":"connecting-service","reason":"operator-prototype-only",
            "nativeStatus":0,"slcObserved":false,"scoObserved":false,
            "audioObservation":{"input":{"uid":"PRIVATE_INPUT","alive":true,"running":false},
                                "output":{"uid":"PRIVATE_OUTPUT","alive":true,"running":true}}
        }"#,
        )
        .unwrap();
        let (ours, first) = record_snapshot(&mut slot, 1, snapshot.clone());
        assert!(ours);
        let (sequence, metadata) = first.unwrap();
        assert_eq!(sequence, 1);
        assert_eq!(metadata.native_status, Some(0));
        assert_eq!(metadata.last_input, Some(EndpointStatus { alive: true, running: false }));
        assert_eq!(metadata.last_output, Some(EndpointStatus { alive: true, running: true }));
        assert!(!format!("{metadata:?}").contains("PRIVATE_"));
        assert!(record_snapshot(&mut slot, 1, snapshot.clone()).1.is_none());
        let next = Snapshot { phase: "checking-exact-call".into(), slc_observed: true, ..snapshot };
        let (_, second) = record_snapshot(&mut slot, 1, next);
        let (sequence, metadata) = second.unwrap();
        assert_eq!(sequence, 2);
        assert_eq!(metadata.phase, "checking-exact-call");
        assert!(metadata.slc);
        assert_eq!(slot.unwrap().diagnostics.emitted, 2);
    }

    #[test]
    fn diagnostics_reject_stale_owner_scrub_unknown_labels_and_bound_events() {
        let (owned, _) = fixture();
        let mut slot = Some(owned);
        let untrusted: Snapshot = serde_json::from_str(
            r#"{
            "phase":"PRIVATE_PHONE_NUMBER","reason":"PRIVATE_BLUETOOTH_ADDRESS",
            "nativeStatus":-1,"audioObservation":null
        }"#,
        )
        .unwrap();
        assert_eq!(record_snapshot(&mut slot, 2, untrusted.clone()), (false, None));
        assert_eq!(slot.as_ref().unwrap().snapshot, Snapshot::default());
        let (_, event) = record_snapshot(&mut slot, 1, untrusted);
        let (_, metadata) = event.unwrap();
        assert_eq!(metadata.phase, "unknown");
        assert_eq!(metadata.reason, "unknown");
        assert!(!format!("{metadata:?}").contains("PRIVATE_"));
        let malformed: Snapshot = serde_json::from_str(
            r#"{
            "phase":"service-and-sco-disconnected","retired":true,
            "nativeStatus":"PRIVATE_STATUS","stopLatched":"PRIVATE_STOP",
            "audioObservation":{"input":{"alive":"PRIVATE_UID"}}
        }"#,
        )
        .unwrap();
        assert!(malformed.retired);
        assert!(malformed.native_status.is_none());
        assert!(malformed.stop_latched.is_none());
        assert!(malformed.audio_observation.is_none());
        for n in 1..=DIAGNOSTIC_LIMIT {
            let sample = Snapshot { native_status: Some(i64::from(n)), ..Snapshot::default() };
            let (_, event) = record_snapshot(&mut slot, 1, sample);
            assert_eq!(event.is_some(), n < DIAGNOSTIC_LIMIT);
        }
        let owned = slot.unwrap();
        assert_eq!(owned.diagnostics.emitted, DIAGNOSTIC_LIMIT);
        assert_eq!(owned.snapshot.native_status, Some(i64::from(DIAGNOSTIC_LIMIT)));
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

    const PHONE_A: &str = "AA-BB-CC-DD-EE-01";
    const PHONE_B: &str = "AA-BB-CC-DD-EE-02";

    fn picker(device: DeviceId) -> Listed {
        let row = |address: &str| Candidate {
            address: address.into(),
            name: String::new(),
            device_class_major: 2,
            device_class_minor: 0,
            services_known: false,
            support: SdpSupport::Unknown,
        };
        Listed {
            device,
            at: Instant::now(),
            gateways: Vec::new(),
            candidates: vec![row(PHONE_A), row(PHONE_B)],
        }
    }

    #[test]
    fn a_cancelled_sdp_query_keeps_the_one_slot_until_its_own_completion() {
        let device = DeviceId([7; 32]);
        let mut listed = picker(device);
        let mut slot = None;
        assert_eq!(
            plan_check(Some(&listed), &mut slot, device, PHONE_A, 1),
            CheckStep::Start(PHONE_A.into())
        );
        assert_eq!(shown_support(&listed, &listed.candidates[0], slot.as_ref()), SdpSupport::Checking);
        disown(&mut slot);
        assert_eq!(shown_support(&listed, &listed.candidates[0], slot.as_ref()), SdpSupport::Unknown);
        assert_eq!(plan_check(Some(&listed), &mut slot, device, PHONE_B, 2), CheckStep::Busy);
        assert_eq!(plan_check(Some(&listed), &mut slot, device, PHONE_A, 3), CheckStep::Busy);
        assert!(finish_check(&mut slot, 2).is_none());
        let late = finish_check(&mut slot, 1).expect("A's own completion frees the slot");
        assert!(!late.owned);
        assert_eq!(
            plan_check(Some(&listed), &mut slot, device, PHONE_B, 4),
            CheckStep::Start(PHONE_B.into())
        );
        assert!(finish_check(&mut slot, 1).is_none());
        assert!(slot.as_ref().is_some_and(|c| c.request == 4 && c.address == PHONE_B && c.owned));
        let check = finish_check(&mut slot, 4).expect("B's own completion");
        let proven =
            CheckResult { address: PHONE_B.into(), ok: true, services_known: true, ..CheckResult::default() };
        apply_result(&mut listed, &check, &proven);
        assert_eq!(listed.candidates[1].support, SdpSupport::Unsupported);
        assert_eq!(
            plan_check(Some(&listed), &mut slot, device, PHONE_B, 5),
            CheckStep::Start(PHONE_B.into())
        );
    }

    #[test]
    fn a_failed_sdp_query_never_lets_check_bind_or_claim_unsupported() {
        let device = DeviceId([7; 32]);
        let mut listed = picker(device);
        let mut slot = None;
        assert_eq!(
            plan_check(Some(&listed), &mut slot, device, PHONE_A, 1),
            CheckStep::Start(PHONE_A.into())
        );
        let check = finish_check(&mut slot, 1).expect("the current completion");
        listed.gateways.push(Gateway { address: PHONE_A.into(), name: String::new() });
        listed.candidates[0].support =
            listed_support(false, PHONE_A, &listed.gateways, Some(listed.candidates[0].support));
        assert_eq!(listed.candidates[0].support, SdpSupport::Supported);
        apply_result(&mut listed, &check, &CheckResult { address: PHONE_A.into(), ..CheckResult::default() });
        assert_eq!(listed.candidates[0].support, SdpSupport::Supported);
        listed.candidates[0].support = SdpSupport::CheckFailed;
        assert_eq!(plan_check(Some(&listed), &mut slot, device, PHONE_A, 2), CheckStep::Nothing);
        assert!(slot.is_none());
        assert_eq!(
            plan_check(Some(&listed), &mut slot, device, PHONE_B, 3),
            CheckStep::Start(PHONE_B.into())
        );
        let check = finish_check(&mut slot, 3).expect("the current completion");
        let unproved = CheckResult { address: PHONE_B.into(), ok: true, ..CheckResult::default() };
        apply_result(&mut listed, &check, &unproved);
        assert_eq!(listed.candidates[1].support, SdpSupport::CheckFailed);
    }

    /// `phase_detail` owns the Calls sentence. `duplex_qualified` and
    /// `handset_audible_verified` are the separate `view` predicate
    /// (`verified = duplex_qualified && handset_audible_verified`). The
    /// provider snapshot hardcodes both flags false; these values are a fixture.
    #[test]
    fn sco_phase_owns_the_unverified_sentence_without_the_duplex_flags() {
        let snapshot = Snapshot {
            phase: "sco-observed-unqualified".into(),
            reason: "operator-prototype-only".into(),
            slc_observed: true,
            sco_observed: true,
            duplex_qualified: true,
            handset_audible_verified: true,
            ..Snapshot::default()
        };
        let detail = phase_detail(&snapshot, "Pixel");
        assert_eq!(
            detail,
            "The phone opened a Bluetooth audio link to this Mac. Whether both sides hear each other isn't verified; you can return the audio to the phone at any time."
        );
        assert!(snapshot.duplex_qualified && snapshot.handset_audible_verified);
        assert!(!detail.contains("qualified"));
        let seam = probe_status(Some("Pixel"));
        assert!(!seam.available);
        assert!(!seam.connected);
        assert_ne!(detail, seam.detail);
    }

    #[test]
    fn a_different_phase_does_not_borrow_the_sco_sentence() {
        let connecting =
            Snapshot { phase: "connecting-audio".into(), sco_observed: true, ..Snapshot::default() };
        let detail = phase_detail(&connecting, "Pixel");
        assert!(detail.starts_with("Asking the phone to send this call's audio to the Mac"));
        assert!(!detail.contains("isn't verified"));

        let changed = Snapshot {
            phase: "rejected".into(),
            reason: "selected-current-mic-speaker-not-observed".into(),
            sco_observed: true,
            ..Snapshot::default()
        };
        let route = phase_detail(&changed, "Pixel");
        assert_eq!(
            route,
            "This Mac's microphone or speaker changed since setup. Choose the phone's Bluetooth device again to use the current ones."
        );
        assert!(!route.contains("isn't verified"));
        assert!(phase_detail(&Snapshot::default(), "Pixel").is_empty());

        let ready = setup_detail("ready", "Pixel");
        assert_ne!(ready, detail);
        assert!(!ready.contains("isn't verified"));
    }
}
