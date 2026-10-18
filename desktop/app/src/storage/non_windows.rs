// SPDX-License-Identifier: GPL-3.0-or-later
//! macOS / non-Windows storage stubs.

use nectarlink_core::{DeviceId, NodeEvent};
use std::path::{Path, PathBuf};

pub fn start() {}

pub fn sync() {}

pub fn on_event(_event: &NodeEvent) {}

pub fn shutdown() {}

pub fn unregister_all() {}

pub fn sync_root_path_for(_device: DeviceId) -> Option<PathBuf> {
    None
}

pub fn open_in_explorer(_device: DeviceId) -> Result<(), String> {
    Err("phone storage exploration is not supported on this platform".into())
}

pub fn sanitize_folder_name(device_name: &str) -> String {
    device_name.chars().filter(|c| c.is_alphanumeric() || matches!(c, ' ' | '-' | '_')).collect()
}

pub fn unix_ms_to_filetime_ticks(unix_ms: i64) -> i64 {
    unix_ms * 10_000 + 116_444_736_000_000_000
}

pub fn filetime_ticks_to_unix_ms(ticks: i64) -> i64 {
    (ticks - 116_444_736_000_000_000) / 10_000
}

pub fn relative_path_under_root(root_path: &Path, full_path: &Path) -> Option<String> {
    full_path.strip_prefix(root_path).ok().and_then(|p| p.to_str()).map(|s| s.replace('\\', "/"))
}
