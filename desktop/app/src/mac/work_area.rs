// SPDX-License-Identifier: GPL-3.0-or-later
//! The primary display's work area (the screen minus the menu bar and Dock),
//! read from public `NSScreen` metadata on the main thread. Query only.

#![allow(unsafe_code)]

use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    time::Duration,
};

use dispatch2::DispatchQueue;
use objc2::{
    MainThreadMarker,
    encode::{Encode, Encoding, RefEncode},
    msg_send,
    rc::Retained,
    runtime::{AnyClass, AnyObject},
};

/// `(left, top, width, height)`.
type Area = (i32, i32, i32, i32);

const MAIN_THREAD_TIMEOUT: Duration = Duration::from_millis(500);
const MAX_SCALE: f64 = 8.0;
const MAX_EDGE_PX: f64 = 65_536.0;

#[repr(C)]
#[derive(Clone, Copy, Debug)]
struct CgPoint {
    x: f64,
    y: f64,
}

// SAFETY: `CGGeometry.h` names this struct `CGPoint`; `CGFloat` is `f64` on LP64.
unsafe impl Encode for CgPoint {
    const ENCODING: Encoding = Encoding::Struct("CGPoint", &[f64::ENCODING, f64::ENCODING]);
}

// SAFETY: pointer encoding follows the named struct.
unsafe impl RefEncode for CgPoint {
    const ENCODING_REF: Encoding = Encoding::Pointer(&Self::ENCODING);
}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
struct CgSize {
    width: f64,
    height: f64,
}

// SAFETY: `CGGeometry.h` names this struct `CGSize`; `CGFloat` is `f64` on LP64.
unsafe impl Encode for CgSize {
    const ENCODING: Encoding = Encoding::Struct("CGSize", &[f64::ENCODING, f64::ENCODING]);
}

// SAFETY: pointer encoding follows the named struct.
unsafe impl RefEncode for CgSize {
    const ENCODING_REF: Encoding = Encoding::Pointer(&Self::ENCODING);
}

/// `NSRect` is `CGRect` on 64-bit macOS.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
struct CgRect {
    origin: CgPoint,
    size: CgSize,
}

// SAFETY: `CGGeometry.h` names this struct `CGRect`.
unsafe impl Encode for CgRect {
    const ENCODING: Encoding = Encoding::Struct("CGRect", &[CgPoint::ENCODING, CgSize::ENCODING]);
}

// SAFETY: pointer encoding follows the named struct.
unsafe impl RefEncode for CgRect {
    const ENCODING_REF: Encoding = Encoding::Pointer(&Self::ENCODING);
}

/// The primary display's work area as `(left, top, width, height)` in backing
/// pixels, measured from that display's top-left corner (Qt's global origin).
/// Dividing by the screen's `devicePixelRatio` gives Qt logical coordinates.
/// `None` without a usable screen, or when the main thread does not answer in time.
pub fn get() -> Option<Area> {
    on_main(query)
}

/// Runs `f` on the main thread, waiting at most [`MAIN_THREAD_TIMEOUT`].
fn on_main(f: fn() -> Option<Area>) -> Option<Area> {
    if MainThreadMarker::new().is_some() {
        return f();
    }
    let abandoned = Arc::new(AtomicBool::new(false));
    let queued_abandoned = Arc::clone(&abandoned);
    let (tx, rx) = mpsc::sync_channel(1);
    DispatchQueue::main().exec_async(move || {
        if !queued_abandoned.load(Ordering::Acquire) {
            let _ = tx.send(f());
        }
    });
    let area = rx.recv_timeout(MAIN_THREAD_TIMEOUT).ok().flatten();
    abandoned.store(true, Ordering::Release);
    area
}

/// Main thread only.
fn query() -> Option<Area> {
    objc2::rc::autoreleasepool(|_| {
        let cls = AnyClass::get(c"NSScreen")?;
        // SAFETY: `+[NSScreen screens]` returns an `NSArray` (nil without a window server).
        let screens: Option<Retained<AnyObject>> = unsafe { msg_send![cls, screens] };
        let screens = screens?;
        // `screens[0]` carries the menu bar and anchors global coordinates;
        // `mainScreen` follows the key window instead.
        // SAFETY: `-[NSArray firstObject]` returns an element or nil.
        let primary: Option<Retained<AnyObject>> = unsafe { msg_send![&*screens, firstObject] };
        let primary = primary?;
        // SAFETY: `NSScreen` getters returning `NSRect` and `CGFloat`.
        let (frame, visible, scale): (CgRect, CgRect, f64) = unsafe {
            (
                msg_send![&*primary, frame],
                msg_send![&*primary, visibleFrame],
                msg_send![&*primary, backingScaleFactor],
            )
        };
        to_physical(frame, visible, scale)
    })
}

/// Converts Cocoa point rects (y grows upward from the bottom edge) into a
/// top-left-origin pixel rect relative to `frame`, clipped to `frame`.
fn to_physical(frame: CgRect, visible: CgRect, scale: f64) -> Option<Area> {
    let values = [
        frame.origin.x,
        frame.origin.y,
        frame.size.width,
        frame.size.height,
        visible.origin.x,
        visible.origin.y,
        visible.size.width,
        visible.size.height,
        scale,
    ];
    if !values.iter().all(|v| v.is_finite())
        || scale <= 0.0
        || scale > MAX_SCALE
        || frame.size.width <= 0.0
        || frame.size.height <= 0.0
    {
        return None;
    }
    let frame_right = frame.origin.x + frame.size.width;
    let frame_top = frame.origin.y + frame.size.height;
    let left = visible.origin.x.max(frame.origin.x) - frame.origin.x;
    let right = (visible.origin.x + visible.size.width).min(frame_right) - frame.origin.x;
    let top = frame_top - (visible.origin.y + visible.size.height).min(frame_top);
    let bottom = frame_top - visible.origin.y.max(frame.origin.y);
    let px = |points: f64| {
        let pixels = (points * scale).round();
        (0.0..=MAX_EDGE_PX).contains(&pixels).then_some(pixels as i32)
    };
    let (l, t, r, b) = (px(left)?, px(top)?, px(right)?, px(bottom)?);
    (r > l && b > t).then_some((l, t, r - l, b - t))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(x: f64, y: f64, width: f64, height: f64) -> CgRect {
        CgRect { origin: CgPoint { x, y }, size: CgSize { width, height } }
    }

    #[test]
    fn retina_menu_bar_and_bottom_dock_flip_to_top_left_pixels() {
        let frame = rect(0.0, 0.0, 1512.0, 982.0);
        let visible = rect(0.0, 70.0, 1512.0, 887.0);
        assert_eq!(to_physical(frame, visible, 2.0), Some((0, 50, 3024, 1774)));
    }

    #[test]
    fn left_dock_at_1x_and_invalid_metadata() {
        let frame = rect(0.0, 0.0, 1920.0, 1080.0);
        let visible = rect(64.0, 0.0, 1856.0, 1055.0);
        assert_eq!(to_physical(frame, visible, 1.0), Some((64, 25, 1856, 1055)));
        assert_eq!(to_physical(frame, visible, f64::NAN), None);
        assert_eq!(to_physical(frame, visible, 0.0), None);
        assert_eq!(to_physical(frame, rect(3000.0, 0.0, 10.0, 10.0), 1.0), None);
        assert_eq!(to_physical(rect(0.0, 0.0, 0.0, 1080.0), visible, 1.0), None);
    }
}
