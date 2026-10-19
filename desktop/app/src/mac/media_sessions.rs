// SPDX-License-Identifier: GPL-3.0-or-later
//! macOS media session integration.
//!
//! macOS has no public API that reads or drives other apps' Now Playing
//! state, so this Mac never reports players (`media.control` isn't
//! advertised) and a command for one is refused as unsupported rather than
//! acknowledged without acting.

use nectarlink_core::{MediaAction, MediaError, MediaPlayer};

pub fn start(_on_change: impl Fn(Vec<MediaPlayer>) + Send + Sync + 'static) {}

pub fn command(_player: &str, _action: MediaAction, _pos: Option<u64>) -> Result<(), MediaError> {
    Err(MediaError::Unsupported)
}

pub fn pause_for_call() {}

pub fn resume_after_call() {}

#[allow(dead_code)]
pub fn says_video(_id: &str) -> bool {
    false
}
