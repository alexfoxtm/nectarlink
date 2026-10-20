// SPDX-License-Identifier: GPL-3.0-or-later
//! Pure decisions for screen capture: which frames are kept, how large the
//! encoder picture is, and which picker events are still allowed to start
//! capture. No ScreenCaptureKit calls live here.

#![allow(unsafe_code)]

use objc2::encode::{Encode, Encoding, RefEncode};

/// `SCFrameStatusComplete` (`SCStream.h`).
pub(super) const FRAME_COMPLETE: i64 = 0;
/// `kCVPixelFormatType_420YpCbCr8BiPlanarVideoRange` (`'420v'`).
pub(super) const PIXEL_420V: u32 = 0x3432_3076;
/// `kCFNumberSInt64Type` (`CFNumber.h`).
pub(super) const CF_NUMBER_SINT64: isize = 4;
/// `SCContentSharingPickerModeSingleDisplay` (`1 << 4`).
pub(super) const PICKER_SINGLE_DISPLAY: usize = 1 << 4;
pub(super) const OUTPUT_SCREEN: isize = 0;
pub(super) const QUEUE_DEPTH: isize = 5;
pub(super) const MIN_FPS: u32 = 10;
pub(super) const MAX_FPS: u32 = 60;
pub(super) const MIN_EDGE: u32 = 320;
pub(super) const MAX_EDGE: u32 = 1920;
/// Slot plus the caller's held buffer.
pub(super) const MAX_RETAINED_FRAMES: usize = 2;
/// `SCStreamErrorAttemptToStopStreamState`.
pub(super) const ATTEMPT_TO_STOP: isize = -3808;

pub(super) const PROTOCOLS: [&std::ffi::CStr; 3] =
    [c"SCStreamOutput", c"SCStreamDelegate", c"SCContentSharingPickerObserver"];

/// Anonymous `CMTime` (`CMTime.h`). `@encode` is `{?=qiIq}` because the
/// struct tag is unnamed. 24 bytes, 8-byte aligned on LP64.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub(super) struct CmTime {
    pub(super) value: i64,
    pub(super) timescale: i32,
    pub(super) flags: u32,
    pub(super) epoch: i64,
}

// SAFETY: `repr(C)` matches `CMTime.h`, and the name is `?` because the
// header uses an anonymous struct. Debug `msg_send!` checks this encoding.
unsafe impl Encode for CmTime {
    const ENCODING: Encoding =
        Encoding::Struct("?", &[i64::ENCODING, i32::ENCODING, u32::ENCODING, i64::ENCODING]);
}

// SAFETY: a pointer to `CMTime` is a pointer to that anonymous struct.
unsafe impl RefEncode for CmTime {
    const ENCODING_REF: Encoding = Encoding::Pointer(&Self::ENCODING);
}

/// `CGFloat` on LP64 macOS.
pub(super) type CgFloat = f64;

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub(super) struct CgPoint {
    pub(super) x: CgFloat,
    pub(super) y: CgFloat,
}

// SAFETY: `CGGeometry.h` names this struct `CGPoint`.
unsafe impl Encode for CgPoint {
    const ENCODING: Encoding = Encoding::Struct("CGPoint", &[f64::ENCODING, f64::ENCODING]);
}

// SAFETY: pointer encoding follows the named struct.
unsafe impl RefEncode for CgPoint {
    const ENCODING_REF: Encoding = Encoding::Pointer(&Self::ENCODING);
}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub(super) struct CgSize {
    pub(super) width: CgFloat,
    pub(super) height: CgFloat,
}

// SAFETY: `CGGeometry.h` names this struct `CGSize`.
unsafe impl Encode for CgSize {
    const ENCODING: Encoding = Encoding::Struct("CGSize", &[f64::ENCODING, f64::ENCODING]);
}

// SAFETY: pointer encoding follows the named struct.
unsafe impl RefEncode for CgSize {
    const ENCODING_REF: Encoding = Encoding::Pointer(&Self::ENCODING);
}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub(super) struct CgRect {
    pub(super) origin: CgPoint,
    pub(super) size: CgSize,
}

// SAFETY: `CGGeometry.h` names this struct `CGRect`.
unsafe impl Encode for CgRect {
    const ENCODING: Encoding = Encoding::Struct("CGRect", &[CgPoint::ENCODING, CgSize::ENCODING]);
}

// SAFETY: pointer encoding follows the named struct.
unsafe impl RefEncode for CgRect {
    const ENCODING_REF: Encoding = Encoding::Pointer(&Self::ENCODING);
}

/// `CFStringRef` (`^{__CFString=}`), used for the assign `colorMatrix` property.
#[repr(transparent)]
#[derive(Clone, Copy, Debug)]
pub(super) struct CfStringRef(pub(super) *const std::ffi::c_void);

// SAFETY: `CFStringRef` is `const struct __CFString *`.
unsafe impl Encode for CfStringRef {
    const ENCODING: Encoding = Encoding::Pointer(&Encoding::Struct("__CFString", &[]));
}

// SAFETY: a pointer to the already-pointer typedef is not what we pass.
unsafe impl RefEncode for CfStringRef {
    const ENCODING_REF: Encoding = Encoding::Pointer(&Self::ENCODING);
}

const _: () = {
    assert!(std::mem::size_of::<usize>() == 8, "screen capture is built for LP64");
    assert!(std::mem::size_of::<CmTime>() == 24);
    assert!(std::mem::align_of::<CmTime>() == 8);
    assert!(std::mem::offset_of!(CmTime, value) == 0);
    assert!(std::mem::offset_of!(CmTime, timescale) == 8);
    assert!(std::mem::offset_of!(CmTime, flags) == 12);
    assert!(std::mem::offset_of!(CmTime, epoch) == 16);
    assert!(std::mem::size_of::<CgRect>() == 32);
    assert!(std::mem::align_of::<CgRect>() == 8);
    assert!(std::mem::offset_of!(CgRect, origin) == 0);
    assert!(std::mem::offset_of!(CgRect, size) == 16);
};

/// Keep only a complete bi-planar `'420v'` frame of the configured size.
pub(super) fn keep_frame(
    status: Option<i64>,
    format: u32,
    planes: usize,
    size: (usize, usize),
    want: (u32, u32),
) -> bool {
    status == Some(FRAME_COMPLETE)
        && format == PIXEL_420V
        && planes == 2
        && size.0 == want.0 as usize
        && size.1 == want.1 as usize
}

/// `true` while fewer than [`MAX_RETAINED_FRAMES`] buffers are already held.
pub(super) fn admit_frame(held: usize) -> bool {
    held < MAX_RETAINED_FRAMES
}

/// Same arithmetic as the Windows mirror loop. Multiples of 16, longest edge
/// inside 320..=1920. Duplicated because that function still lives in the
/// Windows-only module.
pub(super) fn target_dimensions(src_w: u32, src_h: u32, max_size: u32) -> (u32, u32) {
    let max_edge = max_size.clamp(MIN_EDGE, MAX_EDGE) as f64;
    let sw = src_w.max(64) as f64;
    let sh = src_h.max(64) as f64;
    let longest = sw.max(sh);
    let scale = if longest > max_edge { max_edge / longest } else { 1.0 };
    let w = (((sw * scale).round() as u32) / 16).max(4) * 16;
    let h = (((sh * scale).round() as u32) / 16).max(4) * 16;
    (w, h)
}

pub(super) fn clamp_fps(fps: u32) -> u32 {
    fps.clamp(MIN_FPS, MAX_FPS)
}

/// `1/fps` with `kCMTimeFlags_Valid`. `fps` is clamped to 10..=60.
pub(super) fn frame_interval(fps: u32) -> CmTime {
    CmTime { value: 1, timescale: clamp_fps(fps) as i32, flags: 1, epoch: 0 }
}

/// Display pixels from `contentRect.size * pointPixelScale`.
pub(super) fn display_pixels(width_points: f64, height_points: f64, scale: f64) -> Option<(u32, u32)> {
    if !width_points.is_finite() || !height_points.is_finite() || !scale.is_finite() {
        return None;
    }
    if scale <= 0.0 {
        return None;
    }
    let width = width_points * scale;
    let height = height_points * scale;
    if !width.is_finite() || !height.is_finite() || width < 1.0 || height < 1.0 {
        return None;
    }
    if width > u32::MAX as f64 || height > u32::MAX as f64 {
        return None;
    }
    Some((width.round() as u32, height.round() as u32))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Phase {
    Queued,
    Claimed,
    Picking,
    Starting,
    Live,
    Dead,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Event {
    Claim,
    Cancel,
    Presented,
    Filter,
    Started,
    Abort,
}

/// One picker session. `allow` is whether the side effect may run.
/// A cancel or abort that wins moves the phase to [`Phase::Dead`], and every
/// later claim, present, filter, or start returns `allow == false`.
pub(super) fn apply(phase: Phase, event: Event) -> (Phase, bool) {
    match (phase, event) {
        (Phase::Queued, Event::Claim) => (Phase::Claimed, true),
        (Phase::Queued, Event::Cancel | Event::Abort) => (Phase::Dead, true),
        (Phase::Claimed, Event::Presented) => (Phase::Picking, true),
        (Phase::Claimed, Event::Abort) => (Phase::Dead, true),
        (Phase::Picking, Event::Filter) => (Phase::Starting, true),
        (Phase::Picking, Event::Abort) => (Phase::Dead, true),
        (Phase::Starting, Event::Started) => (Phase::Live, true),
        (Phase::Starting, Event::Abort) => (Phase::Dead, true),
        _ => (phase, false),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn complete_420v_frame_of_the_configured_size_is_kept() {
        assert!(keep_frame(Some(FRAME_COMPLETE), PIXEL_420V, 2, (1920, 1080), (1920, 1080)));
        assert!(CmTime::ENCODING.equivalent_to_str("{?=qiIq}"));
        assert!(CgRect::ENCODING.equivalent_to_str("{CGRect={CGPoint=dd}{CGSize=dd}}"));
        assert!(CfStringRef::ENCODING.equivalent_to_str("^{__CFString=}"));

        let (w, h) = target_dimensions(3840, 2160, 8_000);
        assert_eq!((w, h), (1920, 1072));
        assert_eq!(w % 16, 0);
        assert_eq!(h % 16, 0);
        assert_eq!(target_dimensions(3840, 2160, 100), (320, 176));

        let slow = frame_interval(1);
        let fast = frame_interval(1_000);
        let paced = frame_interval(30);
        assert_eq!((slow.value, slow.timescale, slow.flags, slow.epoch), (1, 10, 1, 0));
        assert_eq!(fast.timescale, 60);
        assert_eq!(paced.timescale, 30);
        assert_eq!(clamp_fps(0), MIN_FPS);
        assert_eq!(display_pixels(1920.0, 1080.0, 2.0), Some((3840, 2160)));
        assert_eq!(display_pixels(10.0, 10.0, 0.0), None);
        assert_eq!(display_pixels(f64::NAN, 10.0, 2.0), None);
        assert!(admit_frame(0) && admit_frame(1));
        assert!(!admit_frame(MAX_RETAINED_FRAMES));
    }

    #[test]
    fn idle_or_resized_frame_is_not_kept() {
        let want = (1920, 1080);
        let size = (1920, 1080);
        assert!(!keep_frame(Some(1), PIXEL_420V, 2, size, want));
        assert!(!keep_frame(Some(2), PIXEL_420V, 2, size, want));
        assert!(!keep_frame(Some(FRAME_COMPLETE), PIXEL_420V, 2, (1919, 1080), want));
        assert!(!keep_frame(Some(FRAME_COMPLETE), PIXEL_420V, 2, (1920, 1088), want));
        assert!(!keep_frame(Some(FRAME_COMPLETE), 0, 2, size, want));
        assert!(!keep_frame(Some(FRAME_COMPLETE), PIXEL_420V, 1, size, want));
        assert!(!keep_frame(None, PIXEL_420V, 2, size, want));

        let (dead, allow) = apply(Phase::Queued, Event::Cancel);
        assert!(allow);
        assert_eq!(dead, Phase::Dead);
        assert!(!apply(dead, Event::Claim).1);
        assert!(!apply(dead, Event::Presented).1);
        assert!(!apply(dead, Event::Filter).1);
        assert!(!apply(dead, Event::Started).1);

        let (claimed, allow) = apply(Phase::Queued, Event::Claim);
        assert!(allow);
        assert_eq!(claimed, Phase::Claimed);
        let (still, allow) = apply(claimed, Event::Cancel);
        assert!(!allow);
        assert_eq!(still, Phase::Claimed);
        let (aborted, allow) = apply(still, Event::Abort);
        assert!(allow);
        assert_eq!(aborted, Phase::Dead);
        assert!(!apply(aborted, Event::Presented).1);

        let (aborted, allow) = apply(Phase::Picking, Event::Abort);
        assert!(allow);
        assert!(!apply(aborted, Event::Filter).1);
        let (aborted, allow) = apply(Phase::Starting, Event::Abort);
        assert!(allow);
        assert!(!apply(aborted, Event::Started).1);

        let mut phase = Phase::Queued;
        for event in [Event::Claim, Event::Presented, Event::Filter, Event::Started] {
            let (next, allow) = apply(phase, event);
            assert!(allow);
            phase = next;
        }
        assert_eq!(phase, Phase::Live);
        assert!(!apply(Phase::Starting, Event::Filter).1);
        assert!(!apply(Phase::Live, Event::Abort).1);
    }
}
