// SPDX-License-Identifier: GPL-3.0-or-later
//! macOS image conversion and scaling.

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Bitmap {
    pub width: u32,
    pub height: u32,
    pub bgra: Vec<u8>,
}

pub fn decode(bytes: &[u8]) -> Result<Bitmap, String> {
    if bytes.is_empty() {
        return Err("empty image data".into());
    }
    let out = crate::bridge::native::ffi::decode_image(bytes);
    if out.len() >= 8 {
        let width = u32::from_le_bytes(out[0..4].try_into().unwrap_or([0; 4]));
        let height = u32::from_le_bytes(out[4..8].try_into().unwrap_or([0; 4]));
        let bgra = out[8..].to_vec();
        Ok(Bitmap { width, height, bgra })
    } else {
        Err("failed to decode image".into())
    }
}

pub fn encode_png(bitmap: &Bitmap) -> Result<Vec<u8>, String> {
    // Bounded PR38 reuse: linux/image.rs encode_png validation (upstream PR38
    // head 94f2e461edc6255440baf5220d837d41c2c97daf, GPL-3.0-or-later, same as
    // this file), extended to bound the existing Mac encoder's u32 row,
    // capacity, and IDAT-length arithmetic before it allocates or reads pixels.
    let expected = checked_png_bgra_len(bitmap.width, bitmap.height)?;
    if bitmap.bgra.len() < expected {
        return Err("bitmap buffer too short".into());
    }
    Ok(create_png(bitmap.width, bitmap.height, &bitmap.bgra))
}

fn checked_png_bgra_len(width: u32, height: u32) -> Result<usize, String> {
    if width == 0 || height == 0 {
        return Err("empty bitmap".into());
    }

    // create_png evaluates width * 4 + 1 and height * row_bytes as u32.
    let row_bytes = width
        .checked_mul(4)
        .and_then(|pixels| pixels.checked_add(1))
        .ok_or_else(|| "bitmap dimensions overflow".to_string())?;
    let raw_len_u32 =
        height.checked_mul(row_bytes).ok_or_else(|| "bitmap dimensions overflow".to_string())?;
    let raw_len = <usize as std::convert::TryFrom<u32>>::try_from(raw_len_u32)
        .map_err(|_| "bitmap dimensions overflow".to_string())?;

    // The existing encoder stores raw data in <= 65,535-byte deflate blocks.
    // The IDAT payload is zlib header (2) + raw bytes + 5 bytes per block
    // + Adler-32 (4), and write_chunk stores its length as u32.
    let full_blocks = raw_len / 65_535;
    let stored_blocks = if raw_len % 65_535 == 0 {
        full_blocks
    } else {
        full_blocks.checked_add(1).ok_or_else(|| "bitmap dimensions overflow".to_string())?
    };
    let stored_overhead =
        stored_blocks.checked_mul(5).ok_or_else(|| "bitmap dimensions overflow".to_string())?;
    let idat_len = raw_len
        .checked_add(stored_overhead)
        .and_then(|length| length.checked_add(6))
        .ok_or_else(|| "bitmap dimensions overflow".to_string())?;
    if <u32 as std::convert::TryFrom<usize>>::try_from(idat_len).is_err() {
        return Err("bitmap dimensions overflow".into());
    }

    let width = <usize as std::convert::TryFrom<u32>>::try_from(width)
        .map_err(|_| "bitmap dimensions overflow".to_string())?;
    let height = <usize as std::convert::TryFrom<u32>>::try_from(height)
        .map_err(|_| "bitmap dimensions overflow".to_string())?;
    width
        .checked_mul(height)
        .and_then(|pixels| pixels.checked_mul(4))
        .ok_or_else(|| "bitmap dimensions overflow".to_string())
}

#[allow(dead_code)]
pub fn bmp_from_dib(dib: &[u8]) -> Option<Vec<u8>> {
    Some(dib.to_vec())
}

pub fn scale_to(bitmap: &Bitmap, max: u32) -> Bitmap {
    let longest = bitmap.width.max(bitmap.height);
    if longest <= max || longest == 0 {
        return bitmap.clone();
    }
    let scale = f64::from(max) / f64::from(longest);
    let width = ((f64::from(bitmap.width) * scale).round() as u32).max(1);
    let height = ((f64::from(bitmap.height) * scale).round() as u32).max(1);
    let mut bgra = Vec::with_capacity((width * height * 4) as usize);
    for y in 0..height {
        let (y0, y1) = (
            y * bitmap.height / height,
            ((y + 1) * bitmap.height / height).max(y * bitmap.height / height + 1),
        );
        for x in 0..width {
            let (x0, x1) = (
                x * bitmap.width / width,
                ((x + 1) * bitmap.width / width).max(x * bitmap.width / width + 1),
            );
            let mut sum = [0u32; 4];
            for sy in y0..y1 {
                for sx in x0..x1 {
                    let i = ((sy * bitmap.width + sx) * 4) as usize;
                    if i + 4 <= bitmap.bgra.len() {
                        for (total, &value) in sum.iter_mut().zip(&bitmap.bgra[i..i + 4]) {
                            *total += u32::from(value);
                        }
                    }
                }
            }
            let count = (y1 - y0) * (x1 - x0);
            if count > 0 {
                for total in sum {
                    bgra.push(total.checked_div(count).unwrap_or(0) as u8);
                }
            } else {
                bgra.extend_from_slice(&[0, 0, 0, 255]);
            }
        }
    }
    Bitmap { width, height, bgra }
}

fn create_png(width: u32, height: u32, bgra: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(b"\x89PNG\r\n\x1a\n");
    let mut ihdr = Vec::new();
    ihdr.extend_from_slice(&width.to_be_bytes());
    ihdr.extend_from_slice(&height.to_be_bytes());
    ihdr.push(8);
    ihdr.push(6); // RGBA
    ihdr.push(0);
    ihdr.push(0);
    ihdr.push(0);
    write_chunk(&mut out, b"IHDR", &ihdr);

    let mut raw = Vec::with_capacity((height * (width * 4 + 1)) as usize);
    for y in 0..height {
        raw.push(0);
        let row_start = (y * width * 4) as usize;
        let row_end = row_start + (width * 4) as usize;
        if row_end <= bgra.len() {
            for pixel in bgra[row_start..row_end].as_chunks::<4>().0 {
                raw.push(pixel[2]); // R
                raw.push(pixel[1]); // G
                raw.push(pixel[0]); // B
                raw.push(pixel[3]); // A
            }
        }
    }

    let mut zlib = Vec::new();
    zlib.push(0x78);
    zlib.push(0x01);

    for (i, chunk) in raw.chunks(65535).enumerate() {
        let is_last = i + 1 == raw.chunks(65535).len();
        zlib.push(if is_last { 0x01 } else { 0x00 });
        let len = chunk.len() as u16;
        let nlen = !len;
        zlib.extend_from_slice(&len.to_le_bytes());
        zlib.extend_from_slice(&nlen.to_le_bytes());
        zlib.extend_from_slice(chunk);
    }
    let mut s1: u32 = 1;
    let mut s2: u32 = 0;
    for &b in &raw {
        s1 = (s1 + b as u32) % 65521;
        s2 = (s2 + s1) % 65521;
    }
    let adler = (s2 << 16) | s1;
    zlib.extend_from_slice(&adler.to_be_bytes());

    write_chunk(&mut out, b"IDAT", &zlib);
    write_chunk(&mut out, b"IEND", &[]);
    out
}

fn write_chunk(out: &mut Vec<u8>, tag: &[u8; 4], data: &[u8]) {
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    out.extend_from_slice(tag);
    out.extend_from_slice(data);
    let mut crc = 0xffff_ffff_u32;
    for &b in tag.iter().chain(data.iter()) {
        crc ^= b as u32;
        for _ in 0..8 {
            if crc & 1 != 0 {
                crc = (crc >> 1) ^ 0xedb8_8320;
            } else {
                crc >>= 1;
            }
        }
    }
    out.extend_from_slice(&(crc ^ 0xffff_ffff_u32).to_be_bytes());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decode_rejects_empty_data() {
        assert!(decode(&[]).is_err());
    }

    #[test]
    fn encode_and_decode_roundtrip_preserves_pixels() {
        let original = Bitmap {
            width: 2,
            height: 2,
            bgra: vec![
                255, 0, 0, 255, // blue
                0, 255, 0, 255, // green
                0, 0, 255, 255, // red
                255, 255, 255, 255, // white
            ],
        };
        let png = encode_png(&original).expect("encode_png failed");
        let decoded = decode(&png).expect("decode failed");
        assert_eq!(decoded.width, 2);
        assert_eq!(decoded.height, 2);
        assert_eq!(decoded.bgra.len(), 16);
        assert_eq!(decoded.bgra, original.bgra);
    }

    #[test]
    fn encode_png_rejects_zero_overflow_and_short_buffer() {
        let cases = [
            ("zero width", Bitmap { width: 0, height: 2, bgra: vec![0; 8] }, "empty bitmap"),
            ("zero height", Bitmap { width: 2, height: 0, bgra: vec![0; 8] }, "empty bitmap"),
            (
                "u32 max dimensions exceed row limit",
                Bitmap { width: u32::MAX, height: u32::MAX, bgra: Vec::new() },
                "bitmap dimensions overflow",
            ),
            (
                "u32 row width overflows",
                Bitmap { width: 1_073_741_824, height: 1, bgra: Vec::new() },
                "bitmap dimensions overflow",
            ),
            (
                "u32 raw capacity overflows",
                Bitmap { width: 1_073_741_823, height: 2, bgra: Vec::new() },
                "bitmap dimensions overflow",
            ),
            (
                "IDAT format length overflows u32",
                Bitmap { width: 1, height: 858_993_459, bgra: Vec::new() },
                "bitmap dimensions overflow",
            ),
            (
                "BGRA buffer is short",
                Bitmap { width: 2, height: 2, bgra: vec![0; 15] },
                "bitmap buffer too short",
            ),
        ];
        for (case, bitmap, expected_error) in cases {
            assert_eq!(encode_png(&bitmap).unwrap_err(), expected_error, "{case}");
        }
    }
}
