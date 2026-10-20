// SPDX-License-Identifier: GPL-3.0-or-later
//! Mac view-only phone-screen receive seam (docs/protocol/mirror.md).
//!
//! The production receive path is shared with Windows in
//! [`crate::mirror`]: the core hands an authenticated stream to
//! [`crate::mirror::sink`], whose decoder thread runs [`Decoder`] below and
//! publishes each picture to the production `VideoView` through
//! `ffi::video_frame` under the `<device>/<session>` stream key the QML
//! `MirrorWindow` shows. Closing the window, a disconnect, or a replaced
//! stream cancels by stream ownership (`shows`), never by clearing a
//! successor; `ffi::video_clear` blanks only the ended stream's views.
//!
//! What stays off until Root's native acceptance (no changes here enable
//! them): the core still refuses every incoming video stream on this Mac
//! because `compute_capabilities` does not advertise `mirror.view`, the
//! request carries `audio: false`, no `Player` is ever constructed while
//! `mirror.listen` is unadvertised, `screen_share::READY` stays false so
//! this Mac shares nothing back, and input stays a [`crate::mirror`] no-op
//! (`VIEW_ONLY`). [`seam_check`] proves the decode side is present and the
//! capture/audio sides stay refused, without touching hardware, the
//! network, or any personal device. The `#[cfg(test)]` fixture in
//! [`crate::mirror`] drives this exact seam end to end.

use super::{h264::Decoder, mp4::MirrorRecorder};

/// The Mac receive decoder: VideoToolbox H.264 only, like the production
/// decode thread uses.
#[allow(dead_code)]
pub fn new_decoder() -> Result<Decoder, String> {
    Decoder::new_gpu("h264")
}

/// Proves the receive seam is present and the capture/audio sides stay off,
/// without opening hardware, the network, or any recording:
/// H.264 decodes, anything else is refused, and the recording stub keeps
/// refusing packets (screen recording is unsupported on macOS).
#[allow(dead_code)]
pub fn seam_check() -> Result<(), String> {
    let _decoder = new_decoder()?;
    if Decoder::new_for_codec("hevc").is_ok() {
        return Err("mac receive seam unexpectedly decodes hevc".into());
    }
    let mut recorder = MirrorRecorder::new(std::path::PathBuf::from("seam-check.mp4"), 32, 32, 60, None);
    if recorder.on_packet(true, 0, &[0, 0, 0, 1, 0x65]).is_ok() {
        return Err("mac receive seam unexpectedly records".into());
    }
    // No `audio_out::Player` is constructed here on purpose: the receive
    // path never plays while `mirror.listen` stays unadvertised, and
    // opening an AudioQueue is Root's native-acceptance step, not this check's.
    Ok(())
}
