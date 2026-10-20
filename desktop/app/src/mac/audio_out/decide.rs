// SPDX-License-Identifier: GPL-3.0-or-later
//! PCM format and output-device choice, with no CoreAudio calls.
//!
//! Callers hand over 16-bit little-endian interleaved PCM (mirror stream
//! `pcm_s16le`, phone microphone frames). A selected device id is used only
//! when that id is an output; nothing here substitutes the default device.

use std::{ffi::c_void, io};

/// `kAudioFormatLinearPCM` (`CoreAudioBaseTypes.h`).
pub(super) const LINEAR_PCM: u32 = u32::from_be_bytes(*b"lpcm");
/// `kAudioFormatFlagIsSignedInteger`.
pub(super) const FLAG_SIGNED: u32 = 1 << 2;
/// `kAudioFormatFlagIsPacked`.
pub(super) const FLAG_PACKED: u32 = 1 << 3;
/// `kAudioFormatFlagIsBigEndian`. Left clear: the samples are little-endian.
#[cfg(test)]
pub(super) const FLAG_BIG_ENDIAN: u32 = 1 << 1;
/// `kAudioFormatFlagIsNonInterleaved`. Left clear: channels are interleaved.
#[cfg(test)]
pub(super) const FLAG_NON_INTERLEAVED: u32 = 1 << 5;

/// Longest device UID stored. The HAL reader uses one byte more for the NUL.
pub(super) const MAX_UID_BYTES: usize = 511;
/// Output streams read from one device, at most.
pub(super) const MAX_STREAM_BUFFERS: usize = 32;

/// `AudioBuffer` (`CoreAudioBaseTypes.h`), LP64.
#[repr(C)]
#[allow(dead_code)]
pub(super) struct AudioBuffer {
    channels: u32,
    byte_size: u32,
    data: *mut c_void,
}

/// `AudioBufferList` (`CoreAudioBaseTypes.h`). `buffers` is variable length;
/// the one slot here is the header the HAL lays the rest out after.
#[repr(C)]
#[allow(dead_code)]
pub(super) struct AudioBufferList {
    count: u32,
    pub(super) buffers: [AudioBuffer; 1],
}

const _: () = assert!(std::mem::size_of::<usize>() == 8, "macOS audio output is built for LP64");

/// 16-bit little-endian interleaved PCM, mono or stereo, 8 to 96 kHz.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct PcmFormat {
    pub(super) rate: u32,
    pub(super) channels: u16,
    pub(super) frame: usize,
    pub(super) flags: u32,
    pub(super) buffer_bytes: u32,
}

impl PcmFormat {
    pub(super) fn new(rate: u32, channels: u16) -> io::Result<PcmFormat> {
        if !(8_000..=96_000).contains(&rate) {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "pcm sample rate must be 8 to 96 kHz"));
        }
        if channels != 1 && channels != 2 {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "pcm must be mono or stereo"));
        }
        let frame = 2 * usize::from(channels);
        // 20 ms, a whole number of frames, and at most 16 KiB.
        let frames = (rate / 50).max(1);
        let buffer_bytes = frames.saturating_mul(frame as u32).clamp(frame as u32, 16 * 1024);
        Ok(PcmFormat { rate, channels, frame, flags: FLAG_SIGNED | FLAG_PACKED, buffer_bytes })
    }
}

/// One device the HAL listed, with how many output channels it has.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Listed {
    pub(super) uid: String,
    pub(super) output_channels: u32,
}

/// The device to open.
///
/// `None` is the default output, and only when that device is in `devices`
/// with output channels. `Some(id)` must name one of those outputs. A missing
/// id, an input-only id, or the literal `"default"` is an error.
pub(super) fn choose<'a>(
    requested: Option<&str>,
    devices: &'a [Listed],
    default_uid: Option<&str>,
) -> io::Result<&'a str> {
    let Some(id) = requested else {
        return match default_output(devices, default_uid) {
            Some(device) => Ok(device.uid.as_str()),
            None => Err(io::Error::new(io::ErrorKind::NotFound, "no default output device")),
        };
    };
    if id.is_empty() || id.len() > MAX_UID_BYTES || id.as_bytes().contains(&0) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "the selected output device id is not usable",
        ));
    }
    match devices.iter().find(|device| device.uid == id) {
        Some(device) if device.output_channels > 0 => Ok(device.uid.as_str()),
        Some(_) => Err(io::Error::new(
            io::ErrorKind::Unsupported,
            format!("selected device {id} has no output channels"),
        )),
        None => Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!("selected device {id} is not an output device"),
        )),
    }
}

fn default_output<'a>(devices: &'a [Listed], default_uid: Option<&str>) -> Option<&'a Listed> {
    let uid = default_uid?;
    devices.iter().find(|device| device.uid == uid && device.output_channels > 0)
}

/// Output-channel count in a HAL `AudioBufferList` for the output scope.
/// `Some(0)` is an input-only (or silent) device. `None` is a list this
/// process will not trust.
pub(super) fn output_channels(bytes: &[u8]) -> Option<u32> {
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
    let stride = std::mem::size_of::<AudioBuffer>();
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

pub(super) fn max_stream_bytes() -> usize {
    std::mem::offset_of!(AudioBufferList, buffers) + MAX_STREAM_BUFFERS * std::mem::size_of::<AudioBuffer>()
}
