// SPDX-License-Identifier: GPL-3.0-or-later
//! Read-only CoreAudio output-device enumeration for the Deck.
//!
//! Declarations follow the macOS SDK headers: AudioHardware.h,
//! AudioHardwareBase.h, CoreAudioBaseTypes.h, CFString.h, CFBase.h — the
//! same headers cited by `super::super::audio_out::hal` (see
//! `desktop/app/src/mac/audio_out/hal.rs:4-6`). Selectors and scopes reuse
//! that file's proven values: `kAudioObjectSystemObject` (1),
//! `kAudioHardwarePropertyDevices` (`'dev#'`),
//! `kAudioHardwarePropertyDefaultOutputDevice` (`'dOut'`),
//! `kAudioDevicePropertyDeviceUID` (`'uid '`),
//! `kAudioDevicePropertyStreamConfiguration` (`'slay'`),
//! `kAudioObjectPropertyScopeGlobal` (`'glob'`),
//! `kAudioObjectPropertyScopeOutput` (`'outp'`),
//! `kAudioObjectPropertyElementMain` (0),
//! `kAudioHardwareUnknownPropertyError` (`'who?'`). The friendly name uses
//! `kAudioObjectPropertyName` (`'lnam'`, `AudioHardwareBase.h`, global scope,
//! `CFStringRef` the caller releases) — the only selector this module adds
//! beyond `hal.rs`.
//!
//! Read-only: only `AudioObjectGetPropertyDataSize` /
//! `AudioObjectGetPropertyData` are linked. This module never links or calls
//! `AudioObjectSetPropertyData`, `AudioQueueSetProperty`, or any default-
//! device write, and it implements no output-switch, capability enable, or
//! capture/microphone path. Failures yield an empty (or partial) list, like
//! `win::audio::output_devices`; input-only devices (zero output channels)
//! are ignored; the default output is marked `is_default: true` and sorted
//! first, matching Windows (`desktop/app/src/win/audio.rs:176-204`).
//!
//! The ids are real device UIDs, not the literal `"default"` this module
//! replaces. One caller is affected: `phone_mic::outputs()` now hands the
//! phone-mic player a real UID (or the chosen one) instead of `"default"`,
//! which `audio_out::decide::choose` refuses, so phone-mic playback on macOS
//! could not start before and can now start. That is latent today: a Mac
//! does not advertise `mic.listen`, so no microphone stream reaches it. It
//! changes app state only — no system default is switched and no capability
//! is enabled.

#![allow(unsafe_code)]

use std::{
    ffi::c_void,
    io,
    mem::size_of,
    ptr::{self, NonNull},
};

type OSStatus = i32;
type Boolean = u8;
type CFIndex = isize;
type CFStringEncoding = u32;
type AudioObjectID = u32;

/// `AudioObjectPropertyAddress` (`AudioHardwareBase.h`).
#[repr(C)]
struct AudioObjectPropertyAddress {
    selector: u32,
    scope: u32,
    element: u32,
}

/// `AudioBuffer` (`CoreAudioBaseTypes.h`), LP64.
#[repr(C)]
#[allow(dead_code)]
struct AudioBuffer {
    channels: u32,
    byte_size: u32,
    data: *mut c_void,
}

/// `AudioBufferList` (`CoreAudioBaseTypes.h`). `buffers` is variable length;
/// the one slot here is the header the HAL lays the rest out after.
#[repr(C)]
#[allow(dead_code)]
struct AudioBufferList {
    count: u32,
    buffers: [AudioBuffer; 1],
}

const _: () = assert!(size_of::<usize>() == 8, "macOS audio devices are listed on LP64");
const _: () = assert!(size_of::<AudioObjectPropertyAddress>() == 12);

/// `kCFStringEncodingUTF8`.
const CF_STRING_UTF8: CFStringEncoding = 0x0800_0100;
/// `kAudioObjectSystemObject`.
const SYSTEM: AudioObjectID = 1;
/// `kAudioObjectUnknown`.
const UNKNOWN: AudioObjectID = 0;
/// `kAudioObjectPropertyScopeGlobal` (`'glob'`).
const SCOPE_GLOBAL: u32 = u32::from_be_bytes(*b"glob");
/// `kAudioObjectPropertyScopeOutput` (`'outp'`).
const SCOPE_OUTPUT: u32 = u32::from_be_bytes(*b"outp");
/// `kAudioObjectPropertyElementMain`.
const ELEMENT_MAIN: u32 = 0;
/// `kAudioHardwarePropertyDevices` (`'dev#'`).
const DEVICES: u32 = u32::from_be_bytes(*b"dev#");
/// `kAudioHardwarePropertyDefaultOutputDevice` (`'dOut'`).
const DEFAULT_OUTPUT: u32 = u32::from_be_bytes(*b"dOut");
/// `kAudioObjectPropertyName` (`'lnam'`, `AudioHardwareBase.h`).
const OBJECT_NAME: u32 = u32::from_be_bytes(*b"lnam");
/// `kAudioDevicePropertyDeviceUID` (`'uid '`).
const DEVICE_UID: u32 = u32::from_be_bytes(*b"uid ");
/// `kAudioDevicePropertyStreamConfiguration` (`'slay'`).
const STREAM_CONFIGURATION: u32 = u32::from_be_bytes(*b"slay");
/// `kAudioHardwareUnknownPropertyError` (`'who?'`).
const UNKNOWN_PROPERTY: i32 = i32::from_be_bytes(*b"who?");

/// HAL device ids read in one query, at most. Matches `hal::MAX_DEVICES` (64).
const MAX_HAL_DEVICES: usize = 64;
/// Deck endpoints reported, at most (`deck::MAX_OUTPUT_DEVICES` = 32).
const MAX_DECK_DEVICES: usize = 32;
/// Deck endpoint id bound (`deck::MAX_DEVICE_ID_BYTES` = 256).
const MAX_ID_BYTES: usize = 256;
/// Deck endpoint name bound (`deck::MAX_DEVICE_NAME_BYTES` = 128).
const MAX_NAME_BYTES: usize = 128;
/// Longest UID read from the HAL (`decide::MAX_UID_BYTES` = 511; one more for NUL).
const MAX_UID_BYTES: usize = 511;
/// Output streams read from one device, at most (`decide::MAX_STREAM_BUFFERS`).
const MAX_STREAM_BUFFERS: usize = 32;
/// How many times the device list query is retried when the device set moves.
const LIST_ATTEMPTS: u32 = 2;
/// Fallback name when the HAL has no readable name (mirrors the Windows
/// `"Speakers"` fallback in `win::audio::output_devices`).
const FALLBACK_NAME: &str = "Mac Output";

#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    fn CFRelease(cf: *const c_void);
    fn CFStringGetCString(
        the_string: *const c_void,
        buffer: *mut u8,
        buffer_size: CFIndex,
        encoding: CFStringEncoding,
    ) -> Boolean;
}

#[link(name = "CoreAudio", kind = "framework")]
unsafe extern "C" {
    fn AudioObjectGetPropertyDataSize(
        object: AudioObjectID,
        address: *const AudioObjectPropertyAddress,
        qualifier_size: u32,
        qualifier: *const c_void,
        out_size: *mut u32,
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

/// One reference to a Core Foundation object, released on drop.
struct Owned(NonNull<c_void>);

impl Owned {
    fn new(object: *const c_void) -> Option<Owned> {
        NonNull::new(object.cast_mut()).map(Owned)
    }

    fn get(&self) -> *const c_void {
        self.0.as_ptr()
    }
}

impl Drop for Owned {
    fn drop(&mut self) {
        // SAFETY: this holds exactly one reference.
        unsafe { CFRelease(self.0.as_ptr()) }
    }
}

fn address(selector: u32, scope: u32) -> AudioObjectPropertyAddress {
    AudioObjectPropertyAddress { selector, scope, element: ELEMENT_MAIN }
}

fn status_error(status: i32, action: &str) -> io::Error {
    io::Error::other(format!("{action} failed ({status})"))
}

fn ok(status: i32, action: &str) -> io::Result<()> {
    if status == 0 { Ok(()) } else { Err(status_error(status, action)) }
}

/// Whether `uid` can be reported as a Deck endpoint id: trimmed non-empty,
/// within `deck::MAX_DEVICE_ID_BYTES`, and free of control/NUL bytes (so it
/// always passes `AudioOutputDevice::is_valid` on the id side).
fn valid_uid(uid: &str) -> bool {
    // Exact match against `trimmed` rejects leading/trailing whitespace;
    // control/NUL bytes and overlong ids would fail `AudioOutputDevice::is_valid`.
    let trimmed = uid.trim();
    !trimmed.is_empty()
        && uid == trimmed
        && uid.len() <= MAX_ID_BYTES
        && !uid.as_bytes().contains(&0)
        && !uid.chars().any(char::is_control)
}

/// Sanitizes a HAL friendly name into a Deck endpoint name: trims, drops
/// control chars, truncates to `deck::MAX_DEVICE_NAME_BYTES` at a char
/// boundary, and trims again. Returns `None` when nothing displayable remains.
fn sanitized_name(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    let kept: String = trimmed.chars().filter(|c| !c.is_control()).collect();
    let kept = kept.trim();
    if kept.is_empty() {
        return None;
    }
    let mut end = kept.len().min(MAX_NAME_BYTES);
    while end > 0 && !kept.is_char_boundary(end) {
        end -= 1;
    }
    let name = kept[..end].trim().to_owned();
    (!name.is_empty()).then_some(name)
}

/// Sorts Deck endpoints the Windows way: the default first, then by name.
fn sort_devices(devices: &mut [nectarlink_core::AudioOutputDevice]) {
    devices.sort_by(|a, b| b.is_default.cmp(&a.is_default).then_with(|| a.name.cmp(&b.name)));
}

/// Output-channel count in a HAL `AudioBufferList` for the output scope.
/// `Some(0)` is an input-only (or silent) device. `None` is a list this
/// process will not trust. Same bounds as `decide::output_channels`.
fn output_channel_count(bytes: &[u8]) -> Option<u32> {
    if bytes.len() < 4 {
        return None;
    }
    let count = u32::from_ne_bytes(bytes[..4].try_into().ok()?) as usize;
    if count > MAX_STREAM_BUFFERS {
        return None;
    }
    if count == 0 {
        return Some(0);
    }
    let start = std::mem::offset_of!(AudioBufferList, buffers);
    let stride = size_of::<AudioBuffer>();
    let need = start.checked_add(count.checked_mul(stride)?)?;
    if bytes.len() < need {
        return None;
    }
    let mut channels = 0u32;
    for index in 0..count {
        let offset = start + index * stride;
        let got = u32::from_ne_bytes(bytes[offset..offset + 4].try_into().ok()?);
        channels = channels.saturating_add(got);
    }
    Some(channels)
}

fn max_stream_bytes() -> usize {
    std::mem::offset_of!(AudioBufferList, buffers) + MAX_STREAM_BUFFERS * size_of::<AudioBuffer>()
}

fn device_ids() -> io::Result<Vec<AudioObjectID>> {
    let address = address(DEVICES, SCOPE_GLOBAL);
    let mut last_error: Option<io::Error> = None;
    for _ in 0..LIST_ATTEMPTS {
        let mut size = 0u32;
        // SAFETY: the system object is always id 1; the address is a local.
        let status = unsafe { AudioObjectGetPropertyDataSize(SYSTEM, &address, 0, ptr::null(), &mut size) };
        if status != 0 {
            last_error = Some(status_error(status, "listing audio devices"));
            continue;
        }
        if size == 0 {
            return Ok(Vec::new());
        }
        let count = (size as usize) / size_of::<AudioObjectID>();
        if !(size as usize).is_multiple_of(size_of::<AudioObjectID>()) || count > MAX_HAL_DEVICES {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "the audio device list is not usable"));
        }
        let mut ids = vec![0u32; count];
        // SAFETY: `ids` has room for `size` bytes, and the HAL writes device ids.
        let status = unsafe {
            AudioObjectGetPropertyData(SYSTEM, &address, 0, ptr::null(), &mut size, ids.as_mut_ptr().cast())
        };
        if status != 0 {
            // The device set may have moved between the size query and the
            // read; one retry re-queries the size. Bounded by LIST_ATTEMPTS.
            last_error = Some(status_error(status, "listing audio devices"));
            continue;
        }
        let count = (size as usize) / size_of::<AudioObjectID>();
        ids.truncate(count.min(MAX_HAL_DEVICES));
        return Ok(ids);
    }
    Err(last_error.unwrap_or_else(|| io::Error::other("listing audio devices failed")))
}

fn default_output_id() -> io::Result<Option<AudioObjectID>> {
    let address = address(DEFAULT_OUTPUT, SCOPE_GLOBAL);
    let mut id = UNKNOWN;
    let mut size = size_of::<AudioObjectID>() as u32;
    // SAFETY: `id` is the property's size. Zero means there is no default.
    let status = unsafe {
        AudioObjectGetPropertyData(
            SYSTEM,
            &address,
            0,
            ptr::null(),
            &mut size,
            (&raw mut id).cast::<c_void>(),
        )
    };
    ok(status, "reading the default output device")?;
    Ok((id != UNKNOWN).then_some(id))
}

fn cf_to_string(string: *const c_void, buf: &mut [u8]) -> Option<String> {
    if string.is_null() || buf.is_empty() {
        return None;
    }
    // SAFETY: `string` is a live CFString. The buffer holds UTF-8 plus a NUL.
    let ok = unsafe { CFStringGetCString(string, buf.as_mut_ptr(), buf.len() as CFIndex, CF_STRING_UTF8) };
    if ok == 0 {
        return None;
    }
    let len = buf.iter().position(|byte| *byte == 0)?;
    String::from_utf8(buf[..len].to_vec()).ok()
}

fn device_uid(id: AudioObjectID) -> io::Result<String> {
    let address = address(DEVICE_UID, SCOPE_GLOBAL);
    let mut string: *const c_void = ptr::null();
    let mut size = size_of::<*const c_void>() as u32;
    // SAFETY: AudioHardwareBase.h says the caller releases this CFString.
    let status = unsafe {
        AudioObjectGetPropertyData(
            id,
            &address,
            0,
            ptr::null(),
            &mut size,
            (&raw mut string).cast::<c_void>(),
        )
    };
    ok(status, "reading an output device id")?;
    let owned = Owned::new(string).ok_or_else(|| io::Error::other("an output device has no id"))?;
    let mut buf = vec![0u8; MAX_UID_BYTES + 1];
    cf_to_string(owned.get(), &mut buf).ok_or_else(|| io::Error::other("an output device id is not readable"))
}

fn device_name(id: AudioObjectID) -> io::Result<String> {
    let address = address(OBJECT_NAME, SCOPE_GLOBAL);
    let mut string: *const c_void = ptr::null();
    let mut size = size_of::<*const c_void>() as u32;
    // SAFETY: `kAudioObjectPropertyName` returns a CFString the caller
    // releases (AudioHardwareBase.h). Read-only; no device state changes.
    let status = unsafe {
        AudioObjectGetPropertyData(
            id,
            &address,
            0,
            ptr::null(),
            &mut size,
            (&raw mut string).cast::<c_void>(),
        )
    };
    ok(status, "reading an output device name")?;
    let owned = Owned::new(string).ok_or_else(|| io::Error::other("an output device has no name"))?;
    // `CFStringGetCString` refuses a string that does not fit; it never
    // truncates. Read into a larger bounded buffer so a long HAL name is
    // readable, and let `sanitized_name` cut it to the display length.
    let mut buf = vec![0u8; 4 * MAX_NAME_BYTES + 1];
    cf_to_string(owned.get(), &mut buf)
        .ok_or_else(|| io::Error::other("an output device name is not readable"))
}

fn device_output_channels(id: AudioObjectID) -> io::Result<u32> {
    let address = address(STREAM_CONFIGURATION, SCOPE_OUTPUT);
    let mut size = 0u32;
    // SAFETY: output-scope stream configuration. An unknown property means
    // this device has no output side.
    let status = unsafe { AudioObjectGetPropertyDataSize(id, &address, 0, ptr::null(), &mut size) };
    if status == UNKNOWN_PROPERTY {
        return Ok(0);
    }
    ok(status, "reading an output stream layout")?;
    if size == 0 {
        return Ok(0);
    }
    if size as usize > max_stream_bytes() {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "an output stream layout is too large"));
    }
    let mut bytes = vec![0u8; size as usize];
    // SAFETY: `bytes` is `size` long. Only channel counts are read afterwards.
    let status = unsafe {
        AudioObjectGetPropertyData(id, &address, 0, ptr::null(), &mut size, bytes.as_mut_ptr().cast())
    };
    ok(status, "reading an output stream layout")?;
    let len = (size as usize).min(bytes.len());
    output_channel_count(&bytes[..len])
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "an output stream layout is unreadable"))
}

/// Lists active audio render (output) endpoints on this Mac, marking the
/// current default output (`is_default: true`) and sorting the default first.
///
/// Read-only: no device is opened, switched, enabled, or written. Input-only
/// devices (zero output channels) are ignored. Failures yield an empty (or
/// partial) list, like `win::audio::output_devices`; the shape stays
/// `Vec<nectarlink_core::AudioOutputDevice>` capped at 32.
pub fn output_devices() -> Vec<nectarlink_core::AudioOutputDevice> {
    let ids = match device_ids() {
        Ok(ids) => ids,
        Err(_) => return Vec::new(),
    };
    let mut out = Vec::with_capacity(ids.len().min(MAX_DECK_DEVICES));
    for id in ids {
        let Ok(uid) = device_uid(id) else { continue };
        if !valid_uid(&uid) {
            continue;
        }
        let Ok(channels) = device_output_channels(id) else { continue };
        if channels == 0 {
            continue;
        }
        let name = match device_name(id) {
            Ok(raw) => sanitized_name(&raw).unwrap_or_else(|| FALLBACK_NAME.into()),
            Err(_) => FALLBACK_NAME.into(),
        };
        out.push(nectarlink_core::AudioOutputDevice { id: uid, name, is_default: false });
    }
    // The default is marked wherever it is: the list is cut to the Deck's
    // size after the sort, which keeps the default first. The loop keeps
    // every device the HAL offered (at most `MAX_HAL_DEVICES`), so a Mac
    // with more outputs than the Deck shows still marks its default.
    if let Ok(Some(default_id)) = default_output_id()
        && let Ok(default_uid) = device_uid(default_id)
        && let Some(device) = out.iter_mut().find(|device| device.id == default_uid)
    {
        device.is_default = true;
    }
    sort_devices(&mut out);
    out.truncate(MAX_DECK_DEVICES);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn device(id: &str, name: &str, is_default: bool) -> nectarlink_core::AudioOutputDevice {
        nectarlink_core::AudioOutputDevice { id: id.into(), name: name.into(), is_default }
    }

    #[test]
    fn sorts_default_first_and_sanitizes_display_names() {
        let mut devices = vec![
            device("b", "  B\u{0007}oom  ", false),
            device("a", "Zed", true),
            device("c", "Apple", false),
        ];
        for d in &mut devices {
            d.name = sanitized_name(&d.name).unwrap();
        }
        sort_devices(&mut devices);
        assert_eq!(devices[0].id, "a");
        assert!(devices[0].is_default);
        assert_eq!(devices[1].name, "Apple");
        assert_eq!(devices[2].name, "Boom");
        assert!(valid_uid("AppleHDA:123"));
        let long_name = "n".repeat(MAX_NAME_BYTES + 10);
        assert_eq!(sanitized_name(&long_name).unwrap().len(), MAX_NAME_BYTES);
    }

    #[test]
    fn refuses_bad_ids_and_untrusted_channel_lists() {
        assert!(!valid_uid(""));
        assert!(!valid_uid("  "));
        assert!(!valid_uid("a\u{0000}b"));
        assert!(!valid_uid("a\u{0001}b"));
        assert!(!valid_uid(&"x".repeat(MAX_ID_BYTES + 1)));
        assert!(valid_uid("UID-1"));
        assert_eq!(sanitized_name("   "), None);
        assert_eq!(sanitized_name("\u{0007}"), None);
        let start = std::mem::offset_of!(AudioBufferList, buffers);
        let stride = std::mem::size_of::<AudioBuffer>();
        let mut two = vec![0u8; start + 2 * stride];
        two[..4].copy_from_slice(&2u32.to_ne_bytes());
        two[start..start + 4].copy_from_slice(&1u32.to_ne_bytes());
        two[start + stride..start + stride + 4].copy_from_slice(&2u32.to_ne_bytes());
        assert_eq!(output_channel_count(&two), Some(3));
        assert_eq!(output_channel_count(&[]), None);
        assert_eq!(output_channel_count(&0u32.to_ne_bytes()), Some(0));
    }
}
