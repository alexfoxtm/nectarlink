// SPDX-License-Identifier: GPL-3.0-or-later
//! macOS / non-Windows Play on Phone stubs.

use nectarlink_core::DeviceId;

pub fn phone() -> Option<(DeviceId, String)> {
    None
}

pub fn watchable() -> Option<String> {
    None
}

pub fn state() -> &'static str {
    "idle"
}

pub fn set_on(_on: bool) {}

pub fn play(_player: String) {}

pub struct Watched;

#[derive(Debug, Clone, Copy)]
pub struct Place {
    pub window: isize,
}

impl Watched {
    pub fn follow(_player: &str) -> Watched {
        Watched
    }

    pub fn place(&self) -> Option<Place> {
        None
    }
}
