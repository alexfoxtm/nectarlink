// SPDX-License-Identifier: GPL-3.0-or-later
//! macOS Services provider: "Take Photo / Scan Document with Phone" for
//! other apps (see `docs/waypair/MAC_CAPTURE_SERVICES.md`).
//!
//! AppKit calls `waypairCapture:userData:error:` on the main run loop
//! with the *caller's* pasteboard. The method blocks the caller, so it
//! waits here on the main thread, pumping `NSApplication` events in the
//! real default run-loop mode (`kCFRunLoopDefaultMode`) in short slices.
//! That keeps Qt posted events and timers running and lets the Waypair
//! window Cancel or Quit. Deadlines, from entry:
//! phone acknowledgement 10 s, total 90 s. There is deliberately **no**
//! general-clipboard fallback: on timeout, cancel, disconnect, or any
//! failure the caller gets an error and nothing is written anywhere; a
//! late image stays only where the core already saved it (downloads and
//! the timeline). Only a result that arrives in time is written, and
//! only to the caller's pasteboard passed to the method.
//!
//! Declared return types are PNG and TIFF, and each is written only
//! when a real conversion produced it — JPEG bytes are never relabelled
//! PNG. The phone's own preview/**Send** confirmation is untouched: a
//! Service can only *ask*, exactly like the app window does.
//!
//! Plain-text fields and hosts that don't accept images keep the items
//! disabled — that is AppKit's `validRequestor` behaviour, not a defect,
//! and there is no menu injection or private API here.

use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    time::{Duration, Instant},
};

use nectarlink_core::CameraCaptureMode;
use objc2::MainThreadMarker;
use objc2_app_kit::NSPasteboard;
use objc2_foundation::{NSData, NSString};

use crate::{
    continuity_camera::{self, ServiceEvent, ServiceRefusal, ServiceTicket},
    i18n::tr,
};

/// Time the phone has to acknowledge (`camera.capture.ok`) before the
/// request is cancelled on the phone with reason `no_ack`.
const ACK_TIMEOUT: Duration = Duration::from_secs(10);
/// Hard total wait. Past this the request is cancelled with `expired`
/// and the caller gets an error — never a clipboard fallback.
const TOTAL_TIMEOUT: Duration = Duration::from_secs(90);
/// Run-loop slice length while waiting; keeps Qt/GCD responsive.
const SLICE: Duration = Duration::from_millis(100);

const TYPE_PNG: &str = "public.png";
const TYPE_TIFF: &str = "public.tiff";

static REGISTERED: AtomicBool = AtomicBool::new(false);
static SHUTTING_DOWN: AtomicBool = AtomicBool::new(false);

/// Which capture an `NSUserData` value asks for (`photo` / `scan`, as
/// written by the packager into `NSServices`). Anything else is refused.
fn mode_from_user_data(user_data: &str) -> Option<CameraCaptureMode> {
    match user_data {
        "photo" => Some(CameraCaptureMode::Photo),
        "scan" => Some(CameraCaptureMode::Scan),
        _ => None,
    }
}

/// Peer free text made safe for a banner: no control characters, at most
/// 120 characters.
fn sanitize_reason(reason: &str) -> String {
    reason.chars().filter(|c| !c.is_control()).take(120).collect::<String>().trim().to_owned()
}

/// An `NSPasteboardType` (an `NSString`) for a format UTI, as in
/// `mac/clipboard.rs` (string constants, so no extern statics are read).
fn pasteboard_type(uti: &str) -> objc2::rc::Retained<NSString> {
    NSString::from_str(uti)
}

fn refusal_message(refusal: &ServiceRefusal) -> String {
    match refusal {
        ServiceRefusal::Busy => {
            tr!("MacServices", "A photo or scan is already in progress in Waypair Link.")
        }
        ServiceRefusal::NoPairedDevice => tr!("MacServices", "Pair your phone first."),
        ServiceRefusal::Offline { device_name } => {
            tr!("MacServices", "%1 isn’t connected.", device_name)
        }
        ServiceRefusal::PhotosOff { device_name } => {
            tr!("MacServices", "Allow Photos for %1 in Waypair Link.", device_name)
        }
        ServiceRefusal::Unsupported { device_name } => {
            tr!("MacServices", "%1 can’t capture photos for other apps.", device_name)
        }
        ServiceRefusal::CoreNotReady => {
            tr!("MacServices", "Waypair Link is still starting; try again in a moment.")
        }
    }
}

/// Value of Foundation's `NSDefaultRunLoopMode`. The constant's name is
/// not the mode string; waiting in a mode that has no sources returns
/// immediately.
const DEFAULT_RUN_LOOP_MODE: &str = "kCFRunLoopDefaultMode";

/// Uncompressed RGBA size at which TIFF is skipped. PNG still satisfies
/// the declared return types. Phone bytes are decoded with the existing
/// bounded Qt path before this is measured.
const MAX_TIFF_UNCOMPRESSED_BYTES: u64 = 64 * 1024 * 1024;

fn tiff_pixels_within_bound(width: u32, height: u32) -> bool {
    let pixels = u64::from(width).saturating_mul(u64::from(height));
    width > 0 && height > 0 && pixels.saturating_mul(4) <= MAX_TIFF_UNCOMPRESSED_BYTES
}

/// Pumps one slice of AppKit events in the real default run-loop mode.
/// `nextEventMatchingMask` runs that mode until an event arrives or the
/// slice ends, so Qt posted events and timers keep firing and the window
/// can Cancel or Quit. An early return with no event sleeps out the
/// rest of the slice, so an empty mode cannot spin.
fn pump_slice() {
    use objc2_app_kit::{NSApplication, NSEventMask};
    use objc2_foundation::NSDate;
    let Some(mtm) = MainThreadMarker::new() else {
        std::thread::sleep(SLICE);
        return;
    };
    let started = Instant::now();
    let app = NSApplication::sharedApplication(mtm);
    let mode = NSString::from_str(DEFAULT_RUN_LOOP_MODE);
    let until = NSDate::dateWithTimeIntervalSinceNow(SLICE.as_secs_f64());
    let event =
        app.nextEventMatchingMask_untilDate_inMode_dequeue(NSEventMask::Any, Some(&until), &mode, true);
    if let Some(event) = event {
        app.sendEvent(&event);
        app.updateWindows();
        return;
    }
    let spent = started.elapsed();
    if spent < SLICE {
        std::thread::sleep(SLICE - spent);
    }
}

/// The frontmost app at entry, labelled the requesting app in banners.
/// Never claimed to be the app that will actually receive the image —
/// AppKit decides that — and no banner ever says "pasted"/"inserted".
fn requesting_app_name() -> String {
    use objc2_app_kit::NSWorkspace;
    if MainThreadMarker::new().is_none() {
        return "the requesting app".into();
    }
    NSWorkspace::sharedWorkspace()
        .frontmostApplication()
        .and_then(|app| app.localizedName())
        .map(|name| name.to_string())
        .filter(|name| !name.trim().is_empty())
        .unwrap_or_else(|| "the requesting app".into())
}

/// Converts the phone's image into the declared return types. PNG is
/// the phone's PNG bytes only when they really are PNG; anything else
/// goes through the existing bounded decode + PNG encode. TIFF is the
/// Qt-normalized PNG of that decode, and only when the raster is within
/// [`MAX_TIFF_UNCOMPRESSED_BYTES`]. The original phone bytes are not
/// handed to AppKit. A larger image still returns PNG. Either may be
/// absent; only real conversions are ever written to the caller's
/// pasteboard.
fn encoded_images(mime: &str, data: &[u8]) -> Result<(Option<Vec<u8>>, Option<Vec<u8>>), String> {
    if data.is_empty() || data.len() as u64 > nectarlink_core::CAMERA_CAPTURE_MAX_IMAGE_BYTES {
        return Err("the image from the phone is empty or too large".into());
    }
    let png_passthrough = mime == "image/png" && data.starts_with(b"\x89PNG\r\n\x1a\n");
    // A PNG signature alone must not bypass the decoder's allocation and
    // validity checks before returning the image to another application.
    let decoded = crate::win::image::decode(data)?;
    let png =
        if png_passthrough { Some(data.to_vec()) } else { crate::win::image::encode_png(&decoded).ok() };
    let tiff = if tiff_pixels_within_bound(decoded.width, decoded.height) {
        crate::win::image::encode_png(&decoded).ok().as_deref().and_then(tiff_bytes)
    } else {
        None
    };
    if png.is_none() && tiff.is_none() {
        return Err("the image from the phone couldn’t be converted".into());
    }
    Ok((png, tiff))
}

/// TIFF of an already size-checked image, or `None` when AppKit can't
/// decode it or the TIFF itself exceeds [`MAX_TIFF_UNCOMPRESSED_BYTES`].
/// Main thread (AppKit image objects).
fn tiff_bytes(data: &[u8]) -> Option<Vec<u8>> {
    use objc2::AnyThread;
    use objc2_app_kit::NSBitmapImageRep;
    if MainThreadMarker::new().is_none() {
        return None;
    }
    let rep = NSBitmapImageRep::initWithData(NSBitmapImageRep::alloc(), &NSData::with_bytes(data))?;
    let tiff = rep.TIFFRepresentation()?.to_vec();
    if tiff.len() as u64 > MAX_TIFF_UNCOMPRESSED_BYTES { None } else { Some(tiff) }
}

/// Writes the converted image to the *caller's* pasteboard only. The
/// general pasteboard is never touched from here.
fn write_caller_pasteboard(
    pboard: &NSPasteboard,
    png: Option<&[u8]>,
    tiff: Option<&[u8]>,
) -> Result<(), String> {
    pboard.clearContents();
    let mut wrote = false;
    if let Some(png) = png {
        wrote |= pboard.setData_forType(Some(&NSData::with_bytes(png)), &pasteboard_type(TYPE_PNG));
    }
    if let Some(tiff) = tiff {
        wrote |= pboard.setData_forType(Some(&NSData::with_bytes(tiff)), &pasteboard_type(TYPE_TIFF));
    }
    if wrote { Ok(()) } else { Err("couldn’t put the image on the requesting app’s pasteboard".into()) }
}

/// Inputs to [`wait_stop`]. Evaluated before the next queued
/// [`ServiceEvent`] is read.
#[derive(Clone, Copy)]
struct WaitGate {
    shutting_down: bool,
    elapsed: Duration,
    acked: bool,
}

/// Why the wait must stop without delivering a queued image or treating
/// a late acknowledgement as success. The 10 s ack deadline and the 90 s
/// total deadline stay distinct.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WaitStop {
    Shutdown,
    NoAck,
    Expired,
}

fn wait_stop(gate: WaitGate) -> Option<WaitStop> {
    if gate.shutting_down {
        return Some(WaitStop::Shutdown);
    }
    if !gate.acked && gate.elapsed >= ACK_TIMEOUT {
        return Some(WaitStop::NoAck);
    }
    if gate.elapsed >= TOTAL_TIMEOUT {
        return Some(WaitStop::Expired);
    }
    None
}

fn finish_wait(
    ticket: &ServiceTicket,
    requesting_app: &str,
    stop: WaitStop,
) -> Result<(String, Vec<u8>), String> {
    match stop {
        WaitStop::Shutdown => {
            continuity_camera::cancel_service("cancelled");
            Err(tr!("MacServices", "Camera capture cancelled on %1.", ticket.device_name))
        }
        WaitStop::NoAck => {
            continuity_camera::cancel_service("no_ack");
            Err(tr!("MacServices", "%1 didn’t answer the capture request.", ticket.device_name))
        }
        WaitStop::Expired => {
            continuity_camera::cancel_service("expired");
            Err(tr!(
                "MacServices",
                "The capture from %1 didn’t finish in time, so nothing was returned to %2.",
                ticket.device_name,
                requesting_app
            ))
        }
    }
}

/// Waits for the ticket's terminal event, running the main run loop in
/// slices. On success returns the image's `(mime, data)`; on any failure
/// returns the banner/error text (already localized, device named).
fn wait_for_image(ticket: &ServiceTicket, requesting_app: &str) -> Result<(String, Vec<u8>), String> {
    let started = Instant::now();
    let mut acked = false;
    loop {
        // Quit, the ack deadline, and the total deadline are decided
        // before this iteration reads a queued event. A result or a
        // first acknowledgement that becomes visible only after one of
        // those limits is not delivered. A run-loop slice is not itself
        // a preemption bound.
        if let Some(stop) = wait_stop(WaitGate {
            shutting_down: SHUTTING_DOWN.load(Ordering::Acquire),
            elapsed: started.elapsed(),
            acked,
        }) {
            return finish_wait(ticket, requesting_app, stop);
        }
        match ticket.receiver.try_recv() {
            Ok(ServiceEvent::Accepted) => {
                acked = true;
                let asking = match ticket.mode {
                    CameraCaptureMode::Photo => {
                        tr!(
                            "MacServices",
                            "Take the photo on %1 (%2 waits up to 90 s; Esc stops waiting)",
                            ticket.device_name,
                            requesting_app
                        )
                    }
                    CameraCaptureMode::Scan => {
                        tr!(
                            "MacServices",
                            "Scan the document on %1 (%2 waits up to 90 s; Esc stops waiting)",
                            ticket.device_name,
                            requesting_app
                        )
                    }
                };
                crate::bridge::app::show_message(asking);
            }
            Ok(ServiceEvent::Received { mime, data }) => return Ok((mime, data)),
            Ok(ServiceEvent::Cancelled { reason }) => {
                return Err(match reason.as_deref() {
                    Some("busy") => {
                        tr!("MacServices", "Camera is busy on %1.", ticket.device_name)
                    }
                    Some("permission_denied") => {
                        tr!("MacServices", "Camera permission was denied on %1.", ticket.device_name)
                    }
                    Some(raw) => {
                        let reason = sanitize_reason(raw);
                        if reason.is_empty() {
                            tr!("MacServices", "Camera capture cancelled on %1.", ticket.device_name)
                        } else {
                            tr!(
                                "MacServices",
                                "Camera capture cancelled on %1: %2",
                                ticket.device_name,
                                reason
                            )
                        }
                    }
                    None => tr!("MacServices", "Camera capture cancelled on %1.", ticket.device_name),
                });
            }
            Ok(ServiceEvent::CancelledLocally) => {
                return Err(tr!("MacServices", "Camera capture cancelled on %1.", ticket.device_name));
            }
            Ok(ServiceEvent::Disconnected) => {
                return Err(tr!("MacServices", "%1 isn’t connected.", ticket.device_name));
            }
            Ok(ServiceEvent::Failed(message)) => return Err(message),
            Err(mpsc::TryRecvError::Empty) => {}
            Err(mpsc::TryRecvError::Disconnected) => {
                return Err(tr!("MacServices", "%1 isn’t connected.", ticket.device_name));
            }
        }
        pump_slice();
    }
}

/// The one Service entry point (both `NSUserData` values share it, as
/// declared in the packaging snippet). Synchronous until the caller's
/// pasteboard is written or the wait gives up; the returned `Err` text
/// is only console-logged by AppKit, so every failure is also bannered.
fn handle_capture(pboard: &NSPasteboard, user_data: &str) -> Result<(), String> {
    if MainThreadMarker::new().is_none() {
        // AppKit delivers Service calls on the main run loop; anything
        // else fails closed and starts no request.
        return Err("Waypair Link received a capture request off the main thread".into());
    }
    let Some(mode) = mode_from_user_data(user_data) else {
        return Err(format!("Waypair Link doesn’t know the capture kind “{user_data}”"));
    };
    let requesting_app = requesting_app_name();
    // After shutdown, including a quit that arrived on a nested run-loop
    // slice, refuse before any new Service request is installed.
    if SHUTTING_DOWN.load(Ordering::Acquire) {
        return Err("Waypair Link is closing".into());
    }
    let ticket = match continuity_camera::begin_service(mode) {
        Ok(ticket) => ticket,
        Err(refusal) => {
            let message = refusal_message(&refusal);
            crate::bridge::app::show_message(message.clone());
            return Err(message);
        }
    };
    crate::bridge::app::show_message(tr!("MacServices", "Asking %1…", ticket.device_name));
    match wait_for_image(&ticket, &requesting_app) {
        Ok((mime, data)) => match encoded_images(&mime, &data) {
            Ok((png, tiff)) => match write_caller_pasteboard(pboard, png.as_deref(), tiff.as_deref()) {
                Ok(()) => {
                    let returned = match ticket.mode {
                        CameraCaptureMode::Photo => tr!(
                            "MacServices",
                            "Photo from %1 returned to %2",
                            ticket.device_name,
                            requesting_app
                        ),
                        CameraCaptureMode::Scan => tr!(
                            "MacServices",
                            "Scanned document from %1 returned to %2",
                            ticket.device_name,
                            requesting_app
                        ),
                    };
                    crate::bridge::app::show_message(returned);
                    Ok(())
                }
                Err(message) => {
                    crate::bridge::app::show_message(message.clone());
                    Err(message)
                }
            },
            Err(detail) => {
                let message = tr!(
                    "MacServices",
                    "The image from %1 couldn’t be prepared: %2",
                    ticket.device_name,
                    detail
                );
                crate::bridge::app::show_message(message.clone());
                Err(message)
            }
        },
        Err(message) => {
            crate::bridge::app::show_message(message.clone());
            Err(message)
        }
    }
}

/// The AppKit Services provider object. Stateless — all capture state
/// lives in `crate::continuity_camera` — and kept alive for the whole
/// process (see [`register`]), because AppKit holds providers weakly
/// enough that a dropped provider silently unlists the Services.
#[allow(unsafe_code)]
mod provider {
    use objc2::rc::Retained;
    use objc2::{MainThreadMarker, MainThreadOnly, define_class, msg_send};
    use objc2_app_kit::NSPasteboard;
    use objc2_foundation::{NSObject, NSObjectProtocol, NSString};

    #[derive(Debug)]
    pub struct ProviderIvars;

    define_class!(
        // SAFETY:
        // - The superclass `NSObject` has no subclassing requirements.
        // - `ServiceProvider` does not implement `Drop`.
        #[name = "WaypairLinkServicesProvider"]
        #[unsafe(super = NSObject)]
        #[thread_kind = MainThreadOnly]
        #[ivars = ProviderIvars]
        pub struct ServiceProvider;

        // SAFETY: `NSObjectProtocol` has no safety requirements.
        unsafe impl NSObjectProtocol for ServiceProvider {}

        impl ServiceProvider {
            /// `waypairCapture:userData:error:` — the `NSMessage` both
            /// Services declare. Runs on the main thread (AppKit).
            #[unsafe(method(waypairCapture:userData:error:))]
            fn capture(
                &self,
                pboard: &NSPasteboard,
                user_data: Option<&NSString>,
                error: *mut *mut NSString,
            ) {
                let user_data = user_data.map(|value| value.to_string()).unwrap_or_default();
                if let Err(message) = super::handle_capture(pboard, &user_data)
                    && !error.is_null()
                {
                    // SAFETY: `error` is the caller's non-null `NSString**`
                    // out slot. `autorelease_ptr` hands AppKit a +0 string
                    // in the current pool, which it reads before the pool
                    // drains.
                    unsafe {
                        *error = Retained::autorelease_ptr(NSString::from_str(&message));
                    }
                }
            }
        }
    );

    impl ServiceProvider {
        pub fn new(mtm: MainThreadMarker) -> Retained<Self> {
            let this = Self::alloc(mtm).set_ivars(ProviderIvars);
            // SAFETY: The signature of `NSObject`'s `init` is correct.
            unsafe { msg_send![super(this), init] }
        }
    }
}

use provider::ServiceProvider;

/// Registers the Services provider, once, on the main thread: after the
/// QML engine exists and just before `app.exec()`, since requests can
/// arrive as soon as the provider is set. An existing provider (Qt's or
/// a plugin's) is left alone and logged, never overridden.
#[allow(unsafe_code)]
pub fn register() {
    use objc2_app_kit::NSApplication;
    if SHUTTING_DOWN.load(Ordering::Acquire) {
        return;
    }
    if REGISTERED.swap(true, Ordering::AcqRel) {
        return;
    }
    let Some(mtm) = MainThreadMarker::new() else {
        tracing::warn!("Services provider must be registered on the main thread; skipping");
        REGISTERED.store(false, Ordering::Release);
        return;
    };
    let app = NSApplication::sharedApplication(mtm);
    if app.servicesProvider().is_some() {
        tracing::warn!("an NSApplication services provider is already set; leaving it in place");
        // Not ours: `unregister` must not clear someone else's provider.
        REGISTERED.store(false, Ordering::Release);
        return;
    }
    let provider = ServiceProvider::new(mtm);
    let provider_object: objc2::rc::Retained<objc2::runtime::AnyObject> = provider.clone().into();
    // SAFETY: `provider_object` is our `ServiceProvider`, which
    // implements `waypairCapture:userData:error:` as declared.
    unsafe {
        app.setServicesProvider(Some(&provider_object));
    }
    // Process-lifetime ownership: the provider must never be
    // deallocated while AppKit may still call it, so both retains are
    // deliberately leaked (one object for the life of the process).
    std::mem::forget(provider);
    std::mem::forget(provider_object);
    tracing::info!("registered macOS Services provider (Take Photo / Scan Document)");
}

/// Unregisters the provider and cancels any in-flight Service capture.
/// Idempotent. `aboutToQuit` calls this while a nested run-loop slice may
/// still be inside [`wait_for_image`], so quit does not wait out the
/// remaining deadline; `main` calls it again after `app.exec()` returns.
/// A provider this process did not install is left in place.
#[allow(unsafe_code)]
pub fn unregister() {
    use objc2_app_kit::NSApplication;
    SHUTTING_DOWN.store(true, Ordering::Release);
    continuity_camera::cancel_service("cancelled");
    if !REGISTERED.swap(false, Ordering::AcqRel) {
        return;
    }
    let Some(mtm) = MainThreadMarker::new() else { return };
    let app = NSApplication::sharedApplication(mtm);
    // SAFETY: clearing the provider we set in `register`.
    unsafe {
        app.setServicesProvider(None);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn png_signature_without_a_decodable_image_is_refused() {
        assert!(encoded_images("image/png", b"\x89PNG\r\n\x1a\n").is_err());
    }

    #[test]
    fn user_data_selects_photo_or_scan_and_rejects_unknown() {
        assert_eq!(mode_from_user_data("photo"), Some(CameraCaptureMode::Photo));
        assert_eq!(mode_from_user_data("scan"), Some(CameraCaptureMode::Scan));
        assert_eq!(mode_from_user_data(""), None);
        assert_eq!(mode_from_user_data("video"), None);
    }

    #[test]
    fn shutdown_and_expiry_beat_a_queued_result() {
        // `wait_for_image` applies this gate before `try_recv`. None is
        // the only state in which a queued image may be returned.
        let open = WaitGate { shutting_down: false, elapsed: Duration::from_secs(1), acked: true };
        assert_eq!(wait_stop(open), None);
        assert_eq!(wait_stop(WaitGate { shutting_down: true, ..open }), Some(WaitStop::Shutdown));
        assert_eq!(wait_stop(WaitGate { elapsed: TOTAL_TIMEOUT, ..open }), Some(WaitStop::Expired));
        assert_eq!(wait_stop(WaitGate { elapsed: ACK_TIMEOUT, acked: false, ..open }), Some(WaitStop::NoAck));
        // A first acknowledgement after 10 s does not clear that
        // deadline, and an ack does not extend the 90 s total.
        assert_eq!(wait_stop(WaitGate { elapsed: ACK_TIMEOUT, ..open }), None);
        assert_eq!(wait_stop(WaitGate { elapsed: TOTAL_TIMEOUT - Duration::from_millis(1), ..open }), None);
    }

    #[test]
    fn peer_reason_is_sanitized_for_banners() {
        assert_eq!(sanitize_reason("user\ndismissed\u{7}"), "userdismissed");
        assert_eq!(sanitize_reason("  busy  "), "busy");
        let long = "x".repeat(200);
        assert_eq!(sanitize_reason(&long).chars().count(), 120);
    }
}
