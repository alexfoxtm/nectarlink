// SPDX-License-Identifier: GPL-3.0-or-later
//! macOS / non-Windows reverse mirroring stubs.

use nectarlink_core::{DeviceId, MirrorInput, MirrorStart, NodeEvent};

pub fn active_peer() -> Option<DeviceId> {
    None
}

pub fn is_active() -> bool {
    false
}

pub fn is_watching() -> bool {
    false
}

pub fn stop_watching() {}

pub fn start(_peer: DeviceId, _options: MirrorStart) {}

pub fn stop_for(_peer: &DeviceId) {}

pub fn stop_all() {}

pub fn request_keyframe(_peer: &DeviceId) {}

pub fn handle_input(_peer: &DeviceId, _session: u32, _input: MirrorInput) {}

pub fn on_event(_event: &NodeEvent) {}

pub fn set_mute_while_watched(_on: bool) {}
