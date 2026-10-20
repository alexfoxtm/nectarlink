// SPDX-License-Identifier: GPL-3.0-or-later
//! Jitter buffer for streamed 16-bit PCM. Same timing as the Windows
//! player: wait until a little sound has arrived, play silence through a
//! stall, and drop the oldest whole frames when the queue piles up.

use std::{
    collections::VecDeque,
    sync::atomic::{AtomicUsize, Ordering},
};

/// Sound packets waiting for the player thread, at most.
pub(super) const QUEUE_PACKETS: usize = 64;
/// How much sound is gathered before playing (and again after a stall).
const START_MS: u32 = 40;
/// More than this waiting, and the oldest is dropped down to [`TRIM_TO_MS`].
/// Also the most sound that may wait for the player thread.
const MOST_MS: u32 = 200;
const TRIM_TO_MS: u32 = 60;

fn bytes_per_ms(rate: u32, channels: u16) -> usize {
    (2 * usize::from(channels)).max(1) * rate as usize / 1000
}

/// Bytes handed to the player thread and not yet in the jitter buffer.
/// Holds at most [`MOST_MS`] of sound in this format.
#[derive(Debug)]
pub(super) struct Budget {
    queued: AtomicUsize,
    most: usize,
}

impl Budget {
    pub(super) fn new(rate: u32, channels: u16) -> Budget {
        let most = bytes_per_ms(rate, channels).saturating_mul(MOST_MS as usize);
        Budget { queued: AtomicUsize::new(0), most }
    }

    /// Reserves room for `len` bytes, or refuses when that would queue more
    /// than [`MOST_MS`] of sound.
    pub(super) fn take(&self, len: usize) -> bool {
        self.queued
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |queued| {
                queued.checked_add(len).filter(|total| *total <= self.most)
            })
            .is_ok()
    }

    /// Returns `len` bytes reserved by [`Budget::take`].
    pub(super) fn give(&self, len: usize) {
        let _ = self
            .queued
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |queued| Some(queued.saturating_sub(len)));
    }

    #[cfg(test)]
    pub(super) fn queued(&self) -> usize {
        self.queued.load(Ordering::Acquire)
    }
}

/// The jitter buffer, in bytes of interleaved 16-bit samples.
#[derive(Debug)]
pub(super) struct Jitter {
    bytes: VecDeque<u8>,
    /// Bytes in one sample frame (one sample on every channel).
    frame: usize,
    /// Bytes in one millisecond.
    per_ms: usize,
    /// Gathering sound before playing.
    waiting: bool,
    /// Times it ran dry, and bytes dropped for piling up or for ending in
    /// part of a frame (for the log).
    stalls: u32,
    trimmed: usize,
}

impl Jitter {
    pub(super) fn new(rate: u32, channels: u16) -> Jitter {
        let frame = 2 * usize::from(channels);
        debug_assert!(frame > 0);
        Jitter {
            bytes: VecDeque::new(),
            frame: frame.max(1),
            per_ms: bytes_per_ms(rate, channels),
            waiting: true,
            stalls: 0,
            trimmed: 0,
        }
    }

    pub(super) fn stalls(&self) -> u32 {
        self.stalls
    }

    pub(super) fn trimmed_ms(&self) -> usize {
        self.trimmed / self.per_ms.max(1)
    }

    pub(super) fn push(&mut self, data: &[u8]) {
        // A packet ending in part of a frame loses that part. Kept, it would
        // shift every later sample onto the wrong channel or byte.
        let whole = data.len() / self.frame * self.frame;
        self.trimmed += data.len() - whole;
        let ceiling = self.per_ms.saturating_mul(MOST_MS as usize);
        self.bytes.extend(bounded(&data[..whole], ceiling, self.frame));
        if self.bytes.len() > ceiling {
            let keep = self.per_ms.saturating_mul(TRIM_TO_MS as usize);
            // Whole frames only, or the channels swap.
            let drop_n = self.bytes.len().saturating_sub(keep) / self.frame * self.frame;
            self.bytes.drain(..drop_n);
            self.trimmed += drop_n;
        }
    }

    /// Fills `out` (whole frames): sound when there is some, silence when
    /// gathering or stalled. Bytes are copied in order.
    pub(super) fn fill(&mut self, out: &mut [u8]) {
        if self.waiting && self.bytes.len() >= self.per_ms.saturating_mul(START_MS as usize) {
            self.waiting = false;
        }
        let mut n = 0;
        if !self.waiting {
            n = out.len().min(self.bytes.len()) / self.frame * self.frame;
            if n > 0 {
                let (first_half, second_half) = self.bytes.as_slices();
                let first = first_half.len().min(n);
                out[..first].copy_from_slice(&first_half[..first]);
                if first < n {
                    let second = n - first;
                    out[first..n].copy_from_slice(&second_half[..second]);
                }
                self.bytes.drain(..n);
            }
            if n < out.len() {
                // Ran dry: gather again before playing on.
                self.waiting = true;
                self.stalls += 1;
            }
        }
        out[n..].fill(0);
    }

    #[cfg(test)]
    pub(super) fn queued(&self) -> usize {
        self.bytes.len()
    }
}

/// Keeps the newest whole frames, and never more than `ceiling` bytes.
fn bounded(data: &[u8], ceiling: usize, frame: usize) -> &[u8] {
    if frame == 0 || data.len() <= ceiling {
        return data;
    }
    let extra = data.len() - ceiling;
    let drop_n = extra.div_ceil(frame) * frame;
    if drop_n >= data.len() { &[] } else { &data[drop_n..] }
}
