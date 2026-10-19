// SPDX-License-Identifier: GPL-3.0-or-later
//! Default received-files folder and (on macOS) one-time migration from the
//! legacy `Downloads/Nectarlink` name to `Downloads/Waypair Link`.

use std::path::{Path, PathBuf};

#[cfg(target_os = "macos")]
use serde::{Deserialize, Serialize};
#[cfg(target_os = "macos")]
use std::{
    fs,
    io::{self, Write},
};

const LEGACY_SUBDIR: &str = "Nectarlink";
#[cfg(target_os = "macos")]
const BRANDED_SUBDIR: &str = "Waypair Link";
#[cfg(not(target_os = "macos"))]
const BRANDED_SUBDIR: &str = LEGACY_SUBDIR;

#[cfg(target_os = "macos")]
const MIGRATION_DIR: &str = "migrations";
#[cfg(target_os = "macos")]
const DONE_MARKER: &str = "downloads-waypair-link-v1.done";
#[cfg(target_os = "macos")]
const PENDING_MARKER: &str = "downloads-waypair-link-v1.pending.json";

/// User-visible location label (not an absolute path).
pub fn user_visible_location() -> &'static str {
    #[cfg(target_os = "macos")]
    {
        "Downloads/Waypair Link"
    }
    #[cfg(not(target_os = "macos"))]
    {
        r"Downloads\Nectarlink"
    }
}

/// Legacy detail strings stored in timeline rows for the default folder.
#[cfg(target_os = "macos")]
pub fn legacy_timeline_detail_labels() -> &'static [&'static str] {
    #[cfg(target_os = "macos")]
    {
        &[r"Downloads\Nectarlink", "Downloads/Nectarlink"]
    }
    #[cfg(not(target_os = "macos"))]
    {
        &[r"Downloads\Nectarlink"]
    }
}

#[cfg(target_os = "macos")]
pub fn branded_timeline_detail_label() -> &'static str {
    user_visible_location()
}

/// Absolute folder where incoming transfers are saved by default.
pub fn received_dir() -> PathBuf {
    received_dir_with_fallback(|| crate::core_host::host().data_dir.join("received"))
}

pub fn received_dir_with_fallback(fallback_parent: impl FnOnce() -> PathBuf) -> PathBuf {
    let Some(root) = dirs::download_dir() else {
        return fallback_parent().join(BRANDED_SUBDIR);
    };
    #[cfg(target_os = "macos")]
    {
        save_dir(crate::core_host::try_data_dir(), &root)
    }
    #[cfg(not(target_os = "macos"))]
    {
        root.join(BRANDED_SUBDIR)
    }
}

#[cfg(target_os = "macos")]
#[derive(Debug, Serialize, Deserialize)]
struct PendingMigration {
    old_prefix: PathBuf,
    new_prefix: PathBuf,
    source_dev: u64,
    source_ino: u64,
}

#[cfg(target_os = "macos")]
#[derive(Debug)]
enum PathState {
    Absent,
    Symlink,
    Dir { dev: u64, ino: u64 },
    Other,
}

#[cfg(target_os = "macos")]
enum DoneState {
    Done,
    Absent,
    Retry(String),
}

#[cfg(target_os = "macos")]
enum PendingFile {
    Absent,
    Ready(PendingMigration),
    Unreadable(String),
    Malformed,
}

#[cfg(target_os = "macos")]
fn migration_state_dir(data_dir: &Path) -> PathBuf {
    data_dir.join(MIGRATION_DIR)
}

#[cfg(target_os = "macos")]
fn done_marker_path(data_dir: &Path) -> PathBuf {
    migration_state_dir(data_dir).join(DONE_MARKER)
}

#[cfg(target_os = "macos")]
fn pending_marker_path(data_dir: &Path) -> PathBuf {
    migration_state_dir(data_dir).join(PENDING_MARKER)
}

/// Classifies `path` without following symlinks. Errors other than absence
/// (including a Downloads privacy denial) stay errors so the caller can retry.
#[cfg(target_os = "macos")]
fn probe(path: &Path) -> io::Result<PathState> {
    use std::os::unix::fs::MetadataExt;
    match fs::symlink_metadata(path) {
        Ok(meta) => {
            let kind = meta.file_type();
            if kind.is_symlink() {
                Ok(PathState::Symlink)
            } else if kind.is_dir() {
                Ok(PathState::Dir { dev: meta.dev(), ino: meta.ino() })
            } else {
                Ok(PathState::Other)
            }
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(PathState::Absent),
        Err(e) => Err(e),
    }
}

#[cfg(target_os = "macos")]
fn fsync_dir(path: &Path) -> io::Result<()> {
    fs::File::open(path)?.sync_all()
}

/// Writes `bytes` by syncing a temp file and renaming it into place, then syncing
/// the parent directory. A crash leaves either the previous file or the complete new one.
#[cfg(target_os = "macos")]
fn write_durable_new(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let dir = path.parent().ok_or_else(|| io::Error::other("marker path has no parent"))?;
    fs::create_dir_all(dir)?;
    if let Some(parent) = dir.parent() {
        fsync_dir(parent)?;
    }
    let tmp_name = format!(".{}.tmp", path.file_name().and_then(|n| n.to_str()).unwrap_or("marker"));
    let tmp = dir.join(tmp_name);
    let _ = fs::remove_file(&tmp);
    {
        let mut file = fs::OpenOptions::new().write(true).create_new(true).open(&tmp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
    }
    fs::rename(&tmp, path)?;
    fsync_dir(dir)?;
    Ok(())
}

#[cfg(target_os = "macos")]
fn done_state(data_dir: &Path) -> DoneState {
    match fs::symlink_metadata(done_marker_path(data_dir)) {
        Ok(meta) if meta.is_file() => DoneState::Done,
        Ok(meta) if meta.file_type().is_symlink() => DoneState::Absent,
        Ok(_) => DoneState::Retry("done marker is not a regular file".into()),
        Err(e) if e.kind() == io::ErrorKind::NotFound => DoneState::Absent,
        Err(e) => DoneState::Retry(e.to_string()),
    }
}

#[cfg(target_os = "macos")]
fn write_pending(data_dir: &Path, pending: &PendingMigration) -> io::Result<()> {
    let json = serde_json::to_vec(pending).map_err(io::Error::other)?;
    write_durable_new(&pending_marker_path(data_dir), &json)
}

#[cfg(target_os = "macos")]
fn read_pending(data_dir: &Path) -> PendingFile {
    let path = pending_marker_path(data_dir);
    let text = match fs::symlink_metadata(&path) {
        Err(e) if e.kind() == io::ErrorKind::NotFound => return PendingFile::Absent,
        Err(e) => return PendingFile::Unreadable(e.to_string()),
        Ok(meta) if !meta.is_file() => return PendingFile::Malformed,
        Ok(_) => match fs::read_to_string(&path) {
            Ok(text) => text,
            Err(e) => return PendingFile::Unreadable(e.to_string()),
        },
    };
    match serde_json::from_str::<PendingMigration>(&text) {
        Ok(pending) if pending_paths_present(&pending) => PendingFile::Ready(pending),
        _ => PendingFile::Malformed,
    }
}

#[cfg(target_os = "macos")]
fn pending_paths_present(pending: &PendingMigration) -> bool {
    !pending.old_prefix.as_os_str().is_empty() && !pending.new_prefix.as_os_str().is_empty()
}

#[cfg(target_os = "macos")]
fn clear_pending(data_dir: &Path) -> io::Result<()> {
    let path = pending_marker_path(data_dir);
    match fs::remove_file(&path) {
        Ok(()) => fsync_dir(path.parent().unwrap_or(data_dir)),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e),
    }
}

#[cfg(target_os = "macos")]
fn mark_done(data_dir: &Path) -> io::Result<()> {
    let path = done_marker_path(data_dir);
    match fs::symlink_metadata(&path) {
        Ok(meta) if meta.is_file() => {
            clear_pending(data_dir)?;
            return Ok(());
        }
        Ok(meta) if meta.file_type().is_symlink() => {}
        Ok(_) => {
            return Err(io::Error::other("done marker is not a file"));
        }
        Err(e) if e.kind() != io::ErrorKind::NotFound => return Err(e),
        Err(_) => {}
    }
    write_durable_new(&path, b"")?;
    clear_pending(data_dir)?;
    Ok(())
}

#[cfg(target_os = "macos")]
fn rewrite_timeline(data_dir: &Path, old_prefix: &Path, new_prefix: &Path) -> Result<(), String> {
    let labels: Vec<&str> = legacy_timeline_detail_labels().to_vec();
    match nectarlink_core::rewrite_timeline_downloads_path_prefix(
        data_dir,
        old_prefix,
        new_prefix,
        &labels,
        branded_timeline_detail_label(),
    ) {
        Ok(count) => {
            tracing::info!(
                old = %old_prefix.display(),
                new = %new_prefix.display(),
                rows = count,
                "rewrote timeline paths after downloads folder rename"
            );
            Ok(())
        }
        Err(e) => Err(e.to_string()),
    }
}

#[cfg(target_os = "macos")]
fn finish_rewrite(data_dir: &Path, pending: &PendingMigration) -> Result<(), String> {
    if let Err(e) = rewrite_timeline(data_dir, &pending.old_prefix, &pending.new_prefix) {
        tracing::error!(error = %e, "timeline rewrite failed after downloads rename; will retry");
        return Err(e);
    }
    mark_done(data_dir).map_err(|e| e.to_string())
}

/// Keeps the pending marker when history still names the old folder. Marking done
/// here would freeze those links, and rewriting them would point at a different folder.
#[cfg(target_os = "macos")]
fn hold_identity_mismatch(data_dir: &Path, pending: &PendingMigration) -> Result<(), String> {
    match nectarlink_core::timeline_targets_use_prefix(data_dir, &pending.old_prefix) {
        Ok(true) => {
            tracing::error!(
                old = %pending.old_prefix.display(),
                new = %pending.new_prefix.display(),
                "downloads target is not the renamed folder; keeping the pending marker"
            );
            Ok(())
        }
        Ok(false) => {
            tracing::warn!(
                old = %pending.old_prefix.display(),
                new = %pending.new_prefix.display(),
                "downloads target identity differs and history does not use the old path"
            );
            mark_done(data_dir).map_err(|e| e.to_string())
        }
        Err(e) => {
            tracing::error!(
                error = %e,
                "could not read timeline after an identity mismatch; keeping the pending marker"
            );
            Err(e.to_string())
        }
    }
}

#[cfg(target_os = "macos")]
fn skip_inaccessible(path: &Path, err: &io::Error) -> Result<(), String> {
    tracing::warn!(
        error = %err,
        path = %path.display(),
        "downloads path is not accessible; will retry migration without marking it done"
    );
    Ok(())
}

/// `Ok(true)` means the marker was cleared and a fresh migration should run.
#[cfg(target_os = "macos")]
fn resume_pending(data_dir: &Path, pending: &PendingMigration) -> Result<bool, String> {
    match probe(&pending.new_prefix) {
        Ok(PathState::Dir { dev, ino }) if (dev, ino) == (pending.source_dev, pending.source_ino) => {
            finish_rewrite(data_dir, pending)?;
            Ok(false)
        }
        Ok(PathState::Dir { .. }) => {
            hold_identity_mismatch(data_dir, pending)?;
            Ok(false)
        }
        Ok(PathState::Symlink | PathState::Other) => {
            tracing::warn!(
                path = %pending.new_prefix.display(),
                "migrated downloads path is not a real directory; will retry"
            );
            Ok(false)
        }
        Ok(PathState::Absent) => match probe(&pending.old_prefix) {
            Ok(PathState::Dir { .. }) => {
                clear_pending(data_dir).map_err(|e| e.to_string())?;
                Ok(true)
            }
            Ok(PathState::Absent) => {
                tracing::warn!("downloads migration pending but neither folder exists; leaving the marker");
                Ok(false)
            }
            Ok(_) => {
                tracing::warn!(
                    "downloads migration pending but the legacy path is not a directory; leaving the marker"
                );
                Ok(false)
            }
            Err(e) => skip_inaccessible(&pending.old_prefix, &e).map(|()| false),
        },
        Err(e) => skip_inaccessible(&pending.new_prefix, &e).map(|()| false),
    }
}

/// An empty or truncated marker is from a crash before rename. Start over only
/// while the legacy folder is still the real directory and the branded name is free.
#[cfg(target_os = "macos")]
fn recover_malformed(data_dir: &Path, root: &Path) -> Result<bool, String> {
    let legacy = root.join(LEGACY_SUBDIR);
    let branded = root.join(BRANDED_SUBDIR);
    let legacy_state = match probe(&legacy) {
        Ok(state) => state,
        Err(e) => return skip_inaccessible(&legacy, &e).map(|()| false),
    };
    let branded_state = match probe(&branded) {
        Ok(state) => state,
        Err(e) => return skip_inaccessible(&branded, &e).map(|()| false),
    };
    match (legacy_state, branded_state) {
        (PathState::Dir { .. }, PathState::Absent) => {
            clear_pending(data_dir).map_err(|e| e.to_string())?;
            Ok(true)
        }
        (PathState::Dir { .. }, PathState::Dir { .. }) => {
            tracing::info!("both download folders exist after a damaged migration marker; keeping both");
            mark_done(data_dir).map_err(|e| e.to_string())?;
            Ok(false)
        }
        (PathState::Symlink | PathState::Other, _) => {
            tracing::warn!(
                path = %legacy.display(),
                "legacy downloads path is not a real directory; skipping migration"
            );
            mark_done(data_dir).map_err(|e| e.to_string())?;
            Ok(false)
        }
        (PathState::Absent, PathState::Dir { .. } | PathState::Absent) => {
            tracing::error!("damaged downloads migration marker and no legacy folder; leaving it unmarked");
            Ok(false)
        }
        _ => {
            tracing::warn!("damaged downloads migration marker; leaving it for retry");
            Ok(false)
        }
    }
}

#[cfg(target_os = "macos")]
fn rename_legacy(
    data_dir: &Path,
    root: &Path,
    legacy: &Path,
    branded: &Path,
    dev: u64,
    ino: u64,
) -> Result<(), String> {
    let pending = PendingMigration {
        old_prefix: legacy.to_path_buf(),
        new_prefix: branded.to_path_buf(),
        source_dev: dev,
        source_ino: ino,
    };
    write_pending(data_dir, &pending).map_err(|e| e.to_string())?;
    let rename_result = fs::rename(legacy, branded);
    match probe(branded) {
        Ok(PathState::Dir { dev: got_dev, ino: got_ino }) if (got_dev, got_ino) == (dev, ino) => {
            if let Err(e) = fsync_dir(root) {
                tracing::error!(
                    error = %e,
                    "downloads directory fsync failed after rename; will retry the history rewrite"
                );
                return Err(e.to_string());
            }
            finish_rewrite(data_dir, &pending)
        }
        Ok(PathState::Dir { .. }) => {
            if probe(legacy).ok().is_some_and(|state| matches!(state, PathState::Absent)) {
                let _ = fs::rename(branded, legacy);
                let _ = fsync_dir(root);
            }
            hold_identity_mismatch(data_dir, &pending)
        }
        Ok(PathState::Absent) => {
            clear_pending(data_dir).map_err(|e| e.to_string())?;
            Err(rename_result
                .err()
                .map(|e| e.to_string())
                .unwrap_or_else(|| "failed to rename legacy downloads folder".into()))
        }
        Ok(PathState::Symlink | PathState::Other) => {
            tracing::warn!(
                path = %branded.display(),
                "branded downloads path is not a real directory; leaving the pending marker"
            );
            Ok(())
        }
        Err(e) => Err(e.to_string()),
    }
}

/// Where new files go while a migration is unfinished.
///
/// After a successful rename, saves stay on the branded folder with the files.
/// Before that rename, saves stay on the legacy folder so a later launch can still rename it.
#[cfg(target_os = "macos")]
fn save_dir(data_dir: Option<&Path>, downloads_root: &Path) -> PathBuf {
    let branded = downloads_root.join(BRANDED_SUBDIR);
    let Some(data_dir) = data_dir else {
        return branded;
    };
    if matches!(done_state(data_dir), DoneState::Done) {
        return branded;
    }
    let legacy = downloads_root.join(LEGACY_SUBDIR);
    match (probe(&legacy), probe(&branded)) {
        (Ok(PathState::Dir { .. }), Ok(PathState::Absent | PathState::Symlink | PathState::Other))
        | (Ok(PathState::Dir { .. }), Err(_)) => legacy,
        _ => branded,
    }
}

#[cfg(target_os = "macos")]
fn migrate_at_root(data_dir: &Path, root: &Path) -> Result<(), String> {
    let legacy = root.join(LEGACY_SUBDIR);
    let branded = root.join(BRANDED_SUBDIR);
    let legacy_state = match probe(&legacy) {
        Ok(state) => state,
        Err(e) => return skip_inaccessible(&legacy, &e),
    };
    let branded_state = match probe(&branded) {
        Ok(state) => state,
        Err(e) => return skip_inaccessible(&branded, &e),
    };
    match (legacy_state, branded_state) {
        (legacy_state, PathState::Dir { .. }) => {
            // No pending identity to prove a rename, so do not rewrite history.
            // A lost marker after a durable rename is the remaining power-loss window.
            if !matches!(legacy_state, PathState::Absent) {
                tracing::info!(
                    legacy = %legacy.display(),
                    branded = %branded.display(),
                    "both download folders exist; keeping both, new saves use the branded folder"
                );
            }
            mark_done(data_dir).map_err(|e| e.to_string())
        }
        (PathState::Absent, PathState::Absent) => mark_done(data_dir).map_err(|e| e.to_string()),
        (PathState::Symlink | PathState::Other, _) => {
            tracing::warn!(
                path = %legacy.display(),
                "legacy downloads path is not a real directory; skipping migration"
            );
            mark_done(data_dir).map_err(|e| e.to_string())
        }
        (PathState::Dir { dev, ino }, PathState::Absent) => {
            rename_legacy(data_dir, root, &legacy, &branded, dev, ino)
        }
        (PathState::Dir { .. }, PathState::Symlink | PathState::Other)
        | (PathState::Absent, PathState::Symlink | PathState::Other) => {
            tracing::warn!(
                path = %branded.display(),
                "branded downloads path is not a real directory; will retry without replacing it"
            );
            Ok(())
        }
    }
}

/// Renames the legacy default received-files folder on macOS for normal installs
/// only (`normal_profile` is false for `--data-dir` QA runs). Idempotent.
///
/// `downloads_root` is `Some` for the real Downloads directory. Tests pass a temp root.
#[cfg(target_os = "macos")]
fn migrate_received_folder(
    data_dir: &Path,
    normal_profile: bool,
    downloads_root: Option<&Path>,
) -> Result<(), String> {
    if !normal_profile {
        return Ok(());
    }
    match done_state(data_dir) {
        DoneState::Done => return Ok(()),
        DoneState::Absent => {}
        DoneState::Retry(msg) => {
            tracing::warn!(error = %msg, "downloads migration state is unreadable; will retry");
            return Ok(());
        }
    }

    let mut start_fresh = true;
    match read_pending(data_dir) {
        PendingFile::Unreadable(msg) => {
            tracing::warn!(error = %msg, "downloads migration marker is unreadable; will retry");
            return Ok(());
        }
        PendingFile::Ready(pending) => {
            start_fresh = resume_pending(data_dir, &pending)?;
        }
        PendingFile::Malformed => {
            let Some(root) = downloads_root else {
                tracing::warn!("damaged downloads migration marker and no downloads folder; will retry");
                return Ok(());
            };
            start_fresh = recover_malformed(data_dir, root)?;
        }
        PendingFile::Absent => {}
    }
    if !start_fresh {
        return Ok(());
    }

    let Some(root) = downloads_root else {
        mark_done(data_dir).map_err(|e| e.to_string())?;
        return Ok(());
    };
    if let Err(e) = fs::metadata(root)
        && e.kind() != io::ErrorKind::NotFound
    {
        return skip_inaccessible(root, &e);
    }
    migrate_at_root(data_dir, root)
}

#[cfg(target_os = "macos")]
pub fn maybe_migrate_received_folder(data_dir: &Path, normal_profile: bool) -> Result<(), String> {
    migrate_received_folder(data_dir, normal_profile, dirs::download_dir().as_deref())
}

#[cfg(not(target_os = "macos"))]
pub fn maybe_migrate_received_folder(_data_dir: &Path, _normal_profile: bool) -> Result<(), String> {
    Ok(())
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::*;

    struct Rm(PathBuf);
    impl Drop for Rm {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn layout(name: &str) -> (Rm, PathBuf, PathBuf) {
        let base = std::env::temp_dir().join(format!("waypair-downloads-test-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&base);
        let data = base.join("data");
        let root = base.join("Downloads");
        fs::create_dir_all(&data).unwrap();
        fs::create_dir_all(&root).unwrap();
        (Rm(base), data, root)
    }

    fn path_str(path: &Path) -> String {
        path.to_string_lossy().into_owned()
    }

    fn seed(data_dir: &Path, kind: &str, target: &str, detail: &str) {
        nectarlink_core::insert_timeline_path_row(data_dir, kind, target, detail).unwrap();
    }

    fn rows(data_dir: &Path) -> Vec<(String, String, String)> {
        nectarlink_core::timeline_path_rows(data_dir).unwrap()
    }

    #[test]
    fn migrate_received_folder_rewrites_store_and_keeps_files() {
        let (_rm, data, root) = layout("ok");
        let legacy = root.join(LEGACY_SUBDIR);
        let branded = root.join(BRANDED_SUBDIR);
        let custom = root.join("Custom Folder");
        fs::create_dir_all(&legacy).unwrap();
        fs::create_dir_all(&custom).unwrap();
        fs::write(legacy.join("keep.txt"), b"same-bytes").unwrap();
        fs::write(custom.join("note.txt"), b"custom").unwrap();
        let settings = data.join("settings.json");
        fs::write(&settings, br#"{"keep":true}"#).unwrap();
        let PathState::Dir { dev, ino } = probe(&legacy).unwrap() else { panic!("legacy dir") };

        let old_s = path_str(&legacy);
        let new_s = path_str(&branded);
        let custom_s = path_str(&custom.join("note.txt"));
        seed(&data, "file", &format!("{old_s}/a.jpg\n{old_s}/b.jpg\n{custom_s}"), "Downloads/Nectarlink");
        seed(&data, "photo", &old_s, r"Downloads\Nectarlink");
        seed(&data, "file", &format!("{old_s}X/nope"), "Downloads/Nectarlink");
        seed(&data, "clip", &format!("{old_s}/clip.txt"), "Downloads/Nectarlink");
        seed(&data, "file", &format!("{old_s}/d.jpg"), "Holiday");
        assert_eq!(save_dir(Some(&data), &root), legacy);

        migrate_received_folder(&data, false, Some(&root)).unwrap();
        assert!(legacy.join("keep.txt").is_file());
        assert!(!done_marker_path(&data).exists());
        assert_eq!(rows(&data)[0].1, format!("{old_s}/a.jpg\n{old_s}/b.jpg\n{custom_s}"));

        migrate_received_folder(&data, true, Some(&root)).unwrap();
        let PathState::Dir { dev: dev2, ino: ino2 } = probe(&branded).unwrap() else { panic!("branded dir") };
        assert_eq!((dev, ino), (dev2, ino2));
        assert!(matches!(probe(&legacy).unwrap(), PathState::Absent));
        assert_eq!(fs::read(branded.join("keep.txt")).unwrap(), b"same-bytes");
        assert_eq!(fs::read(custom.join("note.txt")).unwrap(), b"custom");
        assert_eq!(fs::read(&settings).unwrap(), br#"{"keep":true}"#);
        assert!(done_marker_path(&data).is_file());
        assert!(matches!(probe(&pending_marker_path(&data)).unwrap(), PathState::Absent));
        assert_eq!(save_dir(Some(&data), &root), branded);

        let got = rows(&data);
        assert_eq!(got[0].0, "file");
        assert_eq!(got[0].1, format!("{new_s}/a.jpg\n{new_s}/b.jpg\n{custom_s}"));
        assert_eq!(got[0].2, "Downloads/Waypair Link");
        assert_eq!(got[1], ("photo".into(), new_s.clone(), "Downloads/Waypair Link".into()));
        assert_eq!(got[2], ("file".into(), format!("{old_s}X/nope"), "Downloads/Nectarlink".into()));
        assert_eq!(got[3], ("clip".into(), format!("{old_s}/clip.txt"), "Downloads/Nectarlink".into()));
        assert_eq!(got[4], ("file".into(), format!("{new_s}/d.jpg"), "Holiday".into()));

        migrate_received_folder(&data, true, Some(&root)).unwrap();
        assert_eq!(rows(&data), got);
        assert_eq!(fs::read(branded.join("keep.txt")).unwrap(), b"same-bytes");
    }

    #[test]
    fn migration_failures_keep_history_retryable_and_do_not_clobber() {
        // A dangling branded symlink must not be replaced, followed, or marked done.
        {
            let (_rm, data, root) = layout("symlink");
            let legacy = root.join(LEGACY_SUBDIR);
            let branded = root.join(BRANDED_SUBDIR);
            fs::create_dir_all(&legacy).unwrap();
            fs::write(legacy.join("keep.txt"), b"legacy").unwrap();
            std::os::unix::fs::symlink(root.join("missing-target"), &branded).unwrap();
            migrate_received_folder(&data, true, Some(&root)).unwrap();
            assert_eq!(fs::read(legacy.join("keep.txt")).unwrap(), b"legacy");
            assert!(fs::symlink_metadata(&branded).unwrap().file_type().is_symlink());
            assert!(matches!(done_state(&data), DoneState::Absent));
            assert_eq!(save_dir(Some(&data), &root), legacy);
        }

        // An empty pending marker is a crash before rename. Recovery still migrates.
        {
            let (_rm, data, root) = layout("empty-marker");
            let legacy = root.join(LEGACY_SUBDIR);
            let branded = root.join(BRANDED_SUBDIR);
            fs::create_dir_all(&legacy).unwrap();
            fs::write(legacy.join("keep.txt"), b"recovered").unwrap();
            fs::create_dir_all(migration_state_dir(&data)).unwrap();
            fs::write(pending_marker_path(&data), b"").unwrap();
            let old_s = path_str(&legacy);
            seed(&data, "file", &format!("{old_s}/keep.txt"), "Downloads/Nectarlink");
            migrate_received_folder(&data, true, Some(&root)).unwrap();
            assert_eq!(fs::read(branded.join("keep.txt")).unwrap(), b"recovered");
            assert!(matches!(probe(&legacy).unwrap(), PathState::Absent));
            assert_eq!(rows(&data)[0].1, format!("{}/keep.txt", path_str(&branded)));
            assert!(matches!(done_state(&data), DoneState::Done));
            assert!(matches!(probe(&pending_marker_path(&data)).unwrap(), PathState::Absent));
        }

        // A different folder at the branded path is not history's new home.
        {
            let (_rm, data, root) = layout("identity");
            let legacy = root.join(LEGACY_SUBDIR);
            let branded = root.join(BRANDED_SUBDIR);
            fs::create_dir_all(&branded).unwrap();
            fs::write(branded.join("other.txt"), b"stay").unwrap();
            let old_s = path_str(&legacy);
            seed(&data, "file", &format!("{old_s}/gone.bin"), "Downloads/Nectarlink");
            let PathState::Dir { dev, ino } = probe(&branded).unwrap() else { panic!("branded dir") };
            write_pending(
                &data,
                &PendingMigration {
                    old_prefix: legacy.clone(),
                    new_prefix: branded.clone(),
                    source_dev: dev,
                    source_ino: ino.wrapping_add(1),
                },
            )
            .unwrap();
            let before = rows(&data);
            migrate_received_folder(&data, true, Some(&root)).unwrap();
            assert_eq!(fs::read(branded.join("other.txt")).unwrap(), b"stay");
            assert!(matches!(probe(&legacy).unwrap(), PathState::Absent));
            assert_eq!(rows(&data), before);
            assert!(matches!(done_state(&data), DoneState::Absent));
            assert!(pending_marker_path(&data).is_file());
            assert_eq!(save_dir(Some(&data), &root), branded);
        }

        // A database that cannot be opened leaves the renamed files and the old history for retry.
        {
            let (_rm, data, root) = layout("sql");
            let legacy = root.join(LEGACY_SUBDIR);
            let branded = root.join(BRANDED_SUBDIR);
            fs::create_dir_all(&legacy).unwrap();
            fs::write(legacy.join("keep.txt"), b"moved").unwrap();
            let settings = data.join("settings.json");
            fs::write(&settings, b"settings-stay").unwrap();
            let old_s = path_str(&legacy);
            seed(&data, "photo", &format!("{old_s}/keep.txt"), r"Downloads\Nectarlink");
            let snap = data.join("db-snap");
            fs::create_dir_all(&snap).unwrap();
            for name in ["nectarlink.db", "nectarlink.db-wal", "nectarlink.db-shm"] {
                let src = data.join(name);
                if src.is_file() {
                    fs::copy(&src, snap.join(name)).unwrap();
                    fs::remove_file(&src).unwrap();
                }
            }
            fs::create_dir(data.join("nectarlink.db")).unwrap();

            let err = migrate_received_folder(&data, true, Some(&root)).unwrap_err();
            assert!(!err.is_empty());
            assert_eq!(fs::read(branded.join("keep.txt")).unwrap(), b"moved");
            assert!(matches!(probe(&legacy).unwrap(), PathState::Absent));
            assert!(pending_marker_path(&data).is_file());
            assert!(matches!(done_state(&data), DoneState::Absent));
            assert_eq!(save_dir(Some(&data), &root), branded);
            assert_eq!(fs::read(&settings).unwrap(), b"settings-stay");

            fs::remove_dir_all(data.join("nectarlink.db")).unwrap();
            for name in ["nectarlink.db-wal", "nectarlink.db-shm"] {
                let extra = data.join(name);
                if extra.is_file() {
                    fs::remove_file(extra).unwrap();
                }
            }
            for name in ["nectarlink.db", "nectarlink.db-wal", "nectarlink.db-shm"] {
                let src = snap.join(name);
                if src.is_file() {
                    fs::copy(&src, data.join(name)).unwrap();
                }
            }
            assert_eq!(rows(&data)[0].1, format!("{old_s}/keep.txt"));

            migrate_received_folder(&data, true, Some(&root)).unwrap();
            assert_eq!(rows(&data)[0].1, format!("{}/keep.txt", path_str(&branded)));
            assert_eq!(rows(&data)[0].2, "Downloads/Waypair Link");
            assert!(matches!(done_state(&data), DoneState::Done));
            assert!(matches!(probe(&pending_marker_path(&data)).unwrap(), PathState::Absent));
            assert_eq!(fs::read(branded.join("keep.txt")).unwrap(), b"moved");
            assert_eq!(fs::read(&settings).unwrap(), b"settings-stay");
        }
    }
}
