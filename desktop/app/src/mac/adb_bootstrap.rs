// SPDX-License-Identifier: GPL-3.0-or-later
//! One-shot macOS start of a seal-checked bundled `adb` server.
//!
//! The USB watcher in `nectarlink-core` only dials `127.0.0.1:5037`. This
//! module does not download platform-tools, does not search `PATH` or an SDK,
//! and does not run client commands. `adb start-server` and `adb devices` are
//! clients: after a version mismatch they can kill and replace whatever is
//! already bound to that port (`launch_server` / `adb_connect` in AOSP
//! `client/commandline.cpp`). The foreground server argv below is the process
//! that only listens.
//!
//! AOSP `adb_commandline` (platform/packages/modules/adb `client/commandline.cpp`,
//! main) accepts `-L`, `nodaemon`, and `server` as modifiers in one loop.
//! `server` plus `nodaemon` calls `adb_server_main(is_daemon = false, ...)`.
//! `server` without `nodaemon` calls `launch_server`, which is the client path
//! and is not used here. `-L` sets the listen socket and ignores
//! `ADB_SERVER_SOCKET`. `adb_server_main` (`client/main.cpp`) only
//! `install_listener`s; on bind failure it retries for about half a second and
//! then `LOG(FATAL)`s. It does not handshake with, or kill, an existing server.
//!
//! The seal file is `Contents/Resources/PhoneSetup/adb-seal.json`, at most 4 KiB:
//! `{"schema":1,"relativePath":"Helpers/adb","sha256":"<64 lowercase hex>","size":N}`.
//! `sha256` and `size` are the signed `Contents/Helpers/adb` bytes. Packaging
//! writes that file; this module only reads it.

use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File},
    io::Read,
    net::{Ipv4Addr, SocketAddr, TcpStream},
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::Once,
    time::Duration,
};

const SEAL_MAX_BYTES: u64 = 4096;
/// Same positive-size ceiling the packaging checker uses before it copies a helper.
const HELPER_MAX_BYTES: u64 = 64 * 1024 * 1024;
const PROBE_TIMEOUT: Duration = Duration::from_millis(200);
const RELATIVE_HELPER: &str = "Helpers/adb";
const HELPER_PARTS: [&str; 2] = ["Helpers", "adb"];
const SEAL_PARTS: [&str; 3] = ["Resources", "PhoneSetup", "adb-seal.json"];

/// Foreground server only. Not `start-server`, not `fork-server`, not `devices`.
pub(crate) const FOREGROUND_SERVER_ARGS: [&str; 4] = ["-L", "tcp:127.0.0.1:5037", "nodaemon", "server"];

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Decision {
    /// `NECTARLINK_NO_ADB=1`: do not probe and do not spawn.
    Disabled,
    /// Something is already listening. Do not exec.
    UseExistingServer,
    /// Port is closed and the bundled helper matched its seal.
    Spawn { exe: PathBuf },
    /// No bundle, or the helper/seal was refused. The TCP watcher may still retry.
    Skip,
}

/// Starts the bootstrap thread once. Returns immediately. A no-op when
/// `NECTARLINK_NO_ADB=1`. Must be called off the Qt thread; the probe and any
/// child live on `adb-bootstrap`, not the GUI thread.
pub(crate) fn start_once() {
    if adb_disabled() {
        return;
    }
    static STARTED: Once = Once::new();
    STARTED.call_once(|| {
        if let Err(e) = std::thread::Builder::new().name("adb-bootstrap".into()).spawn(bootstrap_thread) {
            tracing::warn!(error = %e, "adb bootstrap thread did not start");
        }
    });
}

fn bootstrap_thread() {
    if adb_disabled() {
        return;
    }
    let probe_open = probe_local_server();
    let Ok(current) = std::env::current_exe() else {
        tracing::info!("adb bootstrap skipped; executable path is unknown");
        return;
    };
    match decide(&current, probe_open, adb_disabled()) {
        Decision::Disabled => {}
        Decision::UseExistingServer => {
            tracing::info!("adb server already listening on 127.0.0.1:5037");
        }
        Decision::Skip => {
            tracing::info!("no sealed bundled adb; USB will keep retrying an existing server");
        }
        Decision::Spawn { exe } => {
            if sealed_helper(&current).as_ref() != Some(&exe) {
                tracing::info!("sealed adb helper changed before start; not executing");
                return;
            }
            match spawn_server(&exe) {
                Ok(mut child) => {
                    tracing::info!("sealed adb server starting");
                    match child.wait() {
                        Ok(status) => tracing::info!(%status, "sealed adb server exited"),
                        Err(e) => tracing::warn!(error = %e, "sealed adb server reap failed"),
                    }
                }
                Err(e) => tracing::warn!(error = %e, "sealed adb server did not start"),
            }
        }
    }
}

/// Pure decision. `probe_open` is injected by tests; production passes one TCP probe.
pub(crate) fn decide(current_exe: &Path, probe_open: bool, adb_disabled: bool) -> Decision {
    if adb_disabled {
        return Decision::Disabled;
    }
    if probe_open {
        return Decision::UseExistingServer;
    }
    match sealed_helper(current_exe) {
        Some(exe) => Decision::Spawn { exe },
        None => Decision::Skip,
    }
}

fn adb_disabled() -> bool {
    std::env::var_os("NECTARLINK_NO_ADB").is_some_and(|v| v == "1")
}

/// One TCP connect to `127.0.0.1:5037`. No ADB bytes are written.
fn probe_local_server() -> bool {
    let addr = SocketAddr::from((Ipv4Addr::LOCALHOST, 5037));
    TcpStream::connect_timeout(&addr, PROBE_TIMEOUT).is_ok()
}

fn spawn_server(exe: &Path) -> std::io::Result<std::process::Child> {
    let meta = fs::symlink_metadata(exe)?;
    if meta.file_type().is_symlink() || !meta.is_file() || meta.permissions().mode() & 0o111 == 0 {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "adb helper is not an executable regular file",
        ));
    }
    Command::new(exe)
        .args(FOREGROUND_SERVER_ARGS)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
}

fn sealed_helper(current_exe: &Path) -> Option<PathBuf> {
    let contents = contents_dir(current_exe)?;
    let (helper, helper_meta) = regular_within(&contents, &HELPER_PARTS)?;
    if helper_meta.permissions().mode() & 0o111 == 0
        || helper_meta.len() == 0
        || helper_meta.len() > HELPER_MAX_BYTES
    {
        return None;
    }
    let (seal_path, seal_meta) = regular_within(&contents, &SEAL_PARTS)?;
    if seal_meta.len() == 0 || seal_meta.len() > SEAL_MAX_BYTES {
        return None;
    }
    let seal = read_seal(&seal_path)?;
    if seal.size > HELPER_MAX_BYTES || seal.size != helper_meta.len() {
        return None;
    }
    if hash_matches(&helper, &seal.sha256, seal.size)? {
        Some(helper)
    } else {
        None
    }
}

/// `Foo.app/Contents/MacOS/<bin>` only. A dev build under `target/debug` is not a bundle.
fn contents_dir(current_exe: &Path) -> Option<PathBuf> {
    let macos = current_exe.parent()?;
    if macos.file_name()? != "MacOS" {
        return None;
    }
    let contents = macos.parent()?;
    if contents.file_name()? != "Contents" {
        return None;
    }
    Some(contents.to_path_buf())
}

/// Walks `rel` under `contents`, refusing symlinks and `..`. The canonical file
/// must be exactly `canonicalize(contents)` joined with `rel`.
fn regular_within(contents: &Path, rel: &[&str]) -> Option<(PathBuf, fs::Metadata)> {
    let root = fs::symlink_metadata(contents).ok()?;
    if root.file_type().is_symlink() || !root.is_dir() {
        return None;
    }
    let mut cur = contents.to_path_buf();
    let mut meta = root;
    for (i, part) in rel.iter().enumerate() {
        if part.is_empty() || *part == "." || *part == ".." {
            return None;
        }
        cur.push(part);
        meta = fs::symlink_metadata(&cur).ok()?;
        if meta.file_type().is_symlink() {
            return None;
        }
        let last = i + 1 == rel.len();
        if last {
            if !meta.is_file() {
                return None;
            }
        } else if !meta.is_dir() {
            return None;
        }
    }
    let canon_root = contents.canonicalize().ok()?;
    let canon_file = cur.canonicalize().ok()?;
    let mut expected = canon_root;
    for part in rel {
        expected.push(part);
    }
    if canon_file != expected {
        return None;
    }
    Some((cur, meta))
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Seal {
    schema: u64,
    #[serde(rename = "relativePath")]
    relative_path: String,
    sha256: String,
    size: u64,
}

fn read_seal(path: &Path) -> Option<Seal> {
    let meta = fs::symlink_metadata(path).ok()?;
    if meta.file_type().is_symlink() || !meta.is_file() {
        return None;
    }
    if meta.len() == 0 || meta.len() > SEAL_MAX_BYTES {
        return None;
    }
    let file = File::open(path).ok()?;
    let mut buf = Vec::new();
    file.take(SEAL_MAX_BYTES + 1).read_to_end(&mut buf).ok()?;
    if buf.len() as u64 != meta.len() || buf.len() > SEAL_MAX_BYTES as usize {
        return None;
    }
    let seal: Seal = serde_json::from_slice(&buf).ok()?;
    if seal.schema != 1
        || seal.relative_path != RELATIVE_HELPER
        || seal.size == 0
        || seal.size > HELPER_MAX_BYTES
    {
        return None;
    }
    decode_sha256_lowercase(&seal.sha256)?;
    Some(seal)
}

fn hash_matches(path: &Path, expected_hex: &str, expected_size: u64) -> Option<bool> {
    let expected = decode_sha256_lowercase(expected_hex)?;
    let meta = fs::symlink_metadata(path).ok()?;
    if meta.file_type().is_symlink() || !meta.is_file() || meta.len() != expected_size {
        return None;
    }
    let mut file = File::open(path).ok()?;
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 64 * 1024];
    let mut hashed: u64 = 0;
    loop {
        let n = file.read(&mut buf).ok()?;
        if n == 0 {
            break;
        }
        hashed = hashed.saturating_add(n as u64);
        if hashed > expected_size {
            return None;
        }
        hasher.update(&buf[..n]);
    }
    if hashed != expected_size {
        return None;
    }
    let digest = hasher.finalize();
    let mut got = [0u8; 32];
    got.copy_from_slice(&digest);
    Some(ct_eq(&got, &expected))
}

fn decode_sha256_lowercase(text: &str) -> Option<[u8; 32]> {
    if text.len() != 64 {
        return None;
    }
    let bytes = text.as_bytes();
    let mut out = [0u8; 32];
    for i in 0..32 {
        let hi = hex_val(bytes[i * 2])?;
        let lo = hex_val(bytes[i * 2 + 1])?;
        out[i] = (hi << 4) | lo;
    }
    Some(out)
}

fn hex_val(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        _ => None,
    }
}

fn ct_eq(a: &[u8; 32], b: &[u8; 32]) -> bool {
    let mut diff = 0u8;
    for i in 0..32 {
        diff |= a[i] ^ b[i];
    }
    diff == 0
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::symlink;

    struct Scratch(PathBuf);
    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn scratch(name: &str) -> Scratch {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let home = std::env::var_os("HOME").filter(|h| !h.is_empty()).expect("HOME");
        let dir = PathBuf::from(home).join("orca/tmp/waypair-adb-bootstrap-tests").join(format!(
            "{name}-{}-{nanos}",
            std::process::id()
        ));
        fs::create_dir_all(dir.join("Contents/MacOS")).unwrap();
        fs::create_dir_all(dir.join("Contents/Helpers")).unwrap();
        fs::create_dir_all(dir.join("Contents/Resources/PhoneSetup")).unwrap();
        Scratch(dir)
    }

    fn current_exe(app: &Path) -> PathBuf {
        app.join("Contents/MacOS/WaypairLink")
    }

    fn write_seal(app: &Path, bytes: &[u8]) {
        let mut hasher = Sha256::new();
        hasher.update(bytes);
        let hex: String = hasher.finalize().iter().map(|b| format!("{b:02x}")).collect();
        let json = format!(
            "{{\"schema\":1,\"relativePath\":\"Helpers/adb\",\"sha256\":\"{hex}\",\"size\":{}}}",
            bytes.len()
        );
        assert!(json.len() <= SEAL_MAX_BYTES as usize);
        fs::write(app.join("Contents/Resources/PhoneSetup/adb-seal.json"), json).unwrap();
    }

    fn write_exec(path: &Path, bytes: &[u8]) {
        fs::write(path, bytes).unwrap();
        let mut perms = fs::metadata(path).unwrap().permissions();
        perms.set_mode(0o755);
        fs::set_permissions(path, perms).unwrap();
    }

    #[test]
    fn sealed_helper_is_selected_only_when_the_port_is_closed() {
        let dir = scratch("main");
        let app = &dir.0;
        let bytes = b"fixture-helper-not-adb";
        let helper = app.join("Contents/Helpers/adb");
        write_exec(&helper, bytes);
        write_seal(app, bytes);
        let exe = current_exe(app);

        match decide(&exe, false, false) {
            Decision::Spawn { exe: got } => {
                assert_eq!(got, helper);
                assert_eq!(FOREGROUND_SERVER_ARGS, ["-L", "tcp:127.0.0.1:5037", "nodaemon", "server"]);
                let meta = fs::symlink_metadata(&got).unwrap();
                assert!(meta.is_file());
                assert!(!meta.file_type().is_symlink());
            }
            other => panic!("expected spawn, got {other:?}"),
        }
        assert_eq!(decide(&exe, true, false), Decision::UseExistingServer);
        assert_eq!(decide(&exe, false, true), Decision::Disabled);
        assert_eq!(decide(&exe, true, true), Decision::Disabled);
    }

    #[test]
    fn symlink_helper_is_refused_even_when_its_target_matches_the_seal() {
        let dir = scratch("fail");
        let app = &dir.0;
        let bytes = b"fixture-helper-not-adb";
        let real = app.join("Contents/Helpers/adb.real");
        write_exec(&real, bytes);
        write_seal(app, bytes);
        symlink(&real, app.join("Contents/Helpers/adb")).unwrap();
        let exe = current_exe(app);
        assert_eq!(decide(&exe, false, false), Decision::Skip);

        fs::remove_file(app.join("Contents/Helpers/adb")).unwrap();
        write_exec(&app.join("Contents/Helpers/adb"), bytes);
        let seal_path = app.join("Contents/Resources/PhoneSetup/adb-seal.json");
        let mut seal = fs::read_to_string(&seal_path).unwrap();
        let mark = "\"sha256\":\"";
        let at = seal.find(mark).unwrap() + mark.len() + 63;
        let flipped = if seal.as_bytes()[at] == b'0' { '1' } else { '0' };
        seal.replace_range(at..at + 1, &flipped.to_string());
        fs::write(&seal_path, seal).unwrap();
        assert_eq!(decide(&exe, false, false), Decision::Skip);
    }
}
