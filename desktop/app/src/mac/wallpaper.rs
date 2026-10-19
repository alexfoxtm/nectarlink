// SPDX-License-Identifier: GPL-3.0-or-later
//! macOS wallpaper color accent seed and artwork palette extraction.

use material_colors::color::Rgb;
use std::path::Path;

/// No wallpaper sampling on macOS yet: `None` lets the theme use its
/// fallback seed instead of a made-up color.
pub fn seed() -> Option<Rgb> {
    None
}

pub fn artwork_scheme_from_path(_path: &Path) -> Option<String> {
    None
}
