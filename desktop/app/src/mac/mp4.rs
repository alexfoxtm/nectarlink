// SPDX-License-Identifier: GPL-3.0-or-later
//! macOS MP4 recording stubs.

use std::{
    path::PathBuf,
    time::{Instant, SystemTime, UNIX_EPOCH},
};

pub fn local_timestamp_filename(prefix: &str, ext: &str) -> String {
    let now = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs());
    format!("{prefix} {now}.{ext}")
}

pub struct MirrorRecorder {
    path: PathBuf,
    started_at: Instant,
    started_epoch_ms: i64,
}

impl std::fmt::Debug for MirrorRecorder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MirrorRecorder").field("path", &self.path).finish_non_exhaustive()
    }
}

impl MirrorRecorder {
    pub fn new(
        path: PathBuf,
        _width: u32,
        _height: u32,
        _fps: u32,
        _audio_cfg: Option<(u32, u8)>,
    ) -> MirrorRecorder {
        let started_epoch_ms =
            SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_millis() as i64);
        MirrorRecorder { path, started_at: Instant::now(), started_epoch_ms }
    }

    pub fn started_at(&self) -> Instant {
        self.started_at
    }

    pub fn started_epoch_ms(&self) -> i64 {
        self.started_epoch_ms
    }

    pub fn set_sps_pps(&mut self, _sps_pps: &[u8]) {}

    pub fn on_config(&mut self, _width: u32, _height: u32) {}

    pub fn on_audio_config(&mut self, _rate: u32, _channels: u8) {}

    pub fn on_packet(&mut self, _keyframe: bool, _time_us: u64, _data: &[u8]) -> Result<(), String> {
        Ok(())
    }

    pub fn on_audio(&mut self, _time_us: u64, _pcm: &[u8]) -> Result<(), String> {
        Ok(())
    }

    pub fn finish(self) -> Result<PathBuf, String> {
        Ok(self.path)
    }
}

pub fn merge_sps_pps(sps_pps: &mut Vec<u8>, data: &[u8]) {
    sps_pps.extend_from_slice(data);
}
