// SPDX-License-Identifier: GPL-3.0-or-later
//! Reverse Mirroring: streaming PC display to phone.

#[cfg(windows)]
mod windows;
#[cfg(windows)]
pub use self::windows::*;

#[cfg(not(windows))]
mod non_windows;
#[cfg(not(windows))]
pub use self::non_windows::*;
