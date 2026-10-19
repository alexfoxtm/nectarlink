// SPDX-License-Identifier: GPL-3.0-or-later
//! macOS H.264 video decoding stubs.

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Colors {
    pub bt601: bool,
    pub full_range: bool,
}

#[derive(Debug)]
#[allow(dead_code)]
pub struct Picture {
    pub width: u32,
    pub height: u32,
    pub stride: usize,
    pub plane_rows: usize,
    pub nv12: Vec<u8>,
    pub colors: Colors,
}

#[derive(Debug, Default)]
pub struct Decoder {
    codec: String,
}

impl Decoder {
    pub fn new_for_codec(codec: &str) -> Result<Decoder, String> {
        Err(format!("video decoding ({codec}) is not yet supported on macOS"))
    }

    pub fn new_gpu(codec: &str) -> Result<Decoder, String> {
        Self::new_for_codec(codec)
    }

    pub fn on_gpu(&self) -> bool {
        false
    }

    pub fn codec(&self) -> &str {
        &self.codec
    }

    pub fn decode(&mut self, _data: &[u8], _time_us: u64) -> Result<Vec<Picture>, String> {
        Err("video decoding is not supported on macOS".into())
    }

    pub fn flush(&mut self) {}
}

pub fn to_bgrx(picture: &Picture, width: u32, height: u32) -> Vec<u8> {
    let (w, h) = (width.min(picture.width) as usize, height.min(picture.height) as usize);
    vec![0; w * h * 4]
}

#[allow(dead_code)]
pub fn ensure_mf_started() -> Result<(), String> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn h264_decoder_reports_unsupported_on_creation() {
        assert!(Decoder::new_for_codec("h264").is_err());
        assert!(Decoder::new_gpu("h264").is_err());
    }

    #[test]
    fn h264_decode_reports_unsupported() {
        let mut decoder = Decoder::default();
        assert!(decoder.decode(&[0, 0, 0, 1], 0).is_err());
    }
}
