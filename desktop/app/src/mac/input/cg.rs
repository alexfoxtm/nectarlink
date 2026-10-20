// SPDX-License-Identifier: GPL-3.0-or-later
//! Public CoreGraphics posting. Symbols and numeric values are taken from the
//! installed macOS SDK:
//! `CoreGraphics.framework/Headers/CGEvent.h`, `CGEventTypes.h`,
//! `CGEventSource.h`, `CGDirectDisplay.h`, `CGGeometry.h`,
//! `ApplicationServices/.../HIServices/AXUIElement.h`,
//! `IOKit/.../hidsystem/IOLLEvent.h`, and `ev_keymap.h`.
//!
//! Trust is `AXIsProcessTrusted` only. `AXIsProcessTrustedWithOptions` and
//! `kAXTrustedCheckOptionPrompt` are not linked, so this code cannot raise
//! the Accessibility prompt or grant itself trust.

#![allow(unsafe_code)]

use std::{
    ffi::c_void,
    ptr,
    sync::{Once, OnceLock},
};

use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2::{ClassType, msg_send};
use objc2_app_kit::{NSEvent, NSEventModifierFlags, NSEventType};
use objc2_foundation::NSPoint;

/// `kCGEventSourceStateCombinedSessionState`.
const COMBINED_SESSION: i32 = 0;
/// `kCGEventSourceStateHIDSystemState`.
const HID_STATE: i32 = 1;
/// `kCGAnyInputEventType` (`(CGEventType)~0`).
const ANY_INPUT: u32 = u32::MAX;
/// `kCGHIDEventTap`. Posting here is not an event tap; taps at this point are
/// root-only, posts are the public injection point used by Quartz samples.
const HID_TAP: u32 = 0;
/// `kCGScrollEventUnitLine`.
const SCROLL_LINE: u32 = 1;
/// `kCGErrorSuccess`.
const CG_OK: i32 = 0;

pub const LEFT_DOWN: u32 = 1; // kCGEventLeftMouseDown / NX_LMOUSEDOWN
pub const LEFT_UP: u32 = 2;
pub const RIGHT_DOWN: u32 = 3;
pub const RIGHT_UP: u32 = 4;
pub const MOVED: u32 = 5;
pub const LEFT_DRAGGED: u32 = 6;
pub const RIGHT_DRAGGED: u32 = 7;
pub const OTHER_DOWN: u32 = 25;
pub const OTHER_UP: u32 = 26;
pub const OTHER_DRAGGED: u32 = 27;

pub const BUTTON_LEFT: u32 = 0; // kCGMouseButtonLeft
pub const BUTTON_RIGHT: u32 = 1;
pub const BUTTON_CENTER: u32 = 2;

const FIELD_CLICK_STATE: u32 = 1; // kCGMouseEventClickState
const FIELD_BUTTON: u32 = 3; // kCGMouseEventButtonNumber
const FIELD_DELTA_X: u32 = 4; // kCGMouseEventDeltaX
const FIELD_DELTA_Y: u32 = 5; // kCGMouseEventDeltaY
const FIELD_SUBTYPE: u32 = 7; // kCGMouseEventSubtype
const FIELD_AUTOREPEAT: u32 = 8; // kCGKeyboardEventAutorepeat
const FIELD_SCROLL_1: u32 = 11; // kCGScrollWheelEventDeltaAxis1
const FIELD_SCROLL_2: u32 = 12; // kCGScrollWheelEventDeltaAxis2
const FIELD_SCROLL_CONTINUOUS: u32 = 88; // kCGScrollWheelEventIsContinuous
/// `kCGEventMouseSubtypeDefault`. Never a tablet or touch subtype.
const MOUSE_SUBTYPE_DEFAULT: i64 = 0;

/// `NX_SUBTYPE_AUX_CONTROL_BUTTONS`.
const AUX_CONTROL: i16 = 8;
/// `NX_KEYDOWN` / `NX_KEYUP`, packed into the aux-control event the way
/// AppKit's system-defined consumer keys carry key state (`state << 8`).
const NX_KEYDOWN: isize = 10;
const NX_KEYUP: isize = 11;

#[repr(C)]
#[derive(Clone, Copy)]
struct CgPoint {
    x: f64,
    y: f64,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct CgSize {
    width: f64,
    height: f64,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct CgRect {
    origin: CgPoint,
    size: CgSize,
}

#[derive(Clone, Copy, Debug)]
pub struct Rect {
    pub origin_x: f64,
    pub origin_y: f64,
    pub width: f64,
    pub height: f64,
}

/// Inclusive edges of the online-display bounding box, in the same top-left
/// global space `CGEvent` locations use.
#[derive(Clone, Copy, Debug)]
pub struct Edges {
    pub min_x: f64,
    pub min_y: f64,
    pub max_x: f64,
    pub max_y: f64,
}

#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" {
    safe fn CGEventSourceSecondsSinceLastEventType(state_id: i32, event_type: u32) -> f64;
    /// Current modifier flags for a source state. This reads; it does not post.
    safe fn CGEventSourceFlagsState(state_id: i32) -> u64;
    safe fn CGMainDisplayID() -> u32;
    safe fn CGDisplayBounds(display: u32) -> CgRect;

    fn CGGetOnlineDisplayList(max_displays: u32, displays: *mut u32, count: *mut u32) -> i32;
    fn CGEventSourceCreate(state_id: i32) -> *mut c_void;
    fn CGEventSourceSetLocalEventsSuppressionInterval(source: *mut c_void, seconds: f64);
    fn CGEventCreate(source: *mut c_void) -> *mut c_void;
    fn CGEventGetLocation(event: *mut c_void) -> CgPoint;
    fn CGEventCreateMouseEvent(
        source: *mut c_void,
        mouse_type: u32,
        position: CgPoint,
        button: u32,
    ) -> *mut c_void;
    fn CGEventCreateKeyboardEvent(source: *mut c_void, key: u16, down: bool) -> *mut c_void;
    fn CGEventCreateScrollWheelEvent2(
        source: *mut c_void,
        units: u32,
        wheel_count: u32,
        wheel1: i32,
        wheel2: i32,
        wheel3: i32,
    ) -> *mut c_void;
    fn CGEventKeyboardSetUnicodeString(event: *mut c_void, len: core::ffi::c_ulong, units: *const u16);
    fn CGEventSetFlags(event: *mut c_void, flags: u64);
    fn CGEventSetIntegerValueField(event: *mut c_void, field: u32, value: i64);
    fn CGEventSetSource(event: *mut c_void, source: *mut c_void);
    fn CGEventPost(tap: u32, event: *mut c_void);
}

#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    fn CFRelease(cf: *const c_void);
}

#[link(name = "ApplicationServices", kind = "framework")]
unsafe extern "C" {
    /// Does not prompt. A zero result means this process must not post events.
    safe fn AXIsProcessTrusted() -> u8;
}

static HOOK: OnceLock<fn()> = OnceLock::new();
static ARMED: Once = Once::new();

unsafe extern "C" fn teardown_trampoline() {
    if let Some(hook) = HOOK.get() {
        hook();
    }
}

/// Registers `hook` once so process exit can lift keys this backend left down.
pub fn arm_teardown(hook: fn()) {
    let _ = HOOK.set(hook);
    ARMED.call_once(|| unsafe {
        unsafe extern "C" {
            fn atexit(cb: Option<unsafe extern "C" fn()>) -> i32;
        }
        let _ = atexit(Some(teardown_trampoline));
    });
}

pub fn trusted() -> bool {
    AXIsProcessTrusted() != 0
}

pub fn idle_seconds() -> f64 {
    CGEventSourceSecondsSinceLastEventType(COMBINED_SESSION, ANY_INPUT)
}

/// One `CGEventSourceFlagsState` snapshot for the combined session.
///
/// Callers treat bits they do not own as physically held. A modifier that
/// changes after this read is an acceptance gate, not something a second
/// guess in this process can prove.
pub fn flags_state() -> u64 {
    CGEventSourceFlagsState(COMBINED_SESSION)
}

fn release(ptr: *mut c_void) {
    if !ptr.is_null() {
        unsafe { CFRelease(ptr.cast()) }
    }
}

fn nonempty(rect: CgRect) -> Option<Rect> {
    if !rect.origin.x.is_finite()
        || !rect.origin.y.is_finite()
        || !rect.size.width.is_finite()
        || !rect.size.height.is_finite()
        || rect.size.width <= 1.0
        || rect.size.height <= 1.0
    {
        return None;
    }
    Some(Rect {
        origin_x: rect.origin.x,
        origin_y: rect.origin.y,
        width: rect.size.width,
        height: rect.size.height,
    })
}

pub fn primary_bounds() -> Option<Rect> {
    nonempty(CGDisplayBounds(CGMainDisplayID()))
}

pub fn desktop_edges() -> Option<Edges> {
    // A NULL list reports the real online count (`CGDirectDisplay.h`). The
    // fill below stores at most `DISPLAY_LIST_CAP` ids, so a larger count is
    // an incomplete union and must not be clamped.
    let mut total = 0u32;
    let probe = unsafe { CGGetOnlineDisplayList(0, ptr::null_mut(), &mut total) };
    if probe == CG_OK && total > super::plan::DISPLAY_LIST_CAP {
        return None;
    }
    let mut ids = [0u32; super::plan::DISPLAY_LIST_CAP as usize];
    let mut count = 0u32;
    let err = unsafe { CGGetOnlineDisplayList(ids.len() as u32, ids.as_mut_ptr(), &mut count) };
    let rects: Vec<Rect> = if err != CG_OK || count == 0 {
        vec![primary_bounds()?]
    } else if count > super::plan::DISPLAY_LIST_CAP
        || !super::plan::display_list_fits(count, ids.len() as u32)
    {
        return None;
    } else {
        ids.iter().take(count as usize).filter_map(|id| nonempty(CGDisplayBounds(*id))).collect()
    };
    if rects.is_empty() {
        return None;
    }
    let mut min_x = f64::INFINITY;
    let mut min_y = f64::INFINITY;
    let mut max_x = f64::NEG_INFINITY;
    let mut max_y = f64::NEG_INFINITY;
    for rect in rects {
        min_x = min_x.min(rect.origin_x);
        min_y = min_y.min(rect.origin_y);
        max_x = max_x.max(rect.origin_x + rect.width - 1.0);
        max_y = max_y.max(rect.origin_y + rect.height - 1.0);
    }
    (min_x.is_finite() && max_x >= min_x && max_y >= min_y).then_some(Edges { min_x, min_y, max_x, max_y })
}

/// Current pointer in global top-left coordinates. This reads; it does not post.
pub fn cursor() -> Option<(f64, f64)> {
    let event = unsafe { CGEventCreate(ptr::null_mut()) };
    if event.is_null() {
        return None;
    }
    let point = unsafe { CGEventGetLocation(event) };
    release(event);
    (point.x.is_finite() && point.y.is_finite()).then_some((point.x, point.y))
}

pub struct Session(*mut c_void);

impl Session {
    pub fn new() -> Option<Self> {
        let source = unsafe { CGEventSourceCreate(HID_STATE) };
        if source.is_null() {
            return None;
        }
        // The default 0.25s suppression freezes the physical pointer after a post.
        unsafe { CGEventSourceSetLocalEventsSuppressionInterval(source, 0.0) };
        Some(Self(source))
    }

    pub fn post_mouse(&self, event_type: u32, button: u32, x: f64, y: f64, dx: i64, dy: i64) -> bool {
        if !x.is_finite() || !y.is_finite() {
            return false;
        }
        let event = unsafe { CGEventCreateMouseEvent(self.0, event_type, CgPoint { x, y }, button) };
        if event.is_null() {
            return false;
        }
        unsafe {
            CGEventSetIntegerValueField(event, FIELD_SUBTYPE, MOUSE_SUBTYPE_DEFAULT);
            CGEventSetIntegerValueField(event, FIELD_CLICK_STATE, 1);
            CGEventSetIntegerValueField(event, FIELD_BUTTON, i64::from(button));
            CGEventSetIntegerValueField(event, FIELD_DELTA_X, dx);
            CGEventSetIntegerValueField(event, FIELD_DELTA_Y, dy);
            CGEventPost(HID_TAP, event);
        }
        release(event);
        true
    }

    pub fn post_key(&self, code: u16, down: bool, flags: u64) -> bool {
        let event = unsafe { CGEventCreateKeyboardEvent(self.0, code, down) };
        if event.is_null() {
            return false;
        }
        unsafe {
            CGEventSetFlags(event, flags);
            CGEventSetIntegerValueField(event, FIELD_AUTOREPEAT, 0);
            CGEventPost(HID_TAP, event);
        }
        release(event);
        true
    }

    /// One scalar, as UTF-16, on keycode 0. AppKit honors the Unicode string;
    /// a framework that ignores it and uses the keycode can also see `A`
    /// (`kVK_ANSI_A` is 0). Callers only pass one or two units. `flags` is the
    /// physical baseline plus owned synthetic bits, never a cleared mask.
    pub fn post_unicode(&self, units: &[u16], down: bool, flags: u64) -> bool {
        if units.is_empty() || units.len() > 2 {
            return false;
        }
        let event = unsafe { CGEventCreateKeyboardEvent(self.0, 0, down) };
        if event.is_null() {
            return false;
        }
        unsafe {
            CGEventSetFlags(event, flags);
            CGEventSetIntegerValueField(event, FIELD_AUTOREPEAT, 0);
            CGEventKeyboardSetUnicodeString(event, units.len() as core::ffi::c_ulong, units.as_ptr());
            CGEventPost(HID_TAP, event);
        }
        release(event);
        true
    }

    pub fn post_scroll(&self, vertical: i32, horizontal: i32) -> bool {
        if vertical == 0 && horizontal == 0 {
            return true;
        }
        let event =
            unsafe { CGEventCreateScrollWheelEvent2(self.0, SCROLL_LINE, 2, vertical, horizontal, 0) };
        if event.is_null() {
            return false;
        }
        unsafe {
            CGEventSetIntegerValueField(event, FIELD_SCROLL_CONTINUOUS, 0);
            CGEventSetIntegerValueField(event, FIELD_SCROLL_1, i64::from(vertical));
            CGEventSetIntegerValueField(event, FIELD_SCROLL_2, i64::from(horizontal));
            CGEventPost(HID_TAP, event);
        }
        release(event);
        true
    }

    /// Play, next, and previous have no `kVK_*` keycode. They are consumer
    /// keys: `NSEventTypeSystemDefined` + `NX_SUBTYPE_AUX_CONTROL_BUTTONS`
    /// with an `NX_KEYTYPE_*` usage, then the event's public `CGEvent`.
    pub fn post_media(&self, usage: i32, down: bool) -> bool {
        let state = if down { NX_KEYDOWN } else { NX_KEYUP };
        let flags = NSEventModifierFlags((state as usize) << 8);
        let data1 = ((usage as isize) << 16) | (state << 8);
        let event: Option<Retained<NSEvent>> = unsafe {
            msg_send![
                NSEvent::class(),
                otherEventWithType: NSEventType::SystemDefined,
                location: NSPoint { x: 0.0, y: 0.0 },
                modifierFlags: flags,
                timestamp: 0.0_f64,
                windowNumber: 0_isize,
                context: None::<&AnyObject>,
                subtype: AUX_CONTROL,
                data1: data1,
                data2: -1_isize,
            ]
        };
        let Some(event) = event else { return false };
        let cg: Option<Retained<AnyObject>> = unsafe { msg_send![&*event, CGEvent] };
        let Some(cg) = cg else { return false };
        let raw = Retained::as_ptr(&cg) as *mut c_void;
        if raw.is_null() {
            return false;
        }
        unsafe {
            CGEventSetSource(raw, self.0);
            CGEventPost(HID_TAP, raw);
        }
        true
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        release(self.0);
        self.0 = ptr::null_mut();
    }
}
