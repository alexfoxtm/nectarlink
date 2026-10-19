// SPDX-License-Identifier: GPL-3.0-or-later
//! The Waypair Link mark (`assets/brand/waypair/waypair-icon-v2.png`): one
//! pearl-white path, two rounded U shapes that cross once at the bottom and
//! loop in the middle, a W for two paired devices, on a royal-blue tile.
//! Drawn in code at any pixel size: the tray, window and toast icons at
//! runtime, and the executable's icon at build time (`build.rs` includes this
//! file), so every size is sharp without bitmap assets. At 32 px and below
//! the stroke is heavier and the tile fills more of the square.

/// The tile's blue from top to bottom, its rim, and the glyph (Waypair v2:
/// `#164FA8` → `#063B86`).
const TILE_TOP: [f32; 3] = rgb(0x16, 0x4F, 0xA8);
const TILE_BOTTOM: [f32; 3] = rgb(0x06, 0x3B, 0x86);
const RIM: [f32; 3] = rgb(0x7F, 0xA8, 0xE8);
const GLYPH: [f32; 3] = rgb(0xF5, 0xF8, 0xFD);

/// The glyph's centerline in a 100×100 design grid, as cubic Béziers (start,
/// control, control, end): left stem, bottom-left curve, up into the loop,
/// over its top, down through the crossing, bottom-right curve, right stem.
const PATH: [[(f32, f32); 4]; 8] = [
    [(29.13, 33.88), (29.13, 40.39), (29.13, 46.82), (29.13, 53.33)],
    [(29.13, 53.33), (29.13, 60.56), (33.84, 66.12), (39.91, 66.12)],
    [(39.91, 66.12), (45.18, 66.12), (54.61, 56.91), (54.61, 49.45)],
    [(54.61, 49.45), (54.61, 46.89), (52.56, 44.85), (50.0, 44.85)],
    [(50.0, 44.85), (47.44, 44.85), (45.39, 46.89), (45.39, 49.45)],
    [(45.39, 49.45), (45.39, 56.91), (54.82, 66.12), (60.09, 66.12)],
    [(60.09, 66.12), (66.16, 66.12), (70.87, 60.56), (70.87, 53.33)],
    [(70.87, 53.33), (70.87, 46.82), (70.87, 40.39), (70.87, 33.88)],
];
/// The strand that passes in front at the crossing, where the two U shapes
/// meet below the loop, and that crossing's center.
const OVER: usize = 5;
const CROSSING: (f32, f32) = (50.0, 59.7);

/// Supersampling grid per pixel (SS×SS samples) for smooth edges.
const SS: usize = 4;

const fn rgb(r: u8, g: u8, b: u8) -> [f32; 3] {
    [r as f32, g as f32, b as f32]
}

/// Sizes in the design grid: the small sizes are drawn bolder.
struct Geometry {
    inset: f32,
    corner: f32,
    stroke: f32,
    /// The tile-colored gap around the front strand at the crossing (none
    /// where it would be under a pixel).
    gap: f32,
}

impl Geometry {
    fn for_size(size: usize) -> Self {
        if size <= 32 {
            Geometry { inset: 2.0, corner: 20.0, stroke: 11.6, gap: 0.0 }
        } else {
            Geometry { inset: 4.0, corner: 22.0, stroke: 10.3, gap: 1.6 }
        }
    }
}

/// Renders the mark as straight-alpha RGBA, row by row, `size`×`size`.
pub fn render(size: usize) -> Vec<u8> {
    let g = Geometry::for_size(size);
    let unit = 100.0 / size as f32;
    let glyph = polyline(&PATH);
    let over = polyline(&PATH[OVER..=OVER]);
    let half = g.stroke / 2.0;
    // The rim is at least about a pixel wide, so the tile keeps an edge on dark taskbars.
    let rim = (1.5f32).max(unit * 0.6);

    let mut out = vec![0u8; size * size * 4];
    for py in 0..size {
        for px in 0..size {
            let mut color = [0f32; 3];
            let mut covered = 0usize;
            for sy in 0..SS {
                for sx in 0..SS {
                    // The sample in the design grid.
                    let x = (px as f32 + (sx as f32 + 0.5) / SS as f32) * unit;
                    let y = (py as f32 + (sy as f32 + 0.5) / SS as f32) * unit;
                    let edge = rounded_square_distance(x, y, g.inset, g.corner);
                    if edge > 0.0 {
                        continue;
                    }
                    covered += 1;
                    let tile = mix(TILE_TOP, TILE_BOTTOM, ((y - g.inset) / (100.0 - 2.0 * g.inset)).clamp(0.0, 1.0));
                    let near_crossing = (x - CROSSING.0).hypot(y - CROSSING.1) <= g.stroke;
                    let in_gap = near_crossing && {
                        let front = distance_to(&over, (x, y));
                        front > half && front <= half + g.gap
                    };
                    let paint = if in_gap {
                        tile
                    } else if distance_to(&glyph, (x, y)) <= half {
                        GLYPH
                    } else if -edge <= rim {
                        mix(tile, RIM, 0.34)
                    } else {
                        tile
                    };
                    for c in 0..3 {
                        color[c] += paint[c];
                    }
                }
            }
            if covered == 0 {
                continue;
            }
            let i = (py * size + px) * 4;
            for c in 0..3 {
                out[i + c] = (color[c] / covered as f32).round() as u8;
            }
            out[i + 3] = ((covered as f32 / (SS * SS) as f32) * 255.0).round() as u8;
        }
    }
    out
}

fn mix(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t, a[2] + (b[2] - a[2]) * t]
}

/// How far (x, y) is outside the rounded square from `inset` to
/// `100 - inset` with corner radius `r` (negative inside).
fn rounded_square_distance(x: f32, y: f32, inset: f32, r: f32) -> f32 {
    let (lo, hi) = (inset + r, 100.0 - inset - r);
    let qx = (x.clamp(lo, hi) - x).abs();
    let qy = (y.clamp(lo, hi) - y).abs();
    if x > lo && x < hi || y > lo && y < hi {
        // Along a straight side.
        let to_x = (x - inset).min(100.0 - inset - x);
        let to_y = (y - inset).min(100.0 - inset - y);
        return -to_x.min(to_y);
    }
    (qx * qx + qy * qy).sqrt() - r
}

/// The path's Béziers as points close enough together to measure against.
fn polyline(segments: &[[(f32, f32); 4]]) -> Vec<(f32, f32)> {
    const STEPS: usize = 32;
    let mut points = Vec::with_capacity(STEPS * segments.len() + 1);
    for segment in segments {
        for step in 0..=STEPS {
            if step == 0 && !points.is_empty() {
                continue;
            }
            let t = step as f32 / STEPS as f32;
            let u = 1.0 - t;
            let (a, b, c, d) = (segment[0], segment[1], segment[2], segment[3]);
            let w = [u * u * u, 3.0 * u * u * t, 3.0 * u * t * t, t * t * t];
            points.push((
                w[0] * a.0 + w[1] * b.0 + w[2] * c.0 + w[3] * d.0,
                w[0] * a.1 + w[1] * b.1 + w[2] * c.1 + w[3] * d.1,
            ));
        }
    }
    points
}

fn distance_to(polyline: &[(f32, f32)], p: (f32, f32)) -> f32 {
    polyline.windows(2).map(|w| dist_to_segment(p, w[0], w[1])).fold(f32::INFINITY, f32::min)
}

fn dist_to_segment(p: (f32, f32), a: (f32, f32), b: (f32, f32)) -> f32 {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let len2 = dx * dx + dy * dy;
    let t = if len2 == 0.0 { 0.0 } else { (((p.0 - a.0) * dx + (p.1 - a.1) * dy) / len2).clamp(0.0, 1.0) };
    let (qx, qy) = (a.0 + t * dx, a.1 + t * dy);
    ((p.0 - qx).powi(2) + (p.1 - qy).powi(2)).sqrt()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pixel(img: &[u8], size: usize, x: usize, y: usize) -> [u8; 4] {
        let i = (y * size + x) * 4;
        [img[i], img[i + 1], img[i + 2], img[i + 3]]
    }

    fn glyph(p: [u8; 4]) -> bool {
        p[0] > 0xE0 && p[1] > 0xE0 && p[2] > 0xE0 && p[3] == 255
    }

    #[test]
    fn corners_are_transparent_and_the_loop_sits_in_the_middle() {
        let size = 64;
        let img = render(size);
        assert_eq!(img.len(), size * size * 4);
        assert_eq!(pixel(&img, size, 0, 0)[3], 0, "rounded corner");
        assert!(glyph(pixel(&img, size, 32, 29)), "the top of the loop");
        // Near the top edge, between the stems: the blue tile.
        let tile = pixel(&img, size, 32, 8);
        assert!(tile[2] > tile[0] && tile[2] > 0x80 && tile[3] == 255, "{tile:?}");
    }

    #[test]
    fn both_stems_are_drawn() {
        let size = 128;
        let img = render(size);
        assert!(glyph(pixel(&img, size, 37, 56)), "left stem");
        assert!(glyph(pixel(&img, size, 90, 56)), "right stem");
        // Between the left stem and the loop there's tile.
        assert!(!glyph(pixel(&img, size, 48, 46)), "gap left of the loop");
    }

    #[test]
    fn renders_every_tray_size() {
        for size in [16, 20, 24, 32, 48, 64, 256] {
            let img = render(size);
            assert!(img.chunks(4).any(|p| p[3] == 255), "{size}px has an opaque tile");
        }
    }
}
