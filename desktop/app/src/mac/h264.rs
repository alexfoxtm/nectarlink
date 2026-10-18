// SPDX-License-Identifier: GPL-3.0-or-later
//! macOS H.264 video decoding stubs.

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Colors {
    pub bt601: bool,
    pub full_range: bool,
}

#[derive(Debug)]
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
        Ok(Decoder { codec: codec.to_string() })
    }

    pub fn new_gpu(codec: &str) -> Result<Decoder, String> {
        Ok(Decoder { codec: codec.to_string() })
    }

    pub fn on_gpu(&self) -> bool {
        false
    }

    pub fn codec(&self) -> &str {
        &self.codec
    }

    pub fn decode(&mut self, _data: &[u8], _time_us: u64) -> Result<Vec<Picture>, String> {
        Ok(Vec::new())
    }

    pub fn flush(&mut self) {}
}

pub fn to_bgrx(picture: &Picture, width: u32, height: u32) -> Vec<u8> {
    let (w, h) = (width.min(picture.width) as usize, height.min(picture.height) as usize);
    vec![0; w * h * 4]
}

pub fn ensure_mf_started() -> Result<(), String> {
    Ok(())
}
