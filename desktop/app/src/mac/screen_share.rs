// SPDX-License-Identifier: GPL-3.0-or-later
//! Sharing this Mac's screen with a paired phone (`mirror` / `video`,
//! `session = 0`), view-only: the display the owner picks in the system
//! picker (`screen_capture`) goes through VideoToolbox (`h264_encoder`) as the
//! same packets the Windows path sends. Play on phone, the phone's touch and
//! keys, and this Mac's sound are not offered here.
//!
//! [`READY`] stays false until Root's native acceptance. Until then every
//! request is refused, the phone is told the mirroring stopped, and this Mac
//! keeps not advertising `mirror.capture`.

use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

use nectarlink_core::{DeviceId, MirrorConfig, MirrorInput, MirrorSend, MirrorStart, NodeEvent, PacketKind};

use super::{
    h264_encoder::{self, Encoder},
    screen_capture::{self, CaptureError, CaptureRequest, ScreenCapture},
};
use crate::core_host;

/// Root sets this true only after native acceptance.
pub const READY: bool = false;

/// The lowest bitrate the stream backs off to on a slow link.
const MIN_BITRATE: u32 = 600_000;

/// `SCStreamErrorUserStopped` (`SCError.h`): stopped from the system's control.
const USER_STOPPED: isize = -3817;

struct ActiveSession {
    peer: DeviceId,
    stop: Arc<AtomicBool>,
    force_keyframe: Arc<AtomicBool>,
}

static ACTIVE: Mutex<Option<ActiveSession>> = Mutex::new(None);

/// What a sharing session sent in the last stats window.
#[derive(Default)]
struct Stats {
    frames: u64,
    keyframes: u64,
    bytes: u64,
    dropped: u64,
    rebuilds: u64,
}

/// Returns the device currently viewing this Mac's screen, if any.
pub fn active_peer() -> Option<DeviceId> {
    ACTIVE.lock().unwrap_or_else(|e| e.into_inner()).as_ref().map(|s| s.peer)
}

/// Whether this Mac's screen is currently being shared with a phone.
pub fn is_active() -> bool {
    active_peer().is_some()
}

/// Starts or replaces the active Mac-to-phone screen stream for `peer`, once
/// the owner picks a display. Play on phone (`watch`) is refused.
pub fn start(peer: DeviceId, options: MirrorStart) {
    if !READY || options.watch.is_some() {
        tracing::info!(%peer, watch = options.watch.is_some(), "not sharing this Mac's screen: not available");
        refuse(peer);
        return;
    }
    stop_all();
    let stop = Arc::new(AtomicBool::new(false));
    let force_keyframe = Arc::new(AtomicBool::new(false));
    *ACTIVE.lock().unwrap_or_else(|e| e.into_inner()) =
        Some(ActiveSession { peer, stop: Arc::clone(&stop), force_keyframe: Arc::clone(&force_keyframe) });
    core_host::host().hub.changed(crate::state::Changes::REMOTE);
    core_host::refresh_capabilities();

    let spawned = std::thread::Builder::new().name("nectarlink-pc-mirror".into()).spawn({
        let stop = Arc::clone(&stop);
        move || run_capture_loop(peer, options, stop, force_keyframe)
    });
    if spawned.is_err() {
        tracing::warn!(%peer, "couldn't start sharing this Mac's screen");
        cleanup_session(&stop);
        refuse(peer);
    }
}

/// Tells the phone this request won't be served, without touching a session
/// already running.
fn refuse(peer: DeviceId) {
    if let Some(node) = core_host::node() {
        node.set_pc_mirror_session_allowed(peer, false);
        core_host::spawn(async move {
            node.mirror_stop(peer, nectarlink_core::MIRROR_SCREEN).await;
        });
    }
}

/// Stops sharing if `peer` is the active viewer.
pub fn stop_for(peer: &DeviceId) {
    let stopped = {
        let mut guard = ACTIVE.lock().unwrap_or_else(|e| e.into_inner());
        if guard.as_ref().is_some_and(|s| &s.peer == peer)
            && let Some(s) = guard.take()
        {
            s.stop.store(true, Ordering::SeqCst);
            true
        } else {
            false
        }
    };
    if stopped {
        core_host::host().hub.changed(crate::state::Changes::REMOTE);
        core_host::refresh_capabilities();
    }
}

/// Stops any active sharing session.
pub fn stop_all() {
    let stopped = {
        let mut guard = ACTIVE.lock().unwrap_or_else(|e| e.into_inner());
        guard.take().map(|s| {
            s.stop.store(true, Ordering::SeqCst);
            s.peer
        })
    };
    if let Some(peer) = stopped {
        if let Some(node) = core_host::node() {
            node.set_pc_mirror_session_allowed(peer, false);
            core_host::spawn(async move {
                node.mirror_stop(peer, nectarlink_core::MIRROR_SCREEN).await;
            });
        }
        core_host::host().hub.changed(crate::state::Changes::REMOTE);
        core_host::refresh_capabilities();
    }
}

/// Asks for an IDR keyframe for `peer`.
pub fn request_keyframe(peer: &DeviceId) {
    let guard = ACTIVE.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(s) = guard.as_ref()
        && &s.peer == peer
    {
        s.force_keyframe.store(true, Ordering::SeqCst);
    }
}

/// View-only on macOS: a phone's touch, scroll, keys and text are ignored.
pub fn handle_input(_peer: &DeviceId, _session: u32, _input: MirrorInput) {}

pub fn on_event(event: &NodeEvent) {
    match event {
        NodeEvent::LinkChanged { device, link: nectarlink_core::LinkState::Offline { .. } }
        | NodeEvent::DeviceRemoved(device) => {
            stop_for(device);
        }
        // No prompt for a request this Mac would refuse.
        NodeEvent::PcMirrorRequested { .. } if READY => {
            core_host::host().hub.update(|s| s.apply(event));
            crate::bridge::app::request_activation();
        }
        _ => {}
    }
}

/// Play on phone isn't offered on macOS, so nothing is muted.
pub fn set_mute_while_watched(_on: bool) {}

fn run_capture_loop(
    peer: DeviceId,
    options: MirrorStart,
    stop: Arc<AtomicBool>,
    force_keyframe: Arc<AtomicBool>,
) {
    let Some(node) = core_host::node() else {
        cleanup_session(&stop);
        return;
    };
    let runtime = &core_host::host().runtime;
    let Ok(sender) = runtime.block_on(node.mirror_open(peer)) else {
        tracing::warn!(%peer, "couldn't open the screen stream to the phone");
        cleanup_session(&stop);
        return;
    };

    let fps = options.fps.clamp(10, 60);
    // The owner picks a display (or cancels) before anything is captured.
    let capture = match ScreenCapture::start(CaptureRequest { max_size: options.max_size, fps }, &stop) {
        Ok(capture) => capture,
        Err(error) => {
            tracing::info!(%peer, ?error, "not sharing this Mac's screen");
            if owner_said_no(&error) {
                node.set_pc_mirror_session_allowed(peer, false);
            }
            sender.close();
            cleanup_session(&stop);
            return;
        }
    };
    let (width, height) = capture.size();

    // Give the phone's PcMirrorView a moment to mount before the config and keyframe.
    std::thread::sleep(Duration::from_millis(120));

    let max_bitrate = options.bitrate.clamp(500_000, 20_000_000);
    // Lowered when the link can't keep up, raised back once it has been clean for a while.
    let mut bitrate = max_bitrate;
    let frame_interval = Duration::from_micros(1_000_000 / u64::from(fps));
    let mut encoder: Option<Encoder> = None;
    // The newest frame, encoded again when a keyframe is due on a still screen.
    let mut last: Option<screen_capture::PixelBuffer> = None;
    let mut resend = false;
    let mut last_time_us = 0u64;
    let started = Instant::now();
    let mut last_drop: Option<Instant> = None;
    let mut last_rate_change = Instant::now();
    let mut stats = Stats::default();
    let mut stats_since = Instant::now();
    tracing::info!(%peer, fps, bitrate, width, height, "sharing this Mac's screen");

    while !stop.load(Ordering::Relaxed) {
        let tick_start = Instant::now();
        if let Some(error) = capture.ended() {
            tracing::info!(%peer, ?error, "the screen capture ended");
            if owner_said_no(&error) {
                node.set_pc_mirror_session_allowed(peer, false);
            }
            break;
        }

        // The VideoToolbox encoder always takes keyframe requests; a new one
        // is only made when there is none (first frame, or after an error).
        let requested_kf = force_keyframe.swap(false, Ordering::SeqCst);
        if encoder.is_none() {
            match Encoder::new(width, height, fps, bitrate) {
                Ok(enc) => {
                    let cfg = MirrorConfig {
                        codec: nectarlink_core::MIRROR_H264.into(),
                        width,
                        height,
                        session: nectarlink_core::MIRROR_SCREEN,
                    };
                    if matches!(sender.send(PacketKind::Config, 0, cfg.to_cbor()), MirrorSend::Closed) {
                        break;
                    }
                    encoder = Some(enc);
                    resend = true;
                    stats.rebuilds += 1;
                }
                Err(error) => {
                    tracing::warn!(%error, "Mac screen H.264 encoder init failed");
                    break;
                }
            }
        } else if requested_kf && let Some(enc) = encoder.as_ref() {
            enc.request_keyframe();
            resend = true;
        }

        let fresh = capture.take_new();
        let resend_now = std::mem::take(&mut resend);
        let due = fresh.is_some() || resend_now;
        if fresh.is_some() {
            last = fresh;
        }
        if due
            && let (Some(enc), Some(frame)) = (encoder.as_mut(), last.as_ref())
            && let Some(pixels) = encodable(frame)
        {
            let time_us = (started.elapsed().as_micros() as u64).max(last_time_us + 1);
            last_time_us = time_us;
            match enc.encode(&pixels, time_us) {
                Ok(packets) => {
                    let mut send_failed = false;
                    for (is_keyframe, bytes) in packets {
                        let kind = if is_keyframe { PacketKind::Keyframe } else { PacketKind::Frame };
                        let len = bytes.len();
                        match sender.send(kind, time_us, bytes) {
                            MirrorSend::Queued => {
                                stats.frames += 1;
                                stats.bytes += len as u64;
                                if is_keyframe {
                                    stats.keyframes += 1;
                                }
                            }
                            MirrorSend::Dropped => stats.dropped += 1,
                            MirrorSend::NeedKeyframe => {
                                stats.dropped += 1;
                                last_drop = Some(Instant::now());
                                force_keyframe.store(true, Ordering::SeqCst);
                                // The link is slower than the stream: ask for less, at most every 2 s.
                                if last_rate_change.elapsed() >= Duration::from_secs(2) {
                                    let lower = (bitrate / 4 * 3).max(MIN_BITRATE);
                                    if lower < bitrate && enc.set_bitrate(lower) {
                                        bitrate = lower;
                                        last_rate_change = Instant::now();
                                    }
                                }
                            }
                            MirrorSend::Closed => {
                                send_failed = true;
                                break;
                            }
                        }
                    }
                    if send_failed {
                        break;
                    }
                    // Clean for 10 s: step the bitrate back up toward what the phone asked for.
                    let clean = last_drop.is_none_or(|t| t.elapsed() >= Duration::from_secs(10));
                    if clean && bitrate < max_bitrate && last_rate_change.elapsed() >= Duration::from_secs(10)
                    {
                        let higher = (bitrate / 4 * 5).min(max_bitrate);
                        if enc.set_bitrate(higher) {
                            bitrate = higher;
                            last_rate_change = Instant::now();
                        }
                    }
                }
                Err(error) => {
                    tracing::warn!(%error, "Mac screen frame encode error; rebuilding the encoder");
                    encoder = None;
                }
            }
        }

        if stats_since.elapsed() >= Duration::from_secs(10) {
            let secs = stats_since.elapsed().as_secs_f64();
            tracing::info!(
                %peer,
                fps = format!("{:.1}", stats.frames as f64 / secs),
                kbps = (stats.bytes as f64 * 8.0 / secs / 1000.0) as u64,
                keyframes = stats.keyframes,
                dropped = stats.dropped,
                rebuilds = stats.rebuilds,
                bitrate,
                "sharing this Mac's screen"
            );
            stats = Stats::default();
            stats_since = Instant::now();
        }

        if let Some(rem) = frame_interval.checked_sub(tick_start.elapsed()) {
            std::thread::sleep(rem);
        }
    }

    tracing::info!(%peer, "stopped sharing this Mac's screen");
    // Frames first, then the capture: dropping it stops ScreenCaptureKit and
    // clears the system's recording indicator.
    drop(last);
    drop(encoder);
    drop(capture);
    sender.close();
    cleanup_session(&stop);
}

/// The owner cancelled, let the picker time out, or stopped sharing from the
/// system's own control: the one-time consent for this phone ends too.
fn owner_said_no(error: &CaptureError) -> bool {
    matches!(error, CaptureError::Cancelled | CaptureError::Declined | CaptureError::Stopped(USER_STOPPED))
}

/// The encoder's checked reference to a captured frame.
#[allow(unsafe_code)]
fn encodable(frame: &screen_capture::PixelBuffer) -> Option<h264_encoder::PixelBuffer> {
    // SAFETY: `frame` holds its own CoreFoundation reference for this whole call.
    match unsafe { h264_encoder::PixelBuffer::retain(frame.as_raw()) } {
        Ok(pixels) => Some(pixels),
        Err(error) => {
            tracing::debug!(%error, "a captured frame the encoder can't take");
            None
        }
    }
}

/// Clears `ACTIVE` only if it is still this loop's session (a newer `start`
/// may have replaced it).
fn cleanup_session(stop: &Arc<AtomicBool>) {
    let removed = {
        let mut guard = ACTIVE.lock().unwrap_or_else(|e| e.into_inner());
        if guard.as_ref().is_some_and(|s| Arc::ptr_eq(&s.stop, stop)) {
            guard.take().is_some()
        } else {
            false
        }
    };
    if removed {
        core_host::host().hub.changed(crate::state::Changes::REMOTE);
        core_host::refresh_capabilities();
    }
}
