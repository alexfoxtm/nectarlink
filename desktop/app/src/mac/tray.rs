// SPDX-License-Identifier: GPL-3.0-or-later
//! macOS status bar / tray integration.
//!
//! There is no status bar item, no global hotkey and no window tracking on
//! macOS yet. Creating the tray therefore fails (the app logs "no tray icon"
//! and keeps no handle) instead of returning an invisible tray, and window
//! focusing reports failure so Continuity Camera never claims it pasted.

#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrayEvent {
    Open,
    CommandPalette,
    Shelf,
    TakePhoto,
    ScanDocument,
    FindPhone,
    OpenLinkOnPhone,
    Quit,
    Resumed,
    AppearanceChanged,
}

#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct MenuLabels {
    pub open: String,
    pub command_palette: String,
    pub shelf: String,
    pub take_photo: String,
    pub scan_document: String,
    pub find_phone: String,
    pub open_link: String,
    pub quit: String,
}

pub struct Tray;

impl Tray {
    pub fn create(
        _tooltip: &str,
        _labels: MenuLabels,
        _handler: impl Fn(TrayEvent) + 'static,
    ) -> Result<Tray, String> {
        Err("status bar item isn't implemented on macOS yet".into())
    }

    pub fn set_tooltip(&self, _text: &str) {}

    pub fn set_find_phone_enabled(&self, _enabled: bool) {}
}

pub fn update_hotkeys(_photo_spec: &str, _scan_spec: &str, _palette_spec: &str, _shelf_spec: &str) {}

#[allow(dead_code)]
pub fn record_foreground_window() {}

pub fn apply_no_activate_style_by_title(_title: &str) {}

/// No window is ever focused from here; `false` keeps the caller on its
/// "image is on your clipboard" path instead of a false "pasted" message.
pub fn focus_window(_hwnd: isize) -> bool {
    false
}

pub fn is_window_alive_and_visible(_hwnd: isize) -> bool {
    false
}

pub fn snapshot_target_window() -> Option<(isize, String)> {
    None
}

pub fn send_paste_fallback_if_not_foreground(_hwnd: isize) {}

#[allow(dead_code)]
pub fn parse_hotkey(_spec: &str) -> Option<(u32, u32)> {
    None
}
