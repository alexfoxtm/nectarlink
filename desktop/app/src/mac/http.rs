// SPDX-License-Identifier: GPL-3.0-or-later
//! Bounded HTTPS GET and atomic downloads using system curl on macOS.

use std::{
    io::{Read, Write},
    path::Path,
    process::{Command, Stdio},
    sync::atomic::{AtomicU64, Ordering},
};

fn validate_https(url: &str) -> Result<(), String> {
    if !url.get(..8).is_some_and(|prefix| prefix.eq_ignore_ascii_case("https://")) {
        return Err("only https:// URLs are fetched".into());
    }
    Ok(())
}

fn transfer(
    url: &str,
    limit: u64,
    output: &mut impl Write,
    mut progress: impl FnMut(u64, u64),
) -> Result<u64, String> {
    validate_https(url)?;
    let mut child = Command::new("/usr/bin/curl")
        .args([
            "-q",
            "-s",
            "-L",
            "-f",
            "--proto",
            "=https",
            "--proto-redir",
            "=https",
            "--max-time",
            "300",
            "--",
            url,
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| e.to_string())?;
    let result = (|| {
        let mut stdout = child.stdout.take().ok_or("missing downloader stdout")?;
        let mut received = 0u64;
        let mut buffer = [0u8; 64 * 1024];
        loop {
            let count = stdout.read(&mut buffer).map_err(|e| e.to_string())?;
            if count == 0 {
                break;
            }
            received = received.checked_add(count as u64).ok_or("too large")?;
            if received > limit {
                return Err("too large".into());
            }
            output.write_all(&buffer[..count]).map_err(|e| e.to_string())?;
            progress(received, 0);
        }
        output.flush().map_err(|e| e.to_string())?;
        let status = child.wait().map_err(|e| e.to_string())?;
        if !status.success() {
            return Err(format!("curl error: {status}"));
        }
        Ok(received)
    })();
    if result.is_err() {
        let _ = child.kill();
        let _ = child.wait();
    }
    result
}

pub fn get(url: &str, limit: usize) -> Result<Vec<u8>, String> {
    let mut bytes = Vec::new();
    transfer(url, limit as u64, &mut bytes, |_, _| {})?;
    Ok(bytes)
}

pub fn get_retrying(url: &str, limit: usize) -> Result<Vec<u8>, String> {
    validate_https(url)?;
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
    limit: u64,
    mut progress: impl FnMut(u64, u64),
) -> Result<(), String> {
    use std::os::unix::fs::OpenOptionsExt;

    validate_https(url)?;
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let name = file.file_name().ok_or("missing download file name")?;
    let mut temporary_name = name.to_os_string();
    temporary_name.push(format!(".download-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
    let temporary = file.with_file_name(temporary_name);
    // An exclusive private sibling avoids following an existing file or symlink.
    // The destination is replaced only after a complete successful transfer.
    let mut output = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&temporary)
        .map_err(|e| e.to_string())?;
    let result = transfer(url, limit, &mut output, &mut progress).and_then(|received| {
        output.sync_all().map_err(|e| e.to_string())?;
        std::fs::rename(&temporary, file).map_err(|e| e.to_string())?;
        progress(received, received);
        Ok(())
    });
    if result.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_https_urls() {
        assert!(validate_https("https://example.com/asset").is_ok());
        assert!(validate_https("HTTPS://example.com/asset").is_ok());
    }

    #[test]
    fn rejects_other_protocols_before_opening_a_download() {
        let dir = tempfile::tempdir().unwrap();
        let destination = dir.path().join("asset");
        std::fs::write(&destination, b"keep").unwrap();
        for url in ["http://example.com/asset", "file:///tmp/asset", "--output=asset"] {
            assert!(get(url, 1024).is_err());
            assert!(get_retrying(url, 1024).is_err());
            assert!(download(url, &destination, 1024, |_, _| {}).is_err());
        }
        assert_eq!(std::fs::read(&destination).unwrap(), b"keep");
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
    }
}
