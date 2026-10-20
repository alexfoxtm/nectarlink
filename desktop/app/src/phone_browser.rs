// SPDX-License-Identifier: GPL-3.0-or-later
//! In-app browser for one paired phone's shared storage or SAF roots.
//!
//! Uses the existing encrypted `storage.list` and `storage.read` stream.
//! `ENABLED` stays false until Root's native acceptance. This module does not
//! advertise capabilities, change either device's `storage` toggle, or mount
//! anything in Finder.
//!
//! Nothing is listed or saved until [`open`], the page's explicit "Browse
//! shared folders" action for one phone. That consent lives only in this
//! session and ends on [`close`], another phone, a disconnect or removal, or
//! the phone refusing access; every later listing needs it, checked under the
//! same lock that starts the listing.
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

#[cfg(feature = "test-support")]
use std::sync::atomic::AtomicBool;

use nectarlink_core::{
    ConnectionPath, DeviceId, Error, LinkState, NodeEvent,
    storage::{StorageEntry, is_valid_storage_dir_path, is_valid_storage_name, is_valid_storage_path},
};
#[cfg(feature = "test-support")]
use nectarlink_core::Node;
use tokio::sync::mpsc;

/// Root sets this true only after native acceptance. While false, every
/// network and UI entry point is a no-op.
pub const ENABLED: bool = false;

/// Test-only overrides so the bounded transport fixture can drive the real
/// production routing (`open`/`on_event`/`view`) against real paired nodes
/// without touching the production gate above. Gated on the non-default
/// `test-support` feature; production builds keep `ENABLED` as the only gate.
#[cfg(feature = "test-support")]
static TEST_ENABLED: AtomicBool = AtomicBool::new(false);
#[cfg(feature = "test-support")]
static TEST_NODE: Mutex<Option<Node>> = Mutex::new(None);
#[cfg(feature = "test-support")]
static TEST_SPAWN: Mutex<Option<tokio::runtime::Handle>> = Mutex::new(None);

/// Production gate, plus the test-only enable while a fixture holds it.
fn browser_enabled() -> bool {
    if ENABLED {
        return true;
    }
    #[cfg(feature = "test-support")]
    if TEST_ENABLED.load(Ordering::Relaxed) {
        return true;
    }
    false
}

/// The node production code issues `storage.list`/`storage.read` against:
/// the core host, or the fixture's real paired node while a test holds it.
fn active_node() -> Option<Node> {
    #[cfg(feature = "test-support")]
    if let Some(node) = TEST_NODE.lock().unwrap_or_else(|e| e.into_inner()).clone() {
        return Some(node);
    }
    crate::core_host::node()
}

/// The runtime production code spawns listing/save work on: the core host,
/// or the fixture's runtime while a test holds it.
fn spawn_task(fut: impl Future<Output = ()> + Send + 'static) {
    #[cfg(feature = "test-support")]
    if let Some(handle) = TEST_SPAWN.lock().unwrap_or_else(|e| e.into_inner()).clone() {
        handle.spawn(fut);
        return;
    }
    crate::core_host::spawn(fut);
}

/// Test-only seam: hold the production gate open for the calling fixture.
/// The fixture must clear it (and the node/runtime below) when done.
#[cfg(all(test, target_os = "macos", feature = "test-support"))]
pub(crate) fn set_test_enabled(on: bool) {
    TEST_ENABLED.store(on, Ordering::Relaxed);
}

/// Test-only seam: the real paired receiver node production code lists against.
#[cfg(all(test, target_os = "macos", feature = "test-support"))]
pub(crate) fn set_test_node(node: Option<Node>) {
    *TEST_NODE.lock().unwrap_or_else(|e| e.into_inner()) = node;
}

/// Test-only seam: the fixture runtime production code spawns listing work on.
#[cfg(all(test, target_os = "macos", feature = "test-support"))]
pub(crate) fn set_test_runtime(handle: Option<tokio::runtime::Handle>) {
    *TEST_SPAWN.lock().unwrap_or_else(|e| e.into_inner()) = handle;
}

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
                        // Access to this phone's files is gone, so are the consent
                        // to browse it and the file being saved.
                        self.session.forget(status);
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

    /// The explicit "Browse shared folders" action for `device`, reached over
    /// `link_path`: the only way the session gets a phone.
    fn open(&mut self, device: DeviceId, link_path: Option<ConnectionPath>) -> Option<ListCommand> {
        if self.session.device.is_some_and(|open| open != device) {
            // Another phone: a save from the last one stops.
            self.cancel_save(true);
        }
        let cmd = self.start_list(device, "");
        self.session.link_path = link_path;
        cmd
    }

    /// The open phone is now reached over `path`. Relay access has its own
    /// switch, so moving onto Relay ends the save and names at once, like any
    /// other access change; other moves only list again.
    fn path_changed(&mut self, device: DeviceId, path: ConnectionPath) -> Option<ListCommand> {
        if self.session.device != Some(device) {
            return None;
        }
        let old = self.session.link_path.replace(path);
        if old == Some(path) {
            None
        } else if path == ConnectionPath::Relay {
            self.access_changed(device)
        } else if old.is_some() {
            self.relist(|session| Some(session.path.clone()))
        } else {
            None
        }
    }

    /// A toggle, a capability or the phone's offer changed for `device`: its
    /// save and any queued listing end now, its names stay hidden, and core
    /// admits the folder again (or refuses, which ends the consent).
    fn access_changed(&mut self, device: DeviceId) -> Option<ListCommand> {
        if self.session.device != Some(device) {
            return None;
        }
        self.queued = None;
        self.cancel_save(true);
        self.relist(|session| Some(session.path.clone()))
    }

    /// Lists `choose`'s folder on the phone already opened; nothing without one.
    fn relist(&mut self, choose: impl FnOnce(&mut Session) -> Option<String>) -> Option<ListCommand> {
        let device = self.session.device?;
        let path = choose(&mut self.session)?;
        self.start_list(device, &path)
    }

    fn start_list(&mut self, device: DeviceId, path: &str) -> Option<ListCommand> {
        let cmd = self.session.begin_list(device, path).ok()?;
        if self.inflight.is_some() {
            self.queued = Some(cmd);
            None
        } else {
            self.inflight = Some(cmd.generation);
            Some(cmd)
        }
    }

    /// Ends the consent: no phone, no names, nothing queued, no save.
    fn close(&mut self) {
        self.session.forget(BrowseStatus::Idle);
        self.queued = None;
        self.cancel_save(false);
        self.save_notice = None;
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
    if !browser_enabled() {
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

/// The user chose "Browse shared folders" for `device`.
pub fn open(device: DeviceId) {
    if !browser_enabled() {
        return;
    }
    let link_path = current_path(device);
    request_list(|st| st.open(device, link_path));
}

/// How `device` is reached now, if it's online.
fn current_path(device: DeviceId) -> Option<ConnectionPath> {
    let node = active_node()?;
    let peer = node.paired_devices().ok()?.into_iter().find(|peer| peer.id == device)?;
    match peer.link {
        LinkState::Online { path, .. } => Some(path),
        _ => None,
    }
}

pub fn up() {
    request_list(|st| st.relist(|session| Some(parent_path(&session.path))));
}

/// Opens the listed folder at `path`.
pub fn open_folder(path: &str) {
    request_list(|st| {
        st.relist(|session| {
            session.rows.iter().any(|row| row.is_dir && row.path == path).then(|| path.to_owned())
        })
    });
}

pub fn refresh() {
    request_list(|st| st.relist(|session| Some(session.path.clone())));
}

pub fn close() {
    if !browser_enabled() {
        return;
    }
    lock().close();
    notify();
}

/// Saves the listed file at `path` into `folder` (an absolute path).
pub fn save_file(path: &str, folder: Option<PathBuf>) {
    if !browser_enabled() {
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
        spawn_task(run_save(job));
    }
    notify();
}

/// Root calls this from `core_host`, beside `storage::on_event`.
pub fn on_event(event: &NodeEvent) {
    if !browser_enabled() {
        return;
    }
    match event {
        NodeEvent::StorageChanged { device, path } => {
            recheck_if(*device, |session| covers(path, &session.path));
        }
        // Emitted when the matrix or the phone's storage offer changed, or a
        // storage toggle was set; battery and round-trip times arrive as
        // other events.
        NodeEvent::Capabilities(matrix) => request_list(|st| st.access_changed(matrix.device)),
        NodeEvent::PeerInfoChanged { device, .. } => recheck_if(*device, |_| true),
        // Only a new path matters, not round-trip times.
        NodeEvent::LinkChanged { device, link: LinkState::Online { path, .. } } => {
            request_list(|st| st.path_changed(*device, *path));
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
    request_list(|st| {
        st.relist(|session| {
            (session.device == Some(device) && changed(session)).then(|| session.path.clone())
        })
    });
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

fn request_list(start: impl FnOnce(&mut State) -> Option<ListCommand>) {
    if !browser_enabled() {
        return;
    }
    let spawn = start(&mut lock());
    if let Some(cmd) = spawn {
        spawn_list(cmd);
    }
    notify();
}

fn spawn_list(command: ListCommand) {
    spawn_task(async move {
        // Await any RPC that we start. Dropping `storage_list` would leave
        // its pending id in the session map; `Session::request` removes that
        // id only when its future finishes.
        let outcome = if list_still_current(&command) {
            match active_node() {
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
    let node = active_node().ok_or(SaveError::Failed)?;
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

    #[test]
    fn browsing_needs_consent_that_close_and_lost_access_end() {
        let dev = device(4);
        let refresh = |session: &mut Session| Some(session.path.clone());
        let mut state = State::empty();
        assert!(state.relist(refresh).is_none());
        assert_eq!(state.session.device, None);

        // Closing while a listing is in flight: its reply can't bring the phone back.
        let first = state.open(dev, None).expect("explicit browse");
        state.close();
        assert!(state.finish_list(&first, ListOutcome::Done(Ok(vec![entry("a.txt", false, 3)]))).is_none());
        assert_eq!(state.session.device, None);
        assert!(state.session.rows.is_empty());
        assert!(state.relist(refresh).is_none());

        // The phone refusing access ends the consent; a recheck can't list again.
        let second = state.open(dev, None).unwrap();
        state.session.accept_list(&second, vec![entry("a.txt", false, 3)]);
        let folder = Path::new("/tmp/phone-browser");
        let job = state.begin_save("a.txt", Some(folder)).unwrap();
        let queued = state.relist(refresh);
        assert!(queued.is_none() && state.queued.is_some());
        let denied = state.queued.clone().unwrap();
        state.inflight = Some(denied.generation);
        assert!(state.finish_list(&denied, ListOutcome::Done(Err(Error::Unsupported))).is_none());
        assert_eq!(state.session.device, None);
        assert_eq!(state.session.status, BrowseStatus::Unsupported);
        assert!(state.relist(refresh).is_none());
        assert_eq!(state.publish(&job, || panic!("linked after access ended")), Err(SaveError::Cancelled));
    }

    #[test]
    fn capability_change_ends_the_save_and_queue_before_core_answers() {
        let (dev, other) = (device(5), device(6));
        let mut state = State::empty();
        let first = state.open(dev, None).unwrap();
        let listed = vec![entry("Notes", true, 0), entry("a.txt", false, 3)];
        assert!(state.finish_list(&first, ListOutcome::Done(Ok(listed))).is_none());
        let job = state.begin_save("a.txt", Some(Path::new("/tmp/phone-browser"))).unwrap();
        let inflight = state.relist(|session| Some(session.path.clone())).unwrap();
        assert!(state.relist(|_| Some("Notes".to_owned())).is_none());
        let navigation = state.queued.clone().expect("queued behind the listing");

        assert!(state.access_changed(other).is_none());
        assert!(state.save_is_current(job.epoch, job.device));

        // A listing is still in flight, so core's answer waits, but the save,
        // the queued listing and the names end now.
        assert!(state.access_changed(dev).is_none());
        assert!(!state.save_is_current(job.epoch, job.device));
        assert_eq!(state.save_notice, Some(false));
        assert!(state.session.rows.is_empty());
        let recheck = state.queued.clone().expect("recheck queued");
        assert!(recheck != navigation && recheck.generation == state.session.generation);

        let next = state.finish_list(&inflight, ListOutcome::Done(Ok(vec![entry("a.txt", false, 3)])));
        assert!(state.session.rows.is_empty());
        assert!(next.as_ref() == Some(&recheck));
        assert!(state.finish_list(&recheck, ListOutcome::Done(Err(Error::Denied))).is_none());
        assert_eq!(state.session.device, None);
    }

    #[test]
    fn moving_onto_relay_ends_the_save_before_core_answers() {
        let dev = device(7);
        let mut state = State::empty();
        let first = state.open(dev, Some(ConnectionPath::Lan)).unwrap();
        let listed = vec![entry("a.txt", false, 3)];
        assert!(state.finish_list(&first, ListOutcome::Done(Ok(listed))).is_none());
        let job = state.begin_save("a.txt", Some(Path::new("/tmp/phone-browser"))).unwrap();

        // The same path again is only a round-trip time; USB lists again.
        assert!(state.path_changed(dev, ConnectionPath::Lan).is_none());
        let usb = state.path_changed(dev, ConnectionPath::Usb).expect("listed again");
        assert!(state.save_is_current(job.epoch, job.device));

        // Relay ends the save and the names now; the recheck waits its turn.
        assert!(state.path_changed(dev, ConnectionPath::Relay).is_none());
        assert!(!state.save_is_current(job.epoch, job.device));
        assert_eq!(state.save_notice, Some(false));
        assert!(state.session.rows.is_empty());
        let recheck = state.queued.clone().expect("recheck queued");
        assert!(state.finish_list(&usb, ListOutcome::Done(Ok(vec![entry("a.txt", false, 3)]))).is_some());
        assert!(state.session.rows.is_empty());
        assert!(state.finish_list(&recheck, ListOutcome::Done(Err(Error::Denied))).is_none());
        assert_eq!(state.session.device, None);
    }
}

/// Bounded synthetic transport fixture for the browser consent gate.
///
/// Two real authenticated [`Node`]s pair over 127.0.0.1 with the
/// `test-support` loopback transport (ephemeral bind, portmapper disabled, no
/// LAN discovery, no relay, no USB/ADB). The phone is a synthetic in-memory
/// platform holding only two synthetic folders/files; it never touches a real
/// provider, an app bundle, or personal data. Every browser change below is
/// driven by genuine core events (pairing link, `Capabilities` from real
/// toggle changes, `DeviceRemoved` from a real unpair) forwarded through the
/// production [`super::on_event`], plus the explicit [`super::open`] UI
/// action. Nothing here hand-constructs a `NodeEvent` or calls the `State`
/// transition helpers directly; the one re-delivered link event is the
/// actually observed loopback event, proving same-path idempotence.
///
/// Production `ENABLED` stays `false`; the non-default `test-support` seam
/// (`set_test_enabled`/`set_test_node`/`set_test_runtime`) is held only while
/// a test owns `FIXTURE_LOCK` and is always cleared afterwards, including on
/// panic via `Drop`.
///
/// What this proves (and what stays a Root device gate) is listed in
/// `~/orca/tmp/waypair-wave33-20261020/LINK5-BROWSER.md`.
#[cfg(all(test, target_os = "macos", feature = "test-support"))]
mod browser_transport_fixture {
    use std::{
        sync::{Arc, Mutex},
        time::Duration,
    };

    use nectarlink_core::{
        DeviceId, DeviceInfo, DeviceKind, LinkState, Node, NodeConfig, NodeEvent, PlainKeyProtector,
        Platform, StorageEntry, StorageError, STORAGE_READ, STORAGE_WRITE,
    };

    /// Serializes the fixture tests: they share the global browser state and
    /// the test-only node/runtime seam.
    static FIXTURE_LOCK: Mutex<()> = Mutex::new(());

    const WAIT: Duration = Duration::from_secs(15);
    const POLL: Duration = Duration::from_millis(20);

    const README_NAME: &str = "readme.txt";
    const README_BYTES: &[u8] = b"synthetic fixture readme";
    const NOTES_DIR: &str = "SharedDocs";
    const NOTE_NAME: &str = "note.txt";
    const NOTE_BYTES: &[u8] = b"synthetic fixture note";
    const MODIFIED: i64 = 1_717_000_000_000;

    /// In-memory synthetic phone storage: two folders/files only. `SharedDocs`
    /// answers slowly so a close racing its reply exercises the production
    /// stale-generation drop over a real transport.
    #[derive(Debug)]
    struct SyntheticPhonePlatform {
        slow_ms: u64,
    }

    impl SyntheticPhonePlatform {
        fn entries(path: &str) -> Result<Vec<StorageEntry>, StorageError> {
            match path {
                "" => Ok(vec![
                    StorageEntry { name: NOTES_DIR.into(), size: 0, modified: MODIFIED, is_dir: true },
                    StorageEntry {
                        name: README_NAME.into(),
                        size: README_BYTES.len() as u64,
                        modified: MODIFIED,
                        is_dir: false,
                    },
                ]),
                NOTES_DIR => Ok(vec![StorageEntry {
                    name: NOTE_NAME.into(),
                    size: NOTE_BYTES.len() as u64,
                    modified: MODIFIED,
                    is_dir: false,
                }]),
                _ => Err(StorageError::NotFound),
            }
        }
    }

    impl Platform for SyntheticPhonePlatform {
        fn storage_list(&self, path: &str) -> Result<Vec<StorageEntry>, StorageError> {
            if path == NOTES_DIR {
                std::thread::sleep(Duration::from_millis(self.slow_ms));
            }
            Self::entries(path)
        }
    }

    fn fixture_device(name: &str, kind: DeviceKind) -> DeviceInfo {
        DeviceInfo {
            name: name.into(),
            kind,
            os: if kind == DeviceKind::Phone { "android" } else { "macos" }.into(),
            os_ver: "fixture".into(),
            model: None,
            accent: None,
            screen: None,
            install_id: None,
        }
    }

    fn fixture_node_config(
        directory: &std::path::Path,
        name: &str,
        kind: DeviceKind,
        capabilities: &[&str],
    ) -> NodeConfig {
        let mut config = NodeConfig::new(directory, fixture_device(name, kind), "0.0.1-test");
        config.away_mode = false;
        config.lan_discovery = false;
        config.localsend_port = 0;
        config.adb_server = None;
        config.usb_port = None;
        config.key_protector = Some(Arc::new(PlainKeyProtector));
        // Loopback bind with no portmapper: the paired-nodes transport. The
        // name mentions the mirror fixture that introduced the flag; no mirror
        // behavior is enabled by it.
        config.enable_loopback_mirror_fixture();
        config.capabilities = capabilities.iter().map(|cap| (*cap).to_owned()).collect();
        config
    }

    async fn wait_for_link(receiver: &Node, phone: DeviceId) {
        tokio::time::timeout(WAIT, async {
            loop {
                if receiver.paired_devices().is_ok_and(|peers| {
                    peers
                        .iter()
                        .any(|peer| peer.id == phone && matches!(&peer.link, LinkState::Online { .. }))
                }) {
                    return;
                }
                tokio::time::sleep(POLL).await;
            }
        })
        .await
        .expect("paired nodes connect over loopback within 15 seconds");
    }

    async fn wait_until(label: &str, mut cond: impl FnMut() -> bool) {
        tokio::time::timeout(WAIT, async {
            while !cond() {
                tokio::time::sleep(POLL).await;
            }
        })
        .await
        .unwrap_or_else(|_| panic!("timed out waiting for {label}"));
    }

    /// The real RPC admission check, polled: proves a toggle change reached
    /// the serving phone before the browser step that depends on it.
    async fn wait_rpc_ok(receiver: &Node, phone: DeviceId, label: &str) {
        tokio::time::timeout(WAIT, async {
            loop {
                if receiver.storage_list(phone, "").await.is_ok() {
                    return;
                }
                tokio::time::sleep(POLL).await;
            }
        })
        .await
        .unwrap_or_else(|_| panic!("timed out waiting for {label}"));
    }

    /// Clears the test-only seam and any consent. Idempotent.
    fn clear_seam() {
        super::close();
        super::set_test_node(None);
        super::set_test_runtime(None);
        super::set_test_enabled(false);
    }

    struct Paired {
        runtime: tokio::runtime::Runtime,
        receiver: Node,
        phone: Node,
        receiver_id: DeviceId,
        phone_id: DeviceId,
        pump: tokio::task::JoinHandle<()>,
        recorded: Arc<Mutex<Vec<NodeEvent>>>,
        _phone_dir: tempfile::TempDir,
        _receiver_dir: tempfile::TempDir,
    }

    impl Drop for Paired {
        fn drop(&mut self) {
            self.pump.abort();
            clear_seam();
        }
    }

    impl Paired {
        /// Starts two nodes, pairs them, installs the test-only seam, and
        /// pumps every genuine receiver event through production `on_event`.
        /// The caller must hold `FIXTURE_LOCK`.
        fn start_locked() -> Self {
            let receiver_dir = tempfile::tempdir().expect("disposable receiver node state");
            let phone_dir = tempfile::tempdir().expect("disposable phone node state");
            let runtime = tokio::runtime::Builder::new_multi_thread()
                .worker_threads(2)
                .enable_all()
                .build()
                .expect("fixture Tokio runtime");
            runtime.block_on(async {
                let phone_platform = Arc::new(SyntheticPhonePlatform { slow_ms: 300 });
                let receiver_config =
                    fixture_node_config(receiver_dir.path(), "Synthetic Mac", DeviceKind::Desktop, &[]);
                let phone_config = fixture_node_config(
                    phone_dir.path(),
                    "Synthetic Phone",
                    DeviceKind::Phone,
                    &[STORAGE_READ, STORAGE_WRITE],
                );
                let receiver = tokio::time::timeout(
                    WAIT,
                    Node::start(receiver_config, Arc::new(nectarlink_core::NoopPlatform)),
                )
                .await
                .expect("synthetic receiver starts within 15 seconds")
                .expect("synthetic receiver starts");
                let phone = tokio::time::timeout(WAIT, Node::start(phone_config, phone_platform))
                    .await
                    .expect("synthetic phone starts within 15 seconds")
                    .expect("synthetic phone starts");
                let receiver_id = receiver.device_id();
                let phone_id = phone.device_id();

                let mut events = receiver.events();
                let invitation = tokio::time::timeout(WAIT, receiver.pairing_start_qr())
                    .await
                    .expect("pairing invitation within 15 seconds")
                    .expect("pairing invitation starts");
                assert!(
                    !invitation.addrs.is_empty()
                        && invitation.addrs.iter().all(|address| address.ip().is_loopback()),
                    "loopback-bound fixture advertised a non-loopback or missing address"
                );
                let pairing_uri = invitation.to_uri();
                tokio::time::timeout(WAIT, phone.pairing_join(&pairing_uri))
                    .await
                    .expect("synthetic loopback pairing within 15 seconds")
                    .expect("synthetic loopback pairing succeeds");
                wait_for_link(&receiver, phone_id).await;
                wait_for_link(&phone, receiver_id).await;

                super::set_test_runtime(Some(runtime.handle().clone()));
                super::set_test_node(Some(receiver.clone()));
                super::set_test_enabled(true);

                let recorded: Arc<Mutex<Vec<NodeEvent>>> = Arc::new(Mutex::new(Vec::new()));
                let pump_recorded = recorded.clone();
                let pump = runtime.spawn(async move {
                    while let Ok(event) = events.recv().await {
                        pump_recorded.lock().unwrap_or_else(|e| e.into_inner()).push(event.clone());
                        super::on_event(&event);
                    }
                });
                Self {
                    runtime,
                    receiver,
                    phone,
                    receiver_id,
                    phone_id,
                    pump,
                    recorded,
                    _phone_dir: phone_dir,
                    _receiver_dir: receiver_dir,
                }
            })
        }

        fn recorded_link_online(&self) -> Option<NodeEvent> {
            self.recorded
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .iter()
                .find(|event| {
                    matches!(
                        event,
                        NodeEvent::LinkChanged { device, link: LinkState::Online { .. } }
                        if *device == self.phone_id
                    )
                })
                .cloned()
        }

        fn shutdown(&self) {
            self.pump.abort();
            self.runtime.block_on(async {
                let phone_down =
                    tokio::time::timeout(Duration::from_secs(5), self.phone.shutdown()).await.is_ok();
                let receiver_down =
                    tokio::time::timeout(Duration::from_secs(5), self.receiver.shutdown()).await.is_ok();
                clear_seam();
                assert!(phone_down, "synthetic phone node shutdown exceeded 5 seconds");
                assert!(receiver_down, "synthetic receiver node shutdown exceeded 5 seconds");
            });
        }
    }

    /// Full consent gate over a real pairing: nothing lists before the
    /// explicit open; grant lists; phone revoke and host opt-out clear through
    /// real `Capabilities` events; a late reply after close is dropped; unpair
    /// clears. Production `ENABLED` stays false throughout.
    #[test]
    fn paired_browser_consent_revoke_optout_and_close_flow() {
        let _lock = FIXTURE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let paired = Paired::start_locked();
        paired.runtime.block_on(async {
            assert!(!super::ENABLED, "production browser gate stays closed");
            let phone = paired.phone_id;

            // Link is up, but nothing lists before the explicit UI action.
            let before = super::view();
            assert!(before.enabled, "test seam opens the browser gate");
            assert_eq!(before.device, None);
            assert!(before.rows.is_empty());

            // Consent refusal before approval, over the real RPC.
            super::open(phone);
            wait_until("denied listing before grant", || {
                let view = super::view();
                view.status == super::BrowseStatus::Denied && view.rows.is_empty()
            })
            .await;
            assert_eq!(super::view().device, None, "refusal ends the consent");

            // Phone grants: the real toggle change propagates, then the
            // explicit open shows the first synthetic list.
            paired
                .phone
                .set_device_toggle(paired.receiver_id, "storage", true)
                .expect("phone grants storage");
            wait_rpc_ok(&paired.receiver, phone, "grant reaches the RPC layer").await;
            super::open(phone);
            wait_until("first list after explicit open", || {
                let view = super::view();
                view.status == super::BrowseStatus::Ready && view.rows.len() == 2
            })
            .await;
            let listed = super::view();
            assert_eq!(listed.device, Some(phone));
            assert_eq!(listed.path, "");
            assert!(!listed.can_up, "root has no parent");
            assert!(!listed.truncated);
            let names: Vec<(&str, bool)> =
                listed.rows.iter().map(|row| (row.name.as_str(), row.is_dir)).collect();
            assert_eq!(names, [(NOTES_DIR, true), (README_NAME, false)]);
            assert_eq!(listed.rows[0].path, NOTES_DIR);
            assert_eq!(listed.rows[1].path, README_NAME);
            assert_eq!(listed.rows[1].size, README_BYTES.len() as u64);

            // Phone revokes: the real `Capabilities` event clears the consent.
            paired
                .phone
                .set_device_toggle(paired.receiver_id, "storage", false)
                .expect("phone revokes storage");
            wait_until("revoke clears the browser", || {
                let view = super::view();
                view.device.is_none()
                    && view.rows.is_empty()
                    && view.status == super::BrowseStatus::Unsupported
            })
            .await;

            // Re-grant, re-open, then the host opts out of browsing that
            // phone: the real receiver-side `Capabilities` event denies and
            // clears.
            paired
                .phone
                .set_device_toggle(paired.receiver_id, "storage", true)
                .expect("phone re-grants storage");
            wait_rpc_ok(&paired.receiver, phone, "re-grant reaches the RPC layer").await;
            super::open(phone);
            wait_until("second list after explicit open", || {
                let view = super::view();
                view.status == super::BrowseStatus::Ready && view.rows.len() == 2
            })
            .await;
            paired.receiver.set_device_toggle(phone, "storage", false).expect("host opts out");
            wait_until("host opt-out clears the browser", || {
                let view = super::view();
                view.device.is_none() && view.rows.is_empty() && view.status == super::BrowseStatus::Denied
            })
            .await;

            // Re-allow on the host and open again: consent works, and the
            // observed same-path link event neither relists nor clears.
            paired.receiver.set_device_toggle(phone, "storage", true).expect("host re-allows");
            super::open(phone);
            wait_until("third list after explicit open", || {
                let view = super::view();
                view.status == super::BrowseStatus::Ready && view.rows.len() == 2
            })
            .await;
            let online = paired.recorded_link_online().expect("the real loopback link event was recorded");
            super::on_event(&online);
            tokio::time::sleep(Duration::from_millis(300)).await;
            let kept = super::view();
            assert_eq!(kept.status, super::BrowseStatus::Ready);
            assert_eq!(kept.device, Some(phone));
            assert_eq!(kept.rows.len(), 2, "same-path event keeps the rows");

            // Slow folder in flight, then close: the late real reply is stale.
            super::open_folder(NOTES_DIR);
            super::close();
            tokio::time::sleep(Duration::from_secs(1)).await;
            let closed = super::view();
            assert_eq!(closed.device, None);
            assert!(closed.rows.is_empty());
            assert_eq!(closed.status, super::BrowseStatus::Idle);

            // Device change clears: a real unpair ends with no names.
            super::open(phone);
            wait_until("list before unpair", || {
                let view = super::view();
                view.status == super::BrowseStatus::Ready && view.rows.len() == 2
            })
            .await;
            paired.receiver.unpair(phone).await.expect("receiver unpairs the phone");
            wait_until("unpair clears the browser", || {
                let view = super::view();
                view.device.is_none() && view.rows.is_empty() && view.status == super::BrowseStatus::Offline
            })
            .await;
        });
        paired.shutdown();
    }

    /// Refusal path: opening while the phone denies lists nothing, and saving
    /// an unlisted file is refused locally without creating anything.
    #[test]
    fn revoked_phone_lists_denied_and_refuses_save() {
        let _lock = FIXTURE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let paired = Paired::start_locked();
        paired.runtime.block_on(async {
            assert!(!super::ENABLED, "production browser gate stays closed");
            let phone = paired.phone_id;

            super::open(phone);
            wait_until("denied listing without grant", || {
                let view = super::view();
                view.device.is_none() && view.rows.is_empty() && view.status == super::BrowseStatus::Denied
            })
            .await;

            let dest = tempfile::tempdir().expect("disposable save folder");
            super::save_file(README_NAME, Some(dest.path().to_path_buf()));
            assert_eq!(
                super::take_save_notice(),
                Some(false),
                "an unlisted file is refused without a transfer"
            );
            assert!(
                std::fs::read_dir(dest.path()).expect("save folder reads").next().is_none(),
                "a refused save creates nothing"
            );
            super::close();
        });
        paired.shutdown();
    }
}
