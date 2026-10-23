// SPDX-License-Identifier: GPL-3.0-or-later
//! Explicit user setup only. Never call from startup, refresh or camera capture.
use sha2::{Digest, Sha256};
use std::os::fd::AsRawFd;
use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt, PermissionsExt};
use std::{
    collections::BTreeMap,
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::Mutex,
    time::{Duration, Instant},
};

#[allow(unsafe_code)]
unsafe extern "C" {
    fn fcntl(fd: i32, command: i32, ...) -> i32;
    fn renameatx_np(
        fromfd: i32,
        from: *const std::ffi::c_char,
        tofd: i32,
        to: *const std::ffi::c_char,
        flags: u32,
    ) -> i32;
}

const DMG: &str = "WaypairCameraHelper-1.3.4.dmg";
const DMG_SHA: &str = "91850c858e83804128f191d9607f56cb8e2d3ac658c2ad0e6dad3270589c6946";
const HOST_SHA: &str = "66c40f57986ed76f48e487f2e724baa9bb78d5def7ee149ac27d508edc0293aa";
const EXT_SHA: &str = "741eb785e6b2d41643ac418a55135733229bea7a8bb3a428ecd438c04fabe626";
const EXT: &str = "Contents/Library/SystemExtensions/com.adrbn.LiveLoop.Extension.systemextension";
const DEST: &str = "/Applications/LiveLoop.app";
static SETUP: Mutex<()> = Mutex::new(());
static PENDING_CHILDREN: Mutex<Vec<Child>> = Mutex::new(Vec::new());

fn pending_children_exited() -> Result<(), String> {
    let mut pending = PENDING_CHILDREN.lock().unwrap_or_else(|error| error.into_inner());
    pending.retain_mut(|child| !matches!(child.try_wait(), Ok(Some(_))));
    if pending.is_empty() {
        Ok(())
    } else {
        Err("Camera setup is blocked until an earlier setup child has exited; cleanup is unconfirmed.".into())
    }
}

/// Means only that the original vendor setup UI was opened. Activation is pending.
#[derive(Debug, PartialEq, Eq)]
pub enum SetupOutcome {
    SetupOpened,
}

#[allow(unsafe_code)]
fn tool(command: &str, args: &[&std::ffi::OsStr]) -> Result<Vec<u8>, String> {
    // Fixed tools only; never retain or expose their diagnostic output.
    let mut child = Command::new(command)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|_| format!("Camera setup could not run {command}."))?;
    let result = (|| {
        let mut stdout = child.stdout.take().ok_or("Camera setup output unavailable.")?;
        // Darwin F_GETFL/F_SETFL and O_NONBLOCK. Polling avoids reader threads
        // surviving the deadline if a tool descendant retains the pipe.
        let flags = unsafe { fcntl(stdout.as_raw_fd(), 3) };
        if flags < 0 || unsafe { fcntl(stdout.as_raw_fd(), 4, flags | 0x4) } < 0 {
            return Err("Camera setup output could not be bounded.".to_owned());
        }
        let deadline = Instant::now() + Duration::from_secs(60);
        let mut out = Vec::new();
        let mut bytes = [0u8; 4096];
        loop {
            if Instant::now() >= deadline {
                return Err(format!("Camera setup tool timed out: {command}."));
            }
            match stdout.read(&mut bytes) {
                Ok(n) => {
                    if out.len() + n > 65536 {
                        return Err("Camera setup tool output exceeded its limit.".into());
                    }
                    out.extend_from_slice(&bytes[..n]);
                    if n != 0 {
                        continue;
                    }
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {}
                Err(_) => return Err("Camera setup tool output failed.".into()),
            }
            if let Some(status) =
                child.try_wait().map_err(|_| "Camera setup tool could not be observed.".to_owned())?
            {
                if !status.success() {
                    return Err(format!("Camera setup verification or copy failed: {command}."));
                }
                // The direct child has exited. Drain only currently available bytes,
                // never wait for descendants which may still hold stdout.
                loop {
                    match stdout.read(&mut bytes) {
                        Ok(0) => break,
                        Ok(n) => {
                            if Instant::now() >= deadline || out.len() + n > 65536 {
                                return Err("Camera setup tool output exceeded its bound.".into());
                            }
                            out.extend_from_slice(&bytes[..n]);
                        }
                        Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => break,
                        Err(_) => return Err("Camera setup tool output failed.".into()),
                    }
                }
                return Ok(out);
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    })();
    // Only the child created here is targeted. Never wait without a deadline.
    let kill_failed = result.is_err() && child.kill().is_err();
    let reap_deadline = Instant::now() + Duration::from_secs(2);
    loop {
        if let Ok(Some(_)) = child.try_wait() {
            return result;
        }
        if Instant::now() >= reap_deadline {
            // Retain ownership rather than dropping an unconfirmed live child.
            // A subsequent explicit setup must observe all retained children exit.
            PENDING_CHILDREN.lock().unwrap_or_else(|error| error.into_inner()).push(child);
            return Err(if kill_failed {
                "Camera setup child termination failed and cleanup is unconfirmed; setup is blocked until its exit is observed.".into()
            } else {
                "Camera setup child cleanup is unconfirmed; setup is blocked until its exit is observed."
                    .into()
            });
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}
fn snapshot_image(resource: &Path, snapshot: &Path) -> Result<(), String> {
    let mut input = fs::OpenOptions::new()
        .read(true)
        .custom_flags(0x100)
        .open(resource)
        .map_err(|_| "Bundled camera helper cannot be opened safely.".to_owned())?;
    let meta = input.metadata().map_err(|_| "Bundled camera helper metadata is unavailable.".to_owned())?;
    if !meta.is_file() || meta.len() != 2329969 {
        return Err("Bundled camera helper differs from the verified release.".into());
    }
    let mut output = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(snapshot)
        .map_err(|_| "Camera helper snapshot could not be created.".to_owned())?;
    let mut digest = Sha256::new();
    let mut total = 0usize;
    let mut bytes = [0u8; 65536];
    loop {
        let n = input.read(&mut bytes).map_err(|_| "Camera helper snapshot read failed.".to_owned())?;
        if n == 0 {
            break;
        }
        total += n;
        if total > 2329969 {
            return Err("Bundled camera helper size changed.".into());
        }
        digest.update(&bytes[..n]);
        output.write_all(&bytes[..n]).map_err(|_| "Camera helper snapshot write failed.".to_owned())?;
    }
    output.sync_all().map_err(|_| "Camera helper snapshot could not be saved.".to_owned())?;
    if total != 2329969
        || digest.finalize().iter().map(|byte| format!("{byte:02x}")).collect::<String>() != DMG_SHA
        || hash_regular(snapshot)? != DMG_SHA
    {
        return Err("Bundled camera helper differs from the verified release.".into());
    }
    Ok(())
}
fn applications_parent() -> Result<fs::File, String> {
    let parent = Path::new("/Applications");
    if fs::canonicalize(parent).map_err(|_| "Applications folder is unavailable.".to_owned())? != parent {
        return Err("Aliased Applications folder is unsupported.".into());
    }
    let dir = fs::OpenOptions::new()
        .read(true)
        .custom_flags(0x100 | 0x100000)
        .open(parent)
        .map_err(|_| "Applications folder cannot be opened safely.".to_owned())?;
    check_parent(&dir)?;
    Ok(dir)
}
fn check_parent(dir: &fs::File) -> Result<(), String> {
    let opened = dir.metadata().map_err(|_| "Applications folder is unavailable.".to_owned())?;
    let current =
        fs::symlink_metadata("/Applications").map_err(|_| "Applications folder changed.".to_owned())?;
    if !opened.is_dir()
        || !current.is_dir()
        || current.file_type().is_symlink()
        || opened.dev() != current.dev()
        || opened.ino() != current.ino()
    {
        return Err("Applications folder changed; setup stopped.".into());
    }
    Ok(())
}
#[allow(unsafe_code)]
fn publish(stage_dir: &fs::File, parent: &fs::File) -> Result<(), String> {
    check_parent(parent)?;
    let name = b"LiveLoop.app\0";
    // Darwin RENAME_EXCL is atomic and refuses every existing destination.
    let result = unsafe {
        renameatx_np(
            stage_dir.as_raw_fd(),
            name.as_ptr().cast(),
            parent.as_raw_fd(),
            name.as_ptr().cast(),
            0x4,
        )
    };
    if result != 0 {
        return Err(
            "Camera helper could not be published without replacement. Existing apps were preserved.".into(),
        );
    }
    check_parent(parent)
}
struct MountedImage {
    mount: PathBuf,
    attempted: bool,
}
impl MountedImage {
    fn detach(&mut self) -> Result<Vec<u8>, String> {
        self.attempted = true;
        tool("/usr/bin/hdiutil", &["detach".as_ref(), self.mount.as_os_str()])
    }
}
impl Drop for MountedImage {
    fn drop(&mut self) {
        if !self.attempted {
            let _ = self.detach();
        }
    }
}
fn check_stage(stage: &Path, opened: &fs::File) -> Result<(), String> {
    let saved = opened.metadata().map_err(|_| "Camera staging metadata is unavailable.".to_owned())?;
    let current = fs::symlink_metadata(stage).map_err(|_| "Camera staging changed.".to_owned())?;
    if !current.is_dir()
        || current.file_type().is_symlink()
        || saved.dev() != current.dev()
        || saved.ino() != current.ino()
    {
        return Err("Camera staging changed; setup stopped.".into());
    }
    Ok(())
}
fn hash_regular(path: &Path) -> Result<String, String> {
    let meta = fs::symlink_metadata(path).map_err(|_| "Camera setup file is missing.".to_owned())?;
    if !meta.file_type().is_file() {
        return Err("Camera setup requires a regular file.".into());
    }
    // O_NOFOLLOW on macOS: refuse a final-component symlink even if replaced after metadata.
    let mut file = fs::OpenOptions::new()
        .read(true)
        .custom_flags(0x100)
        .open(path)
        .map_err(|_| "Camera setup file cannot be read safely.".to_owned())?;
    let mut digest = Sha256::new();
    let mut bytes = [0u8; 65536];
    loop {
        let count = file.read(&mut bytes).map_err(|_| "Camera setup file read failed.".to_owned())?;
        if count == 0 {
            break;
        }
        digest.update(&bytes[..count]);
    }
    Ok(digest.finalize().iter().map(|byte| format!("{byte:02x}")).collect())
}
fn resource_from_executable(exe: &Path) -> Option<PathBuf> {
    let macos = exe.parent()?;
    let contents = macos.parent()?;
    let app = contents.parent()?;
    if macos.file_name()? != "MacOS" || contents.file_name()? != "Contents" || app.extension()? != "app" {
        return None;
    }
    Some(contents.join("Resources/camera-helper").join(DMG))
}
fn plist_value(bundle: &Path, key: &str) -> Result<String, String> {
    let plist = bundle.join("Contents/Info.plist");
    let value = tool(
        "/usr/bin/plutil",
        &["-extract".as_ref(), key.as_ref(), "raw".as_ref(), "-o".as_ref(), "-".as_ref(), plist.as_os_str()],
    )?;
    String::from_utf8(value)
        .map(|v| v.trim().to_owned())
        .map_err(|_| "Camera helper metadata is unreadable.".into())
}
fn verify_bundle(bundle: &Path, id: &str, exe: &str, expected: &str) -> Result<(), String> {
    if !fs::symlink_metadata(bundle).is_ok_and(|m| m.is_dir() && !m.file_type().is_symlink()) {
        return Err("Camera helper is not a regular application bundle.".into());
    }
    if plist_value(bundle, "CFBundleIdentifier")? != id
        || plist_value(bundle, "CFBundleShortVersionString")? != "1.3.4"
        || plist_value(bundle, "CFBundleVersion")? != "22"
        || plist_value(bundle, "CFBundleExecutable")? != exe
    {
        return Err("A different camera helper is present; it was preserved.".into());
    }
    if hash_regular(&bundle.join("Contents/MacOS").join(exe))? != expected {
        return Err("Camera helper executable differs from the verified release.".into());
    }
    let requirement = format!(
        "anchor apple generic and identifier \"{id}\" and certificate leaf[subject.OU] = \"2TWQF4T93E\""
    );
    tool(
        "/usr/bin/codesign",
        &[
            "--verify".as_ref(),
            "--deep".as_ref(),
            "--strict".as_ref(),
            "-R".as_ref(),
            requirement.as_ref(),
            bundle.as_os_str(),
        ],
    )?;
    Ok(())
}
fn verify(app: &Path) -> Result<(), String> {
    verify_bundle(app, "com.adrbn.LiveLoop", "LiveLoop", HOST_SHA)?;
    verify_bundle(&app.join(EXT), "com.adrbn.LiveLoop.Extension", "com.adrbn.LiveLoop.Extension", EXT_SHA)?;
    tool("/usr/sbin/spctl", &["--assess".as_ref(), "--type".as_ref(), "execute".as_ref(), app.as_os_str()])?;
    Ok(())
}
fn tree(app: &Path) -> Result<BTreeMap<PathBuf, String>, String> {
    fn visit(root: &Path, dir: &Path, out: &mut BTreeMap<PathBuf, String>) -> Result<(), String> {
        for entry in fs::read_dir(dir).map_err(|_| "Camera bundle cannot be inspected.".to_owned())? {
            let path = entry.map_err(|_| "Camera bundle entry cannot be inspected.".to_owned())?.path();
            let meta = fs::symlink_metadata(&path)
                .map_err(|_| "Camera bundle metadata is unavailable.".to_owned())?;
            let rel =
                path.strip_prefix(root).map_err(|_| "Camera bundle path escaped.".to_owned())?.to_path_buf();
            if meta.file_type().is_symlink() {
                out.insert(
                    rel,
                    format!(
                        "link:{:?}",
                        fs::read_link(&path).map_err(|_| "Camera bundle link is unreadable.".to_owned())?
                    ),
                );
            } else if meta.is_dir() {
                out.insert(rel, format!("dir:{}", meta.permissions().mode() & 0o7777));
                visit(root, &path, out)?;
            } else if meta.is_file() {
                out.insert(
                    rel,
                    format!("file:{}:{}", meta.permissions().mode() & 0o7777, hash_regular(&path)?),
                );
            } else {
                return Err("Unexpected camera bundle object.".into());
            }
        }
        Ok(())
    }
    let mut out = BTreeMap::new();
    visit(app, app, &mut out)?;
    Ok(out)
}
fn private_scratch() -> Result<PathBuf, String> {
    let mut nonce = [0u8; 16];
    if !super::secure_random(&mut nonce) {
        return Err("Camera setup randomness is unavailable.".into());
    }
    let suffix: String = nonce.iter().map(|b| format!("{b:02x}")).collect();
    let path = std::env::temp_dir().join(format!("waypair-camera-setup-{suffix}"));
    fs::DirBuilder::new()
        .mode(0o700)
        .create(&path)
        .map_err(|_| "Camera setup private staging could not be created.".to_owned())?;
    Ok(path)
}

/// Blocking: run off the UI thread, only for the user's explicit Set up camera click.
/// No privileges, capture, extension activation or unrelated process termination.
/// The original helper still needs Install Camera and normal macOS approval once.
pub fn ensure_installed_and_open_setup() -> Result<SetupOutcome, String> {
    let _guard = match SETUP.try_lock() {
        Ok(guard) => guard,
        Err(std::sync::TryLockError::Poisoned(error)) => error.into_inner(),
        Err(std::sync::TryLockError::WouldBlock) => return Err("Camera setup is already running.".into()),
    };
    pending_children_exited()?;
    let exe =
        std::env::current_exe().map_err(|_| "Waypair application location is unavailable.".to_owned())?;
    let resource =
        resource_from_executable(&exe).ok_or("Camera setup requires the installed Waypair app bundle.")?;
    let scratch = private_scratch()?;
    let snapshot = scratch.join(DMG);
    snapshot_image(&resource, &snapshot)?;
    let parent = applications_parent()?;
    // Retain private owned staging on errors for diagnosis; never delete arbitrary paths.
    let mount = scratch.join("mount.noindex");
    fs::DirBuilder::new()
        .mode(0o700)
        .create(&mount)
        .map_err(|_| "Camera setup mount point could not be created.".to_owned())?;
    let mut mounted = MountedImage { mount: mount.clone(), attempted: false };
    let attach = tool(
        "/usr/bin/hdiutil",
        &[
            "attach".as_ref(),
            "-readonly".as_ref(),
            "-nobrowse".as_ref(),
            "-mountpoint".as_ref(),
            mount.as_os_str(),
            snapshot.as_os_str(),
        ],
    );
    let installed = attach.and_then(|_| {
        let source = mount.join("LiveLoop.app"); verify(&source)?;
        let expected = tree(&source)?;
        let dest = Path::new(DEST);
        match fs::symlink_metadata(dest) {
            Ok(_) => { verify(dest)?; if tree(dest)? != expected { return Err("Existing LiveLoop differs and was preserved. Use its own setup or resolve the installation separately.".into()); } }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                check_parent(&parent)?;
                // Same-volume private staging permits one atomic no-replace rename.
                let stage = Path::new("/Applications").join(scratch.file_name().ok_or("Camera staging name is unavailable.")?);
                fs::DirBuilder::new().mode(0o700).create(&stage)
                    .map_err(|_| "Cannot privately stage the camera helper. No privilege request was made.".to_owned())?;
                check_parent(&parent)?;
                let stage_dir = fs::OpenOptions::new().read(true).custom_flags(0x100 | 0x100000).open(&stage)
                    .map_err(|_| "Camera staging could not be opened safely.".to_owned())?;
                check_stage(&stage, &stage_dir)?;
                let staged = stage.join("LiveLoop.app");
                tool("/usr/bin/ditto", &[source.as_os_str(), staged.as_os_str()])?;
                check_parent(&parent)?;
                check_stage(&stage, &stage_dir)?;
                verify(&staged)?;
                if tree(&staged)? != expected { return Err("Staged camera helper differs; setup stopped. Private staging was retained.".into()); }
                check_stage(&stage, &stage_dir)?;
                publish(&stage_dir, &parent)?;
            }
            Err(_) => return Err("Existing camera helper cannot be inspected and was preserved.".into()),
        }
        Ok(())
    });
    // Always attempt normal detach of this invocation's mount point; never force detach.
    let detach = mounted.detach();
    match (installed, detach) {
        (Err(error), Err(cleanup)) => {
            return Err(format!("{error} Camera setup image detach failed: {cleanup}"));
        }
        (Err(error), Ok(_)) => return Err(error),
        (Ok(_), Err(cleanup)) => {
            return Err(format!(
                "Camera helper was verified, but its setup image detach failed: {cleanup} Setup stopped."
            ));
        }
        (Ok(_), Ok(_)) => {}
    }
    check_parent(&parent)?;
    verify(Path::new(DEST))?;
    tool("/usr/bin/open", &["-a".as_ref(), Path::new(DEST).as_os_str()])?;
    Ok(SetupOutcome::SetupOpened)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn installed_bundle_resolves_only_fixed_helper_resource() {
        assert_eq!(
            resource_from_executable(Path::new("/Applications/Waypair.app/Contents/MacOS/Waypair")),
            Some(PathBuf::from(
                "/Applications/Waypair.app/Contents/Resources/camera-helper/WaypairCameraHelper-1.3.4.dmg"
            ))
        );
    }
    #[test]
    fn non_bundle_executable_has_no_helper_resource() {
        assert_eq!(resource_from_executable(Path::new("/tmp/Waypair")), None);
    }
}
