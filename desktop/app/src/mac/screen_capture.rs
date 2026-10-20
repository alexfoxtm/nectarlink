// SPDX-License-Identifier: GPL-3.0-or-later
//! ScreenCaptureKit display capture for one mirror session.
//!
//! The owner chooses a display in `SCContentSharingPicker`. This file does not
//! call `CGRequestScreenCaptureAccess` or `CGPreflightScreenCaptureAccess`.
//! The screen-share caller stays gated on native acceptance. Nothing here
//! starts capture at process start.

#![allow(unsafe_code)]

mod decide;

use std::ffi::c_void;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::ptr::{self, NonNull};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex, MutexGuard, Once, Weak};
use std::thread;
use std::time::{Duration, Instant};

use block2::RcBlock;
use decide::{
    ATTEMPT_TO_STOP, CF_NUMBER_SINT64, CfStringRef, Event, MAX_EDGE, MIN_EDGE, OUTPUT_SCREEN,
    PICKER_SINGLE_DISPLAY, PROTOCOLS, Phase, QUEUE_DEPTH, admit_frame, apply, clamp_fps, display_pixels,
    frame_interval, keep_frame, target_dimensions,
};
use dispatch2::{DispatchQueue, DispatchQueueAttr, DispatchRetained};
use objc2::rc::{Allocated, Retained};
use objc2::runtime::{AnyClass, AnyObject, AnyProtocol};
use objc2::{AnyThread, ClassType, DefinedClass, MainThreadMarker, define_class, msg_send};
use objc2_foundation::{NSArray, NSObject, NSObjectProtocol};

type Boolean = u8;
type CFIndex = isize;
type CFTypeID = usize;
type CFTypeRef = *const c_void;

#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    fn CFRetain(cf: CFTypeRef) -> CFTypeRef;
    fn CFRelease(cf: CFTypeRef);
    fn CFGetTypeID(cf: CFTypeRef) -> CFTypeID;
    fn CFNumberGetTypeID() -> CFTypeID;
    fn CFNumberGetValue(number: CFTypeRef, the_type: CFIndex, value_ptr: *mut c_void) -> Boolean;
    fn CFArrayGetCount(array: CFTypeRef) -> CFIndex;
    fn CFArrayGetValueAtIndex(array: CFTypeRef, idx: CFIndex) -> CFTypeRef;
    fn CFDictionaryGetValue(dict: CFTypeRef, key: CFTypeRef) -> CFTypeRef;
}

#[link(name = "CoreMedia", kind = "framework")]
unsafe extern "C" {
    fn CMSampleBufferIsValid(sbuf: *mut c_void) -> Boolean;
    fn CMSampleBufferGetSampleAttachmentsArray(sbuf: *mut c_void, create_if_necessary: Boolean) -> CFTypeRef;
    fn CMSampleBufferGetImageBuffer(sbuf: *mut c_void) -> *mut c_void;
}

#[link(name = "CoreVideo", kind = "framework")]
unsafe extern "C" {
    fn CVPixelBufferGetWidth(pixel_buffer: *mut c_void) -> usize;
    fn CVPixelBufferGetHeight(pixel_buffer: *mut c_void) -> usize;
    fn CVPixelBufferGetPixelFormatType(pixel_buffer: *mut c_void) -> u32;
    fn CVPixelBufferIsPlanar(pixel_buffer: *mut c_void) -> Boolean;
    fn CVPixelBufferGetPlaneCount(pixel_buffer: *mut c_void) -> usize;
}

#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" {
    static kCGDisplayStreamYCbCrMatrix_ITU_R_709_2: CFTypeRef;
}

#[link(name = "ScreenCaptureKit", kind = "framework")]
unsafe extern "C" {
    static SCStreamErrorDomain: *const AnyObject;
    static SCStreamFrameInfoStatus: *const AnyObject;
}

const PICK_TIMEOUT: Duration = Duration::from_secs(60);
const STOP_TIMEOUT: Duration = Duration::from_secs(2);
const POLL: Duration = Duration::from_millis(100);

static NEXT_OWNER: AtomicU64 = AtomicU64::new(1);
static PICKER_OWNER: Mutex<Option<u64>> = Mutex::new(None);

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|err| err.into_inner())
}

fn acquire_picker(owner: u64) -> bool {
    let mut guard = lock(&PICKER_OWNER);
    if guard.is_some() {
        return false;
    }
    *guard = Some(owner);
    true
}

fn release_picker(owner: u64) {
    let mut guard = lock(&PICKER_OWNER);
    if *guard == Some(owner) {
        *guard = None;
    }
}

/// What the mirror loop asks the capture to produce.
#[derive(Clone, Copy, Debug)]
pub struct CaptureRequest {
    pub max_size: u32,
    pub fps: u32,
}

/// Why a capture did not start, or why ScreenCaptureKit ended it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CaptureError {
    Unsupported,
    Cancelled,
    Declined,
    Stopped(isize),
    Failed(String),
}

enum StartNote {
    Ready,
    Failed(CaptureError),
}

/// One retained `'420v'` image buffer. Drop releases it.
pub struct PixelBuffer {
    ptr: NonNull<c_void>,
    budget: Arc<AtomicUsize>,
}

impl std::fmt::Debug for PixelBuffer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("PixelBuffer")
    }
}

// SAFETY: the buffer is an IOSurface-backed CF object. The mirror thread
// sends it to the encoder and does not share the pointer across threads.
unsafe impl Send for PixelBuffer {}

impl PixelBuffer {
    pub fn as_raw(&self) -> *mut c_void {
        self.ptr.as_ptr()
    }
}

impl Drop for PixelBuffer {
    fn drop(&mut self) {
        // SAFETY: `ptr` came from `CFRetain` and has not been released yet.
        unsafe { CFRelease(self.ptr.as_ptr()) };
        self.budget.fetch_sub(1, Ordering::Release);
    }
}

struct SendObj(*mut AnyObject);

// SAFETY: the pointer is an Objective-C object we retain, and ScreenCaptureKit
// objects used here are messaged from the mirror thread or the main queue
// with their own internal locking. The pointer itself is not aliased mutably.
unsafe impl Send for SendObj {}
unsafe impl Sync for SendObj {}

impl SendObj {
    fn from_retained(obj: Retained<AnyObject>) -> Self {
        Self(Retained::into_raw(obj))
    }

    fn retain(ptr: *mut AnyObject) -> Self {
        if ptr.is_null() {
            return Self(ptr::null_mut());
        }
        // SAFETY: `ptr` is a live object. This +1 is released in `Drop`.
        Self(unsafe { objc2::ffi::objc_retain(ptr) })
    }

    fn as_ptr(&self) -> *mut AnyObject {
        self.0
    }
}

impl Drop for SendObj {
    fn drop(&mut self) {
        if !self.0.is_null() {
            // SAFETY: `from_retained` and `retain` each leave a +1 count, and this is the last release.
            unsafe { objc2::ffi::objc_release(self.0) };
        }
    }
}

#[derive(Default)]
struct Native {
    stream: Option<SendObj>,
    sink: Option<Retained<CaptureSink>>,
    queue: Option<DispatchRetained<DispatchQueue>>,
}

impl Native {
    fn ready(&self) -> bool {
        self.stream.is_some() && self.sink.is_some() && self.queue.is_some()
    }
}

struct Shared {
    phase: Mutex<Phase>,
    slot: Mutex<Option<PixelBuffer>>,
    ended: Mutex<Option<CaptureError>>,
    tx: Mutex<Option<Sender<StartNote>>>,
    session: Mutex<Weak<Session>>,
    budget: Arc<AtomicUsize>,
    rejected: AtomicU64,
    inflight: AtomicUsize,
    closed: AtomicBool,
    torn_down: AtomicBool,
    observer_added: AtomicBool,
    output_added: AtomicBool,
    activated: AtomicBool,
    stop_sent: AtomicBool,
    width: AtomicU32,
    height: AtomicU32,
    max_size: u32,
    fps: u32,
}

struct Session {
    shared: Arc<Shared>,
    native: Mutex<Native>,
    owner: u64,
    handed_off: AtomicBool,
}

struct Inflight<'a>(&'a AtomicUsize);

impl<'a> Inflight<'a> {
    fn enter(counter: &'a AtomicUsize) -> Self {
        counter.fetch_add(1, Ordering::AcqRel);
        Self(counter)
    }
}

impl Drop for Inflight<'_> {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::Release);
    }
}

impl Shared {
    fn new(max_size: u32, fps: u32) -> Self {
        Self {
            phase: Mutex::new(Phase::Queued),
            slot: Mutex::new(None),
            ended: Mutex::new(None),
            tx: Mutex::new(None),
            session: Mutex::new(Weak::new()),
            budget: Arc::new(AtomicUsize::new(0)),
            rejected: AtomicU64::new(0),
            inflight: AtomicUsize::new(0),
            closed: AtomicBool::new(false),
            torn_down: AtomicBool::new(false),
            observer_added: AtomicBool::new(false),
            output_added: AtomicBool::new(false),
            activated: AtomicBool::new(false),
            stop_sent: AtomicBool::new(false),
            width: AtomicU32::new(0),
            height: AtomicU32::new(0),
            max_size,
            fps,
        }
    }
}

impl Session {
    fn new(owner: u64, max_size: u32, fps: u32) -> Arc<Self> {
        let shared = Arc::new(Shared::new(max_size, fps));
        let session = Arc::new(Self {
            shared: Arc::clone(&shared),
            native: Mutex::new(Native::default()),
            owner,
            handed_off: AtomicBool::new(false),
        });
        *lock(&shared.session) = Arc::downgrade(&session);
        session
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        if !self.handed_off.load(Ordering::Acquire) {
            teardown_session(self);
        }
    }
}

/// A running display capture. Frames are NV12, latest-wins, and at most two
/// buffers stay retained: the unread slot and the buffer `take_new` already
/// handed out.
pub struct ScreenCapture {
    native: Native,
    shared: Arc<Shared>,
    width: u32,
    height: u32,
    owner: u64,
}

impl std::fmt::Debug for ScreenCapture {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ScreenCapture").field("width", &self.width).field("height", &self.height).finish()
    }
}

impl ScreenCapture {
    /// Presents the system picker and waits until the owner picks a display,
    /// cancels, or `stop` is set. Returns when the stream is producing, or
    /// with an error. Must not be called on the main thread: the wait would
    /// block the queue that presents the picker.
    pub fn start(req: CaptureRequest, stop: &AtomicBool) -> Result<Self, CaptureError> {
        if MainThreadMarker::new().is_some() {
            return Err(CaptureError::Failed("Screen capture cannot start on the main thread.".into()));
        }
        if !framework_ready() {
            return Err(CaptureError::Unsupported);
        }
        if stop.load(Ordering::Acquire) {
            return Err(CaptureError::Cancelled);
        }
        let max_size = req.max_size.clamp(MIN_EDGE, MAX_EDGE);
        let fps = clamp_fps(req.fps);
        let owner = NEXT_OWNER.fetch_add(1, Ordering::Relaxed);
        if !acquire_picker(owner) {
            return Err(CaptureError::Failed(
                "Screen capture is already waiting for a display choice.".into(),
            ));
        }
        let session = Session::new(owner, max_size, fps);
        let (tx, rx) = mpsc::channel();
        *lock(&session.shared.tx) = Some(tx);
        let queued = Arc::clone(&session);
        DispatchQueue::main().exec_async(move || present_on_main(queued));
        let result = wait_for_start(&session, &rx, stop, Instant::now() + PICK_TIMEOUT);
        if result.is_err() {
            teardown_session(&session);
        }
        result
    }

    pub fn size(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    pub fn take_new(&self) -> Option<PixelBuffer> {
        lock(&self.shared.slot).take()
    }

    pub fn ended(&self) -> Option<CaptureError> {
        lock(&self.shared.ended).clone()
    }
}

impl Drop for ScreenCapture {
    fn drop(&mut self) {
        if self.shared.torn_down.swap(true, Ordering::AcqRel) {
            return;
        }
        finish_teardown(&self.shared, &mut self.native, self.owner);
    }
}

fn framework_ready() -> bool {
    // SAFETY: the symbols are process-lifetime framework constants. Reading
    // them loads ScreenCaptureKit before any class lookup.
    let (status, domain) = unsafe { (SCStreamFrameInfoStatus, SCStreamErrorDomain) };
    if status.is_null() || domain.is_null() {
        return false;
    }
    AnyClass::get(c"SCStream").is_some()
        && AnyClass::get(c"SCContentSharingPicker").is_some()
        && AnyClass::get(c"SCStreamConfiguration").is_some()
        && AnyClass::get(c"SCContentSharingPickerConfiguration").is_some()
        && AnyClass::get(c"NSNumber").is_some()
}

fn advance(shared: &Shared, event: Event) -> bool {
    let mut phase = lock(&shared.phase);
    let (next, allow) = apply(*phase, event);
    if allow && matches!(event, Event::Abort | Event::Cancel) {
        shared.closed.store(true, Ordering::Release);
    }
    *phase = next;
    allow
}

fn force_closed(shared: &Shared) {
    let mut phase = lock(&shared.phase);
    shared.closed.store(true, Ordering::Release);
    let (next, _) = apply(*phase, Event::Abort);
    *phase = next;
}

fn emit(shared: &Shared, note: StartNote) {
    if let Some(tx) = lock(&shared.tx).take() {
        let _ = tx.send(note);
    }
}

fn internal_error() -> CaptureError {
    CaptureError::Failed("Screen capture hit an internal error.".into())
}

fn wait_for_start(
    session: &Arc<Session>,
    rx: &Receiver<StartNote>,
    stop: &AtomicBool,
    deadline: Instant,
) -> Result<ScreenCapture, CaptureError> {
    loop {
        if stop.load(Ordering::Acquire) {
            return abort(session, rx, CaptureError::Cancelled);
        }
        match rx.recv_timeout(POLL) {
            Ok(note) => return note_to_result(session, note),
            Err(RecvTimeoutError::Timeout) if Instant::now() >= deadline => {
                return abort(session, rx, CaptureError::Declined);
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => {
                return abort(
                    session,
                    rx,
                    CaptureError::Failed("The display picker closed before a choice.".into()),
                );
            }
        }
    }
}

fn abort(
    session: &Arc<Session>,
    rx: &Receiver<StartNote>,
    err: CaptureError,
) -> Result<ScreenCapture, CaptureError> {
    if let Ok(note) = rx.try_recv() {
        return note_to_result(session, note);
    }
    let ended = advance(&session.shared, Event::Cancel) || advance(&session.shared, Event::Abort);
    if let Ok(note) = rx.try_recv() {
        return note_to_result(session, note);
    }
    if ended {
        return Err(err);
    }
    match rx.recv_timeout(STOP_TIMEOUT) {
        Ok(note) => note_to_result(session, note),
        Err(_) => Err(err),
    }
}

fn note_to_result(session: &Arc<Session>, note: StartNote) -> Result<ScreenCapture, CaptureError> {
    match note {
        StartNote::Ready => session
            .take_capture()
            .ok_or_else(|| CaptureError::Failed("The capture stream was not created.".into())),
        StartNote::Failed(err) => Err(err),
    }
}

impl Session {
    fn take_capture(self: &Arc<Self>) -> Option<ScreenCapture> {
        let taken = {
            let mut native = lock(&self.native);
            if !native.ready() {
                return None;
            }
            std::mem::take(&mut *native)
        };
        let capture = ScreenCapture {
            native: taken,
            shared: Arc::clone(&self.shared),
            width: self.shared.width.load(Ordering::Acquire),
            height: self.shared.height.load(Ordering::Acquire),
            owner: self.owner,
        };
        self.handed_off.store(true, Ordering::Release);
        Some(capture)
    }
}

fn present_on_main(session: Arc<Session>) {
    let shared = Arc::clone(&session.shared);
    let panicked = catch_unwind(AssertUnwindSafe(|| present_inner(&session))).is_err();
    if panicked && advance(&shared, Event::Abort) {
        emit(&shared, StartNote::Failed(internal_error()));
    }
}

fn present_inner(session: &Arc<Session>) {
    if !advance(&session.shared, Event::Claim) || session.shared.closed.load(Ordering::Acquire) {
        return;
    }
    let Some(picker) = shared_picker() else {
        let _ = advance(&session.shared, Event::Abort);
        emit(&session.shared, StartNote::Failed(CaptureError::Unsupported));
        return;
    };
    let active: bool = unsafe { msg_send![&*picker, isActive] };
    if active {
        let _ = advance(&session.shared, Event::Abort);
        emit(
            &session.shared,
            StartNote::Failed(CaptureError::Failed("Screen capture is already active.".into())),
        );
        return;
    }
    if let Err(err) = configure_picker(&picker) {
        let _ = advance(&session.shared, Event::Abort);
        emit(&session.shared, StartNote::Failed(err));
        return;
    }
    prepare_class();
    let sink = CaptureSink::new(Arc::clone(&session.shared));
    lock(&session.native).sink = Some(sink);
    if !advance(&session.shared, Event::Presented) || session.shared.closed.load(Ordering::Acquire) {
        return;
    }
    let Some(sink) = lock(&session.native).sink.clone() else {
        if !session.shared.torn_down.load(Ordering::Acquire) {
            let _ = advance(&session.shared, Event::Abort);
            emit(&session.shared, StartNote::Failed(internal_error()));
        }
        return;
    };
    // SAFETY: `sink` is the observer that implements the picker selectors.
    unsafe {
        let _: () = msg_send![&*picker, addObserver: &*sink];
    }
    session.shared.observer_added.store(true, Ordering::Release);
    if !present_still_open(&session.shared) {
        undo_picker(&session.shared, Some(&sink));
        return;
    }
    session.shared.activated.store(true, Ordering::Release);
    // SAFETY: `active` is the documented BOOL property. The picker shows nothing
    // until this is YES.
    unsafe {
        let _: () = msg_send![&*picker, setActive: true];
    }
    if !present_still_open(&session.shared) {
        undo_picker(&session.shared, Some(&sink));
        return;
    }
    // SAFETY: `present` is the macOS 14 picker entry. It returns after showing
    // the UI; the choice arrives on the observer.
    unsafe {
        let _: () = msg_send![&*picker, present];
    }
    let shared = Arc::clone(&session.shared);
    let sink = sink.clone();
    // Runs after `present` returns, or inside it if the picker pumps the main
    // queue. A timeout that already closed the session dismisses here, so a
    // late present cannot keep the picker up. Picker ownership stays with
    // teardown so a second session cannot slip in before removal finishes.
    DispatchQueue::main().exec_async(move || {
        let phase = *lock(&shared.phase);
        if shared.closed.load(Ordering::Acquire) && phase != Phase::Live {
            undo_picker(&shared, Some(&sink));
        }
    });
}

fn present_still_open(shared: &Shared) -> bool {
    let phase = lock(&shared.phase);
    !shared.closed.load(Ordering::Acquire) && *phase == Phase::Picking
}

fn configure_picker(picker: &AnyObject) -> Result<(), CaptureError> {
    let Some(cls) = AnyClass::get(c"SCContentSharingPickerConfiguration") else {
        return Err(CaptureError::Unsupported);
    };
    let config: Option<Retained<AnyObject>> = unsafe { msg_send![cls, new] };
    let Some(config) = config else {
        return Err(CaptureError::Failed("The display picker could not be configured.".into()));
    };
    unsafe {
        let _: () = msg_send![&*config, setAllowedPickerModes: PICKER_SINGLE_DISPLAY];
        let _: () = msg_send![&*config, setAllowsChangingSelectedContent: false];
        let empty = NSArray::<NSObject>::array();
        let _: () = msg_send![&*config, setExcludedWindowIDs: &*empty];
        let _: () = msg_send![picker, setDefaultConfiguration: &*config];
    }
    let Some(numbers) = AnyClass::get(c"NSNumber") else {
        return Err(CaptureError::Unsupported);
    };
    let one: Option<Retained<AnyObject>> = unsafe { msg_send![numbers, numberWithInteger: 1isize] };
    let Some(one) = one else {
        return Err(CaptureError::Failed("The display picker could not be configured.".into()));
    };
    unsafe {
        let _: () = msg_send![picker, setMaximumStreamCount: &*one];
    }
    Ok(())
}

fn begin_stream(session: &Arc<Session>, filter: &AnyObject) {
    if session.shared.closed.load(Ordering::Acquire) {
        return;
    }
    let rect: decide::CgRect = unsafe { msg_send![filter, contentRect] };
    let scale: f32 = unsafe { msg_send![filter, pointPixelScale] };
    let Some((px_w, px_h)) = display_pixels(rect.size.width, rect.size.height, f64::from(scale)) else {
        emit(
            &session.shared,
            StartNote::Failed(CaptureError::Failed("The display size is not usable.".into())),
        );
        return;
    };
    let (width, height) = target_dimensions(px_w, px_h, session.shared.max_size);
    session.shared.width.store(width, Ordering::Release);
    session.shared.height.store(height, Ordering::Release);
    if session.shared.closed.load(Ordering::Acquire) {
        return;
    }
    let Some(matrix) = color_matrix() else {
        emit(
            &session.shared,
            StartNote::Failed(CaptureError::Failed("The capture color matrix is unavailable.".into())),
        );
        return;
    };
    let Some(config_cls) = AnyClass::get(c"SCStreamConfiguration") else {
        emit(&session.shared, StartNote::Failed(CaptureError::Unsupported));
        return;
    };
    let config: Option<Retained<AnyObject>> = unsafe { msg_send![config_cls, new] };
    let Some(config) = config else {
        emit(&session.shared, StartNote::Failed(internal_error()));
        return;
    };
    configure_stream(&config, width as usize, height as usize, session.shared.fps, matrix);
    let Some(stream_cls) = AnyClass::get(c"SCStream") else {
        emit(&session.shared, StartNote::Failed(CaptureError::Unsupported));
        return;
    };
    let Some(sink) = lock(&session.native).sink.clone() else {
        emit(&session.shared, StartNote::Failed(internal_error()));
        return;
    };
    let allocated: Allocated<AnyObject> = unsafe { msg_send![stream_cls, alloc] };
    let stream: Option<Retained<AnyObject>> =
        unsafe { msg_send![allocated, initWithFilter: filter, configuration: &*config, delegate: &*sink] };
    let Some(stream) = stream else {
        emit(
            &session.shared,
            StartNote::Failed(CaptureError::Failed("The capture stream was not created.".into())),
        );
        return;
    };
    let queue = DispatchQueue::new("app.waypair.screen-capture", DispatchQueueAttr::SERIAL);
    if let Err(err) = add_output(&stream, &sink, &queue, &session.shared) {
        emit(&session.shared, StartNote::Failed(err));
        return;
    }
    let ptr = Retained::as_ptr(&stream) as *mut AnyObject;
    // Hold the native lock across `startCapture` so teardown cannot drop the
    // stream between the publish and the call. `startCapture` only queues its
    // completion; that block does not take this lock.
    let mut native = lock(&session.native);
    if session.shared.torn_down.load(Ordering::Acquire) {
        drop(native);
        remove_output(&session.shared, ptr, &sink);
        return;
    }
    native.stream = Some(SendObj::from_retained(stream));
    native.queue = Some(queue);
    if !session.shared.closed.load(Ordering::Acquire) {
        start_capture(session, ptr, &sink);
    }
}

fn configure_stream(config: &AnyObject, width: usize, height: usize, fps: u32, matrix: CfStringRef) {
    let interval = frame_interval(fps);
    unsafe {
        let _: () = msg_send![config, setWidth: width];
        let _: () = msg_send![config, setHeight: height];
        let _: () = msg_send![config, setMinimumFrameInterval: interval];
        let _: () = msg_send![config, setPixelFormat: decide::PIXEL_420V];
        let _: () = msg_send![config, setScalesToFit: true];
        let _: () = msg_send![config, setPreservesAspectRatio: true];
        let _: () = msg_send![config, setShowsCursor: true];
        let _: () = msg_send![config, setQueueDepth: QUEUE_DEPTH];
        let _: () = msg_send![config, setCapturesAudio: false];
        let _: () = msg_send![config, setColorMatrix: matrix];
    }
}

fn color_matrix() -> Option<CfStringRef> {
    // SAFETY: process-lifetime CoreGraphics constant.
    let ptr = unsafe { kCGDisplayStreamYCbCrMatrix_ITU_R_709_2 };
    if ptr.is_null() { None } else { Some(CfStringRef(ptr)) }
}

fn add_output(
    stream: &AnyObject,
    sink: &CaptureSink,
    queue: &DispatchRetained<DispatchQueue>,
    shared: &Shared,
) -> Result<(), CaptureError> {
    let mut err = ptr::null_mut();
    // The queue stays retained by `Native` for the life of the stream.
    let qptr = DispatchRetained::as_ptr(queue).as_ptr() as *mut AnyObject;
    // The `NSError **` is `__autoreleasing`. Read the code inside a pool so a
    // mirror-thread failure does not depend on a pool the caller installed.
    let (ok, code) = objc2::rc::autoreleasepool(|_| {
        let ok: bool = unsafe {
            msg_send![
                stream,
                addStreamOutput: sink,
                type: OUTPUT_SCREEN,
                sampleHandlerQueue: qptr,
                error: &mut err,
            ]
        };
        (ok, error_code(err))
    });
    if ok {
        shared.output_added.store(true, Ordering::Release);
        Ok(())
    } else {
        Err(match code {
            Some(code) => CaptureError::Failed(code.to_string()),
            None => CaptureError::Failed("The stream output was rejected.".into()),
        })
    }
}

fn start_capture(session: &Arc<Session>, stream: *mut AnyObject, sink: &CaptureSink) {
    if stream.is_null() || session.shared.closed.load(Ordering::Acquire) {
        return;
    }
    let session = Arc::clone(session);
    // These retains belong to the completion, not to `Shared`, so a timeout
    // that drops `Native` cannot free the stream before this callback runs.
    let held_stream = SendObj::retain(stream);
    let held_sink: Retained<CaptureSink> = sink.into();
    let block = RcBlock::new(move |err: *mut AnyObject| {
        let stream_ptr = held_stream.as_ptr();
        let _ = catch_unwind(AssertUnwindSafe(|| {
            let code = error_code(err);
            on_started(&session, code, stream_ptr, &held_sink);
        }));
    });
    // SAFETY: the block is `void (^)(NSError * _Nullable)`. ScreenCaptureKit
    // copies it. Dropping `block` below releases only our retain.
    unsafe {
        let _: () = msg_send![&*stream, startCaptureWithCompletionHandler: &*block];
    }
}

fn on_started(session: &Session, error: Option<isize>, stream: *mut AnyObject, sink: &CaptureSink) {
    if let Some(code) = error {
        let _ = advance(&session.shared, Event::Abort);
        emit(&session.shared, StartNote::Failed(CaptureError::Failed(code.to_string())));
        return;
    }
    // A close that lands before or during `Started` must not publish. The
    // second check covers `didStop` setting `closed` without moving the phase.
    if session_shut(session) || !advance(&session.shared, Event::Started) || session_shut(session) {
        session.shared.closed.store(true, Ordering::Release);
        stop_refused_start(stream, sink);
        return;
    }
    emit(&session.shared, StartNote::Ready);
}

fn session_shut(session: &Session) -> bool {
    session.shared.closed.load(Ordering::Acquire) || session.shared.torn_down.load(Ordering::Acquire)
}

fn deliver_frame(shared: &Shared, sample: *mut c_void, kind: isize) {
    if kind != OUTPUT_SCREEN || sample.is_null() || shared.closed.load(Ordering::Acquire) {
        return;
    }
    // SAFETY: `sample` is the borrowed buffer from the stream callback.
    if unsafe { CMSampleBufferIsValid(sample) } == 0 {
        return;
    }
    let status = unsafe { frame_status(sample) };
    let image = unsafe { CMSampleBufferGetImageBuffer(sample) };
    if image.is_null() {
        return;
    }
    let format = unsafe { CVPixelBufferGetPixelFormatType(image) };
    let planes = if unsafe { CVPixelBufferIsPlanar(image) } == 0 {
        0
    } else {
        unsafe { CVPixelBufferGetPlaneCount(image) }
    };
    let size = unsafe { (CVPixelBufferGetWidth(image), CVPixelBufferGetHeight(image)) };
    let want = (shared.width.load(Ordering::Acquire), shared.height.load(Ordering::Acquire));
    if !keep_frame(status, format, planes, size, want) {
        shared.rejected.fetch_add(1, Ordering::Relaxed);
        return;
    }
    store_frame(shared, image);
}

fn store_frame(shared: &Shared, image: *mut c_void) {
    {
        let previous = lock(&shared.slot).take();
        drop(previous);
    }
    if shared.closed.load(Ordering::Acquire) {
        return;
    }
    if !acquire_budget(&shared.budget) {
        shared.rejected.fetch_add(1, Ordering::Relaxed);
        return;
    }
    // SAFETY: `image` is a live CVPixelBuffer from the sample. `CFRetain`
    // bumps the count we drop in `PixelBuffer`.
    let retained = unsafe { CFRetain(image) };
    let Some(ptr) = NonNull::new(retained.cast_mut()) else {
        shared.budget.fetch_sub(1, Ordering::Release);
        return;
    };
    let buffer = PixelBuffer { ptr, budget: Arc::clone(&shared.budget) };
    if shared.closed.load(Ordering::Acquire) {
        return;
    }
    let mut slot = lock(&shared.slot);
    if shared.closed.load(Ordering::Acquire) {
        return;
    }
    *slot = Some(buffer);
}

fn acquire_budget(budget: &AtomicUsize) -> bool {
    let mut held = budget.load(Ordering::Acquire);
    loop {
        if !admit_frame(held) {
            return false;
        }
        match budget.compare_exchange_weak(held, held + 1, Ordering::AcqRel, Ordering::Acquire) {
            Ok(_) => return true,
            Err(current) => held = current,
        }
    }
}

unsafe fn frame_status(sample: *mut c_void) -> Option<i64> {
    let array = unsafe { CMSampleBufferGetSampleAttachmentsArray(sample, 0) };
    if array.is_null() || unsafe { CFArrayGetCount(array) } < 1 {
        return None;
    }
    let dict = unsafe { CFArrayGetValueAtIndex(array, 0) };
    // SAFETY: framework constant, also read by `framework_ready`.
    let key = unsafe { SCStreamFrameInfoStatus } as CFTypeRef;
    if dict.is_null() || key.is_null() {
        return None;
    }
    // SAFETY: the attachment dictionary belongs to the live sample; the
    // number reader checks its type before reading its value.
    unsafe { cf_number_i64(CFDictionaryGetValue(dict, key)) }
}

unsafe fn cf_number_i64(value: CFTypeRef) -> Option<i64> {
    if value.is_null() || unsafe { CFGetTypeID(value) } != unsafe { CFNumberGetTypeID() } {
        return None;
    }
    let mut out = 0i64;
    let ok = unsafe { CFNumberGetValue(value, CF_NUMBER_SINT64, &mut out as *mut i64 as *mut c_void) };
    if ok == 0 { None } else { Some(out) }
}

fn teardown_session(session: &Session) {
    if session.shared.torn_down.swap(true, Ordering::AcqRel) {
        return;
    }
    let mut native = std::mem::take(&mut *lock(&session.native));
    finish_teardown(&session.shared, &mut native, session.owner);
}

fn finish_teardown(shared: &Arc<Shared>, native: &mut Native, owner: u64) {
    force_closed(shared);
    if let Some(stream) = native.stream.as_ref() {
        let sink_for_stop = native.sink.clone();
        stop_if_needed(shared, stream.as_ptr(), sink_for_stop);
    }
    wait_inflight(shared);
    if let (Some(stream), Some(sink)) = (native.stream.as_ref(), native.sink.as_ref()) {
        remove_output(shared, stream.as_ptr(), sink);
    }
    wait_inflight(shared);
    let sink = native.sink.clone();
    let shared_main = Arc::clone(shared);
    run_on_main(move || {
        undo_picker(&shared_main, sink.as_deref());
        release_picker(owner);
    });
    // The stop completion holds its own sink and stream retains until that
    // callback returns. Dropping these after a 2 s timeout does not free the
    // objects while ScreenCaptureKit can still call back.
    native.stream.take();
    native.sink.take();
    native.queue.take();
}

fn stop_if_needed(shared: &Shared, stream: *mut AnyObject, sink: Option<Retained<CaptureSink>>) {
    if stream.is_null() || shared.stop_sent.swap(true, Ordering::AcqRel) {
        return;
    }
    let (tx, rx) = mpsc::channel::<bool>();
    let block = stop_block(stream, sink, Some(tx));
    // SAFETY: the block is `void (^)(NSError * _Nullable)`. ScreenCaptureKit
    // copies it. On timeout this drop releases only our retain; the copy keeps
    // the sink and the stream until the callback returns.
    unsafe {
        let _: () = msg_send![&*stream, stopCaptureWithCompletionHandler: &*block];
    }
    let _ = rx.recv_timeout(STOP_TIMEOUT);
}

/// Stop the stream whose start completed after cancel or teardown. This does
/// not consult `stop_sent`: an earlier stop may have lost with `-3808` before
/// the stream existed. It does not wait, because it runs on the start callback.
fn stop_refused_start(stream: *mut AnyObject, sink: &CaptureSink) {
    if stream.is_null() {
        return;
    }
    let block = stop_block(stream, Some(sink.into()), None);
    unsafe {
        let _: () = msg_send![&*stream, stopCaptureWithCompletionHandler: &*block];
    }
}

fn stop_block(
    stream: *mut AnyObject,
    sink: Option<Retained<CaptureSink>>,
    tx: Option<Sender<bool>>,
) -> RcBlock<dyn Fn(*mut AnyObject)> {
    let held_stream = SendObj::retain(stream);
    let held_sink = sink;
    RcBlock::new(move |err: *mut AnyObject| {
        // Keep both alive for the whole callback. They are not stored on the
        // sink or the session, so the extra retains are not a cycle.
        let _ = held_stream.as_ptr();
        let _ = held_sink.as_ref();
        // `-3808` means the stream was already stopping. The NSError text is
        // not read; this pointer is borrowed.
        let already_stopping = error_code(err) == Some(ATTEMPT_TO_STOP);
        if let Some(tx) = &tx {
            let _ = tx.send(already_stopping);
        }
    })
}

fn remove_output(shared: &Shared, stream: *mut AnyObject, sink: &CaptureSink) {
    if stream.is_null() || !shared.output_added.swap(false, Ordering::AcqRel) {
        return;
    }
    objc2::rc::autoreleasepool(|_| {
        let mut err = ptr::null_mut();
        unsafe {
            let _: bool = msg_send![
                &*stream,
                removeStreamOutput: sink,
                type: OUTPUT_SCREEN,
                error: &mut err,
            ];
        }
        let _ = error_code(err);
    });
}

fn undo_picker(shared: &Shared, sink: Option<&CaptureSink>) {
    let Some(picker) = shared_picker() else {
        shared.observer_added.store(false, Ordering::Release);
        shared.activated.store(false, Ordering::Release);
        return;
    };
    if let Some(sink) = sink
        && shared.observer_added.swap(false, Ordering::AcqRel)
    {
        unsafe {
            let _: () = msg_send![&*picker, removeObserver: sink];
        }
    }
    if shared.activated.swap(false, Ordering::AcqRel) {
        unsafe {
            let _: () = msg_send![&*picker, setActive: false];
        }
    }
}

fn shared_picker() -> Option<Retained<AnyObject>> {
    let cls = AnyClass::get(c"SCContentSharingPicker")?;
    unsafe { msg_send![cls, sharedPicker] }
}

fn run_on_main<F>(work: F)
where
    F: FnOnce() + Send + 'static,
{
    if MainThreadMarker::new().is_some() {
        work();
        return;
    }
    let (tx, rx) = mpsc::channel();
    DispatchQueue::main().exec_async(move || {
        work();
        let _ = tx.send(());
    });
    // If this times out, `work` is already owned by the main queue. Its
    // captures stay alive until that block runs; this wait does not free them.
    let _ = rx.recv_timeout(STOP_TIMEOUT);
}

fn wait_inflight(shared: &Shared) {
    let start = Instant::now();
    while shared.inflight.load(Ordering::Acquire) > 0 && start.elapsed() < STOP_TIMEOUT {
        thread::sleep(Duration::from_millis(10));
    }
}

fn error_code(err: *mut AnyObject) -> Option<isize> {
    if err.is_null() {
        return None;
    }
    // `NSError **` out-parameters on these methods are the Cocoa
    // `__autoreleasing` convention (`SCStream.h` has no `NS_RETURNS_RETAINED`),
    // and completion-handler `NSError *` is borrowed. Read `code` only.
    // `localizedDescription` can contain a display name, so it is not touched.
    Some(unsafe { msg_send![&*err, code] })
}

fn prepare_class() {
    let class = CaptureSink::class();
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        for name in PROTOCOLS {
            let Some(protocol) = AnyProtocol::get(name) else { continue };
            // SAFETY: `class` was just registered by `ClassType::class`. Adding
            // a missing protocol is idempotent; a `false` result means it was
            // already present or the runtime refused, and the methods remain
            // visible through `respondsToSelector:`.
            unsafe {
                let _ = objc2::ffi::class_addProtocol(ptr::from_ref(class).cast_mut(), protocol);
            }
        }
    });
}

struct CaptureIvars {
    shared: Arc<Shared>,
}

define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = AnyThread]
    #[name = "WaypairScreenCaptureSink"]
    #[ivars = CaptureIvars]
    struct CaptureSink;

    unsafe impl NSObjectProtocol for CaptureSink {}

    impl CaptureSink {
        #[unsafe(method(stream:didOutputSampleBuffer:ofType:))]
        fn did_output(&self, _stream: *mut AnyObject, sample: *mut c_void, kind: isize) {
            let shared = Arc::clone(&self.ivars().shared);
            let _guard = Inflight::enter(&shared.inflight);
            let _ = catch_unwind(AssertUnwindSafe(|| deliver_frame(&shared, sample, kind)));
        }

        #[unsafe(method(stream:didStopWithError:))]
        fn did_stop(&self, _stream: *mut AnyObject, error: *mut AnyObject) {
            let shared = Arc::clone(&self.ivars().shared);
            let _guard = Inflight::enter(&shared.inflight);
            let _ = catch_unwind(AssertUnwindSafe(|| {
                let err = match error_code(error) {
                    Some(code) => CaptureError::Stopped(code),
                    None => CaptureError::Failed("The capture stream stopped.".into()),
                };
                *lock(&shared.ended) = Some(err);
                shared.closed.store(true, Ordering::Release);
            }));
        }

        #[unsafe(method(contentSharingPicker:didCancelForStream:))]
        fn did_cancel(&self, _picker: *mut AnyObject, _stream: *mut AnyObject) {
            let shared = Arc::clone(&self.ivars().shared);
            let _guard = Inflight::enter(&shared.inflight);
            let _ = catch_unwind(AssertUnwindSafe(|| {
                if advance(&shared, Event::Abort) {
                    emit(&shared, StartNote::Failed(CaptureError::Cancelled));
                }
            }));
        }

        #[unsafe(method(contentSharingPicker:didUpdateWithFilter:forStream:))]
        fn did_update(&self, _picker: *mut AnyObject, filter: *mut AnyObject, _stream: *mut AnyObject) {
            let shared = Arc::clone(&self.ivars().shared);
            let _guard = Inflight::enter(&shared.inflight);
            let shared_for_panic = Arc::clone(&shared);
            let panicked = catch_unwind(AssertUnwindSafe(|| {
                if filter.is_null() || !advance(&shared, Event::Filter) {
                    return;
                }
                let Some(session) = lock(&shared.session).upgrade() else { return };
                begin_stream(&session, unsafe { &*filter });
            }))
            .is_err();
            if panicked && advance(&shared_for_panic, Event::Abort) {
                emit(&shared_for_panic, StartNote::Failed(internal_error()));
            }
        }

        #[unsafe(method(contentSharingPickerStartDidFailWithError:))]
        fn start_failed(&self, error: *mut AnyObject) {
            let shared = Arc::clone(&self.ivars().shared);
            let _guard = Inflight::enter(&shared.inflight);
            let _ = catch_unwind(AssertUnwindSafe(|| {
                if advance(&shared, Event::Abort) {
                    let err = match error_code(error) {
                        Some(code) => CaptureError::Failed(code.to_string()),
                        None => CaptureError::Failed("The display picker did not open.".into()),
                    };
                    emit(&shared, StartNote::Failed(err));
                }
            }));
        }
    }
);

impl CaptureSink {
    fn new(shared: Arc<Shared>) -> Retained<Self> {
        let this = Self::alloc().set_ivars(CaptureIvars { shared });
        unsafe { msg_send![super(this), init] }
    }
}
