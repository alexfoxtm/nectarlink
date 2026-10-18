// SPDX-License-Identifier: GPL-3.0-or-later
//! macOS sound ringing ("find my PC" from phone).

use std::{
    sync::atomic::{AtomicU64, Ordering},
    time::Duration,
};

const MAX_RING: Duration = Duration::from_secs(60);
static GENERATION: AtomicU64 = AtomicU64::new(0);

pub fn start_ringing() {
    let generation = GENERATION.fetch_add(1, Ordering::SeqCst) + 1;
    let _ = std::thread::Builder::new().name("mac-ring".into()).spawn(move || {
        let start = std::time::Instant::now();
        while start.elapsed() < MAX_RING && GENERATION.load(Ordering::SeqCst) == generation {
            let _ = std::process::Command::new("afplay").arg("/System/Library/Sounds/Ping.aiff").status();
            std::thread::sleep(Duration::from_millis(600));
        }
    });
}

pub fn stop_ringing() {
    GENERATION.fetch_add(1, Ordering::SeqCst);
    let _ = std::process::Command::new("killall").arg("afplay").output();
}
