// SPDX-License-Identifier: GPL-3.0-or-later
//! macOS video-only MP4 recording for mirrored phone screens.
//!
//! Provenance: ISO-BMFF/H.264 container code reused from upstream Linux peer
//! `desktop/app/src/linux/mp4.rs` at exact upstream PR12 head `1de43ef1`
//! (`1de43ef1dd50c603601e4149dfbdd8520ebd5084`). Same GPL-3.0-or-later
//! license; no new authorship claimed for the reused box builders and
//! Annex-B helpers. Reused verbatim or near-verbatim: `split_annexb`,
//! `nal_type`, `has_idr_slice`, `has_inband_sps_pps`, `is_parameter_only`,
//! `extract_sps_pps`, `merge_sps_pps` (dedup, SPS-then-PPS), `TIMESCALE`,
//! `mp4_box`, `mp4_full_box`, `build_ftyp`, `build_moov`, `build_mvhd`,
//! `build_trak`, `build_tkhd`, `build_mdia`, `build_minf`, `build_stbl`,
//! `build_stsd`, `build_stts`, `build_stss`, `build_stsc`, `build_stsz`,
//! `build_stco`, and the `extract_sps_and_pps_vecs` fallback pair.
//!
//! Mac adaptations (minimum, not wholesale copy):
//! - Video-only: no `ffmpeg` shell-out, no `Command`/`Stdio`, no `libc`;
//!   `on_audio` honestly refuses with `Err` instead of buffering/discarding.
//! - Streaming to the caller-owned disposable `path`: samples are converted
//!   Annex-B → AVCC and appended to `mdat` as they arrive. Only per-sample
//!   metadata (`sample_sizes`, `pts`, `sync_samples`, ~16 B/frame) stays in
//!   RAM. The upstream design buffers every `RecordedFrame` plus the full
//!   `mdat_payload` in memory; that unbounded whole-video RAM is not copied.
//! - Layout is `ftyp` + `mdat` (streamed) + `moov` (appended at `finish`)
//!   instead of upstream's upfront `ftyp` + `moov` + `mdat`, so the file can
//!   be streamed without a second pass. `stco` points at the `mdat` payload.
//! - Bounded: per-packet, frame-count, duration, and byte caps are enforced
//!   in `on_packet` (`MAX_*` below). Upstream has no caps.
//! - Clock: keeps the existing Mac `local_timestamp_filename` (epoch seconds
//!   via `SystemTime`). The Linux `local_ymdhms`/`civil_from_days` timezone
//!   logic is deliberately not ported (no timezone abstraction per task).
//! - First-keyframe gating with lazy file creation; `finish` without frames
//!   removes the partial file. `Drop` removes an unfinished partial file.
//!
//! Limitations recorded:
//! - Audio is video-only by design: `new`/`on_audio_config` accept audio
//!   config for caller-API stability but ignore it at mux time; `on_audio`
//!   returns `Err` for non-empty PCM.
//! - `moov` is written at the end (not `faststart`-fronted). Local players
//!   play it; progressive streaming would need a second pass.
//! - `avcC` falls back to a hardcoded baseline SPS/PPS pair when the stream
//!   never yielded both (same fallback as upstream). Dimensions in that
//!   fallback may not match the stream; real streams provide SPS/PPS.
//! - `stco` uses 32-bit offsets, so total file size is capped well below 4 GiB
//!   (`MAX_MDAT_BYTES`). Longer recordings are refused with an error.
//! - Callers that expected the old stub (`"not supported on macOS"` from
//!   `on_packet`/`finish`, e.g. `mac/mirror_receive.rs::seam_check`) must be
//!   updated by Root: valid IDR packets now succeed and `finish` writes a file.

use std::{
    fs::{self, File},
    io::{Seek, SeekFrom, Write},
    path::PathBuf,
    time::{Instant, SystemTime, UNIX_EPOCH},
};

/// Formats `<prefix> <epoch-seconds>.<ext>` using the PC's clock.
///
/// Kept as the existing Mac helper; the Linux local-time `YYYY-MM-DD`
/// form is intentionally not ported (no timezone abstraction on Mac).
pub fn local_timestamp_filename(prefix: &str, ext: &str) -> String {
    let now = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs());
    format!("{prefix} {now}.{ext}")
}

/// Largest Annex-B access unit accepted per `on_packet` call.
/// Matches `mac/h264/annexb.rs::MAX_ACCESS_UNIT` (8 MiB).
const MAX_PACKET_BYTES: usize = 8 << 20;
/// Most NAL units accepted in one access unit (phones send a handful).
/// Matches `mac/h264/annexb.rs::MAX_NAL_UNITS`.
const MAX_NALS_PER_PACKET: usize = 512;
/// Most frames accepted per recording. Bounds in-RAM sample metadata to
/// ~16 B/frame (~2.4 MiB at the cap). ~42 min at 60 fps, ~83 min at 30 fps.
const MAX_FRAMES: usize = 150_000;
/// Longest recording accepted (presentation time span). Refused beyond.
const MAX_DURATION_US: u64 = 3 * 3_600 * 1_000_000;
/// Largest single AVCC sample accepted (post-conversion).
const MAX_SAMPLE_BYTES: usize = 8 << 20;
/// Largest `mdat` payload accepted. Keeps 32-bit `stco`/box sizes valid.
const MAX_MDAT_BYTES: u64 = 1_800_000_000;

/// Splits an Annex-B byte stream into individual NAL payloads (without start codes).
pub fn split_annexb(data: &[u8]) -> Vec<&[u8]> {
    let mut starts = Vec::new();
    let mut i = 0;
    while i + 3 <= data.len() {
        if i + 4 <= data.len() && data[i..i + 4] == [0, 0, 0, 1] {
            starts.push((i, i + 4));
            i += 4;
        } else if data[i..i + 3] == [0, 0, 1] {
            starts.push((i, i + 3));
            i += 3;
        } else {
            i += 1;
        }
    }
    let mut out = Vec::with_capacity(starts.len());
    for (idx, &(_, payload_start)) in starts.iter().enumerate() {
        let payload_end = starts.get(idx + 1).map_or(data.len(), |&(next_code, _)| next_code);
        if payload_start < payload_end {
            out.push(&data[payload_start..payload_end]);
        }
    }
    out
}

/// Returns the 5-bit H.264 NAL unit type of a NAL payload (without start code).
pub fn nal_type(nal: &[u8]) -> Option<u8> {
    nal.first().map(|b| b & 0x1F)
}

/// Returns `true` if `data` contains an H.264 IDR slice (NAL type 5).
pub fn has_idr_slice(data: &[u8]) -> bool {
    split_annexb(data).into_iter().any(|nal| nal_type(nal) == Some(5))
}

/// Returns `true` if `data` already carries both SPS (NAL 7) and PPS (NAL 8) units in-band.
pub fn has_inband_sps_pps(data: &[u8]) -> bool {
    let nals = split_annexb(data);
    nals.iter().any(|nal| nal_type(nal) == Some(7)) && nals.iter().any(|nal| nal_type(nal) == Some(8))
}

/// Returns `true` if `data` is non-empty and every NAL unit is a parameter set (SPS/PPS).
pub fn is_parameter_only(data: &[u8]) -> bool {
    let nals = split_annexb(data);
    !nals.is_empty() && nals.into_iter().all(|nal| matches!(nal_type(nal), Some(7 | 8)))
}

/// Extracts only SPS (NAL 7) and PPS (NAL 8) units from `data` with 4-byte Annex-B start codes.
pub fn extract_sps_pps(data: &[u8]) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    for nal in split_annexb(data) {
        if matches!(nal_type(nal), Some(7 | 8)) {
            out.extend_from_slice(&[0, 0, 0, 1]);
            out.extend_from_slice(nal);
        }
    }
    (!out.is_empty()).then_some(out)
}

/// Extracts any SPS (NAL 7) and PPS (NAL 8) units from `incoming` and merges them
/// into `dest` with 4-byte `00 00 00 01` Annex-B start codes, preserving SPS-then-PPS order.
pub fn merge_sps_pps(dest: &mut Vec<u8>, incoming: &[u8]) {
    let nals = split_annexb(incoming);
    if !nals.iter().any(|nal| matches!(nal_type(nal), Some(7 | 8))) {
        return;
    }
    let existing = split_annexb(dest);
    let mut sps: Option<Vec<u8>> =
        existing.iter().find(|nal| nal_type(nal) == Some(7)).map(|nal| (*nal).to_vec());
    let mut pps: Option<Vec<u8>> =
        existing.iter().find(|nal| nal_type(nal) == Some(8)).map(|nal| (*nal).to_vec());
    for nal in nals {
        match nal_type(nal) {
            Some(7) => sps = Some(nal.to_vec()),
            Some(8) => pps = Some(nal.to_vec()),
            _ => {}
        }
    }
    dest.clear();
    if let Some(s) = sps {
        dest.extend_from_slice(&[0, 0, 0, 1]);
        dest.extend_from_slice(&s);
    }
    if let Some(p) = pps {
        dest.extend_from_slice(&[0, 0, 0, 1]);
        dest.extend_from_slice(&p);
    }
}

/// Records a mirrored H.264 stream into an `.mp4` file (video-only on macOS).
///
/// Same caller API as the upstream Linux `MirrorRecorder`; samples stream to
/// `path` instead of buffering the whole video in RAM.
pub struct MirrorRecorder {
    path: PathBuf,
    width: u32,
    height: u32,
    fps: u32,
    sps_pps: Vec<u8>,
    started_at: Instant,
    started_epoch_ms: i64,
    got_keyframe: bool,
    first_pts_us: Option<u64>,
    last_pts_us: u64,
    frame_count: usize,
    /// Per-sample presentation offsets (µs, relative to first frame).
    pts: Vec<u64>,
    /// Per-sample AVCC byte lengths, in file order.
    sample_sizes: Vec<u32>,
    /// 1-based sample numbers that are keyframes.
    sync_samples: Vec<u32>,
    mdat_bytes: u64,
    ftyp_len: usize,
    /// Created lazily on the first keyframe; `None` until then.
    file: Option<File>,
}

impl std::fmt::Debug for MirrorRecorder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MirrorRecorder").field("path", &self.path).finish_non_exhaustive()
    }
}

impl Drop for MirrorRecorder {
    fn drop(&mut self) {
        // An unfinished recording leaves an unplayable `ftyp`+`mdat` prefix
        // (no `moov`); remove it so Downloads keeps no corrupt partials.
        // `finish` consumes `self`, so this only runs on the unfinished path.
        if self.file.is_some() {
            self.file.take();
            let _ = fs::remove_file(&self.path);
        }
    }
}

impl MirrorRecorder {
    pub fn new(
        path: PathBuf,
        width: u32,
        height: u32,
        fps: u32,
        // Accepted for caller-API stability; ignored: video-only on macOS.
        _audio_cfg: Option<(u32, u8)>,
    ) -> MirrorRecorder {
        let started_epoch_ms =
            SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_millis() as i64);
        MirrorRecorder {
            path,
            width: width.max(1),
            height: height.max(1),
            fps: fps.max(1),
            sps_pps: Vec::new(),
            started_at: Instant::now(),
            started_epoch_ms,
            got_keyframe: false,
            first_pts_us: None,
            last_pts_us: 0,
            frame_count: 0,
            pts: Vec::new(),
            sample_sizes: Vec::new(),
            sync_samples: Vec::new(),
            mdat_bytes: 0,
            ftyp_len: 0,
            file: None,
        }
    }

    pub fn started_at(&self) -> Instant {
        self.started_at
    }

    pub fn started_epoch_ms(&self) -> i64 {
        self.started_epoch_ms
    }

    /// Seeds cached SPS/PPS NAL units collected before recording started.
    pub fn set_sps_pps(&mut self, sps_pps: &[u8]) {
        if !sps_pps.is_empty() {
            merge_sps_pps(&mut self.sps_pps, sps_pps);
        }
    }

    /// Updates stream dimensions before the first keyframe initializes the recording.
    pub fn on_config(&mut self, width: u32, height: u32) {
        if !self.got_keyframe && width > 0 && height > 0 {
            self.width = width;
            self.height = height;
        }
    }

    /// Accepts audio configuration for caller-API stability; ignored:
    /// this recorder is video-only (see `on_audio`).
    pub fn on_audio_config(&mut self, _rate: u32, _channels: u8) {}

    /// Processes an incoming Annex-B H.264 access unit, streaming it to disk.
    pub fn on_packet(&mut self, keyframe: bool, time_us: u64, data: &[u8]) -> Result<(), String> {
        if data.is_empty() {
            return Ok(());
        }
        if data.len() > MAX_PACKET_BYTES {
            return Err(format!("H.264 access unit of {} bytes exceeds the limit", data.len()));
        }
        let nals = split_annexb(data);
        if nals.is_empty() {
            return Err("not Annex-B: no start code".into());
        }
        if nals.len() > MAX_NALS_PER_PACKET {
            return Err(format!("more than {MAX_NALS_PER_PACKET} NAL units in an access unit"));
        }
        if nals.iter().any(|nal| nal.is_empty() || nal[0] & 0x80 != 0) {
            return Err("a malformed NAL unit".into());
        }
        merge_sps_pps(&mut self.sps_pps, data);

        if is_parameter_only(data) {
            return Ok(());
        }

        let is_keyframe = keyframe || has_idr_slice(data);
        if !self.got_keyframe {
            if !is_keyframe {
                return Ok(());
            }
            self.got_keyframe = true;
        }
        if self.frame_count >= MAX_FRAMES {
            return Err(format!("more than {MAX_FRAMES} frames"));
        }

        let first_us = *self.first_pts_us.get_or_insert(time_us);
        let frame_duration_us = (1_000_000 / u64::from(self.fps.max(1))).max(1);
        let mut rel_us = time_us.saturating_sub(first_us);
        if self.frame_count > 0 && rel_us <= self.last_pts_us {
            rel_us = self.last_pts_us.saturating_add(frame_duration_us);
        }
        if rel_us > MAX_DURATION_US {
            return Err("recording duration exceeds the limit".into());
        }

        // Keep the cached parameter sets available to the keyframe for `avcC`
        // discovery, matching upstream. They are stripped from `mdat` below.
        let combined;
        let annexb: &[u8] = if is_keyframe && !has_inband_sps_pps(data) && !self.sps_pps.is_empty() {
            combined = [self.sps_pps.as_slice(), data].concat();
            &combined
        } else {
            data
        };

        let mut sample_bytes = Vec::new();
        for nal in split_annexb(annexb) {
            if nal.is_empty() || matches!(nal_type(nal), Some(7..=9)) {
                continue;
            }
            let len = nal.len() as u32;
            sample_bytes.extend_from_slice(&len.to_be_bytes());
            sample_bytes.extend_from_slice(nal);
        }
        if sample_bytes.is_empty() {
            for nal in split_annexb(annexb) {
                if nal.is_empty() {
                    continue;
                }
                let len = nal.len() as u32;
                sample_bytes.extend_from_slice(&len.to_be_bytes());
                sample_bytes.extend_from_slice(nal);
            }
        }
        if sample_bytes.is_empty() {
            return Ok(());
        }
        if sample_bytes.len() > MAX_SAMPLE_BYTES {
            return Err(format!("H.264 sample of {} bytes exceeds the limit", sample_bytes.len()));
        }
        if self.mdat_bytes.saturating_add(sample_bytes.len() as u64) > MAX_MDAT_BYTES {
            return Err("recording file size exceeds the limit".into());
        }

        self.ensure_file_open()?;
        let file = self.file.as_mut().ok_or("recording file is not open")?;
        file.write_all(&sample_bytes).map_err(|e| e.to_string())?;

        self.pts.push(rel_us);
        self.sample_sizes.push(sample_bytes.len() as u32);
        if is_keyframe {
            self.sync_samples.push((self.frame_count + 1) as u32);
        }
        self.frame_count += 1;
        self.mdat_bytes += sample_bytes.len() as u64;
        self.last_pts_us = rel_us;
        Ok(())
    }

    /// Refuses phone audio: this recorder is video-only on macOS.
    pub fn on_audio(&mut self, _time_us: u64, pcm: &[u8]) -> Result<(), String> {
        if pcm.is_empty() {
            return Ok(());
        }
        Err("audio recording is not supported on macOS (video-only recording)".into())
    }

    /// Finalizes the MP4 container (`moov` appended, `mdat` size patched).
    pub fn finish(mut self) -> Result<PathBuf, String> {
        // `Drop` is implemented (partial-file cleanup), so `self.path` cannot
        // be moved out of `self`; clone the disposable caller path instead.
        let path = self.path.clone();
        if self.frame_count == 0 || !self.got_keyframe || self.file.is_none() {
            self.file.take();
            let _ = fs::remove_file(&path);
            return Err("No video frames were recorded yet.".into());
        }
        let result = self.finish_inner();
        if result.is_err() {
            self.file.take();
            let _ = fs::remove_file(&path);
        }
        result?;
        // Success: close the handle without deleting; `Drop` must see `None`.
        self.file.take();
        Ok(path)
    }

    fn finish_inner(&mut self) -> Result<(), String> {
        let file = self.file.as_mut().ok_or("recording file is not open")?;
        let default_delta = (TIMESCALE / self.fps.max(1)).max(1);
        let mut sample_deltas = Vec::with_capacity(self.pts.len());
        for (idx, &pts) in self.pts.iter().enumerate() {
            let delta = if let Some(&next) = self.pts.get(idx + 1) {
                let diff_us = next.saturating_sub(pts);
                ((diff_us * u64::from(TIMESCALE)) / 1_000_000).max(1) as u32
            } else {
                default_delta
            };
            sample_deltas.push(delta);
        }
        let mut sync_samples = std::mem::take(&mut self.sync_samples);
        if sync_samples.is_empty() {
            sync_samples.push(1);
        }
        let (sps, pps) = extract_sps_and_pps_vecs(&self.sps_pps);
        let ftyp = build_ftyp();
        let chunk_offset = (ftyp.len() + 8) as u32;
        let moov = build_moov(
            self.width.max(1) as u16,
            self.height.max(1) as u16,
            &sps,
            &pps,
            &sample_deltas,
            &sync_samples,
            &self.sample_sizes,
            chunk_offset,
        );
        file.write_all(&moov).map_err(|e| e.to_string())?;
        file.flush().map_err(|e| e.to_string())?;
        // Patch the placeholder `mdat` size now that the payload length is final.
        let mdat_size = (self.mdat_bytes + 8).min(u64::from(u32::MAX)) as u32;
        file.seek(SeekFrom::Start(self.ftyp_len as u64)).map_err(|e| e.to_string())?;
        file.write_all(&mdat_size.to_be_bytes()).map_err(|e| e.to_string())?;
        file.flush().map_err(|e| e.to_string())?;
        Ok(())
    }

    /// Creates/truncates the output file and streams `ftyp` + `mdat` header.
    fn ensure_file_open(&mut self) -> Result<(), String> {
        if self.file.is_some() {
            return Ok(());
        }
        if let Some(parent) = self.path.parent() {
            if !parent.as_os_str().is_empty() {
                fs::create_dir_all(parent).map_err(|e| e.to_string())?;
            }
        }
        let ftyp = build_ftyp();
        let mut file =
            File::create(&self.path).map_err(|e| format!("create {}: {e}", self.path.display()))?;
        file.write_all(&ftyp).map_err(|e| e.to_string())?;
        file.write_all(&0u32.to_be_bytes()).map_err(|e| e.to_string())?;
        file.write_all(b"mdat").map_err(|e| e.to_string())?;
        self.ftyp_len = ftyp.len();
        self.file = Some(file);
        Ok(())
    }
}

const TIMESCALE: u32 = 90_000;

fn mp4_box(fourcc: &[u8; 4], payload: &[u8]) -> Vec<u8> {
    let total = (8 + payload.len()) as u32;
    let mut out = Vec::with_capacity(total as usize);
    out.extend_from_slice(&total.to_be_bytes());
    out.extend_from_slice(fourcc);
    out.extend_from_slice(payload);
    out
}

fn mp4_full_box(fourcc: &[u8; 4], version: u8, flags: u32, body: &[u8]) -> Vec<u8> {
    let total = (12 + body.len()) as u32;
    let mut out = Vec::with_capacity(total as usize);
    out.extend_from_slice(&total.to_be_bytes());
    out.extend_from_slice(fourcc);
    let vf = (u32::from(version) << 24) | (flags & 0x00FF_FFFF);
    out.extend_from_slice(&vf.to_be_bytes());
    out.extend_from_slice(body);
    out
}

fn extract_sps_and_pps_vecs(sps_pps: &[u8]) -> (Vec<u8>, Vec<u8>) {
    let mut sps = None;
    let mut pps = None;
    for nal in split_annexb(sps_pps) {
        match nal_type(nal) {
            Some(7) => sps = Some(nal.to_vec()),
            Some(8) => pps = Some(nal.to_vec()),
            _ => {}
        }
    }
    // Same hardcoded baseline fallback as upstream when the stream never
    // yielded both parameter sets.
    let sps = sps.unwrap_or_else(|| vec![0x67, 0x42, 0xC0, 0x1F, 0xDA, 0x01, 0x40, 0x16, 0xEC, 0x04, 0x40]);
    let pps = pps.unwrap_or_else(|| vec![0x68, 0xCE, 0x3C, 0x80]);
    (sps, pps)
}

fn build_ftyp() -> Vec<u8> {
    let mut body = Vec::with_capacity(24);
    body.extend_from_slice(b"isom");
    body.extend_from_slice(&0x0000_0200u32.to_be_bytes());
    body.extend_from_slice(b"isom");
    body.extend_from_slice(b"iso2");
    body.extend_from_slice(b"avc1");
    body.extend_from_slice(b"mp41");
    mp4_box(b"ftyp", &body)
}

#[allow(clippy::too_many_arguments)]
fn build_moov(
    width: u16,
    height: u16,
    sps: &[u8],
    pps: &[u8],
    sample_deltas: &[u32],
    sync_samples: &[u32],
    sample_sizes: &[u32],
    chunk_offset: u32,
) -> Vec<u8> {
    let total_duration: u32 = sample_deltas.iter().copied().fold(0u32, u32::saturating_add).max(1);
    let mvhd = build_mvhd(total_duration);
    let trak = build_trak(
        width,
        height,
        total_duration,
        sps,
        pps,
        sample_deltas,
        sync_samples,
        sample_sizes,
        chunk_offset,
    );
    let mut body = Vec::with_capacity(mvhd.len() + trak.len());
    body.extend_from_slice(&mvhd);
    body.extend_from_slice(&trak);
    mp4_box(b"moov", &body)
}

fn build_mvhd(duration: u32) -> Vec<u8> {
    let mut body = Vec::with_capacity(96);
    body.extend_from_slice(&0u32.to_be_bytes()); // creation_time
    body.extend_from_slice(&0u32.to_be_bytes()); // modification_time
    body.extend_from_slice(&TIMESCALE.to_be_bytes());
    body.extend_from_slice(&duration.to_be_bytes());
    body.extend_from_slice(&0x0001_0000u32.to_be_bytes()); // rate 1.0
    body.extend_from_slice(&0x0100u16.to_be_bytes()); // volume 1.0
    body.extend_from_slice(&[0u8; 10]); // reserved
    // Unity matrix
    for val in [0x0001_0000u32, 0, 0, 0, 0x0001_0000, 0, 0, 0, 0x4000_0000] {
        body.extend_from_slice(&val.to_be_bytes());
    }
    body.extend_from_slice(&[0u8; 24]); // pre_defined
    body.extend_from_slice(&2u32.to_be_bytes()); // next_track_ID
    mp4_full_box(b"mvhd", 0, 0, &body)
}

#[allow(clippy::too_many_arguments)]
fn build_trak(
    width: u16,
    height: u16,
    duration: u32,
    sps: &[u8],
    pps: &[u8],
    sample_deltas: &[u32],
    sync_samples: &[u32],
    sample_sizes: &[u32],
    chunk_offset: u32,
) -> Vec<u8> {
    let tkhd = build_tkhd(width, height, duration);
    let mdia = build_mdia(
        width,
        height,
        duration,
        sps,
        pps,
        sample_deltas,
        sync_samples,
        sample_sizes,
        chunk_offset,
    );
    let mut body = Vec::with_capacity(tkhd.len() + mdia.len());
    body.extend_from_slice(&tkhd);
    body.extend_from_slice(&mdia);
    mp4_box(b"trak", &body)
}

fn build_tkhd(width: u16, height: u16, duration: u32) -> Vec<u8> {
    let mut body = Vec::with_capacity(80);
    body.extend_from_slice(&0u32.to_be_bytes()); // creation_time
    body.extend_from_slice(&0u32.to_be_bytes()); // modification_time
    body.extend_from_slice(&1u32.to_be_bytes()); // track_ID
    body.extend_from_slice(&0u32.to_be_bytes()); // reserved
    body.extend_from_slice(&duration.to_be_bytes());
    body.extend_from_slice(&[0u8; 8]); // reserved
    body.extend_from_slice(&0u16.to_be_bytes()); // layer
    body.extend_from_slice(&0u16.to_be_bytes()); // alternate_group
    body.extend_from_slice(&0u16.to_be_bytes()); // volume (0 for video)
    body.extend_from_slice(&0u16.to_be_bytes()); // reserved
    for val in [0x0001_0000u32, 0, 0, 0, 0x0001_0000, 0, 0, 0, 0x4000_0000] {
        body.extend_from_slice(&val.to_be_bytes());
    }
    body.extend_from_slice(&(u32::from(width) << 16).to_be_bytes());
    body.extend_from_slice(&(u32::from(height) << 16).to_be_bytes());
    mp4_full_box(b"tkhd", 0, 0x0000_0003, &body)
}

#[allow(clippy::too_many_arguments)]
fn build_mdia(
    width: u16,
    height: u16,
    duration: u32,
    sps: &[u8],
    pps: &[u8],
    sample_deltas: &[u32],
    sync_samples: &[u32],
    sample_sizes: &[u32],
    chunk_offset: u32,
) -> Vec<u8> {
    let mut mdhd_body = Vec::with_capacity(20);
    mdhd_body.extend_from_slice(&0u32.to_be_bytes());
    mdhd_body.extend_from_slice(&0u32.to_be_bytes());
    mdhd_body.extend_from_slice(&TIMESCALE.to_be_bytes());
    mdhd_body.extend_from_slice(&duration.to_be_bytes());
    mdhd_body.extend_from_slice(&0x55C4u16.to_be_bytes()); // "und"
    mdhd_body.extend_from_slice(&0u16.to_be_bytes());
    let mdhd = mp4_full_box(b"mdhd", 0, 0, &mdhd_body);

    let mut hdlr_body = Vec::with_capacity(32);
    hdlr_body.extend_from_slice(&0u32.to_be_bytes()); // pre_defined
    hdlr_body.extend_from_slice(b"vide");
    hdlr_body.extend_from_slice(&[0u8; 12]); // reserved
    hdlr_body.extend_from_slice(b"VideoHandler\0");
    let hdlr = mp4_full_box(b"hdlr", 0, 0, &hdlr_body);

    let minf = build_minf(width, height, sps, pps, sample_deltas, sync_samples, sample_sizes, chunk_offset);

    let mut body = Vec::with_capacity(mdhd.len() + hdlr.len() + minf.len());
    body.extend_from_slice(&mdhd);
    body.extend_from_slice(&hdlr);
    body.extend_from_slice(&minf);
    mp4_box(b"mdia", &body)
}

#[allow(clippy::too_many_arguments)]
fn build_minf(
    width: u16,
    height: u16,
    sps: &[u8],
    pps: &[u8],
    sample_deltas: &[u32],
    sync_samples: &[u32],
    sample_sizes: &[u32],
    chunk_offset: u32,
) -> Vec<u8> {
    let vmhd = mp4_full_box(b"vmhd", 0, 1, &[0u8; 8]);
    let url = mp4_full_box(b"url ", 0, 1, &[]);
    let mut dref_body = Vec::with_capacity(4 + url.len());
    dref_body.extend_from_slice(&1u32.to_be_bytes());
    dref_body.extend_from_slice(&url);
    let dref = mp4_full_box(b"dref", 0, 0, &dref_body);
    let dinf = mp4_box(b"dinf", &dref);

    let stbl = build_stbl(width, height, sps, pps, sample_deltas, sync_samples, sample_sizes, chunk_offset);

    let mut body = Vec::with_capacity(vmhd.len() + dinf.len() + stbl.len());
    body.extend_from_slice(&vmhd);
    body.extend_from_slice(&dinf);
    body.extend_from_slice(&stbl);
    mp4_box(b"minf", &body)
}

#[allow(clippy::too_many_arguments)]
fn build_stbl(
    width: u16,
    height: u16,
    sps: &[u8],
    pps: &[u8],
    sample_deltas: &[u32],
    sync_samples: &[u32],
    sample_sizes: &[u32],
    chunk_offset: u32,
) -> Vec<u8> {
    let stsd = build_stsd(width, height, sps, pps);
    let stts = build_stts(sample_deltas);
    let stss = build_stss(sync_samples);
    let stsc = build_stsc(sample_sizes.len() as u32);
    let stsz = build_stsz(sample_sizes);
    let stco = build_stco(chunk_offset);

    let mut body = Vec::new();
    body.extend_from_slice(&stsd);
    body.extend_from_slice(&stts);
    body.extend_from_slice(&stss);
    body.extend_from_slice(&stsc);
    body.extend_from_slice(&stsz);
    body.extend_from_slice(&stco);
    mp4_box(b"stbl", &body)
}

fn build_stsd(width: u16, height: u16, sps: &[u8], pps: &[u8]) -> Vec<u8> {
    let profile_idc = *sps.get(1).unwrap_or(&0x42);
    let profile_compat = *sps.get(2).unwrap_or(&0xC0);
    let level_idc = *sps.get(3).unwrap_or(&0x1F);

    let mut avcc_body = vec![
        1, // configurationVersion
        profile_idc,
        profile_compat,
        level_idc,
        0xFF, // 4-byte NAL unit lengths
        0xE1, // 1 SPS
    ];
    avcc_body.extend_from_slice(&(sps.len() as u16).to_be_bytes());
    avcc_body.extend_from_slice(sps);
    avcc_body.push(1); // 1 PPS
    avcc_body.extend_from_slice(&(pps.len() as u16).to_be_bytes());
    avcc_body.extend_from_slice(pps);
    let avcc = mp4_box(b"avcC", &avcc_body);

    let mut avc1_body = Vec::with_capacity(78 + avcc.len());
    avc1_body.extend_from_slice(&[0u8; 6]); // reserved
    avc1_body.extend_from_slice(&1u16.to_be_bytes()); // data_reference_index
    avc1_body.extend_from_slice(&[0u8; 16]); // pre_defined + reserved
    avc1_body.extend_from_slice(&width.to_be_bytes());
    avc1_body.extend_from_slice(&height.to_be_bytes());
    avc1_body.extend_from_slice(&0x0048_0000u32.to_be_bytes()); // horizresolution 72 dpi
    avc1_body.extend_from_slice(&0x0048_0000u32.to_be_bytes()); // vertresolution 72 dpi
    avc1_body.extend_from_slice(&0u32.to_be_bytes()); // reserved
    avc1_body.extend_from_slice(&1u16.to_be_bytes()); // frame_count
    avc1_body.extend_from_slice(&[0u8; 32]); // compressorname
    avc1_body.extend_from_slice(&0x0018u16.to_be_bytes()); // depth = 24
    avc1_body.extend_from_slice(&(-1i16).to_be_bytes()); // pre_defined = -1
    avc1_body.extend_from_slice(&avcc);
    let avc1 = mp4_box(b"avc1", &avc1_body);

    let mut stsd_body = Vec::with_capacity(4 + avc1.len());
    stsd_body.extend_from_slice(&1u32.to_be_bytes()); // entry_count = 1
    stsd_body.extend_from_slice(&avc1);
    mp4_full_box(b"stsd", 0, 0, &stsd_body)
}

fn build_stts(sample_deltas: &[u32]) -> Vec<u8> {
    let mut runs: Vec<(u32, u32)> = Vec::new();
    for &delta in sample_deltas {
        if let Some(last) = runs.last_mut()
            && last.1 == delta
        {
            last.0 += 1;
        } else {
            runs.push((1, delta));
        }
    }
    let mut body = Vec::with_capacity(4 + runs.len() * 8);
    body.extend_from_slice(&(runs.len() as u32).to_be_bytes());
    for (count, delta) in runs {
        body.extend_from_slice(&count.to_be_bytes());
        body.extend_from_slice(&delta.to_be_bytes());
    }
    mp4_full_box(b"stts", 0, 0, &body)
}

fn build_stss(sync_samples: &[u32]) -> Vec<u8> {
    let mut body = Vec::with_capacity(4 + sync_samples.len() * 4);
    body.extend_from_slice(&(sync_samples.len() as u32).to_be_bytes());
    for &idx in sync_samples {
        body.extend_from_slice(&idx.to_be_bytes());
    }
    mp4_full_box(b"stss", 0, 0, &body)
}

fn build_stsc(total_samples: u32) -> Vec<u8> {
    let mut body = Vec::with_capacity(16);
    body.extend_from_slice(&1u32.to_be_bytes()); // entry_count = 1
    body.extend_from_slice(&1u32.to_be_bytes()); // first_chunk = 1
    body.extend_from_slice(&total_samples.to_be_bytes()); // samples_per_chunk
    body.extend_from_slice(&1u32.to_be_bytes()); // sample_description_index = 1
    mp4_full_box(b"stsc", 0, 0, &body)
}

fn build_stsz(sample_sizes: &[u32]) -> Vec<u8> {
    let mut body = Vec::with_capacity(8 + sample_sizes.len() * 4);
    body.extend_from_slice(&0u32.to_be_bytes()); // sample_size = 0 (variable)
    body.extend_from_slice(&(sample_sizes.len() as u32).to_be_bytes());
    for &sz in sample_sizes {
        body.extend_from_slice(&sz.to_be_bytes());
    }
    mp4_full_box(b"stsz", 0, 0, &body)
}

fn build_stco(chunk_offset: u32) -> Vec<u8> {
    let mut body = Vec::with_capacity(8);
    body.extend_from_slice(&1u32.to_be_bytes()); // entry_count = 1
    body.extend_from_slice(&chunk_offset.to_be_bytes());
    mp4_full_box(b"stco", 0, 0, &body)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn contains_subslice(haystack: &[u8], needle: &[u8]) -> bool {
        haystack.windows(needle.len()).any(|w| w == needle)
    }

    #[test]
    fn records_red32x32_fixture_to_playable_mp4() {
        let dir = tempfile::tempdir().unwrap();
        let mp4_path = dir.path().join("red-32x32.mp4");
        let fixture = include_bytes!("testdata/red-32x32.annexb");

        let mut rec = MirrorRecorder::new(mp4_path.clone(), 32, 32, 30, None);
        rec.on_packet(false, 0, fixture).unwrap();
        // A second fixture frame exercises the monotonic timestamp fix-up.
        rec.on_packet(false, 0, fixture).unwrap();
        let saved = rec.finish().unwrap();
        assert_eq!(saved, mp4_path);
        let bytes = std::fs::read(&saved).unwrap();
        assert!(bytes.len() > 256, "unexpectedly small MP4: {} B", bytes.len());
        assert_eq!(&bytes[4..8], b"ftyp");
        assert!(contains_subslice(&bytes, b"moov"), "missing moov box");
        assert!(contains_subslice(&bytes, b"mdat"), "missing mdat box");
        assert!(contains_subslice(&bytes, b"avc1"), "missing avc1 sample entry");
        assert!(contains_subslice(&bytes, b"avcC"), "missing avcC config box");
    }

    #[test]
    fn refuses_non_annexb_audio_and_empty_finish() {
        let dir = tempfile::tempdir().unwrap();
        let mp4_path = dir.path().join("refused.mp4");
        let fixture = include_bytes!("testdata/red-32x32.annexb");

        let mut rec = MirrorRecorder::new(mp4_path.clone(), 32, 32, 30, None);
        // Parameter sets alone carry no picture: accepted but not recorded.
        let params = extract_sps_pps(fixture).expect("fixture has SPS and PPS");
        rec.on_packet(false, 0, &params).unwrap();
        // Non-Annex-B bytes are refused rather than muxed as garbage.
        assert!(rec.on_packet(true, 0, &[0x65, 0x88, 0x84]).is_err());
        // Video-only: phone audio is refused, never silently dropped.
        assert!(rec.on_audio(0, &[0u8; 480]).is_err());
        assert!(rec.finish().is_err());
        assert!(!mp4_path.exists(), "empty recording must leave no file");
    }

    #[test]
    fn local_timestamp_filename_formats_correctly() {
        let name = local_timestamp_filename("Screen", "mp4");
        assert!(name.starts_with("Screen "));
        assert!(name.ends_with(".mp4"));
    }
}
