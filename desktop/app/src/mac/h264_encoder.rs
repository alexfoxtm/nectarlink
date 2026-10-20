// SPDX-License-Identifier: GPL-3.0-or-later
//! H.264 encoding with macOS's own VideoToolbox, for showing this Mac's
//! screen on a phone: NV12 pixel buffers in, Annex B access units out right
//! away (docs/protocol/mirror.md §3). Constrained Baseline, no reordering,
//! a keyframe on request; every IDR carries its SPS and PPS first, as the
//! phone's decoder needs to start from that packet alone. The declarations
//! follow the macOS SDK headers (VTCompressionSession.h, VTCompressionProperties.h,
//! VTSession.h, VTErrors.h, CMFormatDescription.h, CMBlockBuffer.h,
//! CMSampleBuffer.h, CMTime.h, CVPixelBuffer.h, CFBase.h, CFArray.h,
//! CFDictionary.h, CFNumber.h).

#![allow(unsafe_code)]

use std::{
    ffi::{c_int, c_void},
    panic::{AssertUnwindSafe, catch_unwind},
    ptr::{self, NonNull},
    sync::{
        Mutex,
        atomic::{AtomicBool, Ordering},
    },
};

type OSStatus = i32;
type Boolean = u8;
type CFIndex = isize;
type CFTypeID = usize;
type CFTypeRef = *const c_void;
type CFStringRef = *const c_void;
type CMFormatDescriptionRef = *const c_void;
type CMBlockBufferRef = *mut c_void;
type CMSampleBufferRef = *mut c_void;
type VTCompressionSessionRef = *mut c_void;
type CVImageBufferRef = *mut c_void;

#[repr(C)]
#[derive(Clone, Copy)]
struct CMTime {
    value: i64,
    timescale: i32,
    flags: u32,
    epoch: i64,
}

type OutputCallback = unsafe extern "C" fn(
    output_callback_ref_con: *mut c_void,
    source_frame_ref_con: *mut c_void,
    status: OSStatus,
    info_flags: u32,
    sample_buffer: CMSampleBufferRef,
);

#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    // Callback tables, only ever passed by address.
    static kCFTypeDictionaryKeyCallBacks: u8;
    static kCFTypeDictionaryValueCallBacks: u8;
    static kCFBooleanTrue: CFTypeRef;
    static kCFBooleanFalse: CFTypeRef;

    fn CFRetain(cf: CFTypeRef) -> CFTypeRef;
    fn CFRelease(cf: CFTypeRef);
    fn CFGetTypeID(cf: CFTypeRef) -> CFTypeID;
    fn CFBooleanGetTypeID() -> CFTypeID;
    fn CFBooleanGetValue(boolean: CFTypeRef) -> Boolean;
    fn CFNumberCreate(allocator: CFTypeRef, the_type: CFIndex, value_ptr: *const c_void) -> CFTypeRef;
    fn CFArrayGetCount(array: CFTypeRef) -> CFIndex;
    fn CFArrayGetValueAtIndex(array: CFTypeRef, index: CFIndex) -> CFTypeRef;
    fn CFDictionaryGetTypeID() -> CFTypeID;
    fn CFDictionaryGetValue(dictionary: CFTypeRef, key: CFTypeRef) -> CFTypeRef;
    fn CFDictionaryCreate(
        allocator: CFTypeRef,
        keys: *const CFTypeRef,
        values: *const CFTypeRef,
        num_values: CFIndex,
        key_call_backs: *const c_void,
        value_call_backs: *const c_void,
    ) -> CFTypeRef;
}

#[link(name = "CoreMedia", kind = "framework")]
unsafe extern "C" {
    static kCMSampleAttachmentKey_NotSync: CFStringRef;

    fn CMSampleBufferGetNumSamples(sbuf: CMSampleBufferRef) -> CFIndex;
    fn CMSampleBufferGetDataBuffer(sbuf: CMSampleBufferRef) -> CMBlockBufferRef;
    fn CMSampleBufferGetFormatDescription(sbuf: CMSampleBufferRef) -> CMFormatDescriptionRef;
    fn CMSampleBufferGetSampleAttachmentsArray(
        sbuf: CMSampleBufferRef,
        create_if_necessary: Boolean,
    ) -> CFTypeRef;
    fn CMBlockBufferGetDataLength(buffer: CMBlockBufferRef) -> usize;
    fn CMBlockBufferCopyDataBytes(
        source: CMBlockBufferRef,
        offset_to_data: usize,
        data_length: usize,
        destination: *mut c_void,
    ) -> OSStatus;
    fn CMVideoFormatDescriptionGetH264ParameterSetAtIndex(
        video_desc: CMFormatDescriptionRef,
        parameter_set_index: usize,
        parameter_set_pointer_out: *mut *const u8,
        parameter_set_size_out: *mut usize,
        parameter_set_count_out: *mut usize,
        nal_unit_header_length_out: *mut c_int,
    ) -> OSStatus;
}

#[link(name = "CoreVideo", kind = "framework")]
unsafe extern "C" {
    fn CVPixelBufferGetTypeID() -> CFTypeID;
    fn CVPixelBufferGetWidth(pixel_buffer: CVImageBufferRef) -> usize;
    fn CVPixelBufferGetHeight(pixel_buffer: CVImageBufferRef) -> usize;
    fn CVPixelBufferGetPixelFormatType(pixel_buffer: CVImageBufferRef) -> u32;
    fn CVPixelBufferIsPlanar(pixel_buffer: CVImageBufferRef) -> Boolean;
    fn CVPixelBufferGetPlaneCount(pixel_buffer: CVImageBufferRef) -> usize;
}

#[link(name = "VideoToolbox", kind = "framework")]
unsafe extern "C" {
    static kVTVideoEncoderSpecification_EnableLowLatencyRateControl: CFStringRef;
    static kVTCompressionPropertyKey_RealTime: CFStringRef;
    static kVTCompressionPropertyKey_ProfileLevel: CFStringRef;
    static kVTCompressionPropertyKey_AllowFrameReordering: CFStringRef;
    static kVTCompressionPropertyKey_MaxKeyFrameInterval: CFStringRef;
    static kVTCompressionPropertyKey_MaxKeyFrameIntervalDuration: CFStringRef;
    static kVTCompressionPropertyKey_AverageBitRate: CFStringRef;
    static kVTCompressionPropertyKey_ExpectedFrameRate: CFStringRef;
    static kVTCompressionPropertyKey_PrioritizeEncodingSpeedOverQuality: CFStringRef;
    static kVTProfileLevel_H264_ConstrainedBaseline_AutoLevel: CFStringRef;
    static kVTProfileLevel_H264_Baseline_AutoLevel: CFStringRef;
    static kVTEncodeFrameOptionKey_ForceKeyFrame: CFStringRef;

    fn VTCompressionSessionCreate(
        allocator: CFTypeRef,
        width: i32,
        height: i32,
        codec_type: u32,
        encoder_specification: CFTypeRef,
        source_image_buffer_attributes: CFTypeRef,
        compressed_data_allocator: CFTypeRef,
        output_callback: Option<OutputCallback>,
        output_callback_ref_con: *mut c_void,
        compression_session_out: *mut VTCompressionSessionRef,
    ) -> OSStatus;
    fn VTCompressionSessionPrepareToEncodeFrames(session: VTCompressionSessionRef) -> OSStatus;
    fn VTCompressionSessionEncodeFrame(
        session: VTCompressionSessionRef,
        image_buffer: CVImageBufferRef,
        presentation_time_stamp: CMTime,
        duration: CMTime,
        frame_properties: CFTypeRef,
        source_frame_ref_con: *mut c_void,
        info_flags_out: *mut u32,
    ) -> OSStatus;
    fn VTCompressionSessionCompleteFrames(
        session: VTCompressionSessionRef,
        complete_until_presentation_time_stamp: CMTime,
    ) -> OSStatus;
    fn VTCompressionSessionInvalidate(session: VTCompressionSessionRef);
    fn VTSessionSetProperty(
        session: CFTypeRef,
        property_key: CFStringRef,
        property_value: CFTypeRef,
    ) -> OSStatus;
}

const CODEC_H264: u32 = u32::from_be_bytes(*b"avc1");
const NV12_VIDEO_RANGE: u32 = u32::from_be_bytes(*b"420v");
const NV12_FULL_RANGE: u32 = u32::from_be_bytes(*b"420f");
const CF_NUMBER_SINT32: CFIndex = 3;
const CF_NUMBER_FLOAT64: CFIndex = 6;
const CM_TIME_VALID: u32 = 1;
const CM_TIME_INVALID: CMTime = CMTime { value: 0, timescale: 0, flags: 0, epoch: 0 };
const ENCODE_INFO_FRAME_DROPPED: u32 = 1 << 1;
/// `kVTPropertyNotSupportedErr` (VTErrors.h).
const VT_PROPERTY_NOT_SUPPORTED: OSStatus = -12900;
/// `kVTAllocationFailedErr`.
const VT_ALLOCATION_FAILED: OSStatus = -12904;

/// The protocol's video packet limit and the header it counts against it:
/// `nectarlink_protocol`'s `MAX_VIDEO_PACKET` and `HEADER` (kind byte plus
/// `time_us`) in `video.rs`. Neither is reachable from this crate.
const MAX_VIDEO_PACKET: usize = 8 << 20;
const VIDEO_PACKET_HEADER: usize = 1 + 8;
/// The largest access unit that fits one video packet; a larger one is
/// refused here rather than silently skipped by the sender.
pub const MAX_ACCESS_UNIT: usize = MAX_VIDEO_PACKET - VIDEO_PACKET_HEADER;
/// NAL units in one access unit, at most.
const MAX_NAL_UNITS: usize = 512;
/// A parameter set larger than this is refused (real ones are tens of bytes).
const MAX_PARAMETER_SET: usize = 4096;
/// Parameter sets in one format description, at most.
const MAX_PARAMETER_SETS: usize = 16;
/// Access units one encode call keeps, at most (one frame normally gives one).
const MAX_PENDING: usize = 4;
/// Width and height bounds; both even, as NV12 halves them for chroma.
const MIN_DIMENSION: u32 = 16;
const MAX_DIMENSION: u32 = 4096;
const MAX_FPS: u32 = 120;
/// The bitrate range the Windows encoder also keeps to.
const MIN_BITRATE: u32 = 500_000;
const MAX_BITRATE: u32 = 20_000_000;
/// Seconds between keyframes, at most, so a lost packet heals.
const KEYFRAME_SECONDS: u32 = 2;

const START_CODE: [u8; 4] = [0, 0, 0, 1];
const SLICE: u8 = 1;
const SLICE_IDR: u8 = 5;
const SPS: u8 = 7;
const PPS: u8 = 8;

/// One reference to a Core Foundation object, released on drop.
#[derive(Debug)]
struct Owned(NonNull<c_void>);

impl Owned {
    /// Takes over the reference a Create or Copy call returned.
    fn new(object: *const c_void) -> Option<Owned> {
        NonNull::new(object.cast_mut()).map(Owned)
    }

    fn get(&self) -> *mut c_void {
        self.0.as_ptr()
    }
}

impl Drop for Owned {
    fn drop(&mut self) {
        // SAFETY: this holds exactly one reference.
        unsafe { CFRelease(self.0.as_ptr()) }
    }
}

/// A reference of its own to an NV12 CoreVideo pixel buffer of a size the
/// encoder takes, checked once so [`Encoder::encode`] is safe to call.
#[derive(Debug)]
pub struct PixelBuffer {
    buffer: Owned,
    width: u32,
    height: u32,
}

// SAFETY: CoreFoundation retain and release are thread-safe, and this type
// never touches the pixels; only VideoToolbox reads them, inside `encode`.
unsafe impl Send for PixelBuffer {}

impl PixelBuffer {
    /// Retains `buffer` once checked to be a two-plane NV12 (`420v` or `420f`)
    /// pixel buffer with even sides in the encoder's range. The caller keeps
    /// its own reference. Whoever produced the pixels must not change them
    /// while this lives (ScreenCaptureKit and buffer pools don't reuse a
    /// buffer still referenced).
    ///
    /// # Safety
    /// `buffer` is null or a live Core Foundation object for the duration of
    /// this call.
    pub unsafe fn retain(buffer: *mut c_void) -> Result<PixelBuffer, String> {
        if buffer.is_null() {
            return Err("no pixel buffer".into());
        }
        // SAFETY: a live CF object, as the caller promises; CoreVideo
        // queries run only once it's known to be a pixel buffer, and the
        // retained reference is ours.
        unsafe {
            if CFGetTypeID(buffer) != CVPixelBufferGetTypeID() {
                return Err("an image that isn't a CoreVideo pixel buffer".into());
            }
            let format = CVPixelBufferGetPixelFormatType(buffer);
            if !matches!(format, NV12_VIDEO_RANGE | NV12_FULL_RANGE) {
                let code = String::from_utf8_lossy(&format.to_be_bytes()).into_owned();
                return Err(format!("a '{code}' pixel buffer, not NV12"));
            }
            if CVPixelBufferIsPlanar(buffer) == 0 || CVPixelBufferGetPlaneCount(buffer) != 2 {
                return Err("an NV12 pixel buffer without two planes".into());
            }
            let (width, height) = (CVPixelBufferGetWidth(buffer), CVPixelBufferGetHeight(buffer));
            let (width, height) = (
                u32::try_from(width).map_err(|_| format!("a pixel buffer {width} wide"))?,
                u32::try_from(height).map_err(|_| format!("a pixel buffer {height} high"))?,
            );
            check_size(width, height)?;
            let buffer = Owned::new(CFRetain(buffer)).ok_or("CFRetain failed")?;
            Ok(PixelBuffer { buffer, width, height })
        }
    }

    pub fn size(&self) -> (u32, u32) {
        (self.width, self.height)
    }
}

fn check_size(width: u32, height: u32) -> Result<(), String> {
    let fits = |side: u32| (MIN_DIMENSION..=MAX_DIMENSION).contains(&side) && side.is_multiple_of(2);
    if fits(width) && fits(height) {
        Ok(())
    } else {
        Err(format!(
            "a {width}x{height} picture; H.264 here takes even sides of {MIN_DIMENSION} to {MAX_DIMENSION}"
        ))
    }
}

/// Where the output callback leaves what it encoded.
#[derive(Debug, Default)]
struct Sink {
    output: Mutex<Output>,
}

#[derive(Debug, Default)]
struct Output {
    /// `(is_keyframe, Annex B)` access units, at most [`MAX_PENDING`].
    units: Vec<(bool, Vec<u8>)>,
    error: Option<String>,
    dropped: u32,
}

impl Sink {
    fn record(&self, result: Result<Option<(bool, Vec<u8>)>, String>) {
        let mut output = self.output.lock().unwrap_or_else(|e| e.into_inner());
        match result {
            Ok(Some(unit)) if output.units.len() < MAX_PENDING => output.units.push(unit),
            Ok(Some(_)) => {
                output.error.get_or_insert_with(|| format!("more than {MAX_PENDING} encoded frames at once"));
            }
            Ok(None) => output.dropped += 1,
            Err(error) => {
                output.error.get_or_insert(error);
            }
        }
    }

    fn take(&self) -> Output {
        std::mem::take(&mut *self.output.lock().unwrap_or_else(|e| e.into_inner()))
    }
}

/// A VideoToolbox H.264 compression session encoding synchronously: each
/// frame's access unit is out before [`Encoder::encode`] returns.
#[derive(Debug)]
pub struct Encoder {
    session: Owned,
    /// Owned here (a leaked `Box`) and freed only after the session is
    /// invalidated, as the callback writes to it until then.
    sink: NonNull<Sink>,
    width: u32,
    height: u32,
    /// The next frame must be an IDR (the first one, or on request).
    force_keyframe: AtomicBool,
    /// Whether an IDR went out yet; nothing before one is any use.
    started: bool,
    last_time_us: Option<i64>,
}

impl Encoder {
    /// A low-latency encoder for `width` × `height` NV12 frames at up to
    /// `fps`, aiming at `bitrate` (kept to 0.5–20 Mbit/s).
    pub fn new(width: u32, height: u32, fps: u32, bitrate: u32) -> Result<Encoder, String> {
        check_size(width, height)?;
        if !(1..=MAX_FPS).contains(&fps) {
            return Err(format!("{fps} frames per second; H.264 here takes 1 to {MAX_FPS}"));
        }
        let bitrate = bitrate.clamp(MIN_BITRATE, MAX_BITRATE);
        // SAFETY: the dictionary retains the framework's own constants.
        let low_latency = unsafe {
            dictionary(&[(kVTVideoEncoderSpecification_EnableLowLatencyRateControl, kCFBooleanTrue)])?
        };
        // A low-latency session that can't be created or configured as the
        // stream needs gives way to the standard encoder, once. The inputs
        // are already checked, and out-of-memory is not retried.
        match Encoder::start(width, height, fps, bitrate, low_latency.get()) {
            Ok(encoder) => Ok(encoder),
            Err(StartError { status: Some(status), message }) if status != VT_ALLOCATION_FAILED => {
                tracing::info!(%message, "no low-latency VideoToolbox H.264 encoder; using the standard one");
                Encoder::start(width, height, fps, bitrate, ptr::null()).map_err(|e| e.message)
            }
            Err(error) => Err(error.message),
        }
    }

    /// One attempt: a session with its own sink, configured and prepared.
    /// On failure everything it made is released (dropping the encoder
    /// invalidates the session before freeing the sink).
    fn start(
        width: u32,
        height: u32,
        fps: u32,
        bitrate: u32,
        specification: CFTypeRef,
    ) -> Result<Encoder, StartError> {
        let sink = NonNull::from(Box::leak(Box::<Sink>::default()));
        let session = match create(width, height, specification, sink) {
            Ok(session) => session,
            Err(status) => {
                // SAFETY: no session refers to the sink.
                drop(unsafe { Box::from_raw(sink.as_ptr()) });
                return Err(StartError::vt(status, "starting the H.264 encoder"));
            }
        };
        // From here, dropping `encoder` invalidates the session and frees the sink.
        let encoder = Encoder {
            session,
            sink,
            width,
            height,
            force_keyframe: AtomicBool::new(true),
            started: false,
            last_time_us: None,
        };
        encoder.configure(fps, bitrate)?;
        Ok(encoder)
    }

    fn configure(&self, fps: u32, bitrate: u32) -> Result<(), StartError> {
        // SAFETY: a live session; the keys and values are the frameworks'
        // own constants or numbers alive across each call.
        unsafe {
            // B-frames would hold pictures back and reorder them; never.
            self.require(kVTCompressionPropertyKey_AllowFrameReordering, kCFBooleanFalse, "no reordering")?;
            if let Err(error) = self.require(
                kVTCompressionPropertyKey_ProfileLevel,
                kVTProfileLevel_H264_ConstrainedBaseline_AutoLevel,
                "Constrained Baseline",
            ) {
                tracing::info!(error = %error.message, "falling back to the H.264 Baseline profile");
                self.require(
                    kVTCompressionPropertyKey_ProfileLevel,
                    kVTProfileLevel_H264_Baseline_AutoLevel,
                    "Baseline",
                )?;
            }
            let rate = number_i32(bitrate as i32)?;
            self.require(kVTCompressionPropertyKey_AverageBitRate, rate.get(), "the bitrate")?;
            self.prefer(kVTCompressionPropertyKey_RealTime, kCFBooleanTrue, "real time")?;
            let interval = number_i32((fps * KEYFRAME_SECONDS) as i32)?;
            self.prefer(
                kVTCompressionPropertyKey_MaxKeyFrameInterval,
                interval.get(),
                "the keyframe interval",
            )?;
            let seconds = number_f64(f64::from(KEYFRAME_SECONDS))?;
            self.prefer(
                kVTCompressionPropertyKey_MaxKeyFrameIntervalDuration,
                seconds.get(),
                "the keyframe interval in seconds",
            )?;
            let rate = number_i32(fps as i32)?;
            self.prefer(kVTCompressionPropertyKey_ExpectedFrameRate, rate.get(), "the frame rate")?;
            self.prefer(
                kVTCompressionPropertyKey_PrioritizeEncodingSpeedOverQuality,
                kCFBooleanTrue,
                "speed over quality",
            )?;
            StartError::check(
                VTCompressionSessionPrepareToEncodeFrames(self.session.get()),
                "preparing the H.264 encoder",
            )
        }
    }

    /// Asks for the next frame to be an IDR. Always possible here.
    pub fn request_keyframe(&self) -> bool {
        self.force_keyframe.store(true, Ordering::SeqCst);
        true
    }

    /// Changes the target bitrate in place (kept to 0.5–20 Mbit/s). False
    /// when the encoder won't.
    pub fn set_bitrate(&self, bitrate: u32) -> bool {
        let Ok(rate) = number_i32(bitrate.clamp(MIN_BITRATE, MAX_BITRATE) as i32) else {
            return false;
        };
        // SAFETY: a live session, the framework's key, a live number.
        let status = unsafe {
            VTSessionSetProperty(self.session.get(), kVTCompressionPropertyKey_AverageBitRate, rate.get())
        };
        if status != 0 {
            tracing::warn!(error = %describe(status, "changing the H.264 bitrate"), "bitrate unchanged");
        }
        status == 0
    }

    /// Encodes one frame shown at `time_us` (each later than the last) into
    /// Annex B access units `(is_keyframe, bytes)`; empty when VideoToolbox
    /// dropped the frame. Keyframes start with the SPS and PPS.
    pub fn encode(&mut self, frame: &PixelBuffer, time_us: u64) -> Result<Vec<(bool, Vec<u8>)>, String> {
        if frame.size() != (self.width, self.height) {
            let (w, h) = frame.size();
            return Err(format!("a {w}x{h} frame for a {}x{} encoder", self.width, self.height));
        }
        let time = i64::try_from(time_us).map_err(|_| format!("a frame time of {time_us} µs"))?;
        if let Some(last) = self.last_time_us.filter(|&last| time <= last) {
            return Err(format!("a frame at {time} µs, not after the last one at {last} µs"));
        }
        self.last_time_us = Some(time);
        let force = self.force_keyframe.swap(false, Ordering::SeqCst);
        let result = self.encode_frame(frame, time, force);
        let keyframe = result.as_ref().is_ok_and(|units| units.iter().any(|&(keyframe, _)| keyframe));
        if force && !keyframe {
            // Asked for and not given: ask again with the next frame.
            self.force_keyframe.store(true, Ordering::SeqCst);
            if result.as_ref().is_ok_and(|units| !units.is_empty()) {
                tracing::warn!("VideoToolbox encoded a requested keyframe as a non-IDR frame");
            }
        }
        let units = result?;
        if !self.started && !units.is_empty() && !keyframe {
            return Err("the H.264 encoder's first frame isn't an IDR".into());
        }
        self.started |= keyframe;
        Ok(units)
    }

    fn encode_frame(
        &self,
        frame: &PixelBuffer,
        time: i64,
        force: bool,
    ) -> Result<Vec<(bool, Vec<u8>)>, String> {
        // SAFETY: the dictionary retains the framework's own constants.
        let options = if force {
            Some(unsafe { dictionary(&[(kVTEncodeFrameOptionKey_ForceKeyFrame, kCFBooleanTrue)])? })
        } else {
            None
        };
        let stale = self.sink().take();
        if !stale.units.is_empty() || stale.error.is_some() {
            tracing::debug!(units = stale.units.len(), "dropping H.264 output left from an earlier frame");
        }
        let presentation = CMTime { value: time, timescale: 1_000_000, flags: CM_TIME_VALID, epoch: 0 };
        let mut info = 0;
        // SAFETY: the session, the retained pixel buffer and the options are
        // live; VideoToolbox retains the buffer for as long as it needs it.
        let status = unsafe {
            VTCompressionSessionEncodeFrame(
                self.session.get(),
                frame.buffer.get(),
                presentation,
                CM_TIME_INVALID,
                options.as_ref().map_or(ptr::null(), |o| o.get().cast_const()),
                ptr::null_mut(),
                &mut info,
            )
        };
        check(status, "encoding H.264")?;
        // SAFETY: a live session. Once this returns, every frame up to this
        // one has been through the callback.
        let status = unsafe { VTCompressionSessionCompleteFrames(self.session.get(), presentation) };
        let output = self.sink().take();
        check(status, "finishing an H.264 frame")?;
        if let Some(error) = output.error {
            return Err(error);
        }
        if output.units.is_empty() && (output.dropped > 0 || info & ENCODE_INFO_FRAME_DROPPED != 0) {
            tracing::debug!("VideoToolbox dropped a frame");
        }
        Ok(output.units)
    }

    /// Sets a property the stream can't do without.
    ///
    /// # Safety
    /// `key` and `value` are live CF objects.
    unsafe fn require(&self, key: CFStringRef, value: CFTypeRef, what: &str) -> Result<(), StartError> {
        // SAFETY: as the caller promises, on a live session.
        StartError::check(
            unsafe { VTSessionSetProperty(self.session.get(), key, value) },
            &format!("setting {what}"),
        )
    }

    /// Sets a property the stream is better with; one this encoder doesn't
    /// support is noted and skipped, any other failure is an error (the SDK
    /// declares every key used here read/write, so read-only is not expected).
    ///
    /// # Safety
    /// `key` and `value` are live CF objects.
    unsafe fn prefer(&self, key: CFStringRef, value: CFTypeRef, what: &str) -> Result<(), StartError> {
        // SAFETY: as the caller promises, on a live session.
        match unsafe { VTSessionSetProperty(self.session.get(), key, value) } {
            0 => Ok(()),
            VT_PROPERTY_NOT_SUPPORTED => {
                tracing::debug!("the H.264 encoder doesn't support {what}");
                Ok(())
            }
            status => Err(StartError::vt(status, &format!("setting {what}"))),
        }
    }

    fn sink(&self) -> &Sink {
        // SAFETY: freed only in `drop`.
        unsafe { self.sink.as_ref() }
    }
}

impl Drop for Encoder {
    fn drop(&mut self) {
        // SAFETY: once invalidated the session calls back no more, so the
        // sink can go; the session's reference is released after this.
        unsafe {
            let status = VTCompressionSessionCompleteFrames(self.session.get(), CM_TIME_INVALID);
            if status != 0 {
                tracing::debug!(error = %describe(status, "finishing H.264 frames"), "closing the encoder");
            }
            VTCompressionSessionInvalidate(self.session.get());
            drop(Box::from_raw(self.sink.as_ptr()));
        }
    }
}

/// Why an encoder attempt failed: a VideoToolbox status (the low-latency
/// attempt gives way to the standard one on it), or none (out of memory in
/// CoreFoundation, not retried).
#[derive(Debug)]
struct StartError {
    status: Option<OSStatus>,
    message: String,
}

impl StartError {
    fn vt(status: OSStatus, doing: &str) -> StartError {
        StartError { status: Some(status), message: describe(status, doing) }
    }

    fn check(status: OSStatus, doing: &str) -> Result<(), StartError> {
        if status == 0 { Ok(()) } else { Err(StartError::vt(status, doing)) }
    }
}

impl From<String> for StartError {
    fn from(message: String) -> StartError {
        StartError { status: None, message }
    }
}

/// A compression session writing to `sink`, or the OSStatus it failed with.
fn create(width: u32, height: u32, specification: CFTypeRef, sink: NonNull<Sink>) -> Result<Owned, OSStatus> {
    let mut out = ptr::null_mut();
    // SAFETY: all arguments are live (the specification may be null); the
    // session comes back with one reference, ours. Sides are at most 4096.
    let status = unsafe {
        VTCompressionSessionCreate(
            ptr::null(),
            width as i32,
            height as i32,
            CODEC_H264,
            specification,
            ptr::null(),
            ptr::null(),
            Some(on_output),
            sink.as_ptr().cast(),
            &mut out,
        )
    };
    match (status, Owned::new(out)) {
        (0, Some(session)) => Ok(session),
        (status, session) => {
            if let Some(session) = session {
                // SAFETY: a session that failed to start must not call back
                // into the sink, which may be freed after this.
                unsafe { VTCompressionSessionInvalidate(session.get()) };
            }
            Err(if status == 0 { VT_ALLOCATION_FAILED } else { status })
        }
    }
}

/// The session's output callback: turns the sample into Annex B, or notes
/// the error.
unsafe extern "C" fn on_output(
    sink: *mut c_void,
    _frame: *mut c_void,
    status: OSStatus,
    info: u32,
    sample: CMSampleBufferRef,
) {
    // SAFETY: the session's sink, alive until the session is invalidated.
    let sink = unsafe { &*sink.cast::<Sink>() };
    // A panic must not unwind into VideoToolbox.
    let converted = catch_unwind(AssertUnwindSafe(|| {
        if status != 0 {
            Err(describe(status, "encoding H.264"))
        } else if info & ENCODE_INFO_FRAME_DROPPED != 0 {
            Ok(None)
        } else if sample.is_null() {
            Err("VideoToolbox encoded no sample".into())
        } else {
            // SAFETY: VideoToolbox keeps `sample` alive during the callback.
            unsafe { annexb_sample(sample) }.map(Some)
        }
    }));
    let result = converted.unwrap_or_else(|_| Err("the H.264 encoder's output handler panicked".into()));
    let _ = catch_unwind(AssertUnwindSafe(|| sink.record(result)));
}

/// An encoded sample as `(is_keyframe, Annex B)`.
///
/// # Safety
/// `sample` is a live CoreMedia sample buffer.
unsafe fn annexb_sample(sample: CMSampleBufferRef) -> Result<(bool, Vec<u8>), String> {
    // SAFETY: CoreMedia queries on a live sample. What they return without
    // "Create" or "Copy" is borrowed from it; the parameter sets point into
    // its format description, read before this returns.
    unsafe {
        let count = CMSampleBufferGetNumSamples(sample);
        if count != 1 {
            return Err(format!("an encoded sample buffer of {count} samples"));
        }
        let sync = is_sync(sample);
        let format = CMSampleBufferGetFormatDescription(sample);
        if format.is_null() {
            return Err("an encoded sample without a format".into());
        }
        let (mut sets, mut length_size): (usize, c_int) = (0, 0);
        let status = CMVideoFormatDescriptionGetH264ParameterSetAtIndex(
            format,
            0,
            ptr::null_mut(),
            ptr::null_mut(),
            &mut sets,
            &mut length_size,
        );
        check(status, "reading the encoder's parameter sets")?;
        if !(2..=MAX_PARAMETER_SETS).contains(&sets) {
            return Err(format!("{sets} parameter sets from the encoder"));
        }
        let mut parameter_sets = Vec::with_capacity(sets);
        for index in 0..sets {
            let (mut set, mut size): (*const u8, usize) = (ptr::null(), 0);
            let status = CMVideoFormatDescriptionGetH264ParameterSetAtIndex(
                format,
                index,
                &mut set,
                &mut size,
                ptr::null_mut(),
                ptr::null_mut(),
            );
            check(status, "reading an encoder parameter set")?;
            if set.is_null() || !(1..=MAX_PARAMETER_SET).contains(&size) {
                return Err(format!("an encoder parameter set of {size} bytes"));
            }
            parameter_sets.push(std::slice::from_raw_parts(set, size));
        }
        let block = CMSampleBufferGetDataBuffer(sample);
        if block.is_null() {
            return Err("an encoded sample without data".into());
        }
        let len = CMBlockBufferGetDataLength(block);
        if !(1..=MAX_ACCESS_UNIT).contains(&len) {
            return Err(format!("an encoded sample of {len} bytes"));
        }
        let mut avcc = vec![0u8; len];
        check(
            CMBlockBufferCopyDataBytes(block, 0, len, avcc.as_mut_ptr().cast()),
            "reading an encoded sample",
        )?;
        let length_size =
            usize::try_from(length_size).map_err(|_| format!("NAL lengths of {length_size} bytes"))?;
        let (annexb, idr) = avcc_to_annexb(&avcc, length_size, &parameter_sets)?;
        if sync && !idr {
            return Err("a sync sample without an IDR slice".into());
        }
        Ok((idr, annexb))
    }
}

/// Whether the sample is a sync sample: no `NotSync` attachment, or a false one.
///
/// # Safety
/// `sample` is a live CoreMedia sample buffer.
unsafe fn is_sync(sample: CMSampleBufferRef) -> bool {
    // SAFETY: borrowed attachments of a live sample; the value's type is
    // checked before it's read as a boolean.
    unsafe {
        let attachments = CMSampleBufferGetSampleAttachmentsArray(sample, 0);
        if attachments.is_null() || CFArrayGetCount(attachments) < 1 {
            return true;
        }
        let first = CFArrayGetValueAtIndex(attachments, 0);
        if first.is_null() || CFGetTypeID(first) != CFDictionaryGetTypeID() {
            return true;
        }
        let not_sync = CFDictionaryGetValue(first, kCMSampleAttachmentKey_NotSync);
        not_sync.is_null()
            || (CFGetTypeID(not_sync) == CFBooleanGetTypeID() && CFBooleanGetValue(not_sync) == 0)
    }
}

/// An encoded sample (NAL units after `length_size`-byte big-endian lengths)
/// as Annex B, each NAL unit after a 4-byte start code. With an IDR slice,
/// the SPS and then the PPS from `parameter_sets` come first (other units in
/// there, such as SEI, are left out), so a receiver can start its decoder
/// from this packet alone. Returns the bytes and whether there's an IDR.
pub(crate) fn avcc_to_annexb(
    avcc: &[u8],
    length_size: usize,
    parameter_sets: &[&[u8]],
) -> Result<(Vec<u8>, bool), String> {
    if !matches!(length_size, 1 | 2 | 4) {
        return Err(format!("NAL lengths of {length_size} bytes"));
    }
    if avcc.is_empty() || avcc.len() > MAX_ACCESS_UNIT {
        return Err(format!("an encoded sample of {} bytes", avcc.len()));
    }
    let mut nal_units = Vec::new();
    let mut rest = avcc;
    while !rest.is_empty() {
        if nal_units.len() == MAX_NAL_UNITS {
            return Err(format!("more than {MAX_NAL_UNITS} NAL units in a sample"));
        }
        let (length, after) = rest.split_at_checked(length_size).ok_or("a sample cut inside a NAL length")?;
        let len = length.iter().fold(0usize, |len, &byte| (len << 8) | usize::from(byte));
        if len == 0 {
            return Err("an empty NAL unit".into());
        }
        let (nal, after) = after
            .split_at_checked(len)
            .ok_or_else(|| format!("a {len}-byte NAL unit past the sample's end"))?;
        if nal[0] & 0x80 != 0 {
            return Err("a NAL unit with its forbidden bit set".into());
        }
        nal_units.push(nal);
        rest = after;
    }
    fn kind(nal: &[u8]) -> u8 {
        nal[0] & 0x1f
    }
    let idr = nal_units.iter().any(|nal| kind(nal) == SLICE_IDR);
    if !nal_units.iter().any(|nal| matches!(kind(nal), SLICE | SLICE_IDR)) {
        return Err("an encoded sample without a slice".into());
    }
    let mut header: Vec<&[u8]> = Vec::new();
    if idr {
        for set in parameter_sets {
            if set.len() < 2 || set.len() > MAX_PARAMETER_SET || set[0] & 0x80 != 0 {
                return Err(format!("a parameter set of {} bytes", set.len()));
            }
        }
        let of = |wanted: u8| parameter_sets.iter().copied().filter(move |set| kind(set) == wanted);
        if of(SPS).next().is_none() || of(PPS).next().is_none() {
            return Err("an IDR without both an SPS and a PPS".into());
        }
        header.extend(of(SPS).chain(of(PPS)));
    }
    let total = header
        .iter()
        .chain(&nal_units)
        .try_fold(0usize, |total, nal| total.checked_add(START_CODE.len() + nal.len()))
        .filter(|&total| total <= MAX_ACCESS_UNIT)
        .ok_or("an access unit over the packet limit")?;
    let mut out = Vec::with_capacity(total);
    for nal in header.iter().chain(&nal_units) {
        out.extend_from_slice(&START_CODE);
        out.extend_from_slice(nal);
    }
    Ok((out, idr))
}

/// A CFDictionary of `pairs`, which it retains.
///
/// # Safety
/// Every key and value is a live CF object.
unsafe fn dictionary(pairs: &[(CFTypeRef, CFTypeRef)]) -> Result<Owned, String> {
    let keys: Vec<CFTypeRef> = pairs.iter().map(|&(key, _)| key).collect();
    let values: Vec<CFTypeRef> = pairs.iter().map(|&(_, value)| value).collect();
    // SAFETY: `keys` and `values` hold `pairs.len()` live objects; the
    // callback tables are CoreFoundation's own.
    unsafe {
        Owned::new(CFDictionaryCreate(
            ptr::null(),
            keys.as_ptr(),
            values.as_ptr(),
            pairs.len() as CFIndex,
            (&raw const kCFTypeDictionaryKeyCallBacks).cast(),
            (&raw const kCFTypeDictionaryValueCallBacks).cast(),
        ))
    }
    .ok_or_else(|| "CFDictionaryCreate failed".into())
}

fn number_i32(value: i32) -> Result<Owned, String> {
    // SAFETY: CoreFoundation copies the value; the number is ours.
    Owned::new(unsafe { CFNumberCreate(ptr::null(), CF_NUMBER_SINT32, (&raw const value).cast()) })
        .ok_or_else(|| "CFNumberCreate failed".into())
}

fn number_f64(value: f64) -> Result<Owned, String> {
    // SAFETY: as above.
    Owned::new(unsafe { CFNumberCreate(ptr::null(), CF_NUMBER_FLOAT64, (&raw const value).cast()) })
        .ok_or_else(|| "CFNumberCreate failed".into())
}

fn check(status: OSStatus, doing: &str) -> Result<(), String> {
    if status == 0 { Ok(()) } else { Err(describe(status, doing)) }
}

/// An OSStatus from VideoToolbox or CoreMedia, named where VTErrors.h names it.
fn describe(status: OSStatus, doing: &str) -> String {
    let name = match status {
        VT_PROPERTY_NOT_SUPPORTED => "kVTPropertyNotSupportedErr",
        -12901 => "kVTPropertyReadOnlyErr",
        -12902 => "kVTParameterErr",
        -12903 => "kVTInvalidSessionErr",
        VT_ALLOCATION_FAILED => "kVTAllocationFailedErr",
        -12905 => "kVTPixelTransferNotSupportedErr",
        -12907 => "kVTCouldNotCreateInstanceErr",
        -12908 => "kVTCouldNotFindVideoEncoderErr",
        -12912 => "kVTVideoEncoderMalfunctionErr",
        -12915 => "kVTVideoEncoderNotAvailableNowErr",
        -17691 => "kVTSessionMalfunctionErr",
        -17693 => "kVTVideoEncoderNeedsRosettaErr",
        _ => "",
    };
    if name.is_empty() {
        format!("{doing} failed ({status})")
    } else {
        format!("{doing} failed: {name} ({status})")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SPS_NAL: &[u8] = &[0x67, 0x42, 0xc0, 0x1f, 0xda];
    const PPS_NAL: &[u8] = &[0x68, 0xce, 0x3c, 0x80];
    const SEI_NAL: &[u8] = &[0x06, 0x05, 0x01, 0x80];

    #[test]
    fn idr_sample_becomes_annexb_with_sps_and_pps_before_the_slice() {
        let idr = [0x65, 0x88, 0x84, 0x00, 0x33];
        let mut avcc = (idr.len() as u32).to_be_bytes().to_vec();
        avcc.extend_from_slice(&idr);
        // The format description's sets may hold SEI and list the PPS first.
        let (bytes, keyframe) = avcc_to_annexb(&avcc, 4, &[PPS_NAL, SEI_NAL, SPS_NAL]).unwrap();
        assert!(keyframe);
        let start: &[u8] = &START_CODE;
        assert_eq!(bytes, [start, SPS_NAL, start, PPS_NAL, start, &idr[..]].concat());

        // A P slice after a 2-byte length: no parameter sets.
        let (bytes, keyframe) = avcc_to_annexb(&[0, 3, 0x41, 0x9a, 0x02], 2, &[SPS_NAL, PPS_NAL]).unwrap();
        assert!(!keyframe);
        assert_eq!(bytes, [0, 0, 0, 1, 0x41, 0x9a, 0x02]);

        // The largest access unit, which still fits one video packet with its header.
        let (bytes, _) = avcc_to_annexb(&p_slice_filling(MAX_ACCESS_UNIT), 4, &[]).unwrap();
        assert_eq!(bytes.len(), MAX_ACCESS_UNIT);
    }

    /// A 4-byte-length P slice that makes a `total`-byte sample.
    fn p_slice_filling(total: usize) -> Vec<u8> {
        let mut avcc = vec![0x41; total];
        avcc[..4].copy_from_slice(&((total - 4) as u32).to_be_bytes());
        avcc
    }

    #[test]
    fn malformed_samples_and_missing_parameter_sets_are_refused() {
        let sets: &[&[u8]] = &[SPS_NAL, PPS_NAL];
        // A length past the end, a zero length, a cut length, an odd length size.
        assert!(avcc_to_annexb(&[0, 0, 0, 9, 0x65, 0x88], 4, sets).is_err());
        assert!(avcc_to_annexb(&[0, 0, 0, 0, 0x65], 4, sets).is_err());
        assert!(avcc_to_annexb(&[0, 0, 0], 4, sets).is_err());
        assert!(avcc_to_annexb(&[0, 0, 2, 0x65, 0x88], 3, sets).is_err());
        // No slice, and an IDR without a PPS to start from.
        assert!(avcc_to_annexb(&[0, 4, 0x06, 0x05, 0x01, 0x80], 2, sets).is_err());
        assert!(avcc_to_annexb(&[0, 2, 0x65, 0x88], 2, &[SPS_NAL]).is_err());
        // Past the packet limit: a longer sample, or a full-size IDR once its SPS and PPS go first.
        assert!(avcc_to_annexb(&p_slice_filling(MAX_ACCESS_UNIT + 1), 4, &[]).is_err());
        let mut idr = p_slice_filling(MAX_ACCESS_UNIT);
        idr[4] = 0x65;
        assert!(avcc_to_annexb(&idr, 4, sets).is_err());
    }
}
