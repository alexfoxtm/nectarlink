// SPDX-License-Identifier: GPL-3.0-or-later
//! Ringing this Mac ("find my PC" from the phone) with one `afplay` child.
//!
//! Stop and restart signal only that child. They never use `killall`, so
//! another app's `afplay` keeps running. The generation and the child share
//! one mutex: replacing a ring takes the previous child out and stops it
//! before the new one is stored, and a child spawned after its generation
//! was superseded is stopped before it can be dropped.

use std::{
    process::{Child, Command, Stdio},
    sync::Mutex,
    time::{Duration, Instant},
};

const PING: &str = "/System/Library/Sounds/Ping.aiff";
const MAX_RING: Duration = Duration::from_secs(60);
const GAP: Duration = Duration::from_millis(600);
const POLL: Duration = Duration::from_millis(50);

struct Ring {
    generation: u64,
    child: Option<Child>,
}

static RING: Mutex<Ring> = Mutex::new(Ring { generation: 0, child: None });

fn ring() -> std::sync::MutexGuard<'static, Ring> {
    RING.lock().unwrap_or_else(|err| err.into_inner())
}

/// `Child`'s destructor does not stop the process. Wait only after a kill,
/// and never kill a child `try_wait` has already reaped: its pid may belong
/// to someone else by then.
fn stop_child(child: &mut Child) {
    let _ = child.kill();
    let _ = child.wait();
}

/// Ends the current ring and returns the generation the next one must use.
fn supersede(ring: &mut Ring) -> u64 {
    if let Some(mut child) = ring.child.take() {
        stop_child(&mut child);
    }
    ring.generation = ring.generation.wrapping_add(1);
    ring.generation
}

pub fn start_ringing() {
    let generation = {
        let mut ring = ring();
        supersede(&mut ring)
    };
    if let Err(err) = std::thread::Builder::new().name("mac-ring".into()).spawn(move || ring_loop(generation))
    {
        tracing::warn!(error = %err, "can't start the find-my-PC ring");
    }
}

pub fn stop_ringing() {
    let mut ring = ring();
    let _ = supersede(&mut ring);
}

fn ring_loop(generation: u64) {
    let started = Instant::now();
    let mut warned = false;
    while started.elapsed() < MAX_RING {
        if !still(generation) {
            return;
        }
        match spawn_own_child(generation, &mut warned) {
            Spawned::Stopped => return,
            Spawned::Retry => {
                if !pause(generation) {
                    return;
                }
            }
            Spawned::Playing => {
                if !wait_own_child(generation) || !pause(generation) {
                    return;
                }
            }
        }
    }
    let mut ring = ring();
    if ring.generation == generation
        && let Some(mut child) = ring.child.take()
    {
        stop_child(&mut child);
    }
}

fn still(generation: u64) -> bool {
    ring().generation == generation
}

enum Spawned {
    Playing,
    Retry,
    Stopped,
}

fn spawn_own_child(generation: u64, warned: &mut bool) -> Spawned {
    let mut child = match Command::new("afplay")
        .arg(PING)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
    {
        Ok(child) => child,
        Err(err) => {
            if !*warned {
                tracing::warn!(error = %err, "can't start the find-my-PC sound");
                *warned = true;
            }
            return Spawned::Retry;
        }
    };
    {
        let mut ring = ring();
        if ring.generation == generation {
            if let Some(mut previous) = ring.child.take() {
                stop_child(&mut previous);
            }
            ring.child = Some(child);
            return Spawned::Playing;
        }
    }
    stop_child(&mut child);
    Spawned::Stopped
}

/// `true` when this generation's child has exited and the loop may play again.
fn wait_own_child(generation: u64) -> bool {
    loop {
        let mut ring = ring();
        if ring.generation != generation {
            return false;
        }
        let state = match ring.child.as_mut() {
            None => ChildState::Absent,
            Some(child) => match child.try_wait() {
                Ok(None) => ChildState::Running,
                Ok(Some(_)) => ChildState::Exited,
                Err(_) => ChildState::Lost,
            },
        };
        match state {
            ChildState::Running => {
                drop(ring);
                std::thread::sleep(POLL);
            }
            ChildState::Exited | ChildState::Absent => {
                ring.child = None;
                return true;
            }
            ChildState::Lost => {
                if let Some(mut child) = ring.child.take() {
                    stop_child(&mut child);
                }
                return true;
            }
        }
    }
}

enum ChildState {
    Running,
    Exited,
    Lost,
    Absent,
}

fn pause(generation: u64) -> bool {
    let mut left = GAP;
    while !left.is_zero() {
        if !still(generation) {
            return false;
        }
        let slice = POLL.min(left);
        std::thread::sleep(slice);
        left = left.saturating_sub(slice);
    }
    still(generation)
}
