// SPDX-License-Identifier: GPL-3.0-or-later
//! macOS window capture stubs.

#[derive(Debug)]
pub struct WindowCapture;

impl WindowCapture {
    pub fn new(_window: &super::browser::Window) -> Option<Self> {
        None
    }
    pub fn capture_frame(&mut self) -> Option<Vec<u8>> {
        None
    }
}
