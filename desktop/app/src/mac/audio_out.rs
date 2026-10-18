// SPDX-License-Identifier: GPL-3.0-or-later
//! macOS audio output streaming player.

use std::sync::mpsc::{SyncSender, sync_channel};

#[derive(Debug)]
pub struct Player {
    #[allow(dead_code)]
    queue: SyncSender<Vec<u8>>,
}

impl Player {
    pub fn start(rate: u32, channels: u16) -> std::io::Result<Player> {
        Self::start_on(None, rate, channels)
    }

    pub fn start_on(_device: Option<String>, _rate: u32, _channels: u16) -> std::io::Result<Player> {
        let (queue, _) = sync_channel(64);
        Ok(Player { queue })
    }

    pub fn push(&self, _data: Vec<u8>) {}
}
