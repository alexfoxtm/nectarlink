// SPDX-License-Identifier: GPL-3.0-or-later
//! Streaming PC loopback audio to the phone.

#[cfg(windows)]
mod windows;
#[cfg(windows)]
pub use self::windows::*;

#[cfg(not(windows))]
mod non_windows;
#[cfg(not(windows))]
pub use self::non_windows::*;
