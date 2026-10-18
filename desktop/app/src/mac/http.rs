// SPDX-License-Identifier: GPL-3.0-or-later
//! HTTPS GET and download using system curl on macOS.

use std::path::Path;

pub fn get(url: &str, limit: usize) -> Result<Vec<u8>, String> {
    let output = std::process::Command::new("curl")
        .args(["-s", "-L", "-f", "--max-time", "30", url])
        .output()
        .map_err(|e| e.to_string())?;
    if !output.status.success() {
        return Err(format!("curl error: {}", String::from_utf8_lossy(&output.stderr)));
    }
    if output.stdout.len() > limit {
        return Err("too large".into());
    }
    Ok(output.stdout)
}

pub fn get_retrying(url: &str, limit: usize) -> Result<Vec<u8>, String> {
    for _ in 0..3 {
        if let Ok(data) = get(url, limit) {
            return Ok(data);
        }
        std::thread::sleep(std::time::Duration::from_secs(1));
    }
    get(url, limit)
}

pub fn download(
    url: &str,
    file: &Path,
    _limit: u64,
    mut progress: impl FnMut(u64, u64),
) -> Result<(), String> {
    let status = std::process::Command::new("curl")
        .args(["-s", "-L", "-f", "-C", "-", "-o", &file.to_string_lossy(), url])
        .status()
        .map_err(|e| e.to_string())?;
    if !status.success() {
        return Err("download failed".into());
    }
    let len = file.metadata().map(|m| m.len()).unwrap_or(0);
    progress(len, len);
    Ok(())
}
