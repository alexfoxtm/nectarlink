// SPDX-License-Identifier: GPL-3.0-or-later
//! Phone storage in File Explorer (Windows CloudFiles) and stubs for other platforms.

#[cfg(windows)]
mod windows;
#[cfg(windows)]
pub use self::windows::*;

#[cfg(not(windows))]
mod non_windows;
#[cfg(not(windows))]
pub use self::non_windows::*;
