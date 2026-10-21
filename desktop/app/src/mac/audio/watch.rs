// SPDX-License-Identifier: GPL-3.0-or-later
//! CoreAudio output-property watcher: volume, mute, and default output.
//!
//! Declarations follow the macOS SDK headers `AudioHardware.h`
//! (`AudioObjectPropertyListenerProc`, `AudioObjectAddPropertyListener`,
//! `AudioObjectRemovePropertyListener`, `kAudioHardwarePropertyDevices`,
//! `kAudioHardwarePropertyDefaultOutputDevice`,
//! `kAudioHardwarePropertyServiceRestarted`,
//! `kAudioDevicePropertyVolumeScalar`, `kAudioDevicePropertyMute`) and
//! `AudioHardwareBase.h` (`AudioObjectPropertyAddress` selector/scope/element
//! order, `kAudioObjectPropertyScopeGlobal`, `kAudioObjectPropertyScopeOutput`,
//! `kAudioObjectPropertyElementMain`, `kAudioObjectSystemObject`,
//! `kAudioObjectUnknown`). `CoreAudioBaseTypes.h` supplies the `AudioObjectID`
//! and four-char-code value shapes. Constant values are duplicated here from
//! `audio_out/hal.rs` by design so the two modules stay disjoint (no shared
//! header edits); both were checked against the installed SDK headers above
//! (`MacOSX.sdk`, `CoreAudio.framework/Headers`).
//!
//! Read-only listeners only: this module never sets a property, never selects
//! a device, and never touches source capability flags (source caps stay OFF,
//! no output switch). `deck.rs` keeps its Deck-active polling loop; this only
//! adds the push path that `mac/audio.rs::watch` currently lacks (no-op),
//! matching the `win/audio.rs::watch` contract (`changed` on a worker thread
//! after a burst settles).
//!
//! The HAL delivers listener calls on its notification run loop, which since
//! macOS 10.6 is the process's main run loop (`CFRunLoopGetMain()`, see
//! `kAudioHardwarePropertyRunLoop` in `AudioHardwareDeprecated.h`). This module
//! never changes that; its listener only sends on a channel.
//!
//! Bounds: at most 9 listeners (3 system + 6 device), at most one `changed`
//! call per burst (a burst ends after 150 ms without a change, same as
//! Windows, or 500 ms after it began, whichever comes first), at most 64
//! addresses scanned per HAL callback, exactly one watcher context and one
//! thread per process. Volume and mute callbacks from a device that is no
//! longer the bound default are ignored (stale removals cannot grow the
//! listener bound; at most one extra Deck refresh is lost). Nothing runs
//! while nothing changes.
//!
//! Lifetime: like `win::audio::watch`, `watch` returns `()` and nothing can
//! stop it. The thread and its listeners stay until the process exits.

#![allow(unsafe_code)]

use std::{
    ffi::c_void,
    mem::size_of,
    ptr,
    sync::{
        atomic::{AtomicBool, AtomicU32, Ordering},
        mpsc::{Receiver, Sender},
    },
    time::{Duration, Instant},
};

type OSStatus = i32;
type AudioObjectID = u32;

/// `AudioObjectPropertyListenerProc` (`AudioHardware.h`). The return value is
/// unused and must always be 0.
type Listener =
    unsafe extern "C" fn(AudioObjectID, u32, *const AudioObjectPropertyAddress, *mut c_void) -> OSStatus;

/// `AudioObjectPropertyAddress` (`AudioHardwareBase.h`): selector, scope,
/// element, in that order.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct AudioObjectPropertyAddress {
    selector: u32,
    scope: u32,
    element: u32,
}

const _: () = assert!(size_of::<AudioObjectPropertyAddress>() == 12);

/// `kAudioObjectSystemObject`.
const SYSTEM: AudioObjectID = 1;
/// `kAudioObjectUnknown`.
const UNKNOWN: AudioObjectID = 0;
/// `kAudioObjectPropertyScopeGlobal` (`'glob'`).
const SCOPE_GLOBAL: u32 = u32::from_be_bytes(*b"glob");
/// `kAudioObjectPropertyScopeOutput` (`'outp'`). Volume and mute live here;
/// the `AudioDevice` class carries a main element plus one element per channel
/// (`AudioHardware.h`), so the master and the first two channels are watched.
const SCOPE_OUTPUT: u32 = u32::from_be_bytes(*b"outp");
/// `kAudioObjectPropertyElementMain`.
const ELEMENT_MAIN: u32 = 0;
/// `kAudioHardwarePropertyDevices` (`'dev#'`): the device set changed.
const DEVICES: u32 = u32::from_be_bytes(*b"dev#");
/// `kAudioHardwarePropertyDefaultOutputDevice` (`'dOut'`): the default moved.
const DEFAULT_OUTPUT: u32 = u32::from_be_bytes(*b"dOut");
/// `kAudioHardwarePropertyServiceRestarted` (`'srst'`, `AudioHardware.h:573-577`
/// and `:631`): the HAL service reset; cached state and added listeners must
/// be re-established by the client. The value has no meaning, only the
/// notification matters.
const SERVICE_RESTARTED: u32 = u32::from_be_bytes(*b"srst");
/// `kAudioDevicePropertyVolumeScalar` (`'volm'`).
const VOLUME_SCALAR: u32 = u32::from_be_bytes(*b"volm");
/// `kAudioDevicePropertyMute` (`'mute'`).
const MUTE: u32 = u32::from_be_bytes(*b"mute");

/// Master plus the first two output channels, per watched selector.
const WATCH_CHANNELS: [u32; 3] = [ELEMENT_MAIN, 1, 2];
/// Two selectors (volume, mute) times three elements.
const MAX_DEVICE_TARGETS: usize = 6;
/// Longest HAL address batch scanned before a conservative rebind.
const MAX_ADDRESSES_PER_CALL: u32 = 64;
/// A burst ends after this long without a change. Matches `win/audio.rs`.
const SETTLE: Duration = Duration::from_millis(150);
/// A burst that never goes quiet (a long slider drag, a device that keeps
/// posting) still ends this long after it began, so `changed` keeps coming.
const MAX_BURST: Duration = Duration::from_millis(500);

#[link(name = "CoreAudio", kind = "framework")]
unsafe extern "C" {
    fn AudioObjectAddPropertyListener(
        object: AudioObjectID,
        address: *const AudioObjectPropertyAddress,
        listener: Listener,
        client: *mut c_void,
    ) -> OSStatus;
    fn AudioObjectRemovePropertyListener(
        object: AudioObjectID,
        address: *const AudioObjectPropertyAddress,
        listener: Listener,
        client: *mut c_void,
    ) -> OSStatus;
    fn AudioObjectGetPropertyData(
        object: AudioObjectID,
        address: *const AudioObjectPropertyAddress,
        qualifier_size: u32,
        qualifier: *const c_void,
        io_size: *mut u32,
        out_data: *mut c_void,
    ) -> OSStatus;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Notice {
    /// Default output volume or mute moved.
    Volume,
    /// Device set or default output moved; device listeners must rebind.
    Devices,
    /// HAL service restarted; system and device listeners must be
    /// re-established.
    Restart,
}

/// Process-lifetime listener context, leaked exactly once (see `watch`: the
/// `STARTED` guard allows a single leak), so a HAL callback in flight during
/// a rebind can never observe freed memory. No Core Foundation object is
/// held, so there is no CF ownership to manage. `Sync` because `Sender` and
/// `AtomicU32` are.
#[derive(Debug)]
struct Context {
    tx: Sender<Notice>,
    /// Currently bound default output, or `UNKNOWN` when there is none.
    /// Written by `rebind` on the worker thread, read by `on_property` on the
    /// HAL notification run loop to drop stale volume callbacks from a device
    /// whose removal failed. Staleness only costs an extra or missed Deck
    /// refresh; the next device notice corrects the binding.
    bound: AtomicU32,
}

/// Only the first `watch` call arms; later calls are no-ops. This is what
/// makes a stale second watcher impossible: no second context is ever
/// allocated, so no dropped context can orphan live HAL registrations (which
/// would fire into freed memory).
static STARTED: AtomicBool = AtomicBool::new(false);

fn address(selector: u32, scope: u32, element: u32) -> AudioObjectPropertyAddress {
    AudioObjectPropertyAddress { selector, scope, element }
}

/// Pure: which notice one property selector produces, if any. Unlisted
/// selectors (pan, decibels, wildcard batches for properties this watcher
/// never asked about) stay silent instead of waking the Deck.
fn classify_selector(selector: u32) -> Option<Notice> {
    match selector {
        VOLUME_SCALAR | MUTE => Some(Notice::Volume),
        DEFAULT_OUTPUT | DEVICES => Some(Notice::Devices),
        SERVICE_RESTARTED => Some(Notice::Restart),
        _ => None,
    }
}

/// Pure: whether a volume or mute notice for `object` is current. A stale
/// device whose removal failed keeps firing; only the bound default (and, for
/// robustness against HAL batches that include unasked addresses, the system
/// object itself) may wake the Deck. Device-set, default-output, and restart
/// notices are never filtered here: missing a move is worse than an extra
/// refresh.
fn is_current_volume(object: AudioObjectID, bound: Option<AudioObjectID>) -> bool {
    object == SYSTEM || Some(object) == bound
}

/// Pure: every device address this watcher registers, in add/remove order.
/// Listener removal uses this same array, so add and remove tuples are
/// identical by construction.
fn device_addresses() -> [AudioObjectPropertyAddress; MAX_DEVICE_TARGETS] {
    let mut out = [address(VOLUME_SCALAR, SCOPE_OUTPUT, ELEMENT_MAIN); MAX_DEVICE_TARGETS];
    let mut next = 0;
    for selector in [VOLUME_SCALAR, MUTE] {
        for element in WATCH_CHANNELS {
            out[next] = address(selector, SCOPE_OUTPUT, element);
            next += 1;
        }
    }
    out
}

fn system_addresses() -> [AudioObjectPropertyAddress; 3] {
    [
        address(DEFAULT_OUTPUT, SCOPE_GLOBAL, ELEMENT_MAIN),
        address(DEVICES, SCOPE_GLOBAL, ELEMENT_MAIN),
        address(SERVICE_RESTARTED, SCOPE_GLOBAL, ELEMENT_MAIN),
    ]
}

/// Returns whether the HAL accepted the listener.
fn add(object: AudioObjectID, addr: &AudioObjectPropertyAddress, client: *mut c_void) -> bool {
    // SAFETY: `object`/`addr` describe a live registration tuple, `client`
    // is the leaked context, and `on_property` matches the SDK signature.
    // The HAL copies the address during the call.
    let status = unsafe { AudioObjectAddPropertyListener(object, addr, on_property, client) };
    if status != 0 {
        tracing::debug!(object, selector = addr.selector, status, "audio watch: listener not added");
    }
    status == 0
}

fn remove(object: AudioObjectID, addr: &AudioObjectPropertyAddress, client: *mut c_void) {
    // SAFETY: same tuple previously added (or best-effort teardown on a
    // device that already vanished); a nonzero status only means there was
    // nothing left to remove.
    let status = unsafe { AudioObjectRemovePropertyListener(object, addr, on_property, client) };
    if status != 0 {
        tracing::debug!(object, selector = addr.selector, status, "audio watch: listener not removed");
    }
}

fn watch_device(device: AudioObjectID, client: *mut c_void) {
    let added = device_addresses().iter().filter(|addr| add(device, addr, client)).count();
    if added == 0 {
        tracing::warn!(device, "audio watch: no volume or mute listener on the default output");
    }
}

fn unwatch_device(device: AudioObjectID, client: *mut c_void) {
    for addr in device_addresses() {
        remove(device, &addr, client);
    }
}

/// Reads the current default output id. `Ok(None)` means there is no
/// default (`kAudioObjectUnknown`); `Err` carries the HAL status of a failed
/// read, which says nothing about whether the default moved.
fn current_default() -> Result<Option<AudioObjectID>, OSStatus> {
    let addr = address(DEFAULT_OUTPUT, SCOPE_GLOBAL, ELEMENT_MAIN);
    let mut id = UNKNOWN;
    let mut size = size_of::<AudioObjectID>() as u32;
    // SAFETY: the system object is always id 1; `id` is exactly the
    // property's size.
    let status = unsafe {
        AudioObjectGetPropertyData(SYSTEM, &addr, 0, ptr::null(), &mut size, (&raw mut id).cast::<c_void>())
    };
    if status != 0 {
        return Err(status);
    }
    Ok((id != UNKNOWN).then_some(id))
}

/// Binds the device listeners to the current default output, after a device
/// notice (and once at start). A failed read keeps the current binding: a
/// transient HAL error must not leave a still-current default unwatched.
/// Otherwise the old device's listeners go and the default's are added, even
/// when the id is unchanged: a device that vanished and came back under the
/// same id lost its listeners, and an earlier failed add gets another try.
/// `client` never changes, so an in-flight callback still addresses live
/// memory. The bound id is published for stale-callback filtering; on a
/// failed read it is left untouched with the listeners.
fn rebind(watched: &mut Option<AudioObjectID>, client: *mut c_void) {
    // SAFETY: `client` is the leaked `Context` created by `watch` and passed
    // through `watch_thread`; it lives for the rest of the process.
    let context = unsafe { &*(client as *const Context) };
    let next = match current_default() {
        Ok(next) => next,
        Err(status) => {
            tracing::warn!(status, "audio watch: default output unreadable; keeping the current binding");
            return;
        }
    };
    if let Some(old) = watched.take() {
        unwatch_device(old, client);
    }
    *watched = next;
    context.bound.store(next.unwrap_or(UNKNOWN), Ordering::Release);
    if let Some(device) = next {
        watch_device(device, client);
    }
}

/// Re-establishes every listener after a `kAudioHardwarePropertyServiceRestarted`
/// notice: the HAL dropped all registrations, so the system pair plus restart
/// listener and the current default's device listeners must be added again.
/// Best-effort removes run first so a spurious restart (no actual reset) cannot
/// accumulate duplicate system listeners; the bound stays at most 3 + 6 = 9.
/// Device work goes through `rebind`, so a failed default read keeps the good
/// binding exactly as on the device path.
fn reestablish(watched: &mut Option<AudioObjectID>, client: *mut c_void) {
    for addr in system_addresses() {
        remove(SYSTEM, &addr, client);
    }
    for addr in system_addresses() {
        add(SYSTEM, &addr, client);
    }
    rebind(watched, client);
}

/// # Safety
///
/// `client` is null or the leaked `Context` from `watch`, which lives for the
/// process. `addrs` is null or points at `count` HAL-owned addresses valid
/// for this call (`AudioHardware.h`). Returns 0, the SDK's unused value.
///
/// The body performs callback-safe work only: a bounded scan plus channel
/// sends. No CoreAudio calls (no re-entrancy into the HAL), no CF touches, no
/// locks, no panicking operations, so unwinding across the FFI boundary is
/// impossible. Stale volume callbacks (object is neither the bound default
/// nor the system object) are dropped here, so a failed removal cannot wake
/// the Deck.
unsafe extern "C" fn on_property(
    object: AudioObjectID,
    count: u32,
    addrs: *const AudioObjectPropertyAddress,
    client: *mut c_void,
) -> OSStatus {
    if client.is_null() || addrs.is_null() {
        return 0;
    }
    // SAFETY: `client` is the leaked context (or null, checked above).
    let context = unsafe { &*(client as *const Context) };
    let bound = match context.bound.load(Ordering::Acquire) {
        UNKNOWN => None,
        id => Some(id),
    };
    // A corrupt `count` cannot drive an unbounded scan; a truncated scan
    // conservatively rebinds instead of missing a device move.
    let mut devices_changed = count > MAX_ADDRESSES_PER_CALL;
    let mut restart_changed = false;
    let len = count.min(MAX_ADDRESSES_PER_CALL) as usize;
    // SAFETY: `addrs` is non-null and HAL-live for `count` entries, and
    // `len` does not exceed `count`. No element is retained past this call.
    let changed = unsafe { std::slice::from_raw_parts(addrs, len) };
    let mut volume_changed = false;
    for addr in changed {
        match classify_selector(addr.selector) {
            Some(Notice::Restart) => restart_changed = true,
            Some(Notice::Devices) => devices_changed = true,
            Some(Notice::Volume) if is_current_volume(object, bound) => volume_changed = true,
            Some(Notice::Volume) | None => {}
        }
    }
    if restart_changed {
        let _ = context.tx.send(Notice::Restart);
    } else if devices_changed {
        let _ = context.tx.send(Notice::Devices);
    } else if volume_changed {
        let _ = context.tx.send(Notice::Volume);
    }
    0
}

fn watch_thread(changed: fn(), rx: Receiver<Notice>, context: &'static Context) {
    // Every add/remove uses this one pointer to the leaked context, so an
    // in-flight HAL callback during a rebind still addresses live memory.
    let client = ptr::from_ref(context).cast_mut().cast::<c_void>();
    // The default-output listener goes first: a default that moves between
    // the read below and this registration would otherwise go unnoticed.
    for addr in system_addresses() {
        add(SYSTEM, &addr, client);
    }
    let mut watched = None;
    rebind(&mut watched, client);
    // `rx` never disconnects: the leaked context keeps its sender for the
    // rest of the process, so this loop runs until the process exits.
    while let Ok(first) = rx.recv() {
        // Coalesce: a slider drag or a device switch delivers a burst, and
        // one `changed` call covers it. Later notices stay queued for the
        // next burst. A restart subsumes device notices in the same burst.
        let deadline = Instant::now() + MAX_BURST;
        let mut restart_changed = matches!(first, Notice::Restart);
        let mut devices_changed = matches!(first, Notice::Devices);
        loop {
            let wait = deadline.saturating_duration_since(Instant::now()).min(SETTLE);
            if wait.is_zero() {
                break;
            }
            match rx.recv_timeout(wait) {
                Ok(next) => {
                    restart_changed |= matches!(next, Notice::Restart);
                    devices_changed |= matches!(next, Notice::Devices);
                }
                Err(_) => break,
            }
        }
        if restart_changed {
            reestablish(&mut watched, client);
        } else if devices_changed {
            rebind(&mut watched, client);
        }
        changed();
    }
}

/// Calls `changed` (on a worker thread, a moment after a burst of changes)
/// whenever CoreAudio reports a change to the default output's volume or
/// mute, to the output devices (one plugged in or out, a new default), or to
/// the HAL service itself (restarted, so listeners are re-established).
/// This replaces polling: nothing runs while nothing changes. Shape matches
/// `win/audio.rs::watch`.
///
/// Process lifetime: returns `()` and there is no way to stop the watcher;
/// its thread and listeners stay until the process exits. Only the first
/// call arms; a later call, and its `changed`, is ignored. If the thread
/// cannot start, nothing is watched and the Deck keeps its poll.
pub(super) fn watch(changed: fn()) {
    if STARTED.swap(true, Ordering::SeqCst) {
        return;
    }
    let (tx, rx) = std::sync::mpsc::channel();
    let context: &'static Context = Box::leak(Box::new(Context { tx, bound: AtomicU32::new(UNKNOWN) }));
    let spawned = std::thread::Builder::new()
        .name("audio-watch".into())
        .spawn(move || watch_thread(changed, rx, context));
    if let Err(error) = spawned {
        tracing::warn!(%error, "audio watch: thread not started; the Deck keeps its poll");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn watch_targets_cover_output_volume_mute_and_default() {
        assert_eq!(VOLUME_SCALAR, u32::from_be_bytes(*b"volm"));
        assert_eq!(MUTE, u32::from_be_bytes(*b"mute"));
        assert_eq!(DEFAULT_OUTPUT, u32::from_be_bytes(*b"dOut"));
        assert_eq!(DEVICES, u32::from_be_bytes(*b"dev#"));
        assert_eq!(SERVICE_RESTARTED, u32::from_be_bytes(*b"srst"));
        assert_eq!(SCOPE_OUTPUT, u32::from_be_bytes(*b"outp"));
        assert_eq!(SCOPE_GLOBAL, u32::from_be_bytes(*b"glob"));
        assert_eq!(ELEMENT_MAIN, 0);
        let device = device_addresses();
        assert_eq!(device.len(), MAX_DEVICE_TARGETS);
        for addr in &device {
            assert_eq!(addr.scope, SCOPE_OUTPUT);
            assert!(classify_selector(addr.selector).is_some());
        }
        for selector in [VOLUME_SCALAR, MUTE] {
            for element in WATCH_CHANNELS {
                assert!(
                    device.contains(&AudioObjectPropertyAddress { selector, scope: SCOPE_OUTPUT, element }),
                    "missing ({selector:#x}, {element})"
                );
            }
        }
        let system = system_addresses();
        assert_eq!(system.len(), 3);
        assert!(system.contains(&address(DEFAULT_OUTPUT, SCOPE_GLOBAL, ELEMENT_MAIN)));
        assert!(system.contains(&address(DEVICES, SCOPE_GLOBAL, ELEMENT_MAIN)));
        assert!(system.contains(&address(SERVICE_RESTARTED, SCOPE_GLOBAL, ELEMENT_MAIN)));
        assert!(matches!(classify_selector(VOLUME_SCALAR), Some(Notice::Volume)));
        assert!(matches!(classify_selector(MUTE), Some(Notice::Volume)));
        assert!(matches!(classify_selector(DEFAULT_OUTPUT), Some(Notice::Devices)));
        assert!(matches!(classify_selector(DEVICES), Some(Notice::Devices)));
        assert!(matches!(classify_selector(SERVICE_RESTARTED), Some(Notice::Restart)));
        // Current volume: the bound default and the system object count.
        assert!(is_current_volume(7, Some(7)));
        assert!(is_current_volume(SYSTEM, Some(7)));
        assert!(is_current_volume(SYSTEM, None));
    }

    #[test]
    fn watch_targets_reject_unwatched_selectors() {
        for selector in
            [u32::from_be_bytes(*b"vold"), u32::from_be_bytes(*b"span"), u32::from_be_bytes(*b"****")]
        {
            assert!(classify_selector(selector).is_none(), "{selector:#x} must stay unwatched");
        }
        // Stale volume: a device that is no longer bound must stay silent.
        assert!(!is_current_volume(5, Some(7)));
        assert!(!is_current_volume(5, None));
    }
}
