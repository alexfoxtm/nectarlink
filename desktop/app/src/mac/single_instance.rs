// SPDX-License-Identifier: GPL-3.0-or-later
//! macOS single-instance lock and IPC using a held file lock and Unix domain socket.

use std::{
    fs::{self, File, OpenOptions, TryLockError},
    io::{self, Read, Write},
    os::unix::{
        fs::{FileTypeExt, MetadataExt, OpenOptionsExt, PermissionsExt},
        net::{UnixListener, UnixStream},
    },
    path::{Path, PathBuf},
    sync::Mutex,
};

const LOCK_FILE: &str = "instance.lock";
const SOCKET_FILE: &str = "nectarlink.sock";

// Darwin sys/fcntl.h value.
const O_NOFOLLOW: i32 = 0x0000_0100;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct SocketIdentity {
    device: u64,
    inode: u64,
}

impl SocketIdentity {
    fn from_metadata(metadata: &fs::Metadata) -> Self {
        Self { device: metadata.dev(), inode: metadata.ino() }
    }
}

/// The primary holds the locked file open for its full lifetime. Dropping that
/// file handle releases the lock, including after process exit or a crash.
#[derive(Debug)]
pub struct PrimaryInstance {
    _lock_file: File,
    listener: Mutex<Option<UnixListener>>,
    sock_path: PathBuf,
    socket_identity: SocketIdentity,
}

#[derive(Debug)]
pub enum Instance {
    Primary(PrimaryInstance),
    Secondary,
}

fn lock_path(data_dir: &Path) -> PathBuf {
    data_dir.join(LOCK_FILE)
}

fn socket_path(data_dir: &Path) -> PathBuf {
    data_dir.join(SOCKET_FILE)
}

fn invalid_path(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

/// Acquires the single-instance lock for `data_dir`, or reports that another
/// instance already holds it.
pub fn acquire(data_dir: &Path) -> io::Result<Instance> {
    fs::create_dir_all(data_dir)?;
    if !fs::symlink_metadata(data_dir)?.is_dir() {
        return Err(invalid_path("single-instance data path is not a directory"));
    }

    let lock_file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .custom_flags(O_NOFOLLOW)
        .mode(0o600)
        .open(lock_path(data_dir))?;
    if !lock_file.metadata()?.is_file() {
        return Err(invalid_path("single-instance lock path is not a regular file"));
    }
    // OpenOptionsExt::mode applies only when creating a file; enforce the mode
    // on an existing lock file as well.
    lock_file.set_permissions(fs::Permissions::from_mode(0o600))?;

    match lock_file.try_lock() {
        Ok(()) => {}
        Err(TryLockError::WouldBlock) => return Ok(Instance::Secondary),
        Err(TryLockError::Error(error)) => return Err(error),
    }

    let sock_path = socket_path(data_dir);
    match fs::symlink_metadata(&sock_path) {
        Ok(metadata) => {
            if !metadata.file_type().is_socket() {
                return Err(invalid_path("single-instance socket path is not a socket"));
            }

            match UnixStream::connect(&sock_path) {
                Ok(_) => return Ok(Instance::Secondary),
                Err(error) if error.kind() == io::ErrorKind::ConnectionRefused => {
                    // Only remove the exact stale socket we checked. If the path
                    // changed, fail closed and leave it untouched.
                    match fs::symlink_metadata(&sock_path) {
                        Ok(current)
                            if current.file_type().is_socket()
                                && SocketIdentity::from_metadata(&current)
                                    == SocketIdentity::from_metadata(&metadata) =>
                        {
                            match fs::remove_file(&sock_path) {
                                Ok(()) => {}
                                Err(remove_error) if remove_error.kind() == io::ErrorKind::NotFound => {}
                                Err(remove_error) => return Err(remove_error),
                            }
                        }
                        Err(current_error) if current_error.kind() == io::ErrorKind::NotFound => {}
                        Ok(_) => {
                            return Err(invalid_path("single-instance socket changed during validation"));
                        }
                        Err(current_error) => return Err(current_error),
                    }
                }
                Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                Err(error) => return Err(error),
            }
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }

    let listener = UnixListener::bind(&sock_path)?;
    let metadata = fs::symlink_metadata(&sock_path)?;
    if !metadata.file_type().is_socket() {
        return Err(invalid_path("bound single-instance path is not a socket"));
    }

    Ok(Instance::Primary(PrimaryInstance {
        _lock_file: lock_file,
        listener: Mutex::new(Some(listener)),
        sock_path,
        socket_identity: SocketIdentity::from_metadata(&metadata),
    }))
}

pub fn wake(data_dir: &Path) {
    let sock_path = socket_path(data_dir);
    if let Ok(mut stream) = UnixStream::connect(&sock_path) {
        let _ = stream.write_all(b"wake\n");
    }
}

impl PrimaryInstance {
    pub fn watch(&'static self, on_activate: impl Fn() + Send + 'static) {
        let listener = match self.listener.lock() {
            Ok(mut guard) => guard.take(),
            Err(_) => None,
        };
        let Some(listener) = listener else {
            return;
        };

        std::thread::Builder::new()
            .name("single-instance".into())
            .spawn(move || {
                for mut s in listener.incoming().flatten() {
                    let mut buf = [0u8; 16];
                    let _ = s.read(&mut buf);
                    on_activate();
                }
            })
            .ok();
    }
}

impl Drop for PrimaryInstance {
    fn drop(&mut self) {
        match fs::symlink_metadata(&self.sock_path) {
            Ok(metadata)
                if metadata.file_type().is_socket()
                    && SocketIdentity::from_metadata(&metadata) == self.socket_identity =>
            {
                if let Err(error) = fs::remove_file(&self.sock_path) {
                    tracing::debug!(error = %error, "can't remove single-instance socket");
                }
            }
            Ok(_) => tracing::debug!("single-instance socket changed; leaving it in place"),
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => {
                tracing::debug!(error = %error, "can't inspect single-instance socket during shutdown")
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::{PermissionsExt, symlink};

    #[test]
    fn file_lock_is_held_until_primary_drops_then_can_be_reacquired() {
        let dir = tempfile::tempdir().unwrap();
        let primary = match acquire(dir.path()).unwrap() {
            Instance::Primary(primary) => primary,
            Instance::Secondary => panic!("first instance must acquire primary ownership"),
        };
        let lock_mode = fs::metadata(lock_path(dir.path())).unwrap().permissions().mode() & 0o777;
        assert_eq!(lock_mode, 0o600);
        assert!(matches!(acquire(dir.path()).unwrap(), Instance::Secondary));

        let sock_path = socket_path(dir.path());
        drop(primary);
        assert!(fs::symlink_metadata(&sock_path).is_err());
        assert!(matches!(acquire(dir.path()).unwrap(), Instance::Primary(_)));
    }

    #[test]
    fn refuses_symlinked_data_directory_lock_and_socket_paths_without_removing_targets() {
        let data_parent = tempfile::tempdir().unwrap();
        let data_target = data_parent.path().join("data-target");
        let data_link = data_parent.path().join("data-link");
        fs::create_dir(&data_target).unwrap();
        symlink(&data_target, &data_link).unwrap();
        assert!(acquire(&data_link).is_err());
        assert!(fs::symlink_metadata(&data_link).unwrap().file_type().is_symlink());
        assert!(fs::read_dir(&data_target).unwrap().next().is_none());

        let lock_dir = tempfile::tempdir().unwrap();
        let lock_target = lock_dir.path().join("lock-target");
        let lock_link = lock_path(lock_dir.path());
        fs::write(&lock_target, b"keep lock target").unwrap();
        symlink(&lock_target, &lock_link).unwrap();
        assert!(acquire(lock_dir.path()).is_err());
        assert!(fs::symlink_metadata(&lock_link).unwrap().file_type().is_symlink());
        assert_eq!(fs::read(&lock_target).unwrap(), b"keep lock target");

        let socket_dir = tempfile::tempdir().unwrap();
        let socket_target = socket_dir.path().join("socket-target");
        let socket_link = socket_path(socket_dir.path());
        fs::write(&socket_target, b"keep socket target").unwrap();
        symlink(&socket_target, &socket_link).unwrap();
        assert!(acquire(socket_dir.path()).is_err());
        assert!(fs::symlink_metadata(&socket_link).unwrap().file_type().is_symlink());
        assert_eq!(fs::read(&socket_target).unwrap(), b"keep socket target");
    }
}
