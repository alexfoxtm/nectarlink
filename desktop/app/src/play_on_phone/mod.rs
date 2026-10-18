// SPDX-License-Identifier: GPL-3.0-or-later
//! Play media on phone and window tracking.

#[cfg(windows)]
mod windows;
#[cfg(windows)]
pub use self::windows::*;

#[cfg(not(windows))]
mod non_windows;
#[cfg(not(windows))]
pub use self::non_windows::*;
