// SPDX-License-Identifier: GPL-3.0-or-later
//! AudioQueue playback and CoreAudio device lookup.
//!
//! Declarations follow the macOS SDK headers: AudioQueue.h,
//! AudioHardware.h, AudioHardwareBase.h, CoreAudioBaseTypes.h, CFString.h,
//! CFBase.h. Playback only: there is no input queue and no microphone.

#![allow(unsafe_code)]

use std::{
    ffi::c_void,
    io,
    mem::size_of,
    panic::{AssertUnwindSafe, catch_unwind},
    ptr::{self, NonNull},
    slice,
    sync::{
        Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc::{Receiver, RecvTimeoutError, SyncSender},
    },
    time::Duration,
};

use super::{
    decide::{self, Listed, PcmFormat, output_channels},
    jitter::{Budget, Jitter},
};

type OSStatus = i32;
type Boolean = u8;
type CFIndex = isize;
type CFStringEncoding = u32;
type AudioObjectID = u32;
type AudioQueueRef = *mut c_void;
type AudioQueueBufferRef = *mut AudioQueueBuffer;

type AudioQueueOutputCallback = unsafe extern "C" fn(*mut c_void, AudioQueueRef, AudioQueueBufferRef);

/// `AudioQueueBuffer` (`AudioQueue.h`). On LP64 the pointers are 8-byte aligned.
#[repr(C)]
struct AudioQueueBuffer {
    audio_data_bytes_capacity: u32,
    audio_data: *mut c_void,
    audio_data_byte_size: u32,
    user_data: *mut c_void,
    packet_description_capacity: u32,
    packet_descriptions: *mut c_void,
    packet_description_count: u32,
}

/// `AudioStreamBasicDescription` (`CoreAudioBaseTypes.h`).
#[repr(C)]
struct AudioStreamBasicDescription {
    sample_rate: f64,
    format_id: u32,
    format_flags: u32,
    bytes_per_packet: u32,
    frames_per_packet: u32,
    bytes_per_frame: u32,
    channels_per_frame: u32,
    bits_per_channel: u32,
    reserved: u32,
}

/// `AudioObjectPropertyAddress` (`AudioHardwareBase.h`).
#[repr(C)]
struct AudioObjectPropertyAddress {
    selector: u32,
    scope: u32,
    element: u32,
}

const _: () = assert!(size_of::<usize>() != 8 || size_of::<AudioQueueBuffer>() == 56);
const _: () = assert!(size_of::<AudioStreamBasicDescription>() == 40);
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
/// `kAudioDevicePropertyDeviceUID` (`'uid '`).
const DEVICE_UID: u32 = u32::from_be_bytes(*b"uid ");
/// `kAudioDevicePropertyStreamConfiguration` (`'slay'`).
const STREAM_CONFIGURATION: u32 = u32::from_be_bytes(*b"slay");
/// `kAudioHardwareUnknownPropertyError` (`'who?'`).
const UNKNOWN_PROPERTY: i32 = i32::from_be_bytes(*b"who?");
/// `kAudioQueueProperty_CurrentDevice` (`'aqcd'`). Value is `CFStringRef`.
const CURRENT_DEVICE: u32 = u32::from_be_bytes(*b"aqcd");

/// Devices read from the HAL, at most. More than this and selection fails
/// closed instead of ignoring the tail.
const MAX_DEVICES: usize = 64;
/// Buffers kept inside AudioQueue. Each is about 20 ms.
const QUEUE_BUFFERS: usize = 3;

#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    fn CFRelease(cf: *const c_void);
    fn CFStringCreateWithBytes(
        allocator: *const c_void,
        bytes: *const u8,
        num_bytes: CFIndex,
        encoding: CFStringEncoding,
        is_external_representation: Boolean,
    ) -> *const c_void;
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

#[link(name = "AudioToolbox", kind = "framework")]
unsafe extern "C" {
    fn AudioQueueNewOutput(
        format: *const AudioStreamBasicDescription,
        callback: AudioQueueOutputCallback,
        user_data: *mut c_void,
        callback_run_loop: *mut c_void,
        callback_run_loop_mode: *const c_void,
        flags: u32,
        out_queue: *mut AudioQueueRef,
    ) -> OSStatus;
    fn AudioQueueDispose(queue: AudioQueueRef, immediate: Boolean) -> OSStatus;
    fn AudioQueueAllocateBuffer(
        queue: AudioQueueRef,
        byte_size: u32,
        out_buffer: *mut AudioQueueBufferRef,
    ) -> OSStatus;
    fn AudioQueueEnqueueBuffer(
        queue: AudioQueueRef,
        buffer: AudioQueueBufferRef,
        packet_descriptions: u32,
        packet_descs: *const c_void,
    ) -> OSStatus;
    fn AudioQueueStart(queue: AudioQueueRef, start_time: *const c_void) -> OSStatus;
    fn AudioQueueStop(queue: AudioQueueRef, immediate: Boolean) -> OSStatus;
    fn AudioQueueSetProperty(queue: AudioQueueRef, property: u32, data: *const c_void, size: u32)
    -> OSStatus;
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

/// The output device `start_on` asked for.
///
/// `None` resolves to the default output. `Some(id)` must already be an
/// output device UID; this does not substitute the default.
fn resolve(requested: Option<&str>) -> io::Result<String> {
    if let Some(id) = requested
        && (id.is_empty() || id.len() > decide::MAX_UID_BYTES || id.as_bytes().contains(&0))
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "the selected output device id is not usable",
        ));
    }
    let (devices, default_uid) = inventory()?;
    decide::choose(requested, &devices, default_uid.as_deref()).map(str::to_owned)
}

fn inventory() -> io::Result<(Vec<Listed>, Option<String>)> {
    let ids = device_ids()?;
    let mut devices = Vec::with_capacity(ids.len());
    for id in ids {
        let Ok(uid) = device_uid(id) else { continue };
        if uid.is_empty() || uid.len() > decide::MAX_UID_BYTES {
            continue;
        }
        let Ok(channels) = device_output_channels(id) else { continue };
        devices.push(Listed { uid, output_channels: channels });
    }
    let default_uid = match default_output_id()? {
        Some(id) => device_uid(id).ok().filter(|uid| !uid.is_empty()),
        None => None,
    };
    Ok((devices, default_uid))
}

fn device_ids() -> io::Result<Vec<AudioObjectID>> {
    let address = address(DEVICES, SCOPE_GLOBAL);
    let mut size = 0u32;
    // SAFETY: the system object is always id 1; the address is a local.
    let status = unsafe { AudioObjectGetPropertyDataSize(SYSTEM, &address, 0, ptr::null(), &mut size) };
    ok(status, "listing audio devices")?;
    if size == 0 {
        return Ok(Vec::new());
    }
    let count = (size as usize) / size_of::<AudioObjectID>();
    if !(size as usize).is_multiple_of(size_of::<AudioObjectID>()) || count > MAX_DEVICES {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "the audio device list is not usable"));
    }
    let mut ids = vec![0u32; count];
    // SAFETY: `ids` has room for `size` bytes, and the HAL writes device ids.
    let status = unsafe {
        AudioObjectGetPropertyData(SYSTEM, &address, 0, ptr::null(), &mut size, ids.as_mut_ptr().cast())
    };
    ok(status, "listing audio devices")?;
    let count = (size as usize) / size_of::<AudioObjectID>();
    ids.truncate(count.min(MAX_DEVICES));
    Ok(ids)
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
    cf_to_string(owned.get()).ok_or_else(|| io::Error::other("an output device id is not readable"))
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
    if size as usize > decide::max_stream_bytes() {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "an output stream layout is too large"));
    }
    let mut bytes = vec![0u8; size as usize];
    // SAFETY: `bytes` is `size` long. The list's audio pointers are null in
    // this property; only the channel counts are read afterwards.
    let status = unsafe {
        AudioObjectGetPropertyData(id, &address, 0, ptr::null(), &mut size, bytes.as_mut_ptr().cast())
    };
    ok(status, "reading an output stream layout")?;
    let len = (size as usize).min(bytes.len());
    output_channels(&bytes[..len])
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "an output stream layout is unreadable"))
}

fn cf_string(text: &str) -> io::Result<Owned> {
    // SAFETY: `text` is live UTF-8. A created string comes back with one
    // reference, ours. External-representation is false.
    let created = unsafe {
        CFStringCreateWithBytes(ptr::null(), text.as_ptr(), text.len() as CFIndex, CF_STRING_UTF8, 0)
    };
    Owned::new(created).ok_or_else(|| io::Error::other("couldn't name the output device"))
}

fn cf_to_string(string: *const c_void) -> Option<String> {
    if string.is_null() {
        return None;
    }
    let mut buf = [0u8; decide::MAX_UID_BYTES + 1];
    // SAFETY: `string` is a live CFString. The buffer holds the UTF-8 and a NUL.
    let ok = unsafe { CFStringGetCString(string, buf.as_mut_ptr(), buf.len() as CFIndex, CF_STRING_UTF8) };
    if ok == 0 {
        return None;
    }
    let len = buf.iter().position(|byte| *byte == 0)?;
    String::from_utf8(buf[..len].to_vec()).ok()
}

struct Shared {
    jitter: Mutex<Jitter>,
    failed: AtomicBool,
    stopping: AtomicBool,
    frame: usize,
    /// Bytes allocated for each queue buffer. The callback never writes more.
    buffer_bytes: usize,
}

impl Shared {
    fn render(&self, data: *mut u8, capacity: usize) -> Option<u32> {
        if self.stopping.load(Ordering::Acquire) {
            return None;
        }
        let frame = self.frame;
        let capacity = capacity.min(self.buffer_bytes);
        if data.is_null() || frame == 0 || capacity < frame {
            self.failed.store(true, Ordering::Release);
            return None;
        }
        let count = capacity / frame * frame;
        // SAFETY: `data` is a queue buffer this player allocated. The slice is
        // no longer than that allocation, and it ends before the function returns.
        let out = unsafe { slice::from_raw_parts_mut(data, count) };
        match self.jitter.lock() {
            Ok(mut jitter) => jitter.fill(out),
            Err(_) => {
                out.fill(0);
                self.failed.store(true, Ordering::Release);
                return None;
            }
        }
        if self.stopping.load(Ordering::Acquire) || self.failed.load(Ordering::Acquire) {
            return None;
        }
        Some(count as u32)
    }
}

/// Playback queue. Dropped on the thread that created it, and only after
/// `AudioQueueStop` / `AudioQueueDispose` have returned, which is when
/// AudioQueue.h says callbacks have stopped.
struct Engine {
    queue: AudioQueueRef,
    shared: Option<NonNull<Shared>>,
    /// The UID string handed to `kAudioQueueProperty_CurrentDevice`. Kept
    /// until the queue is disposed: AudioQueue.h states no ownership rule
    /// for that property, so it is never read back or released early.
    device: Option<Owned>,
}

impl Engine {
    fn open(uid: &str, format: PcmFormat) -> io::Result<Engine> {
        // `NonNull::from` ends the leaked reference immediately, so the
        // callback's later borrow does not alias a live `&mut`.
        let shared = NonNull::from(Box::leak(Box::new(Shared {
            jitter: Mutex::new(Jitter::new(format.rate, format.channels)),
            failed: AtomicBool::new(false),
            stopping: AtomicBool::new(false),
            frame: format.frame,
            buffer_bytes: format.buffer_bytes as usize,
        })));
        let mut queue: AudioQueueRef = ptr::null_mut();
        let description = stream_description(format);
        // SAFETY: `description` and `shared` outlive the queue. The callback
        // run loop is null, so AudioQueue calls back on its own thread. Flags
        // are reserved and must be 0 (AudioQueue.h).
        let status = unsafe {
            AudioQueueNewOutput(
                &description,
                on_buffer,
                shared.as_ptr().cast::<c_void>(),
                ptr::null_mut(),
                ptr::null(),
                0,
                &mut queue,
            )
        };
        if status != 0 || queue.is_null() {
            // The out pointer is only a queue when the call succeeds. Shared
            // state was not retained.
            // SAFETY: NewOutput did not keep `shared`.
            drop(unsafe { Box::from_raw(shared.as_ptr()) });
            return Err(if status == 0 {
                io::Error::other("creating the audio queue failed")
            } else {
                status_error(status, "creating the audio queue")
            });
        }
        let mut engine = Engine { queue, shared: Some(shared), device: None };
        engine.arm(uid, format)?;
        tracing::debug!(
            rate = format.rate,
            channels = format.channels,
            device = uid,
            "playing a phone's sound"
        );
        Ok(engine)
    }

    fn arm(&mut self, uid: &str, format: PcmFormat) -> io::Result<()> {
        self.select_device(uid)?;
        for _ in 0..QUEUE_BUFFERS {
            self.enqueue_silence(format.buffer_bytes)?;
        }
        // SAFETY: the queue is live, the device is already selected, and
        // three silent buffers are queued. Null starts now.
        ok(unsafe { AudioQueueStart(self.queue, ptr::null()) }, "starting playback")
    }

    /// Points the queue at `uid`, which `resolve` already proved is an
    /// output device. A refused SetProperty is an error and the queue is
    /// disposed unstarted, so it never plays on the device it was created on.
    fn select_device(&mut self, uid: &str) -> io::Result<()> {
        let name = cf_string(uid)?;
        let name_ref = name.get();
        self.device = Some(name);
        // SAFETY: the data is the address of a live CFStringRef, which is the
        // value `kAudioQueueProperty_CurrentDevice` takes. The string is held
        // in `Engine::device` until after the queue is disposed, whether or
        // not the queue accepts it. Set happens before `AudioQueueStart`.
        let status = unsafe {
            AudioQueueSetProperty(
                self.queue,
                CURRENT_DEVICE,
                (&raw const name_ref).cast::<c_void>(),
                size_of::<*const c_void>() as u32,
            )
        };
        ok(status, "selecting the output device")
    }

    fn enqueue_silence(&self, bytes: u32) -> io::Result<()> {
        let mut buffer: AudioQueueBufferRef = ptr::null_mut();
        // SAFETY: the queue is live and not started yet. `bytes` is the
        // capacity AudioQueue.h says is then fixed.
        let status = unsafe { AudioQueueAllocateBuffer(self.queue, bytes, &mut buffer) };
        ok(status, "allocating a playback buffer")?;
        if buffer.is_null() {
            return Err(io::Error::other("a playback buffer was not allocated"));
        }
        // SAFETY: the buffer was just allocated. Capacity is at least `bytes`,
        // and the audio pointer is the queue's storage for that many bytes.
        unsafe {
            let capacity = (*buffer).audio_data_bytes_capacity;
            let data = (*buffer).audio_data;
            if data.is_null() || capacity < bytes {
                return Err(io::Error::other("a playback buffer is smaller than requested"));
            }
            ptr::write_bytes(data.cast::<u8>(), 0, bytes as usize);
            (*buffer).audio_data_byte_size = bytes;
        }
        // SAFETY: PCM has one frame per packet, so packet descriptions are
        // null and the count is 0 (AudioQueue.h).
        ok(unsafe { AudioQueueEnqueueBuffer(self.queue, buffer, 0, ptr::null()) }, "queueing silence")
    }

    fn run(self, packets: Receiver<Vec<u8>>, budget: &Budget) {
        loop {
            if self.failed() {
                tracing::warn!("the sound stopped");
                break;
            }
            match packets.recv_timeout(Duration::from_millis(200)) {
                Ok(data) => {
                    budget.give(data.len());
                    self.accept(&data);
                }
                Err(RecvTimeoutError::Timeout) => continue,
                Err(RecvTimeoutError::Disconnected) => break,
            }
            while let Ok(data) = packets.try_recv() {
                budget.give(data.len());
                self.accept(&data);
            }
        }
        self.log_jitter();
    }

    fn accept(&self, data: &[u8]) {
        let Some(shared) = self.shared else { return };
        // SAFETY: `shared` is alive until this engine drops, which is after `run`.
        let shared = unsafe { shared.as_ref() };
        match shared.jitter.lock() {
            Ok(mut jitter) => jitter.push(data),
            Err(_) => shared.failed.store(true, Ordering::Release),
        }
    }

    fn failed(&self) -> bool {
        // SAFETY: `shared` is alive until this engine drops.
        self.shared.is_some_and(|shared| unsafe { shared.as_ref().failed.load(Ordering::Acquire) })
    }

    fn log_jitter(&self) {
        let Some(shared) = self.shared else { return };
        // SAFETY: still owned here; `Drop` has not disposed the queue.
        let shared = unsafe { shared.as_ref() };
        let Ok(jitter) = shared.jitter.lock() else { return };
        tracing::debug!(
            stalls = jitter.stalls(),
            dropped_ms = jitter.trimmed_ms(),
            "the phone's sound stopped"
        );
    }
}

impl Drop for Engine {
    fn drop(&mut self) {
        if let Some(shared) = self.shared {
            // SAFETY: the flag is read by the callback, which `Stop` waits
            // for when it is called from this thread (AudioQueue.h).
            unsafe { shared.as_ref().stopping.store(true, Ordering::Release) };
        }
        if !self.queue.is_null() {
            // SAFETY: called from the owner thread, never from the callback.
            // Immediate stop finishes in-flight callbacks before it returns.
            // Immediate dispose then promises no further callbacks. The shared
            // state is freed only after that.
            unsafe {
                let _ = AudioQueueStop(self.queue, 1);
                let _ = AudioQueueDispose(self.queue, 1);
            }
            self.queue = ptr::null_mut();
        }
        self.device.take();
        if let Some(shared) = self.shared.take() {
            // SAFETY: the queue has disposed, so the callback no longer runs.
            unsafe { drop(Box::from_raw(shared.as_ptr())) };
        }
    }
}

fn stream_description(format: PcmFormat) -> AudioStreamBasicDescription {
    let channels = u32::from(format.channels);
    let bytes_per_frame = format.frame as u32;
    AudioStreamBasicDescription {
        sample_rate: f64::from(format.rate),
        format_id: decide::LINEAR_PCM,
        format_flags: format.flags,
        bytes_per_packet: bytes_per_frame,
        frames_per_packet: 1,
        bytes_per_frame,
        channels_per_frame: channels,
        bits_per_channel: 16,
        reserved: 0,
    }
}

fn deliver(shared: &Shared, queue: AudioQueueRef, buffer: AudioQueueBufferRef) {
    if shared.stopping.load(Ordering::Acquire) {
        return;
    }
    if queue.is_null() || buffer.is_null() {
        shared.failed.store(true, Ordering::Release);
        return;
    }
    // SAFETY: AudioQueue keeps `buffer` alive for this callback. The audio
    // pointer and the size field are not aliased at the same time.
    let (data, capacity) = unsafe {
        let view = &*buffer;
        (view.audio_data.cast::<u8>(), view.audio_data_bytes_capacity)
    };
    let Some(bytes) = shared.render(data, capacity as usize) else {
        return;
    };
    // SAFETY: same live buffer; `render` no longer holds its audio slice.
    unsafe { (*buffer).audio_data_byte_size = bytes };
    if shared.stopping.load(Ordering::Acquire) || shared.failed.load(Ordering::Acquire) {
        return;
    }
    // SAFETY: the buffer was filled up to its capacity, packet descriptions
    // are unused for linear PCM, and this is the callback thread so it must
    // not stop or dispose the queue.
    let status = unsafe { AudioQueueEnqueueBuffer(queue, buffer, 0, ptr::null()) };
    if status != 0 {
        shared.failed.store(true, Ordering::Release);
    }
}

/// # Safety
///
/// `user` is null, or it is the `Shared` allocation owned by the queue's
/// `Engine`. That allocation stays alive until `AudioQueueDispose` on the
/// owner thread has returned. `queue` and `buffer` are the queue's own
/// arguments for this callback.
unsafe extern "C" fn on_buffer(user: *mut c_void, queue: AudioQueueRef, buffer: AudioQueueBufferRef) {
    if user.is_null() {
        return;
    }
    // SAFETY: `user` is the leaked `Shared`, freed only after dispose returns.
    let shared = unsafe { &*user.cast::<Shared>() };
    let panicked = catch_unwind(AssertUnwindSafe(|| deliver(shared, queue, buffer))).is_err();
    if panicked {
        shared.failed.store(true, Ordering::Release);
    }
}

/// Finds the device, opens the queue, tells `ready` whether that worked,
/// then plays `packets` until the sender is dropped or the queue fails.
///
/// When `start_on` stopped waiting, `ready` is closed and a queue that
/// opened late is stopped and disposed here, on this thread, before its
/// callback state is freed.
pub(super) fn play(
    requested: Option<&str>,
    format: PcmFormat,
    packets: Receiver<Vec<u8>>,
    budget: &Budget,
    ready: SyncSender<io::Result<()>>,
) {
    let opened = resolve(requested).and_then(|uid| Engine::open(&uid, format));
    match opened {
        Ok(engine) => {
            if ready.send(Ok(())).is_err() {
                return;
            }
            engine.run(packets, budget);
        }
        Err(error) => {
            let _ = ready.send(Err(error));
        }
    }
}
