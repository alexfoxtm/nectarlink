// SPDX-License-Identifier: GPL-3.0-or-later
//! `PhoneCall`: the call in progress on the phone Home and Calls show, the
//! controls QML sends to it, the phone's call log and contacts, and dialing
//! a number from the PC.

use crate::i18n::tr;
use std::pin::Pin;

use cxx_qt::{CxxQtType, Threading};
use cxx_qt_lib::QString;
use nectarlink_core::{CallCommand, DeviceId};

use crate::{calls, state::Changes};

#[cxx_qt::bridge]
pub mod qobject {
    unsafe extern "C++" {
        include!("cxx-qt-lib/qstring.h");
        type QString = cxx_qt_lib::QString;
    }

    unsafe extern "RustQt" {
        #[qobject]
        #[qml_element]
        #[qml_singleton]
        /// The phone whose call is shown (set by Home).
        #[qproperty(QString, device, READ, WRITE = set_device, NOTIFY)]
        /// A call is in progress or ringing on it.
        #[qproperty(bool, active)]
        /// Whether the call is currently ringing (incoming).
        #[qproperty(bool, ringing)]
        /// Who's on the call, as well as the phone knows.
        #[qproperty(QString, caller)]
        /// The number, when there's a name to show above it.
        #[qproperty(QString, number)]
        /// When it was answered (Unix ms), or 0 when the phone didn't say.
        #[qproperty(f64, since)]
        /// The PC can end the call (and change the volume).
        #[qproperty(bool, can_end, cxx_name = "canEnd")]
        /// The phone controls the call for the PC: mute, speaker, hold, keypad.
        #[qproperty(bool, controls)]
        #[qproperty(bool, muted)]
        #[qproperty(bool, speaker)]
        #[qproperty(bool, held)]
        #[qproperty(bool, can_hold, cxx_name = "canHold")]
        /// Call log status: "idle", "loading", "ready", "offline", "off", "unsupported" or "failed".
        #[qproperty(QString, log_status, cxx_name = "logStatus")]
        /// Recent calls, newest first (JSON).
        #[qproperty(QString, call_log, cxx_name = "callLog")]
        /// Older call log entries can be loaded.
        #[qproperty(bool, more_log, cxx_name = "moreLog")]
        #[qproperty(bool, loading_older, cxx_name = "loadingOlder")]
        /// Contacts status: "idle", "loading", "ready", "offline", "off", "unsupported" or "failed".
        #[qproperty(QString, contacts_status, cxx_name = "contactsStatus")]
        /// Contacts, favorites first (JSON).
        #[qproperty(QString, contacts)]
        /// The PC can ask the phone to dial a number.
        #[qproperty(bool, can_dial, cxx_name = "canDial")]
        /// A dial request is on its way to the phone.
        #[qproperty(bool, dialing)]
        /// Whether a paired Bluetooth HFP / audio playback endpoint is available for the phone.
        #[qproperty(bool, hfp_available, cxx_name = "hfpAvailable")]
        /// Whether call audio is currently routed through this PC over Bluetooth.
        #[qproperty(bool, hfp_connected, cxx_name = "hfpConnected")]
        /// Human-readable Bluetooth HFP status or pairing guidance.
        #[qproperty(QString, hfp_detail, cxx_name = "hfpDetail")]
        /// Trying a Mac for the phone's call audio (see `calls::PcAudioView`):
        /// "unsupported", "unknown", "unavailable", "bluetooth-denied",
        /// "microphone-denied", "choose-phone", "microphone-undecided",
        /// "bluetooth-undecided" or "ready".
        #[qproperty(QString, pc_audio_setup, cxx_name = "pcAudioSetup")]
        /// The Bluetooth device the user chose for the phone (shown only).
        #[qproperty(QString, pc_audio_bluetooth_name, cxx_name = "pcAudioBluetoothName")]
        /// The attempt's native phase, or "" without one.
        #[qproperty(QString, pc_audio_phase, cxx_name = "pcAudioPhase")]
        /// The Bluetooth link to the phone was observed (not proof of audio).
        #[qproperty(bool, pc_audio_slc_observed, cxx_name = "pcAudioSlcObserved")]
        /// A Bluetooth audio link was observed (not proof either side hears).
        #[qproperty(bool, pc_audio_sco_observed, cxx_name = "pcAudioScoObserved")]
        /// Both directions heard and the handset return confirmed.
        #[qproperty(bool, pc_audio_verified, cxx_name = "pcAudioVerified")]
        #[qproperty(bool, pc_audio_can_start, cxx_name = "pcAudioCanStart")]
        #[qproperty(bool, pc_audio_can_stop, cxx_name = "pcAudioCanStop")]
        /// Plain-words status or the next step.
        #[qproperty(QString, pc_audio_detail, cxx_name = "pcAudioDetail")]
        /// The paired Bluetooth devices just listed (JSON `[{address, name}]`).
        #[qproperty(QString, pc_audio_gateways, cxx_name = "pcAudioGateways")]
        type PhoneCall = super::PhoneCallRust;
    }

    impl cxx_qt::Threading for PhoneCall {}
    impl cxx_qt::Initialize for PhoneCall {}

    unsafe extern "RustQt" {
        #[cxx_name = "setDevice"]
        fn set_device(self: Pin<&mut PhoneCall>, device: QString);
        /// Opens a phone's call log and contacts (reloads when it's the one shown).
        #[qinvokable]
        fn open(self: Pin<&mut PhoneCall>, device: &QString);
        /// Loads a phone's contacts in the background if not yet loaded.
        #[qinvokable]
        #[cxx_name = "ensureContacts"]
        fn ensure_contacts(self: &PhoneCall, device: &QString);
        /// Reads the call log and contacts again.
        #[qinvokable]
        fn refresh(self: &PhoneCall);
        #[qinvokable]
        #[cxx_name = "loadOlder"]
        fn load_older(self: &PhoneCall);
        /// Asks the phone to dial `number`.
        #[qinvokable]
        fn dial(self: &PhoneCall, number: &QString);
        /// Puts text on this PC's clipboard (it isn't sent back to the phone).
        #[qinvokable]
        fn copy(self: &PhoneCall, text: &QString);
        /// Answers an incoming ringing call.
        #[qinvokable]
        fn answer(self: &PhoneCall);
        /// Hangs up or declines.
        #[qinvokable]
        fn end(self: &PhoneCall);
        #[qinvokable]
        #[cxx_name = "setMute"]
        fn set_mute(self: &PhoneCall, on: bool);
        #[qinvokable]
        #[cxx_name = "setSpeakerOn"]
        fn set_speaker_on(self: &PhoneCall, on: bool);
        #[qinvokable]
        #[cxx_name = "setHold"]
        fn set_hold(self: &PhoneCall, on: bool);
        /// A keypad key: "0".."9", "*" or "#".
        #[qinvokable]
        fn press(self: &PhoneCall, key: &QString);
        /// The call's volume up (true) or down.
        #[qinvokable]
        fn volume(self: &PhoneCall, up: bool);
        /// Connects Bluetooth HFP / audio playback for the active phone.
        #[qinvokable]
        #[cxx_name = "connectPcAudio"]
        fn connect_pc_audio(self: Pin<&mut PhoneCall>);
        /// Opens Windows Bluetooth & devices settings (`ms-settings:bluetooth`).
        #[qinvokable]
        #[cxx_name = "openBluetoothSettings"]
        fn open_bluetooth_settings(self: Pin<&mut PhoneCall>);
        /// Lists the Mac's already-paired phones that offer call audio (`pcAudioGateways`).
        #[qinvokable]
        #[cxx_name = "findBluetoothPhones"]
        fn find_bluetooth_phones(self: Pin<&mut PhoneCall>);
        /// The user picked this listed address as the phone's Bluetooth device.
        #[qinvokable]
        #[cxx_name = "chooseBluetoothPhone"]
        fn choose_bluetooth_phone(self: Pin<&mut PhoneCall>, address: &QString);
        /// Checks this listed phone's call audio support; never chooses it.
        #[qinvokable]
        #[cxx_name = "checkBluetoothPhone"]
        fn check_bluetooth_phone(self: Pin<&mut PhoneCall>, address: &QString);
        #[qinvokable]
        #[cxx_name = "forgetBluetoothPhone"]
        fn forget_bluetooth_phone(self: Pin<&mut PhoneCall>);
        /// Asks macOS for Bluetooth access while it's undecided.
        #[qinvokable]
        #[cxx_name = "allowBluetooth"]
        fn allow_bluetooth(self: Pin<&mut PhoneCall>);
        /// Asks macOS for microphone access while it's undecided.
        #[qinvokable]
        #[cxx_name = "allowMicrophone"]
        fn allow_microphone(self: Pin<&mut PhoneCall>);
        /// Tries this Mac for the shown call's audio.
        #[qinvokable]
        #[cxx_name = "startPcAudio"]
        fn start_pc_audio(self: Pin<&mut PhoneCall>);
        /// Returns the call's audio to the phone.
        #[qinvokable]
        #[cxx_name = "stopPcAudio"]
        fn stop_pc_audio(self: Pin<&mut PhoneCall>);
    }
}

#[derive(Default)]
pub struct PhoneCallRust {
    device: QString,
    active: bool,
    ringing: bool,
    caller: QString,
    number: QString,
    since: f64,
    can_end: bool,
    controls: bool,
    muted: bool,
    speaker: bool,
    held: bool,
    can_hold: bool,
    log_status: QString,
    call_log: QString,
    more_log: bool,
    loading_older: bool,
    contacts_status: QString,
    contacts: QString,
    can_dial: bool,
    dialing: bool,
    hfp_available: bool,
    hfp_connected: bool,
    hfp_detail: QString,
    pc_audio_setup: QString,
    pc_audio_bluetooth_name: QString,
    pc_audio_phase: QString,
    pc_audio_slc_observed: bool,
    pc_audio_sco_observed: bool,
    pc_audio_verified: bool,
    pc_audio_can_start: bool,
    pc_audio_can_stop: bool,
    pc_audio_detail: QString,
    pc_audio_gateways: QString,
    /// The call's ID on the phone, for its commands.
    call: Option<(DeviceId, String)>,
    /// What the Calls page last showed, to skip JSON updates that changed nothing.
    last_view: Option<calls::View>,
}

impl cxx_qt::Initialize for qobject::PhoneCall {
    fn initialize(mut self: Pin<&mut Self>) {
        self.as_mut().set_log_status(QString::from("idle"));
        self.as_mut().set_call_log(QString::from("[]"));
        self.as_mut().set_contacts_status(QString::from("idle"));
        self.as_mut().set_contacts(QString::from("[]"));
        self.as_mut().set_pc_audio_gateways(QString::from("[]"));
        super::subscribe(self.qt_thread(), Changes::CALLS | Changes::CAPABILITIES, Self::refresh_state);
    }
}

impl qobject::PhoneCall {
    pub fn set_device(mut self: Pin<&mut Self>, device: QString) {
        if self.rust().device == device {
            return;
        }
        self.as_mut().rust_mut().device = device;
        self.as_mut().device_changed();
        self.refresh_state();
    }

    pub fn open(mut self: Pin<&mut Self>, device: &QString) {
        if self.rust().device != *device {
            self.as_mut().rust_mut().device = device.clone();
            self.as_mut().device_changed();
        }
        if let Some(id) = super::parse_device(device) {
            calls::open_device(id);
        }
        self.refresh_state();
    }

    pub fn ensure_contacts(&self, device: &QString) {
        if let Some(id) = super::parse_device(device) {
            calls::ensure_contacts(id);
        }
    }

    pub fn refresh(&self) {
        calls::reload();
    }

    pub fn load_older(&self) {
        calls::load_older();
    }

    pub fn dial(&self, number: &QString) {
        if let Some(device) = super::parse_device(&self.rust().device) {
            calls::dial(device, String::from(number));
        }
    }

    pub fn copy(&self, text: &QString) {
        if let Err(reason) = crate::win::clipboard::write(&String::from(text)) {
            tracing::warn!(reason, "can't copy");
        }
    }

    pub fn connect_pc_audio(self: Pin<&mut Self>) {
        let device = super::parse_device(&self.rust().device);
        calls::connect_pc_audio(device);
        self.refresh_state();
    }

    pub fn open_bluetooth_settings(self: Pin<&mut Self>) {
        calls::open_bluetooth_settings();
    }

    /// The phone PC call audio set-up is for: the one shown, else the call's.
    fn pc_audio_device(&self) -> Option<DeviceId> {
        super::parse_device(&self.rust().device).or_else(|| self.rust().call.as_ref().map(|(d, _)| *d))
    }

    pub fn find_bluetooth_phones(self: Pin<&mut Self>) {
        if let Some(device) = self.pc_audio_device() {
            calls::find_bluetooth_phones(device);
        }
        self.refresh_state();
    }

    pub fn choose_bluetooth_phone(self: Pin<&mut Self>, address: &QString) {
        if let Some(device) = self.pc_audio_device() {
            calls::bind_bluetooth_phone(device, &String::from(address));
        }
        self.refresh_state();
    }

    pub fn check_bluetooth_phone(self: Pin<&mut Self>, address: &QString) {
        #[cfg(target_os = "macos")]
        if let Some(device) = self.pc_audio_device()
            && let Err(message) = crate::win::bt_hfp::check_phone(device, &String::from(address))
            && !message.is_empty()
        {
            super::app::show_message(message);
        }
        #[cfg(not(target_os = "macos"))]
        let _ = address;
        self.refresh_state();
    }

    pub fn forget_bluetooth_phone(self: Pin<&mut Self>) {
        if let Some(device) = self.pc_audio_device() {
            calls::forget_bluetooth_phone(device);
        }
        self.refresh_state();
    }

    pub fn allow_bluetooth(self: Pin<&mut Self>) {
        calls::allow_bluetooth();
        self.refresh_state();
    }

    pub fn allow_microphone(self: Pin<&mut Self>) {
        calls::allow_microphone();
        self.refresh_state();
    }

    /// Only for the call this page shows.
    pub fn start_pc_audio(self: Pin<&mut Self>) {
        if let Some((device, _)) = self.rust().call.clone() {
            calls::start_pc_audio(device);
        }
        self.refresh_state();
    }

    pub fn stop_pc_audio(self: Pin<&mut Self>) {
        calls::stop_pc_audio();
        self.refresh_state();
    }

    fn refresh_state(mut self: Pin<&mut Self>) {
        let preferred_device = super::parse_device(&self.rust().device);
        let active_call = calls::active_or_ringing(preferred_device);
        let call_device = active_call.as_ref().map(|(d, _, _)| *d).or(preferred_device);
        let is_ringing = active_call.as_ref().is_some_and(|(_, _, r)| *r);
        let call = active_call.map(|(_, c, _)| c);
        let controls = call.as_ref().and_then(|c| c.controls);
        self.as_mut().rust_mut().call = call_device.zip(call.as_ref().map(|c| c.id.clone()));
        let (caller, number) = match &call {
            Some(call) => {
                let name = call.name.clone().filter(|n| !n.trim().is_empty());
                let number = call.number.clone().filter(|n| !n.trim().is_empty());
                match name {
                    Some(name) => (name, number.unwrap_or_default()),
                    None => (number.unwrap_or_else(|| tr!("Calls", "Unknown caller")), String::new()),
                }
            }
            None => Default::default(),
        };
        self.as_mut().set_caller(QString::from(&caller));
        self.as_mut().set_number(QString::from(&number));
        self.as_mut().set_since(call.as_ref().and_then(|c| c.since).unwrap_or(0) as f64);
        self.as_mut().set_can_end(call_device.is_some_and(calls::can_control));
        self.as_mut().set_can_dial(preferred_device.or(call_device).is_some_and(calls::can_dial));
        self.as_mut().set_controls(controls.is_some());
        let controls = controls.unwrap_or_default();
        self.as_mut().set_muted(controls.muted);
        self.as_mut().set_speaker(controls.speaker);
        self.as_mut().set_held(controls.held);
        self.as_mut().set_can_hold(controls.can_hold);
        self.as_mut().set_ringing(is_ringing);
        self.as_mut().set_active(call.is_some());

        let hfp = calls::hfp_status(call_device);
        self.as_mut().set_hfp_available(hfp.available);
        self.as_mut().set_hfp_connected(hfp.connected);
        self.as_mut().set_hfp_detail(QString::from(&hfp.detail));

        let pc_audio_device = preferred_device.or(call_device);
        let shown_call =
            call.as_ref().filter(|_| !is_ringing && call_device == pc_audio_device).map(|c| c.id.as_str());
        let pc_audio = calls::pc_audio_view(pc_audio_device, shown_call);
        self.as_mut().set_pc_audio_setup(QString::from(&pc_audio.setup));
        self.as_mut().set_pc_audio_bluetooth_name(QString::from(&pc_audio.bluetooth_name));
        self.as_mut().set_pc_audio_phase(QString::from(&pc_audio.phase));
        self.as_mut().set_pc_audio_slc_observed(pc_audio.slc_observed);
        self.as_mut().set_pc_audio_sco_observed(pc_audio.sco_observed);
        self.as_mut().set_pc_audio_verified(pc_audio.verified);
        self.as_mut().set_pc_audio_can_start(pc_audio.can_start);
        self.as_mut().set_pc_audio_can_stop(pc_audio.can_stop);
        self.as_mut().set_pc_audio_detail(QString::from(&pc_audio.detail));
        self.as_mut().set_pc_audio_gateways(QString::from(&pc_audio.gateways.to_string()));

        let view = calls::view();
        if self.rust().last_view.as_ref() != Some(&view) {
            let last = self.rust().last_view.clone();
            let differs = |f: fn(&calls::View) -> String| last.as_ref().is_none_or(|l| f(l) != f(&view));
            self.as_mut().set_log_status(QString::from(view.log_status.as_str()));
            if differs(|v| v.call_log.to_string()) {
                self.as_mut().set_call_log(QString::from(&view.call_log.to_string()));
            }
            self.as_mut().set_more_log(view.more_log);
            self.as_mut().set_loading_older(view.loading_older);
            self.as_mut().set_contacts_status(QString::from(view.contacts_status.as_str()));
            if differs(|v| v.contacts.to_string()) {
                self.as_mut().set_contacts(QString::from(&view.contacts.to_string()));
            }
            self.as_mut().set_dialing(view.dialing);
            self.as_mut().rust_mut().last_view = Some(view);
        }
    }

    fn send(&self, command: CallCommand) {
        if let Some((device, call)) = self.rust().call.clone() {
            calls::command(device, call, command);
        }
    }

    pub fn answer(&self) {
        self.send(CallCommand::Answer);
    }

    pub fn end(&self) {
        self.send(CallCommand::Decline);
    }

    pub fn set_mute(&self, on: bool) {
        self.send(CallCommand::Mute(on));
    }

    pub fn set_speaker_on(&self, on: bool) {
        self.send(CallCommand::Speaker(on));
    }

    pub fn set_hold(&self, on: bool) {
        self.send(CallCommand::Hold(on));
    }

    pub fn press(&self, key: &QString) {
        let key = String::from(key);
        if let [digit] = key.chars().collect::<Vec<_>>()[..]
            && (digit.is_ascii_digit() || digit == '*' || digit == '#')
        {
            self.send(CallCommand::Dtmf(digit));
        }
    }

    pub fn volume(&self, up: bool) {
        self.send(CallCommand::Volume(up));
    }
}
