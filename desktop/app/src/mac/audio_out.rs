// SPDX-License-Identifier: GPL-3.0-or-later
//! Playing a mirrored phone's sound on this Mac: 16-bit little-endian
//! interleaved PCM goes through a small jitter buffer to an AudioQueue.
//!
//! `start(rate, channels)` plays on the default output. `start_on` with a
//! device id plays on that output device, or returns an error when the id
//! is missing or has no output channels. It does not substitute the default.
//! Packets are the bytes callers already send (`pcm_s16le` from the mirror
//! stream, and the phone microphone after its own gain step).

mod decide;
mod hal;
mod jitter;

use std::{
    io,
    sync::{
        Arc,
        mpsc::{Receiver, RecvTimeoutError, SyncSender, TrySendError, sync_channel},
    },
    thread::JoinHandle,
    time::Duration,
};

use decide::PcmFormat;
use jitter::Budget;

/// How long `start_on` waits for the queue to open, and `drop` for it to
/// close. Past that the thread is left to finish by itself: it still frees
/// the callback state only after the queue is disposed.
const START_WAIT: Duration = Duration::from_secs(2);
const STOP_WAIT: Duration = Duration::from_secs(2);

/// Plays sound until dropped.
#[derive(Debug)]
pub struct Player {
    queue: Option<SyncSender<Vec<u8>>>,
    budget: Arc<Budget>,
    /// Disconnects once the playback thread has disposed the queue.
    done: Receiver<()>,
    thread: Option<JoinHandle<()>>,
}

impl Player {
    /// Starts playing `channels` × 16-bit PCM at `rate` on the default
    /// output device.
    pub fn start(rate: u32, channels: u16) -> io::Result<Player> {
        Self::start_on(None, rate, channels)
    }

    /// As [`Player::start`], on the output device with this UID, or the
    /// default output when `device` is `None`. A UID that is not an output
    /// device is an error.
    ///
    /// Waits at most [`START_WAIT`] for the device lookup and the queue.
    pub fn start_on(device: Option<String>, rate: u32, channels: u16) -> io::Result<Player> {
        let format = PcmFormat::new(rate, channels)?;
        let budget = Arc::new(Budget::new(rate, channels));
        let (queue, packets) = sync_channel(jitter::QUEUE_PACKETS);
        let (ready_tx, ready_rx) = sync_channel(1);
        let (done_tx, done) = sync_channel::<()>(0);
        let thread_budget = Arc::clone(&budget);
        let thread = std::thread::Builder::new().name("mirror-sound".into()).spawn(move || {
            let _done = done_tx;
            hal::play(device.as_deref(), format, packets, &thread_budget, ready_tx);
        })?;
        match ready_rx.recv_timeout(START_WAIT) {
            Ok(Ok(())) => Ok(Player { queue: Some(queue), budget, done, thread: Some(thread) }),
            Ok(Err(error)) => {
                let _ = thread.join();
                Err(error)
            }
            Err(RecvTimeoutError::Disconnected) => {
                let _ = thread.join();
                Err(io::Error::other("audio output stopped before the queue started"))
            }
            // Returning drops `ready_rx`, so a queue that opens later is
            // closed by its own thread.
            Err(RecvTimeoutError::Timeout) => {
                Err(io::Error::new(io::ErrorKind::TimedOut, "the audio output did not start in time"))
            }
        }
    }

    /// Hands over a packet without waiting. It is dropped when the player
    /// is far behind (it would be too late to hear anyway), or when it
    /// would put more than 200 ms of sound in front of the player thread.
    pub fn push(&self, data: Vec<u8>) {
        let Some(queue) = &self.queue else { return };
        let len = data.len();
        if !self.budget.take(len) {
            tracing::debug!("a sound packet was dropped");
            return;
        }
        if let Err(error) = queue.try_send(data) {
            self.budget.give(len);
            if let TrySendError::Full(_) = error {
                tracing::debug!("a sound packet was dropped");
            }
        }
    }
}

impl Drop for Player {
    fn drop(&mut self) {
        // Dropping the sender unblocks the thread, which stops and disposes
        // the queue before it frees the callback state.
        self.queue.take();
        let Some(thread) = self.thread.take() else { return };
        match self.done.recv_timeout(STOP_WAIT) {
            Err(RecvTimeoutError::Timeout) => tracing::warn!("the sound output is slow to stop"),
            _ => {
                let _ = thread.join();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::io::ErrorKind;

    use super::{decide, jitter};

    /// 48 kHz stereo: 192 bytes a millisecond.
    const MS: usize = 192;

    #[test]
    fn plays_interleaved_pcm_after_the_jitter_buffer_fills() {
        let format = decide::PcmFormat::new(48_000, 2).unwrap();
        assert_eq!(format.frame, 4);
        assert_eq!(format.flags, decide::FLAG_SIGNED | decide::FLAG_PACKED);
        assert_eq!(format.flags & decide::FLAG_BIG_ENDIAN, 0);
        assert_eq!(format.flags & decide::FLAG_NON_INTERLEAVED, 0);
        assert_eq!(format.buffer_bytes % 4, 0);
        assert!(format.buffer_bytes <= 16 * 1024);
        let mono = decide::PcmFormat::new(16_000, 1).unwrap();
        assert_eq!(mono.frame, 2);
        assert_eq!(mono.flags, format.flags);
        assert_eq!(decide::PcmFormat::new(96_000, 2).unwrap().buffer_bytes, 7_680);
        assert_eq!(decide::PcmFormat::new(8_000, 1).unwrap().buffer_bytes, 320);

        let mut packet = Vec::with_capacity(40 * MS);
        for n in 0..40 * (MS / 4) {
            let left = n as u16;
            let right = left ^ 0x00FF;
            packet.extend_from_slice(&left.to_le_bytes());
            packet.extend_from_slice(&right.to_le_bytes());
        }
        let mut jitter = jitter::Jitter::new(48_000, 2);
        let mut out = vec![9u8; 10 * MS];
        jitter.push(&packet[..30 * MS]);
        jitter.fill(&mut out);
        assert!(out.iter().all(|&byte| byte == 0), "silence while gathering");
        jitter.push(&packet[30 * MS..]);
        jitter.fill(&mut out);
        assert_eq!(out, packet[..10 * MS], "bytes stay little-endian and interleaved");

        let mut stalled = jitter::Jitter::new(48_000, 2);
        stalled.push(&vec![1; 45 * MS]);
        let mut dry = vec![9u8; 50 * MS];
        stalled.fill(&mut dry);
        assert!(dry[..45 * MS].iter().all(|&byte| byte == 1));
        assert!(dry[45 * MS..].iter().all(|&byte| byte == 0), "silence when dry");
        stalled.push(&vec![2; 10 * MS]);
        let mut again = vec![9u8; 5 * MS];
        stalled.fill(&mut again);
        assert!(again.iter().all(|&byte| byte == 0), "gathers before playing on");

        let mut piled = jitter::Jitter::new(48_000, 2);
        piled.push(&vec![1; 150 * MS]);
        piled.push(&vec![2; 60 * MS]);
        let mut kept = vec![9u8; 80 * MS];
        piled.fill(&mut kept);
        assert!(kept[..60 * MS].iter().all(|&byte| byte == 2), "the newest whole frames stay");
        assert!(kept[60 * MS..].iter().all(|&byte| byte == 0), "older frames were dropped");
    }

    #[test]
    fn rejects_a_selected_device_that_cannot_play() {
        let devices = vec![
            decide::Listed { uid: "spk".into(), output_channels: 2 },
            decide::Listed { uid: "mic".into(), output_channels: 0 },
        ];
        assert_eq!(decide::choose(None, &devices, Some("spk")).unwrap(), "spk");
        assert_eq!(decide::choose(Some("spk"), &devices, Some("mic")).unwrap(), "spk");

        let missing = decide::choose(Some("gone"), &devices, Some("spk")).unwrap_err();
        assert_eq!(missing.kind(), ErrorKind::NotFound);
        assert_eq!(missing.to_string(), "selected device gone is not an output device");

        let literal_default = decide::choose(Some("default"), &devices, Some("spk")).unwrap_err();
        assert_eq!(literal_default.kind(), ErrorKind::NotFound);
        assert_eq!(literal_default.to_string(), "selected device default is not an output device");

        let input_only = decide::choose(Some("mic"), &devices, Some("spk")).unwrap_err();
        assert_eq!(input_only.kind(), ErrorKind::Unsupported);
        assert_eq!(input_only.to_string(), "selected device mic has no output channels");

        assert_eq!(decide::choose(None, &devices, Some("mic")).unwrap_err().kind(), ErrorKind::NotFound);
        assert_eq!(decide::choose(None, &devices, None).unwrap_err().kind(), ErrorKind::NotFound);
        assert_eq!(
            decide::choose(Some(""), &devices, Some("spk")).unwrap_err().kind(),
            ErrorKind::InvalidInput
        );

        assert_eq!(decide::PcmFormat::new(7_000, 2).unwrap_err().kind(), ErrorKind::InvalidInput);
        assert_eq!(decide::PcmFormat::new(48_000, 6).unwrap_err().kind(), ErrorKind::InvalidInput);
        assert_eq!(decide::PcmFormat::new(48_000, 0).unwrap_err().kind(), ErrorKind::InvalidInput);

        assert_eq!(decide::output_channels(&[]), None);
        assert_eq!(decide::output_channels(&0u32.to_ne_bytes()), Some(0));
        assert_eq!(decide::output_channels(&audio_buffer_list(&[2])), Some(2));
        assert_eq!(decide::output_channels(&audio_buffer_list(&[0, 0])), Some(0));
        let mut too_many = [0u8; 4];
        too_many.copy_from_slice(&1_000u32.to_ne_bytes());
        assert_eq!(decide::output_channels(&too_many), None);
        assert_eq!(decide::output_channels(&1u32.to_ne_bytes()), None);
        let cut_short = audio_buffer_list(&[2, 2]);
        assert_eq!(decide::output_channels(&cut_short[..cut_short.len() - 1]), None);
        assert_eq!(decide::max_stream_bytes(), 8 + 32 * 16);
        keeps_frames_whole_and_bounds_queued_sound();
    }

    fn keeps_frames_whole_and_bounds_queued_sound() {
        let mut jitter = jitter::Jitter::new(48_000, 2);
        let mut ragged = vec![0x11u8; 40 * MS];
        ragged.extend_from_slice(&[0xAA, 0xBB, 0xCC]);
        jitter.push(&ragged);
        assert_eq!(jitter.queued(), 40 * MS, "the partial frame is not kept");
        jitter.push(&[0xEE]);
        assert_eq!(jitter.queued(), 40 * MS);
        jitter.push(&[1, 2, 3, 4, 5, 6, 7, 8]);
        let mut out = vec![9u8; 40 * MS + 8];
        jitter.fill(&mut out);
        assert!(out[..40 * MS].iter().all(|&byte| byte == 0x11));
        assert_eq!(out[40 * MS..], [1, 2, 3, 4, 5, 6, 7, 8], "the next packet stays on its channels");

        let mut mono = jitter::Jitter::new(8_000, 1);
        mono.push(&[1, 2, 3]);
        assert_eq!(mono.queued(), 2);

        let mut big = jitter::Jitter::new(48_000, 2);
        let mut packet = vec![1u8; 100 * MS];
        packet.resize(300 * MS, 2);
        big.push(&packet);
        assert_eq!(big.queued(), 200 * MS, "one packet never holds more than 200 ms");
        let mut kept = vec![9u8; 200 * MS];
        big.fill(&mut kept);
        assert!(kept.iter().all(|&byte| byte == 2), "the newest frames stay");

        let budget = jitter::Budget::new(48_000, 2);
        assert!(budget.take(100 * MS));
        assert!(budget.take(100 * MS));
        assert!(!budget.take(4), "no more than 200 ms waits for the player thread");
        budget.give(100 * MS);
        assert!(budget.take(10 * MS));
        assert_eq!(budget.queued(), 110 * MS);
        assert!(!budget.take(usize::MAX));
        assert_eq!(budget.queued(), 110 * MS);
        budget.give(usize::MAX);
        assert_eq!(budget.queued(), 0);

        let fast = jitter::Budget::new(96_000, 2);
        assert!(fast.take(76_800));
        assert!(!fast.take(1));
        let slow = jitter::Budget::new(8_000, 1);
        assert!(!slow.take(3_201));
        assert!(slow.take(3_200));
    }

    fn audio_buffer_list(channels: &[u32]) -> Vec<u8> {
        let start = std::mem::offset_of!(decide::AudioBufferList, buffers);
        let stride = std::mem::size_of::<decide::AudioBuffer>();
        let mut bytes = vec![0u8; start + channels.len() * stride];
        bytes[..4].copy_from_slice(&(channels.len() as u32).to_ne_bytes());
        for (index, channel_count) in channels.iter().enumerate() {
            let offset = start + index * stride;
            bytes[offset..offset + 4].copy_from_slice(&channel_count.to_ne_bytes());
        }
        bytes
    }
}
