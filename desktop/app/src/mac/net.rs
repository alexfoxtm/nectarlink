// SPDX-License-Identifier: GPL-3.0-or-later
//! Owned SystemConfiguration notifications for interface, address, and route changes.
//! Bursts are debounced by main::watch_network.
#![allow(unsafe_code)]

use std::{
    ffi::c_void,
    panic::{AssertUnwindSafe, catch_unwind},
    ptr,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread::{self, JoinHandle},
};

type Ref = *const c_void;
type Callback = Box<dyn Fn() + Send + Sync>;

#[repr(C)]
struct StoreContext {
    version: isize,
    info: *mut c_void,
    retain: Option<unsafe extern "C" fn(*const c_void) -> *const c_void>,
    release: Option<unsafe extern "C" fn(*const c_void)>,
    copy_description: Option<unsafe extern "C" fn(*const c_void) -> Ref>,
}

#[repr(C)]
struct SourceContext {
    version: isize,
    info: *mut c_void,
    retain: Option<unsafe extern "C" fn(*const c_void) -> *const c_void>,
    release: Option<unsafe extern "C" fn(*const c_void)>,
    copy_description: Option<unsafe extern "C" fn(*const c_void) -> Ref>,
    equal: Option<unsafe extern "C" fn(*const c_void, *const c_void) -> u8>,
    hash: Option<unsafe extern "C" fn(*const c_void) -> usize>,
    schedule: Option<unsafe extern "C" fn(*mut c_void, Ref, Ref)>,
    cancel: Option<unsafe extern "C" fn(*mut c_void, Ref, Ref)>,
    perform: Option<unsafe extern "C" fn(*mut c_void)>,
}

#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    static kCFRunLoopDefaultMode: Ref;
    static kCFTypeArrayCallBacks: c_void;
    fn CFRetain(value: Ref) -> Ref;
    fn CFRelease(value: Ref);
    fn CFStringCreateWithBytes(
        allocator: Ref,
        bytes: *const u8,
        length: isize,
        encoding: u32,
        external: u8,
    ) -> Ref;
    fn CFArrayCreate(allocator: Ref, values: *const Ref, count: isize, callbacks: Ref) -> Ref;
    fn CFRunLoopGetCurrent() -> Ref;
    fn CFRunLoopSourceCreate(allocator: Ref, order: isize, context: *mut SourceContext) -> Ref;
    fn CFRunLoopAddSource(run_loop: Ref, source: Ref, mode: Ref);
    fn CFRunLoopRemoveSource(run_loop: Ref, source: Ref, mode: Ref);
    fn CFRunLoopSourceSignal(source: Ref);
    fn CFRunLoopWakeUp(run_loop: Ref);
    fn CFRunLoopStop(run_loop: Ref);
    fn CFRunLoopRun();
}

#[link(name = "SystemConfiguration", kind = "framework")]
unsafe extern "C" {
    fn SCDynamicStoreCreate(
        allocator: Ref,
        name: Ref,
        callback: unsafe extern "C" fn(Ref, Ref, *mut c_void),
        context: *mut StoreContext,
    ) -> Ref;
    fn SCDynamicStoreSetNotificationKeys(store: Ref, keys: Ref, patterns: Ref) -> u8;
    fn SCDynamicStoreCreateRunLoopSource(allocator: Ref, store: Ref, order: isize) -> Ref;
}

// Every value is a +1 CoreFoundation reference confined to its creating thread.
struct OwnedRef(Ref);
impl OwnedRef {
    fn new(value: Ref) -> Option<Self> {
        if value.is_null() { None } else { Some(Self(value)) }
    }

    fn string(value: &str) -> Option<Self> {
        // SAFETY: UTF-8 bytes are borrowed for this call and CF copies them.
        Self::new(unsafe {
            CFStringCreateWithBytes(ptr::null(), value.as_ptr(), value.len() as isize, 0x0800_0100, 0)
        })
    }
}
impl Drop for OwnedRef {
    fn drop(&mut self) {
        // SAFETY: this wrapper owns one retain and releases it once.
        unsafe { CFRelease(self.0) };
    }
}

struct Notification {
    active: Arc<AtomicBool>,
    callback: Callback,
}
impl Notification {
    fn notify(&self) {
        if self.active.load(Ordering::Acquire)
            && catch_unwind(AssertUnwindSafe(|| (self.callback)())).is_err()
        {
            tracing::warn!("network change callback panicked");
        }
    }
}

unsafe extern "C" fn changed(_: Ref, _: Ref, info: *mut c_void) {
    // SAFETY: the worker owns Notification until the store is unscheduled and
    // released; the callback only runs on that worker's run loop.
    unsafe { &*info.cast::<Notification>() }.notify();
}

unsafe extern "C" fn terminate(_: *mut c_void) {
    // SAFETY: a signalled stop source runs on its owning worker's run loop.
    unsafe { CFRunLoopStop(CFRunLoopGetCurrent()) };
}

struct Watcher {
    active: Arc<AtomicBool>,
    // Independently retained references transferred as addresses. The worker
    // also owns the stop source for its entire run.
    run_loop: usize,
    stop_source: usize,
    worker: JoinHandle<()>,
}

impl Watcher {
    fn stop(self) {
        self.active.store(false, Ordering::Release);
        // SAFETY: these references were retained before the startup handshake
        // and remain owned here even if the worker has already returned.
        unsafe {
            CFRunLoopSourceSignal(self.stop_source as Ref);
            CFRunLoopWakeUp(self.run_loop as Ref);
        }
        // A callback may stop its own watch. Its worker must finish the current
        // invocation before returning through CFRunLoopRun and releasing state.
        if self.worker.thread().id() != thread::current().id() {
            let _ = self.worker.join();
        }
        // SAFETY: consume exactly the two retains transferred at startup.
        unsafe {
            CFRelease(self.stop_source as Ref);
            CFRelease(self.run_loop as Ref);
        }
    }
}

static WATCHER: Mutex<Option<Watcher>> = Mutex::new(None);

/// Calls callback on an owned native worker when network configuration changes.
/// Repeated watch calls while active are ignored. After stop, a new callback can
/// be installed. Native registration failure leaves the watcher restartable.
pub fn watch(callback: impl Fn() + Send + Sync + 'static) {
    let mut slot = WATCHER.lock().unwrap_or_else(|e| e.into_inner());
    if slot.as_ref().is_some_and(|watcher| !watcher.worker.is_finished()) {
        return;
    }
    if let Some(previous) = slot.take() {
        // A finished worker cannot be running a callback that needs this mutex.
        previous.stop();
    }
    let active = Arc::new(AtomicBool::new(true));
    let notification = Notification { active: active.clone(), callback: Box::new(callback) };
    let (ready, started) = mpsc::sync_channel(1);
    let worker = match thread::Builder::new().name("network-changes".into()).spawn(move || {
        // SAFETY: callback context is owned by this worker until native teardown.
        unsafe { run(notification, ready) };
    }) {
        Ok(worker) => worker,
        Err(error) => {
            tracing::warn!(%error, "cannot start network change watcher");
            return;
        }
    };
    match started.recv() {
        Ok((run_loop, stop_source)) => {
            *slot = Some(Watcher { active, run_loop, stop_source, worker });
        }
        Err(_) => {
            let _ = worker.join();
            tracing::warn!("cannot register macOS network change notifications");
        }
    }
}

/// Cancels the owned watch and drains callbacks before returning. When called
/// from its callback, that invocation finishes normally. Inactive stop is harmless.
pub fn stop() {
    let watcher = {
        let mut slot = WATCHER.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(watcher) = slot.as_ref() {
            watcher.active.store(false, Ordering::Release);
        }
        slot.take()
    };
    if let Some(watcher) = watcher {
        watcher.stop();
    }
}

unsafe fn run(notification: Notification, ready: mpsc::SyncSender<(usize, usize)>) {
    let Some(name) = OwnedRef::string("Waypair network changes") else { return };
    let mut context = StoreContext {
        version: 0,
        info: (&notification as *const Notification).cast_mut().cast(),
        retain: None,
        release: None,
        copy_description: None,
    };
    // SAFETY: CF copies context fields; Notification remains on this stack.
    let Some(store) =
        OwnedRef::new(unsafe { SCDynamicStoreCreate(ptr::null(), name.0, changed, &mut context) })
    else {
        return;
    };
    let patterns: Option<Vec<_>> = [
        "State:/Network/Global/.*",
        "State:/Network/Interface/.*/(IPv4|IPv6|Link)",
        "State:/Network/Service/.*/(IPv4|IPv6|DNS|Proxies)",
    ]
    .into_iter()
    .map(OwnedRef::string)
    .collect();
    let Some(patterns) = patterns else { return };
    let values: Vec<_> = patterns.iter().map(|value| value.0).collect();
    // SAFETY: CFArray retains each CFString with its standard type callbacks.
    let Some(pattern_array) = OwnedRef::new(unsafe {
        CFArrayCreate(ptr::null(), values.as_ptr(), values.len() as isize, &raw const kCFTypeArrayCallBacks)
    }) else {
        return;
    };
    // SAFETY: all arguments are live CF objects; no exact keys are needed.
    if unsafe { SCDynamicStoreSetNotificationKeys(store.0, ptr::null(), pattern_array.0) } == 0 {
        return;
    }
    // SAFETY: store remains alive until after its source has been removed.
    let Some(source) = OwnedRef::new(unsafe { SCDynamicStoreCreateRunLoopSource(ptr::null(), store.0, 0) })
    else {
        return;
    };
    let mut stop_context = SourceContext {
        version: 0,
        info: ptr::null_mut(),
        retain: None,
        release: None,
        copy_description: None,
        equal: None,
        hash: None,
        schedule: None,
        cancel: None,
        perform: Some(terminate),
    };
    // A signalled source handles stop arriving before Run starts: its pending
    // signal survives until Run begins, unlike stopping a run loop not yet running.
    let Some(stop_source) =
        OwnedRef::new(unsafe { CFRunLoopSourceCreate(ptr::null(), -1, &mut stop_context) })
    else {
        return;
    };
    // SAFETY: the worker's current run loop is valid for this thread's lifetime.
    let run_loop = unsafe { CFRunLoopGetCurrent() };
    unsafe {
        CFRunLoopAddSource(run_loop, source.0, kCFRunLoopDefaultMode);
        CFRunLoopAddSource(run_loop, stop_source.0, kCFRunLoopDefaultMode);
        CFRetain(run_loop);
        CFRetain(stop_source.0);
    }
    if ready.send((run_loop as usize, stop_source.0 as usize)).is_ok() {
        // SAFETY: native sources and Notification stay live until Run returns.
        unsafe { CFRunLoopRun() };
    } else {
        // SAFETY: startup did not transfer ownership to a Watcher.
        unsafe {
            CFRelease(stop_source.0);
            CFRelease(run_loop);
        }
    }
    notification.active.store(false, Ordering::Release);
    // SAFETY: remove both sources before releasing the store or callback context.
    unsafe {
        CFRunLoopRemoveSource(run_loop, source.0, kCFRunLoopDefaultMode);
        CFRunLoopRemoveSource(run_loop, stop_source.0, kCFRunLoopDefaultMode);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;

    #[test]
    fn notifications_stop_when_owner_is_cancelled() {
        let active = Arc::new(AtomicBool::new(true));
        let calls = Arc::new(AtomicUsize::new(0));
        let count = calls.clone();
        let notification = Notification {
            active: active.clone(),
            callback: Box::new(move || {
                count.fetch_add(1, Ordering::Relaxed);
            }),
        };
        notification.notify();
        active.store(false, Ordering::Release);
        notification.notify();
        assert_eq!(calls.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn callback_panic_does_not_cross_native_boundary() {
        let notification = Notification {
            active: Arc::new(AtomicBool::new(true)),
            callback: Box::new(|| panic!("callback failure")),
        };
        assert!(catch_unwind(AssertUnwindSafe(|| notification.notify())).is_ok());
    }
}
