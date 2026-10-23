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

#[derive(Debug)]
struct TransferFailure {
    message: String,
    retryable: bool,
}

impl From<String> for TransferFailure {
    fn from(message: String) -> Self {
        Self { message, retryable: false }
    }
}
impl From<&str> for TransferFailure {
    fn from(message: &str) -> Self {
        message.to_owned().into()
    }
}

fn curl_retryable(exit: Option<i32>, http_status: Option<u16>) -> bool {
    match exit {
        Some(22) => http_status.is_some_and(|status| status == 429 || (500..=599).contains(&status)),
        // Name resolution, connection, partial response, timeout, TLS handshake,
        // empty reply, send/receive, and HTTP/2 transport failures. Certificate
        // validation, local I/O, redirect/protocol refusal and unknown exits fail closed.
        Some(5 | 6 | 7 | 16 | 18 | 28 | 35 | 52 | 55 | 56 | 92) => true,
        _ => false,
    }
}

fn transfer(
    url: &str,
    limit: u64,
    output: &mut impl Write,
    progress: impl FnMut(u64, u64),
) -> Result<u64, String> {
    transfer_once(url, limit, output, progress).map_err(|error| error.message)
}

fn transfer_once(
    url: &str,
    limit: u64,
    output: &mut impl Write,
    mut progress: impl FnMut(u64, u64),
) -> Result<u64, TransferFailure> {
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
            // Curl writes only the final numeric HTTP status to stderr.
            "--write-out",
            "%{stderr}%{http_code}",
            "--max-time",
            "300",
            "--",
            url,
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| e.to_string())?;
    let result = (|| -> Result<u64, TransferFailure> {
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
        // Quiet curl plus our fixed write-out emits three status digits, not
        // response headers/body. Bound and validate this private control channel.
        let mut metadata = Vec::new();
        child
            .stderr
            .take()
            .ok_or("missing downloader status")?
            .take(16)
            .read_to_end(&mut metadata)
            .map_err(|e| e.to_string())?;
        let http_status = (metadata.len() == 3 && metadata.iter().all(u8::is_ascii_digit))
            .then(|| std::str::from_utf8(&metadata).ok()?.parse::<u16>().ok())
            .flatten();
        let status = child.wait().map_err(|e| e.to_string())?;
        if !status.success() {
            return Err(TransferFailure {
                message: format!("curl error: {status}"),
                retryable: curl_retryable(status.code(), http_status),
            });
        }
        Ok(received)
    })();
    if result.is_err() {
        let _ = child.kill();
        let _ = child.wait();
    }
    result
}

fn get_once(url: &str, limit: usize) -> Result<Vec<u8>, TransferFailure> {
    let mut bytes = Vec::new();
    transfer_once(url, limit as u64, &mut bytes, |_, _| {})?;
    Ok(bytes)
}

pub fn get(url: &str, limit: usize) -> Result<Vec<u8>, String> {
    get_once(url, limit).map_err(|error| error.message)
}

pub fn get_retrying(url: &str, limit: usize) -> Result<Vec<u8>, String> {
    validate_https(url)?;
    // Preserve the existing maximum of four attempts and three one-second waits.
    for attempt in 0..4 {
        match get_once(url, limit) {
            Ok(bytes) => return Ok(bytes),
            Err(error) if error.retryable && attempt < 3 => {
                std::thread::sleep(std::time::Duration::from_secs(1));
            }
            Err(error) => return Err(error.message),
        }
    }
    unreachable!("the final attempt always returns")
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
    #[test]
    fn retries_transient_http_and_transport_failures() {
        for code in [429, 500, 503, 599] {
            assert!(curl_retryable(Some(22), Some(code)));
        }
        for exit in [5, 6, 7, 16, 18, 28, 35, 52, 55, 56, 92] {
            assert!(curl_retryable(Some(exit), None));
        }
    }

    #[test]
    fn permanent_and_unknown_failures_do_not_retry() {
        for code in [400, 401, 403, 404, 408] {
            assert!(!curl_retryable(Some(22), Some(code)));
        }
        for exit in [1, 3, 23, 47, 60, 77] {
            assert!(!curl_retryable(Some(exit), None));
        }
        assert!(!curl_retryable(Some(22), None));
        assert!(!curl_retryable(None, Some(503)));
        assert!(!TransferFailure::from("too large").retryable);
        assert!(!TransferFailure::from("local write failed".to_owned()).retryable);
    }
}
