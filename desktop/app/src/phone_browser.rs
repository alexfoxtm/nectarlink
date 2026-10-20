// SPDX-License-Identifier: GPL-3.0-or-later
//! In-app browser for one paired phone's shared storage or SAF roots.
//!
//! Uses the existing encrypted `storage.list` and `storage.read` stream.
//! `ENABLED` stays false until Root's native acceptance. This module does not
//! advertise capabilities, change either device's `storage` toggle, or mount
//! anything in Finder.
//!
//! A save streams on the core runtime and writes on a blocking thread: a
//! temporary file made with `create_new` in the chosen folder, then linked
//! into place (never over an existing file) under the state lock, so closing,
//! switching phones or losing access first always wins.

use std::{
    collections::HashSet,
    fs::{File, OpenOptions},
    io::{ErrorKind, Write},
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex, MutexGuard,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};

use nectarlink_core::{
    ConnectionPath, DeviceId, Error, LinkState, NodeEvent,
    storage::{StorageEntry, is_valid_storage_dir_path, is_valid_storage_name, is_valid_storage_path},
};
use tokio::sync::mpsc;

/// Root sets this true only after native acceptance. While false, every
/// network and UI entry point is a no-op.
pub const ENABLED: bool = false;

/// Rows retained from one `storage.list` reply. The phone already trims the
/// frame to 1 MiB; this is a second local cap.
pub const MAX_ROWS: usize = 2048;

/// One save reads at most this many bytes, on a single ranged stream.
pub const SAVE_MAX: u64 = 512 * 1024 * 1024;

/// Local write size. The phone still sends one open-file stream.
const READ_CHUNK: usize = 64 * 1024;

/// How often a save waiting on the phone or the disk checks it's still wanted.
const SAVE_POLL: Duration = Duration::from_secs(1);

/// A save that makes no progress for this long fails.
const SAVE_IDLE: Duration = Duration::from_secs(30);

/// Chunks between the stream and the file writer, at most.
const WRITE_QUEUE: usize = 4;

static TEMP_SEQ: AtomicU64 = AtomicU64::new(1);
static STATE: Mutex<State> = Mutex::new(State::empty());
static LISTENER: Mutex<Option<Arc<dyn Fn() + Send + Sync>>> = Mutex::new(None);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BrowseStatus {
    Hidden,
    Idle,
    Loading,
    Ready,
    Denied,
    Unsupported,
    NotFound,
    Invalid,
    Offline,
    Failed,
}

impl BrowseStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            BrowseStatus::Hidden => "hidden",
            BrowseStatus::Idle => "idle",
            BrowseStatus::Loading => "loading",
            BrowseStatus::Ready => "ready",
            BrowseStatus::Denied => "denied",
            BrowseStatus::Unsupported => "unsupported",
            BrowseStatus::NotFound => "notFound",
            BrowseStatus::Invalid => "invalid",
            BrowseStatus::Offline => "offline",
            BrowseStatus::Failed => "failed",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BrowseReject {
    InvalidPath,
}

/// Why a save didn't publish a file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SaveError {
    /// Not a listed file, a bad name, or a folder that isn't an absolute path.
    Invalid,
    /// Larger than [`SAVE_MAX`].
    TooLarge,
    /// The destination exists; it's never replaced.
    Exists,
    /// Closed, another phone or save, or access lost.
    Cancelled,
    /// No progress for [`SAVE_IDLE`].
    Stalled,
    Failed,
}

#[derive(Clone, PartialEq, Eq)]
pub struct PhoneBrowserRow {
    pub name: String,
    pub path: String,
    pub is_dir: bool,
    pub size: u64,
    pub modified: i64,
}

#[derive(Clone, PartialEq, Eq)]
struct ListCommand {
    generation: u64,
    device: DeviceId,
    path: String,
}

/// One save: the phone file, and where it goes.
#[derive(Debug, Clone)]
struct SaveJob {
    epoch: u64,
    device: DeviceId,
    remote: String,
    dest: PathBuf,
    temp: PathBuf,
}

/// What the stream hands the file writer.
enum Feed {
    Chunk(Vec<u8>),
    /// Everything was sent: this many bytes.
    End(u64),
}

#[derive(Clone)]
struct Session {
    generation: u64,
    device: Option<DeviceId>,
    path: String,
    rows: Vec<PhoneBrowserRow>,
    status: BrowseStatus,
    truncated: bool,
    /// How the open phone was last reached; a change can change access.
    link_path: Option<ConnectionPath>,
}

impl Session {
    const fn new() -> Self {
        Self {
            generation: 0,
            device: None,
            path: String::new(),
            rows: Vec::new(),
            status: BrowseStatus::Idle,
            truncated: false,
            link_path: None,
        }
    }

    /// Bumps the generation so any older reply is stale, including when `path`
    /// is rejected locally and no RPC is started.
    fn begin_list(&mut self, device: DeviceId, path: &str) -> Result<ListCommand, BrowseReject> {
        self.generation = self.generation.saturating_add(1);
        if self.device != Some(device) {
            self.device = Some(device);
            self.rows.clear();
            self.truncated = false;
            self.link_path = None;
        }
        if !is_valid_storage_dir_path(path) {
            self.rows.clear();
            self.path.clear();
            self.truncated = false;
            self.status = BrowseStatus::Invalid;
            return Err(BrowseReject::InvalidPath);
        }
        self.path = path.to_owned();
        self.rows.clear();
        self.truncated = false;
        self.status = BrowseStatus::Loading;
        Ok(ListCommand { generation: self.generation, device, path: path.to_owned() })
    }

    fn accept_list(
        &mut self,
        command: &ListCommand,
        entries: Vec<StorageEntry>,
    ) -> Option<&[PhoneBrowserRow]> {
        if self.generation != command.generation || self.device != Some(command.device) {
            return None;
        }
        let (rows, truncated) = bound_rows(&command.path, entries, MAX_ROWS);
        self.rows = rows;
        self.truncated = truncated;
        self.status = BrowseStatus::Ready;
        Some(&self.rows)
    }

    fn fail_list(&mut self, command: &ListCommand, status: BrowseStatus) -> bool {
        if self.generation != command.generation || self.device != Some(command.device) {
            return false;
        }
        self.rows.clear();
        self.truncated = false;
        self.status = status;
        true
    }

    fn clear_names(&mut self, status: BrowseStatus) {
        self.generation = self.generation.saturating_add(1);
        self.rows.clear();
        self.truncated = false;
        self.status = status;
    }

    /// Forgets the open phone: no device, no folder, no names.
    fn forget(&mut self, status: BrowseStatus) {
        self.clear_names(status);
        self.device = None;
        self.path.clear();
        self.link_path = None;
    }
}

struct State {
    session: Session,
    inflight: Option<u64>,
    queued: Option<ListCommand>,
    save_epoch: u64,
    saving: bool,
    save_notice: Option<bool>,
}

impl State {
    const fn empty() -> Self {
        Self {
            session: Session::new(),
            inflight: None,
            queued: None,
            save_epoch: 0,
            saving: false,
            save_notice: None,
        }
    }

    /// Ends the running save, if any: it stops at its next check and removes
    /// its own temporary file. With `report`, the page hears it failed.
    fn cancel_save(&mut self, report: bool) {
        self.save_epoch = self.save_epoch.saturating_add(1);
        if self.saving && report {
            self.save_notice = Some(false);
        }
        self.saving = false;
    }

    fn save_is_current(&self, epoch: u64, device: DeviceId) -> bool {
        self.saving && self.save_epoch == epoch && self.session.device == Some(device)
    }

    /// Starts saving the listed file at `path` (not a row number, which can
    /// shift under a refresh) into `folder`.
    fn begin_save(&mut self, path: &str, folder: Option<&Path>) -> Result<SaveJob, SaveError> {
        let device = self.session.device.ok_or(SaveError::Invalid)?;
        let (name, size) = self
            .session
            .rows
            .iter()
            .find(|row| row.path == path && !row.is_dir)
            .map(|row| (row.name.clone(), row.size))
            .ok_or(SaveError::Invalid)?;
        if !is_valid_storage_path(path) {
            return Err(SaveError::Invalid);
        }
        if size > SAVE_MAX {
            return Err(SaveError::TooLarge);
        }
        let folder = folder.ok_or(SaveError::Invalid)?;
        let dest = file_in_folder(folder, &name)?;
        self.cancel_save(false);
        let epoch = self.save_epoch;
        let temp = temp_in_folder(folder, epoch, TEMP_SEQ.fetch_add(1, Ordering::Relaxed));
        self.saving = true;
        self.save_notice = None;
        Ok(SaveJob { epoch, device, remote: path.to_owned(), dest, temp })
    }

    /// The last step of a save as one decision: still wanted, then linked into
    /// place without replacing anything, then reported. Whatever ended the
    /// save first (close, another phone, lost access) keeps it unpublished.
    fn publish(
        &mut self,
        job: &SaveJob,
        link: impl FnOnce() -> std::io::Result<()>,
    ) -> Result<(), SaveError> {
        if !self.save_is_current(job.epoch, job.device) {
            return Err(SaveError::Cancelled);
        }
        match link() {
            Ok(()) => {
                self.saving = false;
                self.save_notice = Some(true);
                Ok(())
            }
            Err(error) if error.kind() == ErrorKind::AlreadyExists => Err(SaveError::Exists),
            Err(_) => Err(SaveError::Failed),
        }
    }

    /// Reports a failed save, unless something newer already took over.
    fn fail_save(&mut self, epoch: u64) {
        if self.saving && self.save_epoch == epoch {
            self.saving = false;
            self.save_notice = Some(false);
        }
    }

    fn finish_list(&mut self, command: &ListCommand, outcome: ListOutcome) -> Option<ListCommand> {
        if self.inflight == Some(command.generation) {
            self.inflight = None;
        }
        if let ListOutcome::Done(result) = outcome {
            match result {
                Ok(entries) => {
                    let _ = self.session.accept_list(command, entries);
                }
                Err(err) => {
                    let status = status_for(&err);
                    let lost = matches!(
                        status,
                        BrowseStatus::Denied | BrowseStatus::Unsupported | BrowseStatus::Offline
                    );
                    if self.session.fail_list(command, status) && lost {
                        // Access to this phone's files is gone, so is the file being saved.
                        self.cancel_save(true);
                    }
                }
            }
        }
        let queued = self.queued.take();
        let next = queued.filter(|cmd| {
            cmd.generation == self.session.generation && self.session.device == Some(cmd.device)
        });
        if let Some(cmd) = next.as_ref() {
            self.inflight = Some(cmd.generation);
        }
        next
    }
}

pub struct BrowserView {
    pub enabled: bool,
    pub device: Option<DeviceId>,
    pub path: String,
    pub status: BrowseStatus,
    pub rows: Vec<PhoneBrowserRow>,
    pub can_up: bool,
    pub saving: bool,
    pub truncated: bool,
}

enum ListOutcome {
    /// Generation or device was already stale. No `storage.list` was sent,
    /// so no pending RPC id exists.
    Skipped,
    /// `Node::storage_list` ran to completion, which removes its pending id.
    Done(Result<Vec<StorageEntry>, Error>),
}

fn lock() -> MutexGuard<'static, State> {
    STATE.lock().unwrap_or_else(|e| e.into_inner())
}

pub fn set_ui_listener(listener: impl Fn() + Send + Sync + 'static) {
    *LISTENER.lock().unwrap_or_else(|e| e.into_inner()) = Some(Arc::new(listener));
}

fn notify() {
    let listener = LISTENER.lock().unwrap_or_else(|e| e.into_inner()).clone();
    if let Some(listener) = listener {
        listener();
    }
}

pub fn view() -> BrowserView {
    if !ENABLED {
        return BrowserView {
            enabled: false,
            device: None,
            path: String::new(),
            status: BrowseStatus::Hidden,
            rows: Vec::new(),
            can_up: false,
            saving: false,
            truncated: false,
        };
    }
    let st = lock();
    BrowserView {
        enabled: true,
        device: st.session.device,
        path: st.session.path.clone(),
        status: st.session.status,
        rows: st.session.rows.clone(),
        can_up: !st.session.path.is_empty(),
        saving: st.saving,
        truncated: st.session.truncated,
    }
}

pub fn take_save_notice() -> Option<bool> {
    lock().save_notice.take()
}

pub fn open(device: DeviceId) {
    request_list(device, "");
}

pub fn up() {
    if !ENABLED {
        return;
    }
    let (device, path) = {
        let st = lock();
        let Some(device) = st.session.device else { return };
        (device, parent_path(&st.session.path))
    };
    request_list(device, &path);
}

/// Opens the listed folder at `path`.
pub fn open_folder(path: &str) {
    if !ENABLED {
        return;
    }
    let device = {
        let st = lock();
        let Some(device) = st.session.device else { return };
        if !st.session.rows.iter().any(|row| row.is_dir && row.path == path) {
            return;
        }
        device
    };
    request_list(device, path);
}

pub fn refresh() {
    if !ENABLED {
        return;
    }
    let (device, path) = {
        let st = lock();
        let Some(device) = st.session.device else { return };
        (device, st.session.path.clone())
    };
    request_list(device, &path);
}

pub fn close() {
    if !ENABLED {
        return;
    }
    {
        let mut st = lock();
        st.session.forget(BrowseStatus::Idle);
        st.queued = None;
        st.cancel_save(false);
        st.save_notice = None;
    }
    notify();
}

/// Saves the listed file at `path` into `folder` (an absolute path).
pub fn save_file(path: &str, folder: Option<PathBuf>) {
    if !ENABLED {
        return;
    }
    let job = {
        let mut st = lock();
        match st.begin_save(path, folder.as_deref()) {
            Ok(job) => Some(job),
            Err(error) => {
                tracing::info!(?error, "phone file save refused");
                st.save_notice = Some(false);
                None
            }
        }
    };
    if let Some(job) = job {
        crate::core_host::spawn(run_save(job));
    }
    notify();
}

/// Root calls this from `core_host`, beside `storage::on_event`.
pub fn on_event(event: &NodeEvent) {
    if !ENABLED {
        return;
    }
    match event {
        NodeEvent::StorageChanged { device, path } => {
            recheck_if(*device, |session| covers(path, &session.path));
        }
        // A toggle, a capability or the phone's own offer changed: access may have.
        NodeEvent::Capabilities(matrix) => recheck_if(matrix.device, |_| true),
        NodeEvent::PeerInfoChanged { device, .. } => recheck_if(*device, |_| true),
        // Relay access has its own switch; only a new path matters, not round-trip times.
        NodeEvent::LinkChanged { device, link: LinkState::Online { path, .. } } => {
            recheck_if(*device, |session| session.link_path.replace(*path).is_some_and(|old| old != *path));
        }
        NodeEvent::LinkChanged { device, link: LinkState::Offline { .. } }
        | NodeEvent::DeviceRemoved(device) => {
            session_lost(*device, BrowseStatus::Offline);
        }
        _ => {}
    }
}

/// Lists the open folder again, its names cleared until the reply, when
/// `device` is the open phone and `changed` says something that decides
/// access, or this folder, changed. Core checks access again on the way, and
/// a refusal ends a running save too.
fn recheck_if(device: DeviceId, changed: impl FnOnce(&mut Session) -> bool) {
    let path = {
        let mut st = lock();
        if st.session.device != Some(device) || !changed(&mut st.session) {
            return;
        }
        st.session.path.clone()
    };
    request_list(device, &path);
}

fn session_lost(device: DeviceId, status: BrowseStatus) {
    {
        let mut st = lock();
        if st.session.device != Some(device) {
            return;
        }
        st.session.forget(status);
        st.queued = None;
        st.cancel_save(true);
    }
    notify();
}

fn request_list(device: DeviceId, path: &str) {
    if !ENABLED {
        return;
    }
    let spawn = {
        let mut st = lock();
        if st.session.device.is_some_and(|open| open != device) {
            // Another phone: a save from the last one stops.
            st.cancel_save(true);
        }
        match st.session.begin_list(device, path) {
            Ok(cmd) => {
                if st.inflight.is_some() {
                    st.queued = Some(cmd);
                    None
                } else {
                    st.inflight = Some(cmd.generation);
                    Some(cmd)
                }
            }
            Err(BrowseReject::InvalidPath) => None,
        }
    };
    if let Some(cmd) = spawn {
        spawn_list(cmd);
    }
    notify();
}

fn spawn_list(command: ListCommand) {
    crate::core_host::spawn(async move {
        // Await any RPC that we start. Dropping `storage_list` would leave
        // its pending id in the session map; `Session::request` removes that
        // id only when its future finishes.
        let outcome = if list_still_current(&command) {
            match crate::core_host::node() {
                Some(node) => {
                    ListOutcome::Done(node.storage_list(command.device, command.path.clone()).await)
                }
                None => ListOutcome::Done(Err(Error::Offline)),
            }
        } else {
            ListOutcome::Skipped
        };
        let next = lock().finish_list(&command, outcome);
        if let Some(next) = next {
            spawn_list(next);
        }
        notify();
    });
}

fn list_still_current(command: &ListCommand) -> bool {
    let st = lock();
    st.inflight == Some(command.generation)
        && st.session.generation == command.generation
        && st.session.device == Some(command.device)
}

fn status_for(err: &Error) -> BrowseStatus {
    match err {
        Error::Denied | Error::Declined => BrowseStatus::Denied,
        Error::Unsupported => BrowseStatus::Unsupported,
        Error::NotFound => BrowseStatus::NotFound,
        Error::Offline | Error::NotPaired => BrowseStatus::Offline,
        Error::Protocol(_) => BrowseStatus::Invalid,
        _ => BrowseStatus::Failed,
    }
}

async fn run_save(job: SaveJob) {
    if let Err(error) = save(&job).await {
        if error != SaveError::Cancelled {
            tracing::info!(?error, "phone file not saved");
        }
        lock().fail_save(job.epoch);
    }
    notify();
}

/// Streams the file to a writer on a blocking thread, which publishes it.
async fn save(job: &SaveJob) -> Result<(), SaveError> {
    let (temp, dest) = (job.temp.clone(), job.dest.clone());
    let file = tokio::task::spawn_blocking(move || create_temp(&temp, &dest))
        .await
        .map_err(|_| SaveError::Failed)??;
    let (tx, rx) = mpsc::channel(WRITE_QUEUE);
    let writer = {
        let job = job.clone();
        tokio::task::spawn_blocking(move || write_and_publish(&job, file, rx))
    };
    let streamed = stream(job, &tx).await;
    // Without `Feed::End`, the writer stops and removes its temporary file.
    drop(tx);
    match writer.await.map_err(|_| SaveError::Failed)? {
        Err(SaveError::Cancelled) => streamed.and(Err(SaveError::Cancelled)),
        written => written,
    }
}

/// One explicit length, one stream, one phone-side open.
async fn stream(job: &SaveJob, tx: &mpsc::Sender<Feed>) -> Result<(), SaveError> {
    let node = crate::core_host::node().ok_or(SaveError::Failed)?;
    let open = node.storage_read_stream(job.device, job.remote.clone(), 0, Some(SAVE_MAX));
    let (meta, mut recv) = wait(job, tx, open).await?.map_err(|_| SaveError::Failed)?;
    let expected = accept_stream_meta(SAVE_MAX, meta.size, meta.length)?;
    let mut sent = 0u64;
    while sent < expected {
        let want = usize::try_from(expected - sent).map_or(READ_CHUNK, |left| left.min(READ_CHUNK));
        let Ok(Some(chunk)) = wait(job, tx, recv.read_chunk(want)).await? else {
            return Err(SaveError::Failed);
        };
        let n = chunk.len() as u64;
        if n == 0 || n > expected - sent {
            return Err(SaveError::Failed);
        }
        send(job, tx, Feed::Chunk(chunk.to_vec())).await?;
        sent += n;
    }
    send(job, tx, Feed::End(expected)).await
}

/// Awaits `future`, checking every [`SAVE_POLL`] that the save is still wanted
/// and its writer still there; fails after [`SAVE_IDLE`] without an answer.
/// The future itself is kept across checks, never dropped half-way.
async fn wait<F: Future>(job: &SaveJob, tx: &mpsc::Sender<Feed>, future: F) -> Result<F::Output, SaveError> {
    let mut future = std::pin::pin!(future);
    let mut idle = Duration::ZERO;
    loop {
        if !lock().save_is_current(job.epoch, job.device) {
            return Err(SaveError::Cancelled);
        }
        if tx.is_closed() {
            return Err(SaveError::Failed);
        }
        match tokio::time::timeout(SAVE_POLL, future.as_mut()).await {
            Ok(output) => return Ok(output),
            Err(_) => {
                idle += SAVE_POLL;
                if idle >= SAVE_IDLE {
                    return Err(SaveError::Stalled);
                }
            }
        }
    }
}

/// Hands the writer `feed` once it has room, as long as the save is wanted.
async fn send(job: &SaveJob, tx: &mpsc::Sender<Feed>, feed: Feed) -> Result<(), SaveError> {
    let permit = wait(job, tx, tx.reserve()).await?.map_err(|_| SaveError::Failed)?;
    permit.send(feed);
    Ok(())
}

/// Refuses a destination that exists (even a dangling link) before anything
/// is read, then makes the temporary file, which nothing else can own.
fn create_temp(temp: &Path, dest: &Path) -> Result<File, SaveError> {
    if dest.symlink_metadata().is_ok() {
        return Err(SaveError::Exists);
    }
    OpenOptions::new().write(true).create_new(true).open(temp).map_err(|_| SaveError::Failed)
}

fn write_and_publish(job: &SaveJob, file: File, rx: mpsc::Receiver<Feed>) -> Result<(), SaveError> {
    let result =
        fill(file, rx).and_then(|()| lock().publish(job, || std::fs::hard_link(&job.temp, &job.dest)));
    // Made with `create_new` for this save alone; a published file is its own link.
    let _ = std::fs::remove_file(&job.temp);
    result
}

fn fill(mut file: File, mut rx: mpsc::Receiver<Feed>) -> Result<(), SaveError> {
    let mut written = 0u64;
    while let Some(feed) = rx.blocking_recv() {
        match feed {
            Feed::Chunk(bytes) => {
                written = written.saturating_add(bytes.len() as u64);
                if written > SAVE_MAX || file.write_all(&bytes).is_err() {
                    return Err(SaveError::Failed);
                }
            }
            Feed::End(expected) => {
                return if written == expected && file.sync_all().is_ok() {
                    Ok(())
                } else {
                    Err(SaveError::Failed)
                };
            }
        }
    }
    Err(SaveError::Cancelled)
}

fn join_child(parent: &str, name: &str) -> Result<String, BrowseReject> {
    if !is_valid_storage_name(name) {
        return Err(BrowseReject::InvalidPath);
    }
    let path = if parent.is_empty() { name.to_owned() } else { format!("{parent}/{name}") };
    if !is_valid_storage_path(&path) {
        return Err(BrowseReject::InvalidPath);
    }
    Ok(path)
}

fn parent_path(path: &str) -> String {
    match path.rfind('/') {
        Some(index) => path[..index].to_owned(),
        None => String::new(),
    }
}

/// Whether a change to the phone folder `changed` reaches the open `current`
/// one: the same folder, or one above it (`""` is the top).
fn covers(changed: &str, current: &str) -> bool {
    changed.is_empty()
        || current == changed
        || current.strip_prefix(changed).is_some_and(|rest| rest.starts_with('/'))
}

/// Rows for a listing: valid names only, one row per name (the model keys rows
/// by path), at most `max`.
fn bound_rows(parent: &str, entries: Vec<StorageEntry>, max: usize) -> (Vec<PhoneBrowserRow>, bool) {
    let mut rows = Vec::new();
    let mut names = HashSet::new();
    let mut seen = 0usize;
    for entry in entries {
        let Some(entry) = entry.sanitized() else { continue };
        if !names.insert(entry.name.clone()) {
            tracing::debug!("the phone listed a name twice; showing it once");
            continue;
        }
        seen += 1;
        if rows.len() >= max {
            continue;
        }
        let Ok(path) = join_child(parent, &entry.name) else { continue };
        rows.push(PhoneBrowserRow {
            name: entry.name,
            path,
            is_dir: entry.is_dir,
            size: entry.size,
            modified: entry.modified,
        });
    }
    let truncated = seen > rows.len();
    (rows, truncated)
}

fn accept_stream_meta(requested: u64, file_size: u64, meta_length: u64) -> Result<u64, SaveError> {
    if requested == 0 || requested > SAVE_MAX || meta_length > requested || meta_length > SAVE_MAX {
        return Err(SaveError::TooLarge);
    }
    if file_size > SAVE_MAX {
        return Err(SaveError::TooLarge);
    }
    if meta_length != file_size.min(requested) {
        return Err(SaveError::Failed);
    }
    Ok(meta_length)
}

fn file_in_folder(folder: &Path, name: &str) -> Result<PathBuf, SaveError> {
    if !folder.is_absolute() || !is_valid_storage_name(name) {
        return Err(SaveError::Invalid);
    }
    let dest = folder.join(name);
    if dest.parent() != Some(folder) || dest.file_name().and_then(|s| s.to_str()) != Some(name) {
        return Err(SaveError::Invalid);
    }
    Ok(dest)
}

/// A short temporary name in `folder`, whatever the phone file's name length.
fn temp_in_folder(folder: &Path, epoch: u64, seq: u64) -> PathBuf {
    folder.join(format!(".waypair-save-{epoch}-{seq}.part"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn device(byte: u8) -> DeviceId {
        DeviceId([byte; 32])
    }

    fn entry(name: &str, is_dir: bool, size: u64) -> StorageEntry {
        StorageEntry { name: name.to_owned(), size, modified: 1, is_dir }
    }

    #[test]
    fn lists_a_saf_root_and_bounds_the_read() {
        let mut session = Session::new();
        let dev = device(1);
        let command = session.begin_list(dev, "").expect("root path");
        let rows = session
            .accept_list(&command, vec![entry("Notes", true, 9), entry("readme.txt", false, 12)])
            .expect("current generation");
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].path, "Notes");
        assert!(rows[0].is_dir);
        assert_eq!(rows[0].size, 0);
        assert_eq!(rows[1].path, "readme.txt");
        assert_eq!(rows[1].size, 12);
        assert_eq!(join_child("Notes", "readme.txt").unwrap(), "Notes/readme.txt");
        assert_eq!(parent_path("Notes/readme.txt"), "Notes");
        assert_eq!(parent_path("Notes"), "");

        // The read is the one bounded stream: never larger, never short.
        assert_eq!(accept_stream_meta(SAVE_MAX, 12, 12), Ok(12));
        assert_eq!(accept_stream_meta(SAVE_MAX, SAVE_MAX + 1, SAVE_MAX), Err(SaveError::TooLarge));
        assert_eq!(accept_stream_meta(SAVE_MAX, 12, 11), Err(SaveError::Failed));

        // A name listed twice keeps one row, so each row has its own path.
        let listed =
            vec![entry("a", true, 0), entry("a", false, 1), entry("b", false, 1), entry("c", false, 1)];
        let (kept, truncated) = bound_rows("", listed, 2);
        assert_eq!(kept.iter().map(|row| row.path.as_str()).collect::<Vec<_>>(), ["a", "b"]);
        assert!(kept[0].is_dir);
        assert!(truncated);

        // A save is chosen by path; its temporary name stays short however long
        // the phone's name is, and it publishes once.
        let mut state = State::empty();
        let command = state.session.begin_list(dev, "").unwrap();
        let long = "x".repeat(255);
        state.session.accept_list(&command, vec![entry("Notes", true, 0), entry(&long, false, 12)]);
        let folder = Path::new("/tmp/phone-browser");
        let job = state.begin_save(&long, Some(folder)).unwrap();
        assert_eq!(job.dest, folder.join(&long));
        assert_eq!(job.temp.parent(), Some(folder));
        assert!(job.temp.file_name().unwrap().len() < 64);
        assert!(state.saving);
        let mut links = 0;
        let linked = state.publish(&job, || {
            links += 1;
            Ok(())
        });
        assert_eq!(linked, Ok(()));
        assert_eq!(links, 1);
        assert_eq!(state.save_notice, Some(true));
        assert!(!state.saving);
    }

    #[test]
    fn stale_generation_drops_names_and_rejects_escape() {
        let mut session = Session::new();
        let dev = device(2);
        let other = device(3);
        let first = session.begin_list(dev, "").unwrap();
        assert!(session.begin_list(dev, "../secret").is_err());
        assert!(session.begin_list(dev, "/DCIM").is_err());
        assert!(join_child("", "..").is_err());
        assert!(join_child("", "a/b").is_err());
        assert!(session.accept_list(&first, vec![entry("Notes", true, 0)]).is_none());
        assert!(session.rows.is_empty());

        let current = session.begin_list(dev, "Notes").unwrap();
        session.begin_list(other, "").unwrap();
        assert!(session.accept_list(&current, vec![entry("secret.txt", false, 4)]).is_none());
        assert!(session.rows.is_empty());

        let denied = session.begin_list(other, "").unwrap();
        assert!(session.fail_list(&denied, BrowseStatus::Denied));
        assert!(session.rows.is_empty());
        assert_eq!(session.status, BrowseStatus::Denied);

        let folder = Path::new("/tmp/phone-browser");
        assert_eq!(file_in_folder(folder, "readme.txt").unwrap(), folder.join("readme.txt"));
        assert_eq!(file_in_folder(folder, ".."), Err(SaveError::Invalid));
        assert_eq!(file_in_folder(Path::new("Downloads"), "readme.txt"), Err(SaveError::Invalid));

        // Only listed files are saved, into an absolute folder.
        let mut state = State::empty();
        let command = state.session.begin_list(dev, "").unwrap();
        state.session.accept_list(&command, vec![entry("Notes", true, 0), entry("a.txt", false, 3)]);
        assert_eq!(state.begin_save("Notes", Some(folder)).err(), Some(SaveError::Invalid));
        assert_eq!(state.begin_save("gone.txt", Some(folder)).err(), Some(SaveError::Invalid));
        assert_eq!(state.begin_save("a.txt", None).err(), Some(SaveError::Invalid));
        assert!(!state.saving);

        // Closing first wins over publishing: nothing is linked.
        let job = state.begin_save("a.txt", Some(folder)).unwrap();
        state.cancel_save(false);
        assert_eq!(state.publish(&job, || panic!("linked after close")), Err(SaveError::Cancelled));

        // Losing access clears the names and ends the running save.
        let job = state.begin_save("a.txt", Some(folder)).unwrap();
        let recheck = state.session.begin_list(dev, "").unwrap();
        state.inflight = Some(recheck.generation);
        assert!(state.finish_list(&recheck, ListOutcome::Done(Err(Error::Denied))).is_none());
        assert_eq!(state.session.status, BrowseStatus::Denied);
        assert!(state.session.rows.is_empty());
        assert!(!state.save_is_current(job.epoch, job.device));
        assert_eq!(state.save_notice, Some(false));

        // A change in the open folder or above it rechecks; others don't.
        assert!(covers("", "Notes/Sub"));
        assert!(covers("Notes", "Notes/Sub"));
        assert!(covers("Notes/Sub", "Notes/Sub"));
        assert!(!covers("Note", "Notes/Sub"));
        assert!(!covers("Notes/Sub", "Notes"));
    }
}
