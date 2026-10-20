// SPDX-License-Identifier: GPL-3.0-or-later
//! A phone's screen on this PC (docs/protocol/mirror.md): asks the phone,
//! decodes its video on a thread of its own and hands each picture to the
//! mirror window's `VideoView`. Only the newest picture is shown; when the
//! decoder falls behind or loses its place, the phone is asked for a
//! keyframe. The phone's sound, when it sends it, plays on the PC's
//! speakers.

use crate::i18n::tr;
use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex, OnceLock,
        atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicUsize, Ordering},
        mpsc::{Receiver, SyncSender, TrySendError, sync_channel},
    },
    time::{Duration, Instant},
};

use nectarlink_core::{
    DeviceId, Error, MIRROR_SCREEN, MirrorAudioConfig, MirrorConfig, MirrorSink, MirrorStart, NodeEvent,
};

use crate::{
    bridge::{
        app::{describe, show_message},
        native::ffi,
    },
    core_host,
    state::Changes,
    transfers::{ACTION_OPEN, ACTION_SHOW, TOAST_GROUP},
    win::{
        audio_out::Player,
        h264::{Decoder, to_bgrx},
        image::{Bitmap, encode_png},
        mp4::{MirrorRecorder, local_timestamp_filename, merge_sps_pps},
        toast::{self, Toast},
    },
};

/// What the PC asks for: the phone scales its screen to fit.
const OPTIONS: MirrorStart = MirrorStart {
    max_size: 1920,
    fps: 60,
    bitrate: 8_000_000,
    // A Mac doesn't offer mirror.listen: its core refuses the phone's sound.
    audio: !cfg!(target_os = "macos"),
    session: MIRROR_SCREEN,
    app: None,
    stay_awake: false,
    screen_off: false,
    watch: None,
    // Placeholder: core `mirror::start` stamps the live generation.
    request: None,
};
/// A Mac shows a phone's screen only, until that is validated: no app
/// windows, no input to the phone, no clipboard sent from a mirror window.
pub(crate) const VIEW_ONLY: bool = cfg!(target_os = "macos");
/// Packets waiting for the decoder, at most (about two seconds).
const BACKLOG: usize = 120;
/// Packets in a row that may fail to decode, each retried from a fresh
/// keyframe, before the mirroring ends.
const DECODE_FAILURES: u32 = 3;
/// How often decode times are logged.
const STATS_EVERY: Duration = Duration::from_secs(10);
/// How long an in-window status notice stays visible.
const NOTICE_DURATION: Duration = Duration::from_secs(3);

/// A mirroring on this PC: a phone and the session, 0 for its screen,
/// others for its apps in windows of their own.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Window {
    pub device: DeviceId,
    pub session: u32,
}

impl Window {
    pub fn screen(device: DeviceId) -> Window {
        Window { device, session: MIRROR_SCREEN }
    }

    /// Its name for QML and for `VideoView`'s stream: `<device>/<session>`.
    pub fn key(&self) -> String {
        format!("{}/{}", self.device, self.session)
    }

    pub fn parse(key: &str) -> Option<Window> {
        let (device, session) = key.split_once('/')?;
        Some(Window { device: device.parse().ok()?, session: session.parse().ok()? })
    }
}

/// Where a mirroring stands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Phase {
    /// Waiting for the phone (its user agreeing to share the screen, or the
    /// app starting).
    Asking,
    Showing,
    /// It ended; the reason for the window, if any.
    Ended(Option<String>),
}

/// A mirroring the PC asked for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Shown {
    pub phase: Phase,
    /// For an app window: the app's name, and its package.
    pub app: Option<String>,
    pub pkg: Option<String>,
}

/// A phone's apps that open in windows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Apps {
    Loading,
    Ready(Vec<App>),
    Failed(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct App {
    pub pkg: String,
    pub label: String,
    /// The icon, saved for QML.
    pub icon: Option<PathBuf>,
}

/// Remembered window position and size on this PC for an app window.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct WindowGeometry {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
struct SavedWindows {
    /// Keyed by `<device>/<pkg>`.
    #[serde(default)]
    geometry: HashMap<String, WindowGeometry>,
    /// Keyed by `<device>`, most recently opened `pkg` first.
    #[serde(default)]
    recent: HashMap<String, Vec<String>>,
}

impl SavedWindows {
    fn path() -> PathBuf {
        core_host::host().data_dir.join("app-windows.json")
    }

    fn load() -> Self {
        std::fs::read_to_string(Self::path())
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    fn save(&self) {
        let path = Self::path();
        if let Ok(json) = serde_json::to_string_pretty(self) {
            let tmp = path.with_extension("json.tmp");
            if std::fs::write(&tmp, json).is_ok() {
                let _ = std::fs::rename(&tmp, &path);
            }
        }
    }
}

#[derive(Debug)]
struct State {
    windows: HashMap<Window, Shown>,
    /// Most recently focused or interacted-with mirror window.
    active_window: Option<Window>,
    /// Last clipboard text synced on window focus so unchanged text isn't resent.
    last_focus_clip: Option<String>,
    /// Phones whose sound is coming in.
    sound: HashSet<DeviceId>,
    /// Phones kept awake while mirroring (their screen or app windows).
    stay_awake: HashSet<DeviceId>,
    /// Phones whose physical screen is turned off while mirroring (their
    /// screen or app windows); cleared when the last window closes.
    screen_off: HashSet<DeviceId>,
    /// Phone currently receiving PC keyboard input without screen mirroring.
    keyboard_device: Option<DeviceId>,
    /// Short transient feedback notices per mirror window: `(text, shown_at, token)`.
    notices: HashMap<Window, (String, Instant, u64)>,
    /// The video stream each window shows, by number: a window key is
    /// reused, so a decoder still finishing an older stream mustn't end or
    /// clear the one after it. Cleared when the window closes or asks again.
    streams: HashMap<Window, u64>,
    /// Exact request owner, retained until close or the next request.
    requests: HashMap<Window, u64>,
    /// Closed sessions whose STOP must precede any replacement START.
    pending_stops: HashSet<Window>,
    apps: HashMap<DeviceId, Apps>,
    saved: SavedWindows,
}

impl State {
    /// Whether `device` has a window open or opening (the screen or an app).
    fn mirroring(&self, device: &DeviceId) -> bool {
        self.windows
            .iter()
            .any(|(w, shown)| w.device == *device && matches!(shown.phase, Phase::Asking | Phase::Showing))
    }

    fn set_phase(&mut self, window: Window, phase: Phase) {
        let showing = matches!(phase, Phase::Showing);
        if showing && self.active_window.is_none() {
            self.active_window = Some(window);
        } else if !showing && self.active_window == Some(window) {
            self.active_window = self
                .windows
                .iter()
                .find(|(w, shown)| **w != window && matches!(shown.phase, Phase::Showing))
                .map(|(w, _)| *w);
        }
        if let Some(shown) = self.windows.get_mut(&window) {
            shown.phase = phase;
        }
    }

    fn owns_request(&self, window: &Window, request: u64) -> bool {
        self.requests.get(window) == Some(&request)
    }

    /// Whether `stream` is still the one `window` shows.
    fn shows(&self, window: &Window, stream: u64) -> bool {
        self.streams.get(window) == Some(&stream)
    }
}

impl Default for State {
    fn default() -> Self {
        Self {
            windows: HashMap::new(),
            active_window: None,
            last_focus_clip: None,
            sound: HashSet::new(),
            stay_awake: HashSet::new(),
            screen_off: HashSet::new(),
            keyboard_device: None,
            notices: HashMap::new(),
            streams: HashMap::new(),
            requests: HashMap::new(),
            pending_stops: HashSet::new(),
            apps: HashMap::new(),
            saved: SavedWindows::load(),
        }
    }
}

/// The user turned the phones' sound off on this PC (until turned on).
static MUTED: AtomicBool = AtomicBool::new(false);
/// The next app window's session.
static NEXT_SESSION: AtomicU32 = AtomicU32::new(1);
/// Monotonic token for auto-clearing window notices.
static NOTICE_SEQ: AtomicU64 = AtomicU64::new(1);
/// The next video stream's number (see `State::streams`).
static NEXT_STREAM: AtomicU64 = AtomicU64::new(1);
/// The next request owner, distinct even when a window closes and reopens.
static NEXT_REQUEST: AtomicU64 = AtomicU64::new(1);
// Keep stops ordered with requests reaching the phone.
static TURN: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

type FrameMap = HashMap<Window, (u32, u32, Arc<Vec<u8>>)>;

/// Latest displayed BGRX frame per window, for instant screenshots.
static FRAMES: Mutex<Option<FrameMap>> = Mutex::new(None);
/// Active MP4 recorders per window.
static RECORDERS: Mutex<Option<HashMap<Window, MirrorRecorder>>> = Mutex::new(None);
/// Fast atomic count of active recorders to skip locking RECORDERS on the 60fps packet/audio hot path.
static ACTIVE_RECORDERS: AtomicUsize = AtomicUsize::new(0);
/// Latest video stream dimensions per window.
static LAST_CONFIG: Mutex<Option<HashMap<Window, (u32, u32)>>> = Mutex::new(None);
/// Latest audio stream format `(rate, channels)` per device.
static LAST_AUDIO_CONFIG: Mutex<Option<HashMap<DeviceId, (u32, u8)>>> = Mutex::new(None);
/// Latest Annex-B SPS/PPS bytes per window so mid-stream recordings have parameter sets immediately.
static LAST_SPS_PPS: Mutex<Option<HashMap<Window, Vec<u8>>>> = Mutex::new(None);

pub fn muted() -> bool {
    MUTED.load(Ordering::Relaxed)
}

pub fn set_muted(muted: bool) {
    MUTED.store(muted, Ordering::Relaxed);
    core_host::host().hub.changed(Changes::MIRROR);
}

/// Whether `device`'s sound is coming in.
pub fn has_sound(device: &DeviceId) -> bool {
    state(|s| s.sound.contains(device))
}

fn set_sound(device: DeviceId, on: bool) {
    state(|s| if on { s.sound.insert(device) } else { s.sound.remove(&device) });
    core_host::host().hub.changed(Changes::MIRROR);
}

/// Whether `device` is set to stay awake while mirroring.
pub fn stay_awake(device: &DeviceId) -> bool {
    state(|s| s.stay_awake.contains(device))
}

/// Whether `device`'s physical screen is turned off while mirroring.
pub fn screen_off(device: &DeviceId) -> bool {
    state(|s| s.screen_off.contains(device))
}

/// Toggles keeping the phone awake while mirroring (its screen or app windows).
pub fn set_stay_awake(window: Window, on: bool) {
    let screen_off = state(|s| {
        if on {
            s.stay_awake.insert(window.device);
        } else {
            s.stay_awake.remove(&window.device);
        }
        s.screen_off.contains(&window.device)
    });
    set_notice(
        window,
        if on { "Keeping phone awake while mirroring" } else { "Phone sleep timeout restored" },
    );
    core_host::host().hub.changed(Changes::MIRROR);
    let Some(node) = core_host::node() else { return };
    core_host::spawn(async move {
        if let Err(e) = node.mirror_power(window.device, on, screen_off).await {
            tracing::debug!(error = %e, "mirror.power didn't reach the phone");
        }
    });
}

/// Toggles turning the phone's physical screen off while mirroring (its
/// screen or app windows).
pub fn set_screen_off(window: Window, on: bool) {
    let stay_awake = state(|s| {
        if on {
            s.screen_off.insert(window.device);
        } else {
            s.screen_off.remove(&window.device);
        }
        s.stay_awake.contains(&window.device)
    });
    set_notice(window, if on { "Phone screen turned off" } else { "Phone screen turned back on" });
    core_host::host().hub.changed(Changes::MIRROR);
    let Some(node) = core_host::node() else { return };
    core_host::spawn(async move {
        if let Err(e) = node.mirror_power(window.device, stay_awake, on).await {
            tracing::debug!(error = %e, "mirror.power didn't reach the phone");
        }
    });
}

/// Short status banner for `window` if shown within the last 3 seconds.
pub fn last_notice(window: &Window) -> String {
    state(|s| {
        s.notices
            .get(window)
            .filter(|(_, at, _)| at.elapsed() < NOTICE_DURATION)
            .map(|(text, _, _)| text.clone())
            .unwrap_or_default()
    })
}

fn set_notice(window: Window, text: impl Into<String>) {
    let token = NOTICE_SEQ.fetch_add(1, Ordering::Relaxed);
    state(|s| {
        s.notices.insert(window, (text.into(), Instant::now(), token));
    });
    core_host::host().hub.changed(Changes::MIRROR);
    core_host::spawn(async move {
        tokio::time::sleep(NOTICE_DURATION).await;
        let cleared = state(|s| {
            if s.notices.get(&window).is_some_and(|(_, _, t)| *t == token) {
                s.notices.remove(&window);
                true
            } else {
                false
            }
        });
        if cleared {
            core_host::host().hub.changed(Changes::MIRROR);
        }
    });
}

fn save_latest_frame(window: Window, width: u32, height: u32, bgrx: Vec<u8>) {
    FRAMES
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get_or_insert_with(HashMap::new)
        .insert(window, (width, height, Arc::new(bgrx)));
}

fn clear_window_buffers(window: &Window) {
    if let Some(frames) = FRAMES.lock().unwrap_or_else(|e| e.into_inner()).as_mut() {
        frames.remove(window);
    }
    if let Some(cfg) = LAST_CONFIG.lock().unwrap_or_else(|e| e.into_inner()).as_mut() {
        cfg.remove(window);
    }
    if let Some(sps) = LAST_SPS_PPS.lock().unwrap_or_else(|e| e.into_inner()).as_mut() {
        sps.remove(window);
    }
}

fn latest_bitmap(window: &Window) -> Option<Bitmap> {
    let (width, height, raw) = FRAMES
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .as_ref()
        .and_then(|m| m.get(window))
        .map(|(w, h, b)| (*w, *h, b.clone()))?;
    if width == 0 || height == 0 {
        return None;
    }
    let mut bgra = (*raw).clone();
    for px in bgra.as_chunks_mut::<4>().0 {
        px[3] = 0xFF;
    }
    Some(Bitmap { width, height, bgra })
}

fn free_path(dir: &Path, name: &str) -> PathBuf {
    let first = dir.join(name);
    if !first.exists() {
        return first;
    }
    let path = Path::new(name);
    let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or(name);
    let ext = path.extension().and_then(|e| e.to_str());
    for n in 2..10_000u32 {
        let candidate = match ext {
            Some(ext) => dir.join(format!("{stem} ({n}).{ext}")),
            None => dir.join(format!("{stem} ({n})")),
        };
        if !candidate.exists() {
            return candidate;
        }
    }
    first
}

/// Copies the current mirror frame for `window` to the PC clipboard as PNG.
pub fn screenshot_clipboard(window: Window) {
    let Some(bitmap) = latest_bitmap(&window) else {
        set_notice(window, "Wait for the screen to appear first");
        show_message(tr!("Mirror", "Wait for the screen to appear first."));
        return;
    };
    let copied = encode_png(&bitmap)
        .map_err(|e| e.to_string())
        .and_then(|png| crate::win::clipboard::write_image("image/png", &png));
    match copied {
        Ok(()) => {
            set_notice(window, "Screenshot copied to clipboard");
            show_message(tr!("Mirror", "Screenshot copied to clipboard."));
        }
        Err(e) => {
            tracing::warn!(error = %e, "can't copy mirror screenshot");
            set_notice(window, "Couldn't copy screenshot");
            show_message(tr!("Mirror", "The screenshot couldn't be copied to the clipboard."));
        }
    }
}

/// Saves the current mirror frame for `window` to `Downloads\Nectarlink` as a PNG file.
pub fn screenshot_file(window: Window) {
    let Some(bitmap) = latest_bitmap(&window) else {
        set_notice(window, "Wait for the screen to appear first");
        show_message(tr!("Mirror", "Wait for the screen to appear first."));
        return;
    };
    let png = match encode_png(&bitmap) {
        Ok(png) => png,
        Err(e) => {
            tracing::warn!(error = %e, "can't encode mirror screenshot");
            set_notice(window, "Couldn't save screenshot");
            show_message(tr!("Mirror", "The screenshot couldn't be saved."));
            return;
        }
    };
    let base_name = local_timestamp_filename("Screenshot", "png");
    let dir = core_host::downloads_dir();
    if let Err(e) = std::fs::create_dir_all(&dir) {
        tracing::warn!(error = %e, "can't create Downloads\\Nectarlink");
        set_notice(window, "Couldn't save screenshot");
        show_message(tr!("Mirror", "The screenshot couldn't be saved."));
        return;
    }
    let path = free_path(&dir, &base_name);
    if let Err(e) = std::fs::write(&path, &png) {
        tracing::warn!(error = %e, "can't write mirror screenshot");
        set_notice(window, "Couldn't save screenshot");
        show_message(tr!("Mirror", "The screenshot couldn't be saved."));
        return;
    }
    let file_name = path.file_name().and_then(|n| n.to_str()).unwrap_or(&base_name).to_owned();
    if let Some(node) = core_host::node() {
        node.record_timeline(
            nectarlink_core::TimelineKind::Photo,
            window.device,
            true,
            file_name.clone(),
            tr!("Mirror", "Mirror screenshot"),
            path.to_string_lossy().into_owned(),
            png.len() as u64,
            0,
            None,
        );
    }
    let device_name =
        core_host::host().hub.read(|s| s.name_of(&window.device)).unwrap_or_else(|| "your phone".into());
    toast::show(Toast {
        device: TOAST_GROUP.into(),
        key: path.to_string_lossy().into_owned(),
        title: file_name.clone(),
        body: tr!(
            "Mirror",
            "Screenshot from %1, saved in %2",
            device_name,
            crate::downloads::user_visible_location()
        ),
        attribution: "Waypair Link".into(),
        icon: None,
        image: Some(path),
        actions: vec![(ACTION_OPEN.into(), "Open".into()), (ACTION_SHOW.into(), "Show in folder".into())],
        reply: None,
        silent: false,
        progress: None,
        call: false,
    });
    set_notice(window, tr!("Mirror", "Saved %1", file_name));
    show_message(tr!("Mirror", "Saved %1 to %2.", file_name, crate::downloads::user_visible_location()));
}

/// Whether `window` is currently recording to an MP4 file.
pub fn is_recording(window: &Window) -> bool {
    ACTIVE_RECORDERS.load(Ordering::Relaxed) > 0
        && RECORDERS
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_ref()
            .is_some_and(|m| m.contains_key(window))
}

/// Unix epoch milliseconds when the active recording for `window` started (`0` if not recording).
pub fn recording_started_ms(window: &Window) -> i64 {
    if ACTIVE_RECORDERS.load(Ordering::Relaxed) == 0 {
        return 0;
    }
    RECORDERS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .as_ref()
        .and_then(|m| m.get(window))
        .map_or(0, MirrorRecorder::started_epoch_ms)
}

/// Starts or stops MP4 recording for `window`.
pub fn toggle_recording(window: Window) {
    if is_recording(&window) {
        stop_recording(window);
        return;
    }
    if !matches!(phase(&window), Some(Phase::Showing)) {
        set_notice(window, "Wait for the screen to appear first");
        show_message(tr!("Mirror", "Wait for the screen to appear first."));
        return;
    }
    let base_name = local_timestamp_filename("Mirror", "mp4");
    let dir = core_host::downloads_dir();
    if let Err(e) = std::fs::create_dir_all(&dir) {
        tracing::warn!(error = %e, "can't create Downloads\\Nectarlink");
        set_notice(window, "Couldn't start recording");
        show_message(tr!("Mirror", "Couldn't start recording."));
        return;
    }
    let path = free_path(&dir, &base_name);
    let (width, height) = LAST_CONFIG
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .as_ref()
        .and_then(|m| m.get(&window).copied())
        .or_else(|| {
            FRAMES
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .as_ref()
                .and_then(|m| m.get(&window).map(|(w, h, _)| (*w, *h)))
        })
        .unwrap_or((0, 0));

    let audio_cfg = if window.session == MIRROR_SCREEN && has_sound(&window.device) {
        LAST_AUDIO_CONFIG
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_ref()
            .and_then(|m| m.get(&window.device).copied())
            .or(Some((48_000, 2)))
    } else {
        None
    };

    let mut recorder = MirrorRecorder::new(path, width, height, OPTIONS.fps, audio_cfg);
    if let Some(sps_pps) =
        LAST_SPS_PPS.lock().unwrap_or_else(|e| e.into_inner()).as_ref().and_then(|m| m.get(&window))
    {
        recorder.set_sps_pps(sps_pps);
    }

    {
        let mut guard = RECORDERS.lock().unwrap_or_else(|e| e.into_inner());
        let map = guard.get_or_insert_with(HashMap::new);
        map.insert(window, recorder);
        ACTIVE_RECORDERS.store(map.len(), Ordering::Release);
    }

    set_notice(window, "Recording started");
    core_host::host().hub.changed(Changes::MIRROR);
    if let Some(node) = core_host::node() {
        core_host::spawn(async move { node.mirror_keyframe(window.device, window.session).await });
    }
}

/// Stops and finalizes any active MP4 recording for `window`.
pub fn stop_recording(window: Window) {
    let recorder = {
        let mut guard = RECORDERS.lock().unwrap_or_else(|e| e.into_inner());
        let rec = guard.as_mut().and_then(|m| m.remove(&window));
        ACTIVE_RECORDERS.store(guard.as_ref().map_or(0, HashMap::len), Ordering::Release);
        rec
    };
    let Some(recorder) = recorder else { return };
    finish_recording(window, recorder);
}

/// Feeds `window`'s recorder under the `RECORDERS` lock. Once the stream left
/// the recorded format or the recording reached its limit, removes that same
/// recorder under the same lock, so a newer recording started in between is
/// never the one stopped.
fn feed_recorder(window: Window, feed: impl FnOnce(&mut MirrorRecorder)) -> Option<MirrorRecorder> {
    let mut guard = RECORDERS.lock().unwrap_or_else(|e| e.into_inner());
    let map = guard.as_mut()?;
    let recorder = map.get_mut(&window)?;
    feed(recorder);
    if let Some(reason) = recorder.format_changed() {
        tracing::info!(reason, "mirror recording stopped: the phone's video format changed");
    } else if let Some(reason) = recorder.limit_reached() {
        tracing::info!(reason, "mirror recording stopped: it reached its length or size limit");
    } else {
        return None;
    }
    let recorder = map.remove(&window);
    ACTIVE_RECORDERS.store(map.len(), Ordering::Release);
    recorder
}

/// Saves a recorder already removed from `RECORDERS` and reports the result.
fn finish_recording(window: Window, recorder: MirrorRecorder) {
    core_host::host().hub.changed(Changes::MIRROR);
    let duration_secs = recorder.started_at().elapsed().as_secs().max(1);
    let limit_reached = recorder.limit_reached().is_some();

    core_host::spawn(async move {
        let res = tokio::task::spawn_blocking(move || recorder.finish()).await;
        match res {
            Ok(Ok(path)) => {
                let file_name = path.file_name().and_then(|n| n.to_str()).unwrap_or("Mirror.mp4").to_owned();
                let size_bytes = std::fs::metadata(&path).map_or(0, |m| m.len());
                if let Some(node) = core_host::node() {
                    node.record_timeline(
                        nectarlink_core::TimelineKind::Recording,
                        window.device,
                        true,
                        file_name.clone(),
                        tr!("Mirror", "Mirror recording"),
                        path.to_string_lossy().into_owned(),
                        size_bytes,
                        duration_secs,
                        None,
                    );
                }
                let device_name = core_host::host()
                    .hub
                    .read(|s| s.name_of(&window.device))
                    .unwrap_or_else(|| "your phone".into());
                toast::show(Toast {
                    device: TOAST_GROUP.into(),
                    key: path.to_string_lossy().into_owned(),
                    title: file_name.clone(),
                    body: tr!(
                        "Mirror",
                        "Recording from %1, saved in %2",
                        device_name,
                        crate::downloads::user_visible_location()
                    ),
                    attribution: "Waypair Link".into(),
                    icon: None,
                    image: None,
                    actions: vec![
                        (ACTION_OPEN.into(), "Open".into()),
                        (ACTION_SHOW.into(), tr!("Mirror", "Show in folder")),
                    ],
                    reply: None,
                    silent: false,
                    progress: None,
                    call: false,
                });
                if limit_reached {
                    set_notice(window, tr!("Mirror", "Recording limit reached. Saved %1", file_name));
                } else {
                    set_notice(window, tr!("Mirror", "Saved %1", file_name));
                }
                show_message(tr!(
                    "Mirror",
                    "Saved %1 to %2.",
                    file_name,
                    crate::downloads::user_visible_location()
                ));
            }
            Ok(Err(msg)) => {
                set_notice(window, msg.clone());
                show_message(msg);
            }
            Err(e) => {
                tracing::warn!(error = %e, "mirror recording finalize panicked");
            }
        }
    });
}

static STATE: Mutex<Option<State>> = Mutex::new(None);

fn state<T>(f: impl FnOnce(&mut State) -> T) -> T {
    f(STATE.lock().unwrap_or_else(|e| e.into_inner()).get_or_insert_default())
}

fn set_phase(window: Window, phase: Phase) {
    let showing = matches!(phase, Phase::Showing);
    state(|s| s.set_phase(window, phase));
    if showing {
        ensure_gamepad_loop();
    }
    core_host::host().hub.changed(Changes::MIRROR);
}

/// Where a mirroring stands (`None`: not asked, or closed).
pub fn phase(window: &Window) -> Option<Phase> {
    state(|s| s.windows.get(window).map(|w| w.phase.clone()))
}

/// Every mirroring the PC asked for, oldest first.
pub fn windows() -> Vec<(Window, Shown)> {
    let mut all: Vec<_> = state(|s| s.windows.iter().map(|(w, s)| (*w, s.clone())).collect());
    all.sort_by_key(|(w, _)| (w.device.to_string(), w.session));
    all
}

/// Remembered window position and size for `(device, pkg)`.
pub fn geometry(device: &DeviceId, pkg: &str) -> Option<WindowGeometry> {
    state(|s| s.saved.geometry.get(&format!("{device}/{pkg}")).copied())
}

/// Saves an app window's position and size on this PC.
pub fn save_geometry(window: Window, x: i32, y: i32, width: u32, height: u32) {
    if window.session == MIRROR_SCREEN || width < 200 || height < 200 {
        return;
    }
    state(|s| {
        let Some(pkg) = s.windows.get(&window).and_then(|w| w.pkg.clone()) else { return };
        let key = format!("{}/{pkg}", window.device);
        let geom = WindowGeometry { x, y, width, height };
        if s.saved.geometry.insert(key, geom) != Some(geom) {
            s.saved.save();
        }
    });
}

/// Package names of recent apps on `device` (apps opened from this PC first,
/// followed by apps with active or recent notifications), most recent first.
pub fn recent_apps(device: &DeviceId) -> Vec<String> {
    let saved = state(|s| s.saved.recent.get(&device.to_string()).cloned().unwrap_or_default());
    let (notif_pkgs, hist_pkgs) = core_host::host().hub.read(|s| {
        let active: Vec<String> = s
            .notifications
            .iter()
            .filter(|n| &n.device == device && !n.notification.app.is_empty())
            .map(|n| n.notification.app.clone())
            .collect();
        let history: Vec<String> = s
            .history
            .iter()
            .filter(|h| &h.device == device && !h.notification.app.is_empty())
            .map(|h| h.notification.app.clone())
            .collect();
        (active, history)
    });
    merge_recent_apps(&saved, notif_pkgs, hist_pkgs)
}

pub(crate) fn merge_recent_apps(
    saved: &[String],
    active_notif_pkgs: impl IntoIterator<Item = String>,
    history_pkgs: impl IntoIterator<Item = String>,
) -> Vec<String> {
    let mut out = Vec::with_capacity(12);
    for pkg in saved.iter().cloned().chain(active_notif_pkgs).chain(history_pkgs) {
        if !pkg.is_empty() && !out.contains(&pkg) {
            out.push(pkg);
            if out.len() >= 12 {
                break;
            }
        }
    }
    out
}

fn record_recent(s: &mut State, device: &DeviceId, pkg: &str) {
    let list = s.saved.recent.entry(device.to_string()).or_default();
    list.retain(|p| p != pkg);
    list.insert(0, pkg.to_owned());
    list.truncate(12);
    s.saved.save();
}

/// What `device` is set to while mirroring: `(stay_awake, screen_off)`.
fn power(s: &State, device: &DeviceId) -> (bool, bool) {
    (s.stay_awake.contains(device), s.screen_off.contains(device))
}

/// Asks the phone for its screen.
pub fn start(device: DeviceId) {
    // A showing screen stays: re-asking would orphan the live stream and
    // waste a consent round trip. Mac-only; Windows keeps re-ask behavior.
    if VIEW_ONLY && phase(&Window::screen(device)) == Some(Phase::Showing) {
        return;
    }
    if keyboard_device() == Some(device) {
        set_remote_keyboard(None);
    }
    let window = Window::screen(device);
    let (stay_awake, screen_off) = state(|s| {
        // The phone's screen starts on, unless app windows already turned it off.
        if !s.mirroring(&device) {
            s.screen_off.remove(&device);
        }
        s.windows.insert(window, Shown { phase: Phase::Asking, app: None, pkg: None });
        power(s, &device)
    });
    request(window, MirrorStart { stay_awake, screen_off, ..OPTIONS });
}

/// Phone currently receiving PC keyboard input without screen mirroring.
pub fn keyboard_device() -> Option<DeviceId> {
    state(|s| s.keyboard_device)
}

/// Starts or stops remote keyboard typing on `device` without screen mirroring.
pub fn set_remote_keyboard(device: Option<DeviceId>) {
    let prev = state(|s| {
        let prev = s.keyboard_device;
        s.keyboard_device = device;
        prev
    });
    if prev == device {
        return;
    }
    if let Some(old) = prev {
        input(Window::screen(old), nectarlink_core::MirrorInput::Key { key: "keyboard_off".into() });
    }
    if let Some(new) = device {
        input(Window::screen(new), nectarlink_core::MirrorInput::Key { key: "keyboard_on".into() });
    }
    core_host::host().hub.changed(Changes::MIRROR);
}

/// Toggles remote keyboard typing on `device` without screen mirroring.
pub fn toggle_remote_keyboard(device: DeviceId) {
    if keyboard_device() == Some(device) {
        set_remote_keyboard(None);
    } else {
        set_remote_keyboard(Some(device));
    }
}

/// Sends a special key (`"enter"`, `"backspace"`, `"delete"`, `"left"`, `"right"`, `"up"`, `"down"`, `"tab"`, `"back"`, `"home"`)
/// to `device` while remote keyboard mode is active.
pub fn keyboard_press(device: DeviceId, key_name: String) {
    if keyboard_device() == Some(device) {
        input(Window::screen(device), nectarlink_core::MirrorInput::Key { key: key_name });
    }
}

/// Sends typed text to `device` while remote keyboard mode is active, splitting into protocol-sized chunks.
pub fn keyboard_text(device: DeviceId, text: String) {
    if keyboard_device() != Some(device) || text.is_empty() {
        return;
    }
    let window = Window::screen(device);
    let mut piece = String::new();
    for c in text.chars() {
        if piece.len() + c.len_utf8() > nectarlink_core::MIRROR_MAX_TEXT_BYTES {
            input(window, nectarlink_core::MirrorInput::Text { text: std::mem::take(&mut piece) });
        }
        piece.push(c);
    }
    if !piece.is_empty() {
        input(window, nectarlink_core::MirrorInput::Text { text: piece });
    }
}

/// Pastes the PC's current clipboard text onto `device` while remote keyboard mode is active.
pub fn keyboard_paste(device: DeviceId) {
    if keyboard_device() != Some(device) {
        return;
    }
    if let crate::win::clipboard::Clip::Text { text, .. } = crate::win::clipboard::read_for_paste() {
        keyboard_text(device, text);
    }
}

/// Opens one of the phone's apps in a window of its own (or does nothing
/// when it's already open; reuses an ended window for that app).
pub fn start_app(device: DeviceId, pkg: String, label: String) {
    if VIEW_ONLY {
        return;
    }
    let window = state(|s| {
        if s.windows.iter().any(|(w, shown)| {
            w.device == device
                && shown.pkg.as_deref() == Some(pkg.as_str())
                && matches!(shown.phase, Phase::Asking | Phase::Showing)
        }) {
            return None;
        }
        record_recent(s, &device, &pkg);
        let ended = s
            .windows
            .iter()
            .find(|(w, shown)| w.device == device && shown.pkg.as_deref() == Some(pkg.as_str()))
            .map(|(w, _)| *w);
        let window =
            ended.unwrap_or_else(|| Window { device, session: NEXT_SESSION.fetch_add(1, Ordering::Relaxed) });
        if !s.mirroring(&device) {
            s.screen_off.remove(&device);
        }
        s.windows.insert(window, Shown { phase: Phase::Asking, app: Some(label), pkg: Some(pkg.clone()) });
        Some((window, power(s, &device)))
    });
    let Some((window, (stay_awake, screen_off))) = window else { return };
    request(
        window,
        MirrorStart {
            audio: false,
            session: window.session,
            app: Some(pkg),
            stay_awake,
            screen_off,
            ..OPTIONS
        },
    );
}

/// Reopens a mirroring window that ended (the screen or an app window).
pub fn reopen(window: Window) {
    if window.session == MIRROR_SCREEN {
        start(window.device);
        return;
    }
    if VIEW_ONLY {
        return;
    }
    let pkg = state(|s| {
        if !s.mirroring(&window.device) {
            s.screen_off.remove(&window.device);
        }
        let shown = s.windows.get_mut(&window)?;
        let pkg = shown.pkg.clone()?;
        shown.phase = Phase::Asking;
        record_recent(s, &window.device, &pkg);
        Some((pkg, power(s, &window.device)))
    });
    let Some((pkg, (stay_awake, screen_off))) = pkg else { return };
    request(
        window,
        MirrorStart {
            audio: false,
            session: window.session,
            app: Some(pkg),
            stay_awake,
            screen_off,
            ..OPTIONS
        },
    );
}

/// Resizes an app window's display on the phone (`width` and `height` are the
/// window's logical dimensions on the PC).
pub fn resize(window: Window, width: u32, height: u32) {
    if window.session == MIRROR_SCREEN || width < 120 || height < 120 {
        return;
    }
    let (mut w, mut h) = (width.saturating_mul(2), height.saturating_mul(2));
    let longest = w.max(h);
    if longest > OPTIONS.max_size {
        w = (u64::from(w) * u64::from(OPTIONS.max_size) / u64::from(longest)) as u32;
        h = (u64::from(h) * u64::from(OPTIONS.max_size) / u64::from(longest)) as u32;
    }
    let w = (w & !1).clamp(240, 4096);
    let h = (h & !1).clamp(240, 4096);
    let Some(node) = core_host::node() else { return };
    core_host::spawn(async move {
        if let Err(e) = node.mirror_resize(window.device, window.session, w, h).await {
            tracing::debug!(error = %e, "resize didn't reach the phone");
        }
    });
}

fn request(window: Window, options: MirrorStart) {
    let request = NEXT_REQUEST.fetch_add(1, Ordering::Relaxed);
    state(|s| {
        s.requests.insert(window, request);
        s.streams.remove(&window);
        clear_window_buffers(&window);
        ffi::video_clear(&window.key());
    });
    core_host::host().hub.changed(Changes::MIRROR);
    let Some(node) = core_host::node() else { return };
    let app = options.app.is_some();
    core_host::spawn(async move {
        let started = {
            let _turn = TURN.lock().await;
            if !state(|s| s.owns_request(&window, request)) {
                return;
            }
            if state(|s| s.pending_stops.remove(&window)) {
                node.mirror_stop(window.device, window.session).await;
            }
            if !state(|s| s.owns_request(&window, request)) {
                return;
            }
            node.mirror_start(window.device, options).await
        };
        if let Err(e) = started {
            let reason = match e {
                Error::Denied => tr!("Mirror", "Mirroring is turned off for this phone."),
                Error::Unsupported if app => tr!(
                    "Mirror",
                    "Set up Wireless debugging in Waypair Link on the phone to open its apps here."
                ),
                Error::Unsupported => {
                    tr!(
                        "Mirror",
                        "This phone's app doesn't share its screen yet. Update Waypair Link on the phone."
                    )
                }
                Error::Offline | Error::NotPaired => tr!("Mirror", "The phone isn't connected."),
                e => describe(&e),
            };
            let current = state(|s| {
                if !s.owns_request(&window, request) {
                    return false;
                }
                s.set_phase(window, Phase::Ended(Some(reason)));
                true
            });
            if current {
                core_host::host().hub.changed(Changes::MIRROR);
            }
            return;
        }
        // An app window has no prompt to wait for on the phone: its first frame comes within
        // seconds, or the phone couldn't start it (its stream then ends with no picture).
        if app {
            tokio::time::sleep(APP_START_TIMEOUT).await;
            let _turn = TURN.lock().await;
            let stuck = state(|s| {
                if !s.owns_request(&window, request)
                    || !s.windows.get(&window).is_some_and(|w| w.phase == Phase::Asking)
                {
                    return false;
                }
                s.set_phase(
                    window,
                    Phase::Ended(Some(tr!(
                        "Mirror",
                        "The app didn't open on the phone. Try again, or open Waypair Link on the phone to check Wireless debugging."
                    ))),
                );
                true
            });
            if stuck {
                tracing::warn!(session = window.session, "an app window never started on the phone");
                core_host::host().hub.changed(Changes::MIRROR);
                node.mirror_stop(window.device, window.session).await;
            }
        }
    });
}

/// How long an app window may take to show its first frame.
const APP_START_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(15);

/// Stops a mirroring (its window closed).
pub fn stop(window: Window) {
    stop_recording(window);
    let node = core_host::node();
    state(|s| {
        s.windows.remove(&window);
        s.notices.remove(&window);
        s.requests.remove(&window);
        if node.is_some() {
            s.pending_stops.insert(window);
        }
        s.streams.remove(&window);
        clear_window_buffers(&window);
        ffi::video_clear(&window.key());
        if !s.mirroring(&window.device) {
            s.screen_off.remove(&window.device);
        }
    });
    core_host::host().hub.changed(Changes::MIRROR);
    let Some(node) = node else { return };
    core_host::spawn(async move {
        let _turn = TURN.lock().await;
        // A replacement request may already have sent this STOP before
        // its START. Never send the same close after the replacement.
        if state(|s| s.pending_stops.remove(&window)) {
            node.mirror_stop(window.device, window.session).await;
        }
    });
}

pub fn on_event(event: &NodeEvent) {
    match event {
        NodeEvent::Mirroring { device, session, on: true } => {
            let window = Window { device: *device, session: *session };
            set_phase(window, Phase::Showing);
            if *session != MIRROR_SCREEN {
                let saved = state(|s| {
                    let pkg = s.windows.get(&window).and_then(|w| w.pkg.as_deref())?;
                    s.saved.geometry.get(&format!("{device}/{pkg}")).copied()
                });
                if let Some(geom) = saved {
                    resize(window, geom.width, geom.height);
                }
            }
        }
        // Closed here: nothing to say. Stopped on the phone: say so.
        NodeEvent::Mirroring { device, session, on: false } => {
            let window = Window { device: *device, session: *session };
            stop_recording(window);
            clear_window_buffers(&window);
            if *session == MIRROR_SCREEN {
                let cleared_kb = state(|s| {
                    if s.keyboard_device == Some(*device) {
                        s.keyboard_device = None;
                        true
                    } else {
                        false
                    }
                });
                if cleared_kb {
                    core_host::host().hub.changed(Changes::MIRROR);
                }
            }
            // An ending this PC caused keeps its reason.
            if matches!(phase(&window), Some(Phase::Asking | Phase::Showing)) {
                set_phase(window, Phase::Ended(None));
            }
            state(|s| {
                if !s.mirroring(device) {
                    s.screen_off.remove(device);
                }
            });
        }
        // A phone that went away takes its app list with it.
        NodeEvent::LinkChanged { device, link: nectarlink_core::LinkState::Offline { .. } } => {
            let cleared_kb = state(|s| {
                s.apps.remove(device);
                s.screen_off.remove(device);
                if s.keyboard_device == Some(*device) {
                    s.keyboard_device = None;
                    true
                } else {
                    false
                }
            });
            if cleared_kb {
                core_host::host().hub.changed(Changes::MIRROR);
            }
        }
        NodeEvent::DeviceRemoved(id) => {
            let cleared_kb = state(|s| {
                s.apps.remove(id);
                s.screen_off.remove(id);
                s.requests.retain(|w, _| w.device != *id);
                s.pending_stops.retain(|w| w.device != *id);
                s.streams.retain(|w, _| w.device != *id);
                if s.keyboard_device == Some(*id) {
                    s.keyboard_device = None;
                    true
                } else {
                    false
                }
            });
            if cleared_kb {
                core_host::host().hub.changed(Changes::MIRROR);
            }
        }
        _ => {}
    }
}

/// A phone's apps that open in windows, as last loaded.
pub fn apps(device: &DeviceId) -> Option<Apps> {
    state(|s| s.apps.get(device).cloned())
}

/// Loads (or reloads) a phone's apps, with their icons. A list already
/// loaded (this run, or saved by an earlier one) shows at once and stays up
/// while the phone sends a fresh one: only the first load ever waits.
pub fn load_apps(device: DeviceId) {
    let data_dir = core_host::host().data_dir.clone();
    let shown = state(|s| match s.apps.get(&device) {
        Some(Apps::Ready(_)) => true,
        _ => match saved_apps(&data_dir, &device) {
            Some(saved) => {
                s.apps.insert(device, Apps::Ready(saved));
                true
            }
            None => {
                s.apps.insert(device, Apps::Loading);
                false
            }
        },
    });
    core_host::host().hub.changed(Changes::MIRROR);
    let Some(node) = core_host::node() else { return };
    core_host::spawn(async move {
        let started = Instant::now();
        let loaded = match node.mirror_apps(device).await {
            Ok(apps) => {
                let apps = tokio::task::spawn_blocking(move || {
                    let apps: Vec<App> = apps
                        .into_iter()
                        .map(|a| App {
                            icon: a.icon.and_then(|png| crate::icons::save(&data_dir, &a.pkg, &png).ok()),
                            pkg: a.pkg,
                            label: a.label,
                        })
                        .collect();
                    save_apps(&data_dir, &device, &apps);
                    apps
                })
                .await
                .unwrap_or_default();
                tracing::info!(
                    apps = apps.len(),
                    ms = started.elapsed().as_millis() as u64,
                    "loaded a phone's apps"
                );
                Apps::Ready(apps)
            }
            // A list on screen stays: it is a moment old, not wrong.
            Err(e) if shown => {
                tracing::info!(error = %e, "can't refresh a phone's apps; keeping the last list");
                return;
            }
            Err(Error::Unsupported) => Apps::Failed(tr!(
                "Mirror",
                "Set up Wireless debugging in Waypair Link on the phone to open its apps here."
            )),
            Err(e) => Apps::Failed(describe(&e)),
        };
        state(|s| s.apps.insert(device, loaded));
        core_host::host().hub.changed(Changes::MIRROR);
    });
}

fn saved_apps_path(data_dir: &Path, device: &DeviceId) -> PathBuf {
    data_dir.join("cache").join("apps").join(format!("{device}.json"))
}

#[derive(serde::Serialize, serde::Deserialize)]
struct SavedApp {
    pkg: String,
    label: String,
}

/// The phone's apps as an earlier run saved them, with the icons it kept.
fn saved_apps(data_dir: &Path, device: &DeviceId) -> Option<Vec<App>> {
    let bytes = std::fs::read(saved_apps_path(data_dir, device)).ok()?;
    let saved: Vec<SavedApp> = serde_json::from_slice(&bytes).ok()?;
    let apps: Vec<App> = saved
        .into_iter()
        .filter(|a| nectarlink_core::is_package_name(&a.pkg))
        .map(|a| App { icon: crate::icons::existing(data_dir, &a.pkg), pkg: a.pkg, label: a.label })
        .collect();
    (!apps.is_empty()).then_some(apps)
}

fn save_apps(data_dir: &Path, device: &DeviceId, apps: &[App]) {
    let path = saved_apps_path(data_dir, device);
    let saved: Vec<SavedApp> =
        apps.iter().map(|a| SavedApp { pkg: a.pkg.clone(), label: a.label.clone() }).collect();
    let written = path.parent().map_or(Ok(()), std::fs::create_dir_all).and_then(|()| {
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_vec(&saved).map_err(std::io::Error::other)?)?;
        std::fs::rename(&tmp, &path)
    });
    if let Err(e) = written {
        tracing::warn!(error = %e, "can't save a phone's apps");
    }
}

/// Returns cached `(pkg, label)` pairs for `device` synchronously (0 ms) for
/// the Command Palette, placing recently mirrored apps first and triggering a
/// background app list load if not yet cached.
pub fn palette_apps(device: DeviceId) -> Vec<(String, String)> {
    let recent = recent_apps(&device);
    let current = apps(&device);
    let (app_names, can_load) = core_host::host().hub.read(|s| {
        let online = s
            .devices
            .iter()
            .any(|d| d.id == device && matches!(d.link, nectarlink_core::LinkState::Online { .. }));
        let avail = s.matrices.get(&device).and_then(|m| m.state("mirroring.app_windows"))
            == Some(nectarlink_core::FeatureState::Available);
        (s.app_names.clone(), online && avail)
    });

    if current.is_none() && can_load {
        load_apps(device);
    }

    let ready_list = match current {
        Some(Apps::Ready(list)) => list,
        _ => Vec::new(),
    };

    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();

    for pkg in &recent {
        let pkg_trim = pkg.trim();
        if pkg_trim.is_empty() {
            continue;
        }
        let label = ready_list
            .iter()
            .find(|a| a.pkg == pkg_trim)
            .map(|a| a.label.clone())
            .or_else(|| app_names.get(pkg_trim).cloned())
            .unwrap_or_else(|| pkg_trim.rsplit('.').next().unwrap_or(pkg_trim).to_owned());
        if !label.trim().is_empty() && seen.insert(pkg_trim.to_owned()) {
            out.push((pkg_trim.to_owned(), label));
        }
    }

    for a in ready_list {
        let pkg_trim = a.pkg.trim();
        let label_trim = a.label.trim();
        if !pkg_trim.is_empty() && !label_trim.is_empty() && seen.insert(pkg_trim.to_owned()) {
            out.push((pkg_trim.to_owned(), label_trim.to_owned()));
        }
    }

    for (pkg, label) in app_names {
        let pkg_trim = pkg.trim();
        let label_trim = label.trim();
        if !pkg_trim.is_empty() && !label_trim.is_empty() && seen.insert(pkg_trim.to_owned()) {
            out.push((pkg_trim.to_owned(), label_trim.to_owned()));
        }
    }

    out
}

/// The PC's mouse, keyboard, and gamepad on a mirrored screen or app window,
/// sent in order.
pub fn input(window: Window, input: nectarlink_core::MirrorInput) {
    if VIEW_ONLY {
        return;
    }
    static QUEUE: std::sync::OnceLock<
        tokio::sync::mpsc::UnboundedSender<(Window, nectarlink_core::MirrorInput)>,
    > = std::sync::OnceLock::new();
    let queue = QUEUE.get_or_init(|| {
        let (queue, mut inputs) =
            tokio::sync::mpsc::unbounded_channel::<(Window, nectarlink_core::MirrorInput)>();
        core_host::spawn(async move {
            while let Some((window, input)) = inputs.recv().await {
                if let Some(node) = core_host::node()
                    && let Err(e) = node.mirror_input(window.device, window.session, input).await
                {
                    tracing::debug!(error = %e, "input didn't reach the phone");
                }
            }
        });
        queue
    });
    let _ = queue.send((window, input));
}

/// Resolves the currently active or showing mirror window.
fn active_or_showing_window() -> Option<Window> {
    state(|s| {
        if let Some(active) = s.active_window
            && s.windows.get(&active).is_some_and(|w| matches!(w.phase, Phase::Showing))
        {
            return Some(active);
        }
        s.windows
            .iter()
            .find(|(_, shown)| matches!(shown.phase, Phase::Showing))
            .or_else(|| s.windows.iter().next())
            .map(|(w, _)| *w)
    })
}

/// Parses dropped `file://` URLs or local paths, optionally prefixed by a `<device>/<session>` window key.
pub(crate) fn parse_dropped_file_urls(raw_urls: &[String]) -> (Option<Window>, Vec<PathBuf>) {
    let mut explicit_window: Option<Window> = None;
    let mut paths: Vec<PathBuf> = Vec::new();
    for (idx, raw) in raw_urls.iter().enumerate() {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            continue;
        }
        if idx == 0
            && !trimmed.starts_with("file:")
            && !Path::new(trimmed).exists()
            && let Some(win) = Window::parse(trimmed)
        {
            explicit_window = Some(win);
            continue;
        }
        let local = cxx_qt_lib::QUrl::from(&cxx_qt_lib::QString::from(trimmed))
            .to_local_file()
            .map(|p| String::from(&p))
            .filter(|p| !p.is_empty())
            .or_else(|| {
                trimmed
                    .strip_prefix("file:///")
                    .or_else(|| trimmed.strip_prefix("file://"))
                    .map(|s| s.replace('/', "\\"))
            });
        let pb = PathBuf::from(local.as_deref().unwrap_or(trimmed));
        if pb.exists() {
            paths.push(pb);
        }
    }
    (explicit_window, paths)
}

/// Sends files dropped onto a mirror window to the mirrored phone and shows an
/// in-window toast notice.
pub fn drop_files(raw_urls: &[String]) {
    let (explicit_window, paths) = parse_dropped_file_urls(raw_urls);
    if paths.is_empty() {
        return;
    }
    let Some(window) = explicit_window.or_else(active_or_showing_window) else {
        return;
    };
    let notice = if paths.len() == 1 {
        let name = paths[0].file_name().and_then(|n| n.to_str()).unwrap_or("file");
        tr!("Mirror", "Sending %1…", name)
    } else {
        tr!("Mirror", "Sending %1 files…", paths.len())
    };
    set_notice(window, notice);
    crate::transfers::send(window.device, paths);
}

/// Requests remote input control from the phone for `key` and shows a guidance banner.
pub fn request_control(key: &str) {
    if VIEW_ONLY {
        return;
    }
    let Some(window) = Window::parse(key) else { return };
    set_notice(window, "On your phone, tap Allow / turn on Control from your PC in Waypair Link");
    input(
        window,
        nectarlink_core::MirrorInput::Touch { action: nectarlink_core::TouchAction::Up, x: 0.5, y: 0.5 },
    );
}

/// Notifies PC-to-phone cursor/keyboard Flow when the mouse enters `key`.
pub fn enter_window(key: &str) {
    if let Some(win) = Window::parse(key) {
        state(|s| {
            s.active_window = Some(win);
        });
        if !VIEW_ONLY {
            crate::input_share::enter_remote(win.device);
        }
    }
}

/// Notifies PC-to-phone cursor/keyboard Flow when the mouse leaves a mirror window.
pub fn leave_window() {
    let _ = crate::input_share::leave_remote();
}

/// Records text that arrived from a remote device so focus/hover sync never echoes it back.
#[allow(dead_code)]
pub fn note_remote_clipboard(text: &str) {
    let normalized = text.replace("\r\n", "\n");
    state(|s| {
        s.last_focus_clip = Some(normalized);
    });
}

/// Synchronizes the clipboard from PC to `device` only if the clipboard contains new text.
pub fn sync_clipboard_for_device(device: DeviceId) {
    let Some(node) = core_host::node() else { return };
    let can_send = core_host::host().hub.read(|s| {
        s.matrices.get(&device).and_then(|m| m.state("clipboard.pc_to_phone"))
            == Some(nectarlink_core::FeatureState::Available)
    });
    if !can_send {
        return;
    }

    if let crate::win::clipboard::Clip::Text { text, html, rtf } = crate::win::clipboard::read()
        && !text.is_empty()
    {
        let normalized = text.replace("\r\n", "\n");
        let changed = state(|s| {
            if s.last_focus_clip.as_deref() == Some(normalized.as_str()) {
                false
            } else {
                s.last_focus_clip = Some(normalized);
                true
            }
        });
        if changed {
            core_host::spawn(async move {
                let _ = node.send_clipboard_rich(device, text, html, rtf).await;
            });
        }
    }
}

/// Sends the PC's clipboard to the phone when `window` gains focus, if
/// "Clipboard follows control" is on.
pub fn sync_clipboard_on_focus(window: Window) {
    state(|s| {
        s.active_window = Some(window);
    });
    if !VIEW_ONLY && crate::settings::flow_follow_clipboard() {
        sync_clipboard_for_device(window.device);
    }
}

/// Standard XInput / Xbox left thumbstick deadzone.
#[allow(dead_code)]
pub const LEFT_THUMB_DEADZONE: i16 = 7849;
/// Standard XInput / Xbox right thumbstick deadzone.
#[allow(dead_code)]
pub const RIGHT_THUMB_DEADZONE: i16 = 8689;
/// Standard XInput / Xbox trigger threshold.
#[allow(dead_code)]
pub const TRIGGER_THRESHOLD: u8 = 30;

/// Normalized controller snapshot forwarded via [`nectarlink_core::MirrorInput::Gamepad`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct GamepadState {
    pub buttons: u16,
    pub lx: i16,
    pub ly: i16,
    pub rx: i16,
    pub ry: i16,
    pub lt: u8,
    pub rt: u8,
}

impl GamepadState {
    /// Converts this controller snapshot into a [`nectarlink_core::MirrorInput::Gamepad`] message.
    pub fn to_mirror_input(self) -> nectarlink_core::MirrorInput {
        nectarlink_core::MirrorInput::Gamepad {
            buttons: self.buttons,
            lx: self.lx,
            ly: self.ly,
            rx: self.rx,
            ry: self.ry,
            lt: self.lt,
            rt: self.rt,
        }
    }
}

#[allow(dead_code)]
fn deadzone_axis(val: i16, deadzone: i16) -> i16 {
    if val.saturating_abs() <= deadzone { 0 } else { val }
}

/// Applies stick deadzones and trigger thresholds so idle controller noise does
/// not generate redundant [`nectarlink_core::MirrorInput::Gamepad`] frames.
#[allow(dead_code)]
pub fn normalize_gamepad_state(raw: GamepadState) -> GamepadState {
    GamepadState {
        buttons: raw.buttons,
        lx: deadzone_axis(raw.lx, LEFT_THUMB_DEADZONE),
        ly: deadzone_axis(raw.ly, LEFT_THUMB_DEADZONE),
        rx: deadzone_axis(raw.rx, RIGHT_THUMB_DEADZONE),
        ry: deadzone_axis(raw.ry, RIGHT_THUMB_DEADZONE),
        lt: if raw.lt <= TRIGGER_THRESHOLD { 0 } else { raw.lt },
        rt: if raw.rt <= TRIGGER_THRESHOLD { 0 } else { raw.rt },
    }
}

#[cfg(windows)]
#[allow(unsafe_code)]
fn poll_xinput_gamepad() -> Option<GamepadState> {
    use windows::Win32::UI::Input::XboxController::{XINPUT_STATE, XInputGetState};
    for user_index in 0..4u32 {
        let mut xstate = XINPUT_STATE::default();
        // SAFETY: xstate is a valid stack-allocated XINPUT_STATE for slot 0..3.
        if unsafe { XInputGetState(user_index, &mut xstate) } == 0 {
            let gp = xstate.Gamepad;
            return Some(normalize_gamepad_state(GamepadState {
                buttons: gp.wButtons.0,
                lx: gp.sThumbLX,
                ly: gp.sThumbLY,
                rx: gp.sThumbRX,
                ry: gp.sThumbRY,
                lt: gp.bLeftTrigger,
                rt: gp.bRightTrigger,
            }));
        }
    }
    None
}

#[cfg(windows)]
fn poll_winrt_gamepad() -> Option<GamepadState> {
    use windows::Gaming::Input::{Gamepad, GamepadButtons};
    let gamepads = Gamepad::Gamepads().ok()?;
    if gamepads.Size().ok()? == 0 {
        return None;
    }
    let pad = gamepads.GetAt(0).ok()?;
    let reading = pad.GetCurrentReading().ok()?;
    let b = reading.Buttons;
    let mut buttons = 0u16;
    if (b & GamepadButtons::DPadUp) != GamepadButtons::None {
        buttons |= 0x0001;
    }
    if (b & GamepadButtons::DPadDown) != GamepadButtons::None {
        buttons |= 0x0002;
    }
    if (b & GamepadButtons::DPadLeft) != GamepadButtons::None {
        buttons |= 0x0004;
    }
    if (b & GamepadButtons::DPadRight) != GamepadButtons::None {
        buttons |= 0x0008;
    }
    if (b & GamepadButtons::Menu) != GamepadButtons::None {
        buttons |= 0x0010;
    }
    if (b & GamepadButtons::View) != GamepadButtons::None {
        buttons |= 0x0020;
    }
    if (b & GamepadButtons::LeftThumbstick) != GamepadButtons::None {
        buttons |= 0x0040;
    }
    if (b & GamepadButtons::RightThumbstick) != GamepadButtons::None {
        buttons |= 0x0080;
    }
    if (b & GamepadButtons::LeftShoulder) != GamepadButtons::None {
        buttons |= 0x0100;
    }
    if (b & GamepadButtons::RightShoulder) != GamepadButtons::None {
        buttons |= 0x0200;
    }
    if (b & GamepadButtons::A) != GamepadButtons::None {
        buttons |= 0x1000;
    }
    if (b & GamepadButtons::B) != GamepadButtons::None {
        buttons |= 0x2000;
    }
    if (b & GamepadButtons::X) != GamepadButtons::None {
        buttons |= 0x4000;
    }
    if (b & GamepadButtons::Y) != GamepadButtons::None {
        buttons |= 0x8000;
    }
    let scale_axis = |v: f64| -> i16 { (v.clamp(-1.0, 1.0) * f64::from(i16::MAX)).round() as i16 };
    let scale_trigger = |v: f64| -> u8 { (v.clamp(0.0, 1.0) * 255.0).round() as u8 };
    Some(normalize_gamepad_state(GamepadState {
        buttons,
        lx: scale_axis(reading.LeftThumbstickX),
        ly: scale_axis(reading.LeftThumbstickY),
        rx: scale_axis(reading.RightThumbstickX),
        ry: scale_axis(reading.RightThumbstickY),
        lt: scale_trigger(reading.LeftTrigger),
        rt: scale_trigger(reading.RightTrigger),
    }))
}

#[cfg(windows)]
fn poll_gamepad() -> Option<GamepadState> {
    poll_xinput_gamepad().or_else(poll_winrt_gamepad)
}

#[cfg(not(windows))]
fn poll_gamepad() -> Option<GamepadState> {
    None
}

fn ensure_gamepad_loop() {
    static RUNNING: AtomicBool = AtomicBool::new(false);
    if RUNNING.swap(true, Ordering::AcqRel) {
        return;
    }
    core_host::spawn(async move {
        let mut last_state = GamepadState::default();
        loop {
            let target = active_or_showing_window().filter(|w| matches!(phase(w), Some(Phase::Showing)));
            let Some(window) = target else {
                RUNNING.store(false, Ordering::Release);
                break;
            };
            match poll_gamepad() {
                Some(current) => {
                    if current != last_state {
                        last_state = current;
                        input(window, current.to_mirror_input());
                    }
                    tokio::time::sleep(Duration::from_millis(16)).await;
                }
                None => {
                    if last_state != GamepadState::default() {
                        last_state = GamepadState::default();
                        input(window, last_state.to_mirror_input());
                    }
                    tokio::time::sleep(Duration::from_millis(250)).await;
                }
            }
        }
    });
}

/// Where the core puts a phone's video (or the screen's sound, from a
/// stream of its own): only for mirrorings this PC asked for.
pub fn sink(device: &DeviceId, session: u32) -> Option<Arc<dyn MirrorSink>> {
    let window = Window { device: *device, session };
    let wanted = !VIEW_ONLY || session == MIRROR_SCREEN;
    state(|s| {
        let request = *s.requests.get(&window)?;
        (wanted && s.windows.get(&window).is_some_and(|w| matches!(w.phase, Phase::Asking | Phase::Showing)))
            .then(|| Arc::new(Sink::new(window, request)) as Arc<dyn MirrorSink>)
    })
}

enum Item {
    Config(MirrorConfig),
    Packet { keyframe: bool, time_us: u64, data: Vec<u8> },
    Ended,
}

/// Hands the core's packets to the decoder thread, or its sound to the
/// player; each starts with its stream's first packet (the format).
struct Sink {
    window: Window,
    request: u64,
    video: OnceLock<Video>,
    sound: Mutex<Option<Player>>,
}

/// The decoder thread's end of a sink.
struct Video {
    queue: SyncSender<Item>,
    /// Packets waiting for the decoder.
    waiting: Arc<AtomicUsize>,
    /// A packet was dropped: the frames after it are left out until a
    /// keyframe gets in, so none decodes against the missing one.
    skipping: AtomicBool,
}

impl Sink {
    fn new(window: Window, request: u64) -> Sink {
        Sink { window, request, video: OnceLock::new(), sound: Mutex::new(None) }
    }
}

impl Video {
    fn start(window: Window, request: u64) -> Video {
        let (queue, items) = sync_channel(BACKLOG);
        let waiting = Arc::new(AtomicUsize::new(0));
        let counter = waiting.clone();
        let stream = NEXT_STREAM.fetch_add(1, Ordering::Relaxed);
        state(|s| {
            if s.owns_request(&window, request) {
                s.streams.insert(window, stream);
            }
        });
        let started = std::thread::Builder::new().name("mirror-decoder".into()).spawn(move || {
            decode(window, stream, &items, &counter);
            #[cfg(test)]
            FINISHED_DECODERS
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .get_or_insert_with(HashSet::new)
                .insert(stream);
        });
        if let Err(e) = started {
            tracing::error!(error = %e, "can't start the video decoder");
        }
        Video { queue, waiting, skipping: AtomicBool::new(false) }
    }
}

impl Sink {
    fn keyframe_please(&self) {
        let window = self.window;
        if let Some(node) = core_host::node() {
            core_host::spawn(async move { node.mirror_keyframe(window.device, window.session).await });
        }
    }
}

impl MirrorSink for Sink {
    fn config(&self, config: MirrorConfig) {
        LAST_CONFIG
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get_or_insert_with(HashMap::new)
            .insert(self.window, (config.width, config.height));
        if let Some(ended) = feed_recorder(self.window, |rec| rec.on_config(config.width, config.height)) {
            finish_recording(self.window, ended);
        }
        let video = self.video.get_or_init(|| Video::start(self.window, self.request));
        // The format matters: wait for room rather than drop it.
        let _ = video.queue.send(Item::Config(config));
    }

    fn packet(&self, keyframe: bool, time_us: u64, data: Vec<u8>) {
        if keyframe {
            let mut sps_map = LAST_SPS_PPS.lock().unwrap_or_else(|e| e.into_inner());
            let entry = sps_map.get_or_insert_with(HashMap::new).entry(self.window).or_default();
            merge_sps_pps(entry, &data);
        }
        if ACTIVE_RECORDERS.load(Ordering::Relaxed) > 0
            && let Some(ended) = feed_recorder(self.window, |rec| {
                if let Err(e) = rec.on_packet(keyframe, time_us, &data) {
                    tracing::warn!(error = %e, "mirror recording packet failed");
                }
            })
        {
            finish_recording(self.window, ended);
        }
        let Some(video) = self.video.get() else { return };
        if !keyframe && video.skipping.load(Ordering::Acquire) {
            return;
        }
        match video.queue.try_send(Item::Packet { keyframe, time_us, data }) {
            Ok(()) => {
                video.skipping.store(false, Ordering::Release);
                video.waiting.fetch_add(1, Ordering::AcqRel);
            }
            Err(TrySendError::Disconnected(_)) => {}
            // The decoder is behind; it starts over at the next keyframe.
            Err(TrySendError::Full(_)) => {
                video.skipping.store(true, Ordering::Release);
                self.keyframe_please();
            }
        }
    }

    fn ended(&self) {
        stop_recording(self.window);
        if let Some(video) = self.video.get() {
            let _ = video.queue.send(Item::Ended);
        }
    }

    fn audio_config(&self, config: MirrorAudioConfig) {
        LAST_AUDIO_CONFIG
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get_or_insert_with(HashMap::new)
            .insert(self.window.device, (config.rate, config.channels));
        if self.window.session == MIRROR_SCREEN
            && ACTIVE_RECORDERS.load(Ordering::Relaxed) > 0
            && let Some(rec) = RECORDERS
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .as_mut()
                .and_then(|m| m.get_mut(&self.window))
        {
            rec.on_audio_config(config.rate, config.channels);
        }
        let mut sound = self.sound.lock().unwrap_or_else(|e| e.into_inner());
        // A new format: a new player (the old one ends when dropped).
        *sound = match Player::start(config.rate, u16::from(config.channels)) {
            Ok(player) => Some(player),
            Err(e) => {
                tracing::warn!(error = %e, "can't play the phone's sound");
                None
            }
        };
        set_sound(self.window.device, sound.is_some());
    }

    fn audio(&self, time_us: u64, data: Vec<u8>) {
        if self.window.session == MIRROR_SCREEN
            && ACTIVE_RECORDERS.load(Ordering::Relaxed) > 0
            && let Some(rec) = RECORDERS
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .as_mut()
                .and_then(|m| m.get_mut(&self.window))
            && let Err(e) = rec.on_audio(time_us, &data)
        {
            tracing::debug!(error = %e, "mirror recording audio packet failed");
        }
        if muted() {
            return;
        }
        if let Some(player) = self.sound.lock().unwrap_or_else(|e| e.into_inner()).as_ref() {
            player.push(data);
        }
    }

    fn audio_ended(&self) {
        self.sound.lock().unwrap_or_else(|e| e.into_inner()).take();
        set_sound(self.window.device, false);
    }
}

/// Ends a mirroring this PC can't show, and stops the phone sending it;
/// nothing once `window` has closed or asked again (`id` is then old).
fn give_up(window: Window, id: u64, reason: String) {
    let current = state(|s| {
        let current = s.shows(&window, id);
        if current {
            s.set_phase(window, Phase::Ended(Some(reason)));
        }
        current
    });
    if !current {
        tracing::debug!(session = window.session, "a replaced video stream failed; ignored");
        return;
    }
    core_host::host().hub.changed(Changes::MIRROR);
    let Some(node) = core_host::node() else { return };
    core_host::spawn(async move {
        let _turn = TURN.lock().await;
        if state(|s| s.shows(&window, id)) {
            node.mirror_stop(window.device, window.session).await;
        }
    });
}

#[cfg(all(test, target_os = "macos"))]
static FINISHED_DECODERS: Mutex<Option<HashSet<u64>>> = Mutex::new(None);

/// The decoder thread: decodes everything (later pictures depend on
/// earlier ones), shows the newest picture of each batch.
fn decode(window: Window, id: u64, items: &Receiver<Item>, waiting: &AtomicUsize) {
    let stream = window.key();
    // On the GPU when the PC has one (falling back to software on its own),
    // so big phone screens and several app windows stay smooth.
    let mut decoder = match Decoder::new_gpu("h264") {
        Ok(decoder) => decoder,
        Err(e) => {
            tracing::error!(error = %e, "no H.264 decoder");
            give_up(window, id, tr!("Mirror", "This PC can't decode the phone's video."));
            return;
        }
    };
    tracing::info!(gpu = decoder.on_gpu(), codec = decoder.codec(), "mirror decoder ready");
    let keyframe_please = || {
        if let Some(node) = core_host::node() {
            core_host::spawn(async move { node.mirror_keyframe(window.device, window.session).await });
        }
    };
    let mut size = (0, 0);
    // After a gap, pictures are garbage until the next keyframe.
    let mut need_keyframe = true;
    let mut failures = 0;
    let mut stats = Stats::new();
    while let Ok(item) = items.recv() {
        if !state(|s| s.shows(&window, id)) {
            break;
        }
        match item {
            Item::Config(config) => {
                if decoder.codec() != config.codec {
                    match Decoder::new_gpu(&config.codec) {
                        Ok(next) => {
                            decoder = next;
                            tracing::info!(
                                gpu = decoder.on_gpu(),
                                codec = decoder.codec(),
                                "mirror decoder ready"
                            );
                        }
                        Err(e) => {
                            tracing::error!(error = %e, codec = %config.codec, "no decoder for codec");
                            give_up(
                                window,
                                id,
                                format!("This PC can't decode {} video.", config.codec.to_uppercase()),
                            );
                            return;
                        }
                    }
                } else if size != (0, 0) && size != (config.width, config.height) {
                    decoder.flush();
                }
                size = (config.width, config.height);
                need_keyframe = true;
            }
            Item::Packet { keyframe, time_us, data } => {
                let behind = waiting.fetch_sub(1, Ordering::AcqRel) > 1;
                if need_keyframe && !keyframe {
                    continue;
                }
                need_keyframe = false;
                let started = Instant::now();
                let pictures = match decoder.decode(&data, time_us) {
                    Ok(pictures) => {
                        failures = 0;
                        pictures
                    }
                    Err(e) => {
                        failures += 1;
                        if failures >= DECODE_FAILURES {
                            tracing::warn!(error = %e, failures, "the phone's video keeps failing to decode");
                            give_up(window, id, tr!("Mirror", "This PC can't decode the phone's video."));
                            break;
                        }
                        tracing::debug!(error = %e, "a video packet didn't decode");
                        decoder.flush();
                        need_keyframe = true;
                        keyframe_please();
                        continue;
                    }
                };
                let decoded = started.elapsed();
                // Behind: skip showing this one, the next is already here.
                if let (Some(picture), false) = (pictures.last(), behind) {
                    let converted = Instant::now();
                    let (w, h) = if size.0 > 0 { size } else { (picture.width, picture.height) };
                    let (src_w, src_h) = (w.min(picture.width), h.min(picture.height));
                    let bgrx = to_bgrx(picture, w, h);
                    let (width, height, pixels) = if (src_w, src_h) == (w, h) || src_w == 0 || src_h == 0 {
                        (src_w, src_h, bgrx)
                    } else {
                        (w, h, scale_bgrx(&bgrx, src_w, src_h, w, h))
                    };
                    // Keep ownership through publication: close/reopen cannot
                    // clear the window and then receive this older frame.
                    state(|s| {
                        if s.shows(&window, id) {
                            ffi::video_frame(&stream, width, height, &pixels);
                            save_latest_frame(window, width, height, pixels);
                        }
                    });
                    stats.shown(decoded, converted.elapsed());
                } else {
                    stats.skipped();
                }
            }
            Item::Ended => break,
        }
    }
    state(|s| {
        if s.shows(&window, id) {
            clear_window_buffers(&window);
            ffi::video_clear(&stream);
        }
    });
}

fn scale_bgrx(src: &[u8], src_w: u32, src_h: u32, dst_w: u32, dst_h: u32) -> Vec<u8> {
    let mut dst = vec![0u8; (dst_w * dst_h * 4) as usize];
    for y in 0..dst_h {
        let sy = ((u64::from(y) * u64::from(src_h)) / u64::from(dst_h)) as usize;
        let src_row = &src[sy * (src_w as usize * 4)..(sy + 1) * (src_w as usize * 4)];
        let dst_row = &mut dst[(y as usize) * (dst_w as usize * 4)..(y as usize + 1) * (dst_w as usize * 4)];
        for x in 0..dst_w as usize {
            let sx = ((x as u64 * u64::from(src_w)) / u64::from(dst_w)) as usize;
            dst_row[x * 4..x * 4 + 4].copy_from_slice(&src_row[sx * 4..sx * 4 + 4]);
        }
    }
    dst
}

/// Decode and conversion times, logged now and then.
struct Stats {
    since: Instant,
    shown: u32,
    skipped: u32,
    decode: Duration,
    convert: Duration,
}

impl Stats {
    fn new() -> Stats {
        Stats { since: Instant::now(), shown: 0, skipped: 0, decode: Duration::ZERO, convert: Duration::ZERO }
    }

    fn shown(&mut self, decode: Duration, convert: Duration) {
        self.shown += 1;
        self.decode += decode;
        self.convert += convert;
        self.maybe_log();
    }

    fn skipped(&mut self) {
        self.skipped += 1;
        self.maybe_log();
    }

    fn maybe_log(&mut self) {
        let elapsed = self.since.elapsed();
        if elapsed < STATS_EVERY || self.shown == 0 {
            return;
        }
        tracing::info!(
            fps = format!("{:.1}", f64::from(self.shown) / elapsed.as_secs_f64()),
            skipped = self.skipped,
            decode_ms = format!("{:.1}", self.decode.as_secs_f64() * 1000.0 / f64::from(self.shown)),
            convert_ms = format!("{:.1}", self.convert.as_secs_f64() * 1000.0 / f64::from(self.shown)),
            "mirroring"
        );
        *self = Stats::new();
    }
}

/// The view-only receiver in its real window, as a runnable gate for the
/// non-default `test-support` build:
/// `nectarlink-desktop --mirror-window-gate --data-dir <run>`.
///
/// The production `Mirror` bridge, `MirrorWindow.qml` and `VideoView` show a
/// second, synthetic phone node paired by QR over 127.0.0.1. That phone only
/// answers `mirror.start` and sends the committed 128x128 red keyframe, which
/// the receiver scales to each announced format; nothing is captured, and
/// there is no microphone, sound, input, app window or saved identity. The
/// last window records that keyframe at its own size into the run's
/// `received` folder, never the user's Downloads.
#[cfg(all(target_os = "macos", feature = "test-support"))]
pub mod window_gate {
    use std::{
        os::unix::fs::MetadataExt,
        path::{Path, PathBuf},
        process::ExitCode,
        sync::{Arc, Mutex},
        time::Duration,
    };

    use cxx_qt_lib::{QByteArray, QGuiApplication, QQmlApplicationEngine, QUrl};
    use nectarlink_core::{
        DeviceId, DeviceInfo, DeviceKind, LinkState, MIRROR_CAPTURE, MIRROR_CORRELATION, MIRROR_SCREEN,
        MIRROR_VIEW, MirrorConfig, MirrorSend, MirrorSink, MirrorStart, MirrorStream, Node, NodeConfig,
        PacketKind, PlainKeyProtector, Platform,
    };
    use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};

    use super::{Window, ffi};
    use crate::win::{
        image::Bitmap,
        mp4::{nal_type, split_annexb},
    };

    pub const ARG: &str = "--mirror-window-gate";
    /// The only entry of a fresh run directory: a file holding that
    /// directory's canonical path.
    pub const SENTINEL: &str = ".mirror-window-gate";

    const PASS: i32 = 0;
    const FAILED: i32 = 1;
    const BLOCKED: i32 = 2;
    /// Formats the phone announces; the decoded picture stays 128x128.
    const PORTRAIT: (u32, u32) = (360, 640);
    const LANDSCAPE: (u32, u32) = (640, 360);
    /// The keyframe's own coded size: the format of the recorded stream.
    const NATIVE: (u32, u32) = (128, 128);
    /// Keyframes the phone sends, 250 ms apart, when the recording asks.
    const RECORDED_KEYFRAMES: u32 = 4;
    const KEYFRAME_GAP: Duration = Duration::from_millis(250);
    /// After this the process exits even if Qt never returns.
    const HARD_TIMEOUT: Duration = Duration::from_secs(180);
    const GRAB_WAIT: Duration = Duration::from_secs(30);

    pub fn run(data_dir: Option<&Path>) -> ExitCode {
        let args: Vec<_> = std::env::args_os().skip(1).collect();
        if args.len() != 3 || args[0] != ARG || args[1] != "--data-dir" {
            return blocked(format!("usage: nectarlink-desktop {ARG} --data-dir <fresh run directory>"));
        }
        for var in ["QT_QPA_PLATFORM", "QT_QUICK_BACKEND", "QSG_RHI_BACKEND"] {
            if std::env::var_os(var).is_some() {
                return blocked(format!("{var} is set; the gate only runs the default Cocoa/Metal path"));
            }
        }
        let Some(home) = dirs::home_dir() else { return blocked("no home directory") };
        let run_dir = match validate_run_dir(data_dir, &home.join("orca/tmp")) {
            Ok(dir) => dir,
            Err(e) => return blocked(e),
        };
        let _ = tracing_subscriber::fmt()
            .with_max_level(tracing::Level::INFO)
            .with_writer(std::io::stderr)
            .try_init();
        std::thread::spawn(|| {
            std::thread::sleep(HARD_TIMEOUT);
            eprintln!("mirror-window-gate: BLOCKED: no result within {} s", HARD_TIMEOUT.as_secs());
            std::process::exit(BLOCKED);
        });

        let gate = match Gate::start(&run_dir) {
            Ok(gate) => gate,
            Err(e) => return blocked(e),
        };
        let shown = gate.show();
        let phone = if shown.code == PASS { gate.phone_report() } else { Err("not checked".into()) };
        let recording = if shown.code == PASS { gate.recording_report() } else { Err("not checked".into()) };
        let shut_down = gate.shut_down();

        let code = if shown.code != PASS {
            shown.code
        } else if let Err(e) = &shown.grabs {
            println!("mirror-window-gate: FAILED render: {e}");
            FAILED
        } else if let Err(e) = &phone {
            println!("mirror-window-gate: FAILED phone: {e}");
            FAILED
        } else if let Err(e) = &recording {
            println!("mirror-window-gate: FAILED recording: {e}");
            FAILED
        } else if shown.qml_problems > 0 {
            println!(
                "mirror-window-gate: FAILED: {} QML warning(s) or error(s) from the app's QML",
                shown.qml_problems
            );
            FAILED
        } else if !shut_down {
            println!("mirror-window-gate: FAILED: a node did not shut down within 5 seconds");
            FAILED
        } else {
            println!(
                "mirror-window-gate: PASS lifecycle: Mirror.start opened MirrorWindow; the phone turned it \
                 portrait to landscape; the phone's end blanked it; Try again reused it; closing destroyed \
                 it and stopped the phone; Mirror.start opened a new window; closing destroyed it; \
                 Mirror.start opened a third window at the keyframe's 128x128; Mirror.toggleRecording \
                 showed recording with a running timer, and again stopped it and saved the file; closing \
                 destroyed it"
            );
            for line in shown.grabs.iter().flatten() {
                println!("mirror-window-gate: PASS render {line}");
            }
            if let Ok(line) = &phone {
                println!("mirror-window-gate: PASS phone {line}");
            }
            if let Ok(line) = &recording {
                println!("mirror-window-gate: PASS recording {line}");
            }
            PASS
        };
        ExitCode::from(u8::try_from(code).unwrap_or(1))
    }

    fn blocked(reason: impl std::fmt::Display) -> ExitCode {
        eprintln!("mirror-window-gate: BLOCKED: {reason}");
        ExitCode::from(BLOCKED as u8)
    }

    /// `dir` must be a canonical, owner-only directory at least two levels
    /// under `scratch_root` in a `waypair-*` folder, holding only [`SENTINEL`].
    pub(super) fn validate_run_dir(dir: Option<&Path>, scratch_root: &Path) -> Result<PathBuf, String> {
        let dir = dir.ok_or("--data-dir <fresh run directory> is required")?;
        let meta =
            std::fs::symlink_metadata(dir).map_err(|e| format!("run directory {}: {e}", dir.display()))?;
        if !meta.is_dir() {
            return Err("the run directory must be a directory, not a link or a file".into());
        }
        let canonical = dir.canonicalize().map_err(|e| format!("run directory: {e}"))?;
        if canonical != dir {
            return Err("the run directory must be given as its canonical absolute path".into());
        }
        let root = scratch_root
            .canonicalize()
            .map_err(|e| format!("scratch root {}: {e}", scratch_root.display()))?;
        let relative = canonical
            .strip_prefix(&root)
            .map_err(|_| format!("the run directory must be under {}", root.display()))?;
        let mut parts = relative.components();
        let in_waypair =
            parts.next().and_then(|c| c.as_os_str().to_str()).is_some_and(|c| c.starts_with("waypair-"));
        if !in_waypair || parts.next().is_none() {
            return Err(format!("the run directory must be inside {}/waypair-*/", root.display()));
        }
        let owner = std::fs::metadata(&root).map_err(|e| format!("scratch root: {e}"))?.uid();
        if meta.uid() != owner || meta.mode() & 0o022 != 0 {
            return Err(
                "the run directory must belong to the scratch owner and not be group or world writable"
                    .into(),
            );
        }
        let entries: Vec<_> = std::fs::read_dir(&canonical)
            .map_err(|e| format!("run directory: {e}"))?
            .map(|entry| entry.map(|e| e.file_name()))
            .collect::<Result<_, _>>()
            .map_err(|e| format!("run directory: {e}"))?;
        if entries.len() != 1 || entries[0] != SENTINEL {
            return Err(format!("the run directory must be fresh: only {SENTINEL} may be in it"));
        }
        let sentinel = canonical.join(SENTINEL);
        if !std::fs::symlink_metadata(&sentinel).is_ok_and(|m| m.is_file()) {
            return Err(format!("{SENTINEL} must be a regular file"));
        }
        let names = std::fs::read_to_string(&sentinel).map_err(|e| format!("{SENTINEL}: {e}"))?;
        if Some(names.trim_end_matches('\n')) != canonical.to_str() {
            return Err(format!("{SENTINEL} must hold this run directory's path"));
        }
        Ok(canonical)
    }

    fn device(name: &str, kind: DeviceKind) -> DeviceInfo {
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

    /// A throwaway node bound to 127.0.0.1: no port mapping, discovery,
    /// relay, ADB or USB, a plain key in `directory`, and only `capabilities`.
    pub(super) fn loopback_config(
        directory: &Path,
        name: &str,
        kind: DeviceKind,
        capabilities: &[&str],
    ) -> NodeConfig {
        let mut config = NodeConfig::new(directory, device(name, kind), "0.0.1-test");
        config.away_mode = false;
        config.lan_discovery = false;
        config.adb_server = None;
        config.usb_port = None;
        config.enable_loopback_mirror_fixture();
        config.key_protector = Some(Arc::new(PlainKeyProtector));
        config.capabilities = capabilities.iter().map(|cap| (*cap).to_owned()).collect();
        config
    }

    pub(super) async fn wait_for_link(receiver: &Node, phone: DeviceId) -> Result<(), String> {
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                if receiver.paired_devices().is_ok_and(|peers| {
                    peers
                        .iter()
                        .any(|peer| peer.id == phone && matches!(&peer.link, LinkState::Online { .. }))
                }) {
                    return;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .map_err(|_| "authenticated loopback link did not come online within 5 seconds".to_owned())
    }

    /// The receiver's video goes to the production sink only.
    #[derive(Debug)]
    struct GateReceiver;

    impl Platform for GateReceiver {
        fn mirror_sink(&self, peer: &DeviceId, session: u32) -> Option<Arc<dyn MirrorSink>> {
            super::sink(peer, session)
        }
    }

    #[derive(Debug)]
    enum PhoneEvent {
        Start(DeviceId, MirrorStart),
        Stop(DeviceId, u32),
        Keyframe,
    }

    /// Approves every request in place of a person; never captures anything.
    #[derive(Debug)]
    struct GatePhone {
        events: UnboundedSender<PhoneEvent>,
    }

    impl Platform for GatePhone {
        fn mirror_requested(&self, peer: &DeviceId, options: &MirrorStart) -> Result<(), String> {
            self.events.send(PhoneEvent::Start(*peer, options.clone())).map_err(|_| "gate phone ended".into())
        }
        fn mirror_stop_requested(&self, peer: &DeviceId, session: u32) {
            let _ = self.events.send(PhoneEvent::Stop(*peer, session));
        }
        fn mirror_keyframe_requested(&self, _peer: &DeviceId, _session: u32) {
            let _ = self.events.send(PhoneEvent::Keyframe);
        }
    }

    #[derive(Debug, Default)]
    struct PhoneLog {
        starts: u32,
        stops: u32,
        formats: Vec<(u32, u32)>,
        /// Keyframe requests on the recorded stream, and keyframes sent for them.
        keyframe_requests: u32,
        keyframes: u32,
        problems: Vec<String>,
    }

    fn lock(log: &Mutex<PhoneLog>) -> std::sync::MutexGuard<'_, PhoneLog> {
        log.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn idr() -> Vec<u8> {
        include_bytes!("mac/testdata/red-128x128.annexb").to_vec()
    }

    /// A format, then the keyframe, as a phone sends them after turning.
    async fn send_format(
        stream: &Arc<MirrorStream>,
        format: (u32, u32),
        request: u64,
        time_us: u64,
    ) -> Result<(), String> {
        let (width, height) = format;
        let config = MirrorConfig {
            codec: "h264".into(),
            width,
            height,
            session: MIRROR_SCREEN,
            request: Some(request),
        };
        let stream = stream.clone();
        let queued = tokio::time::timeout(
            Duration::from_secs(5),
            tokio::task::spawn_blocking(move || {
                let config = stream.send(PacketKind::Config, time_us, config.to_cbor());
                (config, stream.send(PacketKind::Keyframe, time_us, idr()))
            }),
        )
        .await
        .map_err(|_| format!("sending {width}x{height} took over 5 seconds"))?
        .map_err(|e| format!("sending {width}x{height} panicked: {e}"))?;
        if queued != (MirrorSend::Queued, MirrorSend::Queued) {
            return Err(format!("{width}x{height} was not queued: {queued:?}"));
        }
        Ok(())
    }

    /// The recording asked for a keyframe: the phone sends a few, spaced.
    async fn send_keyframes(stream: Arc<MirrorStream>, log: Arc<Mutex<PhoneLog>>) {
        for n in 0..RECORDED_KEYFRAMES {
            if n > 0 {
                tokio::time::sleep(KEYFRAME_GAP).await;
            }
            let time_us = 1_000_000 + u64::from(n) * KEYFRAME_GAP.as_micros() as u64;
            let stream = stream.clone();
            let queued = tokio::time::timeout(
                Duration::from_secs(5),
                tokio::task::spawn_blocking(move || stream.send(PacketKind::Keyframe, time_us, idr())),
            )
            .await;
            match queued {
                Ok(Ok(MirrorSend::Queued)) => lock(&log).keyframes += 1,
                other => {
                    lock(&log).problems.push(format!("recorded keyframe {n} was not queued: {other:?}"));
                    return;
                }
            }
        }
    }

    async fn wait_for_grab(path: PathBuf) -> Result<(), String> {
        tokio::time::timeout(GRAB_WAIT, async {
            while !path.exists() {
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        })
        .await
        .map_err(|_| format!("the window did not save {} within {} s", path.display(), GRAB_WAIT.as_secs()))
    }

    /// The phone's side: the first request turns from portrait to landscape
    /// once each picture is grabbed, then stops sharing on its own; the
    /// second answers portrait, the third landscape, both until stopped;
    /// the fourth answers at the keyframe's own size, and its first
    /// keyframe request (the recording's) gets the recorded keyframes.
    async fn phone_script(
        phone: Node,
        receiver: DeviceId,
        grabs: PathBuf,
        mut events: UnboundedReceiver<PhoneEvent>,
        log: Arc<Mutex<PhoneLog>>,
    ) {
        let mut current: Option<Arc<MirrorStream>> = None;
        while let Some(event) = events.recv().await {
            match event {
                PhoneEvent::Start(peer, options) => {
                    let starts = {
                        let mut log = lock(&log);
                        log.starts += 1;
                        log.starts
                    };
                    if peer != receiver
                        || options.session != MIRROR_SCREEN
                        || options.audio
                        || options.app.is_some()
                    {
                        lock(&log).problems.push("a request asked for more than the screen's video".into());
                        continue;
                    }
                    let Some(request) = options.request else {
                        lock(&log).problems.push("mirror.start carried no request token".into());
                        continue;
                    };
                    if let Some(old) = current.take() {
                        old.close();
                    }
                    let stream = match tokio::time::timeout(
                        Duration::from_secs(10),
                        phone.mirror_open(receiver),
                    )
                    .await
                    {
                        Ok(Ok(stream)) => Arc::new(stream),
                        Ok(Err(e)) => {
                            lock(&log).problems.push(format!("mirror_open failed: {e}"));
                            continue;
                        }
                        Err(_) => {
                            lock(&log).problems.push("mirror_open took over 10 seconds".into());
                            continue;
                        }
                    };
                    let format = match starts {
                        3 => LANDSCAPE,
                        4 => NATIVE,
                        _ => PORTRAIT,
                    };
                    if let Err(e) = send_format(&stream, format, request, 0).await {
                        lock(&log).problems.push(e);
                        continue;
                    }
                    lock(&log).formats.push(format);
                    if starts == 1 {
                        let (stream, grabs, log) = (stream.clone(), grabs.clone(), log.clone());
                        tokio::spawn(async move {
                            let turned = async {
                                wait_for_grab(grabs.join("1-portrait.png")).await?;
                                send_format(&stream, LANDSCAPE, request, 1_000_000).await?;
                                lock(&log).formats.push(LANDSCAPE);
                                wait_for_grab(grabs.join("2-landscape.png")).await
                            }
                            .await;
                            stream.close();
                            if let Err(e) = turned {
                                lock(&log).problems.push(e);
                            }
                        });
                    }
                    current = Some(stream);
                }
                PhoneEvent::Stop(peer, session) => {
                    let mut log = lock(&log);
                    log.stops += 1;
                    if peer != receiver || session != MIRROR_SCREEN {
                        log.problems.push("a stop named another device or session".into());
                    }
                    if let Some(stream) = current.take() {
                        stream.close();
                    }
                }
                PhoneEvent::Keyframe => {
                    let Some(stream) = &current else { continue };
                    let recorded_requests = {
                        let mut log = lock(&log);
                        (log.starts == 4).then(|| {
                            log.keyframe_requests += 1;
                            log.keyframe_requests
                        })
                    };
                    match recorded_requests {
                        None => {
                            let _ = stream.send(PacketKind::Keyframe, 0, idr());
                        }
                        Some(1) => {
                            tokio::spawn(send_keyframes(stream.clone(), log.clone()));
                        }
                        Some(_) => {}
                    }
                }
            }
        }
        if let Some(stream) = current {
            stream.close();
        }
    }

    struct Shown {
        code: i32,
        grabs: Result<Vec<String>, String>,
        qml_problems: i32,
    }

    struct Gate {
        run_dir: PathBuf,
        grabs: PathBuf,
        received: PathBuf,
        receiver: Node,
        phone: Node,
        phone_runtime: tokio::runtime::Runtime,
        script: tokio::task::JoinHandle<()>,
        log: Arc<Mutex<PhoneLog>>,
    }

    fn shut_down_node(runtime: &tokio::runtime::Runtime, node: &Node) -> bool {
        runtime
            .block_on(async { tokio::time::timeout(Duration::from_secs(5), node.shutdown()).await })
            .is_ok()
    }

    /// QR pairing over the loopback-only invitation; its secret stays in memory.
    fn pair(
        host: &tokio::runtime::Runtime,
        receiver: &Node,
        phone_runtime: &tokio::runtime::Runtime,
        phone: &Node,
    ) -> Result<(), String> {
        let invitation = host
            .block_on(async {
                tokio::time::timeout(Duration::from_secs(10), receiver.pairing_start_qr()).await
            })
            .map_err(|_| "pairing invitation took over 10 seconds".to_owned())?
            .map_err(|e| format!("pairing invitation failed: {e}"))?;
        if invitation.addrs.is_empty() || !invitation.addrs.iter().all(|a| a.ip().is_loopback()) {
            return Err("the receiver offered a missing or non-loopback address".to_owned());
        }
        let uri = invitation.to_uri();
        drop(invitation);
        phone_runtime
            .block_on(async { tokio::time::timeout(Duration::from_secs(20), phone.pairing_join(&uri)).await })
            .map_err(|_| "loopback QR pairing took over 20 seconds".to_owned())?
            .map_err(|e| format!("loopback QR pairing failed: {e}"))?;
        host.block_on(wait_for_link(receiver, phone.device_id()))
    }

    impl Gate {
        fn start(run_dir: &Path) -> Result<Gate, String> {
            let (receiver_dir, phone_dir, grabs, received) = (
                run_dir.join("receiver"),
                run_dir.join("phone"),
                run_dir.join("grabs"),
                run_dir.join("received"),
            );
            for dir in [&receiver_dir, &phone_dir, &grabs, &received] {
                std::fs::create_dir(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
            }
            crate::downloads::use_window_gate_received_dir(received.clone())?;
            let receiver = crate::core_host::start_window_gate(
                loopback_config(
                    &receiver_dir,
                    "Synthetic Mac",
                    DeviceKind::Desktop,
                    &[MIRROR_VIEW, MIRROR_CORRELATION],
                ),
                Arc::new(GateReceiver),
            )?;
            let host = &crate::core_host::host().runtime;
            let phone_runtime = match tokio::runtime::Builder::new_multi_thread()
                .worker_threads(2)
                .thread_name("gate-phone")
                .enable_all()
                .build()
            {
                Ok(runtime) => runtime,
                Err(e) => {
                    shut_down_node(host, &receiver);
                    return Err(format!("phone runtime: {e}"));
                }
            };
            let (events, phone_events) = unbounded_channel();
            let phone_config = loopback_config(
                &phone_dir,
                "Synthetic Phone",
                DeviceKind::Phone,
                &[MIRROR_CAPTURE, MIRROR_CORRELATION],
            );
            let phone = match phone_runtime.block_on(async {
                tokio::time::timeout(
                    Duration::from_secs(10),
                    Node::start(phone_config, Arc::new(GatePhone { events })),
                )
                .await
            }) {
                Ok(Ok(phone)) => phone,
                Ok(Err(e)) => {
                    shut_down_node(host, &receiver);
                    return Err(format!("synthetic phone node failed to start: {e}"));
                }
                Err(_) => {
                    shut_down_node(host, &receiver);
                    return Err("synthetic phone node did not start within 10 seconds".into());
                }
            };
            if let Err(e) = pair(host, &receiver, &phone_runtime, &phone) {
                shut_down_node(&phone_runtime, &phone);
                shut_down_node(host, &receiver);
                return Err(e);
            }
            let log = Arc::new(Mutex::new(PhoneLog::default()));
            let script = phone_runtime.spawn(phone_script(
                phone.clone(),
                receiver.device_id(),
                grabs.clone(),
                phone_events,
                log.clone(),
            ));
            Ok(Gate {
                run_dir: run_dir.to_owned(),
                grabs,
                received,
                receiver,
                phone,
                phone_runtime,
                script,
                log,
            })
        }

        /// Runs the window sequence in `GATE_QML` on this (the main) thread.
        fn show(&self) -> Shown {
            ffi::prepare_qt_for_ui_check();
            let mut app = QGuiApplication::new();
            ffi::keep_running_without_windows();
            let _ = ffi::load_bundled_fonts();
            let phone = self.phone.device_id();
            let quoted = |s: &str| serde_json::Value::from(s).to_string();
            let qml = GATE_QML
                .replace("@PHONE@", &quoted(&phone.to_string()))
                .replace("@KEY@", &quoted(&Window::screen(phone).key()))
                .replace("@GRABS@", &quoted(&self.grabs.to_string_lossy()));
            let url = format!("file://{}/mirror-window-gate.qml", self.run_dir.display());
            let mut engine = QQmlApplicationEngine::new();
            if let Some(mut engine) = engine.as_mut() {
                ffi::register_qml_app_engine(engine.as_mut());
                engine.load_data(&QByteArray::from(qml.as_str()), &QUrl::from(url.as_str()));
            }
            let code = app.as_mut().map_or(BLOCKED, |app| app.exec());
            drop(engine);
            let grabs = if code == PASS { check_grabs(&self.grabs) } else { Err("not checked".into()) };
            let qml_problems = ffi::qml_problem_count();
            drop(app);
            Shown { code, grabs, qml_problems }
        }

        fn phone_report(&self) -> Result<String, String> {
            for _ in 0..100 {
                if lock(&self.log).stops >= 3 {
                    break;
                }
                std::thread::sleep(Duration::from_millis(50));
            }
            let log = lock(&self.log);
            if !log.problems.is_empty() {
                return Err(log.problems.join("; "));
            }
            if (log.starts, log.stops) != (4, 3) {
                return Err(format!("expected 4 requests and 3 stops, saw {} and {}", log.starts, log.stops));
            }
            if log.formats != [PORTRAIT, LANDSCAPE, PORTRAIT, LANDSCAPE, NATIVE] {
                return Err(format!("unexpected format sequence {:?}", log.formats));
            }
            Ok(format!(
                "4 screen-only, video-only requests with live tokens; 3 stops from closed windows; formats {:?}",
                log.formats
            ))
        }

        /// What the window's recording saved: one video-only MP4 in the
        /// run's own received folder, described at the keyframe's coded size
        /// with its SPS, holding every keyframe sent for the recording.
        fn recording_report(&self) -> Result<String, String> {
            let (requests, keyframes) = {
                let log = lock(&self.log);
                (log.keyframe_requests, log.keyframes)
            };
            if requests == 0 || keyframes != RECORDED_KEYFRAMES {
                return Err(format!(
                    "the recorded stream saw {requests} keyframe request(s) and sent {keyframes} of \
                     {RECORDED_KEYFRAMES} keyframes"
                ));
            }
            let files = std::fs::read_dir(&self.received)
                .and_then(|entries| {
                    entries.map(|entry| entry.map(|e| e.path())).collect::<Result<Vec<_>, _>>()
                })
                .map_err(|e| format!("{}: {e}", self.received.display()))?;
            let [path] = files.as_slice() else {
                return Err(format!("expected 1 file in {}, found {}", self.received.display(), files.len()));
            };
            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or_default().to_owned();
            if !name.starts_with("Mirror ")
                || !name.ends_with(".mp4")
                || !std::fs::symlink_metadata(path).is_ok_and(|m| m.is_file())
            {
                return Err(format!("{name:?} is not a regular Mirror *.mp4 file"));
            }
            let file = std::fs::read(path).map_err(|e| format!("{name}: {e}"))?;
            if !(256..8 << 20).contains(&file.len()) {
                return Err(format!("{name}: {} bytes is outside 256 B to 8 MiB", file.len()));
            }
            let boxes = top_level_boxes(&file).map_err(|e| format!("{name}: {e}"))?;
            let kinds: Vec<_> = boxes.iter().map(|(kind, _)| String::from_utf8_lossy(kind)).collect();
            if kinds != ["ftyp", "mdat", "moov"] {
                return Err(format!("{name}: top-level boxes {kinds:?}, not ftyp, mdat, moov"));
            }
            let moov = boxes[2].1;
            let find = |needle: &[u8]| moov.windows(needle.len()).position(|w| w == needle);
            if find(b"soun").is_some() || find(b"vide").is_none() {
                return Err(format!("{name}: not a single video track"));
            }
            let at = |fourcc: &[u8; 4], offset: usize, len: usize| {
                find(fourcc)
                    .and_then(|start| moov.get(start + 4 + offset..start + 4 + offset + len))
                    .map(|bytes| bytes.iter().fold(0u32, |n, b| (n << 8) | u32::from(*b)))
                    .ok_or_else(|| format!("{name}: no complete {}", String::from_utf8_lossy(fourcc)))
            };
            let size = (at(b"avc1", 24, 2)?, at(b"avc1", 26, 2)?);
            if size != NATIVE {
                return Err(format!("{name}: avc1 describes {size:?}, not the keyframe's {NATIVE:?}"));
            }
            let idr = idr();
            let sps = split_annexb(&idr)
                .into_iter()
                .find(|nal| nal_type(nal) == Some(7))
                .ok_or("the committed keyframe has no SPS")?;
            let avcc = find(b"avcC").ok_or_else(|| format!("{name}: no avcC"))?;
            if !moov[avcc..].windows(sps.len()).any(|w| w == sps) {
                return Err(format!("{name}: avcC does not carry the keyframe's SPS"));
            }
            let samples = at(b"stsz", 8, 4)?;
            if samples != keyframes {
                return Err(format!("{name}: {samples} samples, but the phone sent {keyframes} keyframes"));
            }
            Ok(format!(
                "{name}: {} B video-only MP4 (ftyp, mdat, moov; avc1 {}x{} with the keyframe's SPS; no \
                 sound track) in the run's received folder, holding the {samples} keyframes the phone sent \
                 for {requests} keyframe request(s)",
                file.len(),
                size.0,
                size.1
            ))
        }

        fn shut_down(self) -> bool {
            self.script.abort();
            let phone = shut_down_node(&self.phone_runtime, &self.phone);
            let receiver = shut_down_node(&crate::core_host::host().runtime, &self.receiver);
            super::clear_window_buffers(&Window::screen(self.phone.device_id()));
            phone && receiver
        }
    }

    /// The `(type, payload)` of each top-level MP4 box, in file order.
    fn top_level_boxes(file: &[u8]) -> Result<Vec<([u8; 4], &[u8])>, String> {
        let mut boxes = Vec::new();
        let mut rest = file;
        while !rest.is_empty() {
            let [a, b, c, d, k0, k1, k2, k3, ..] = *rest else {
                return Err(format!("a truncated box header after {} boxes", boxes.len()));
            };
            let size = u32::from_be_bytes([a, b, c, d]) as usize;
            if size < 8 || size > rest.len() {
                return Err(format!("box {} has size {size} of {} bytes left", boxes.len(), rest.len()));
            }
            boxes.push(([k0, k1, k2, k3], &rest[8..size]));
            rest = &rest[size..];
        }
        Ok(boxes)
    }

    fn is_red(px: &[u8; 4]) -> bool {
        px[2] >= 230 && px[1] <= 24 && px[0] <= 24
    }

    /// Checks one window grab (BGRA): a showing window keeps the frame's
    /// aspect and is red inside its picture, away from the toolbar at the
    /// top and the control hint at the bottom; an ended one has no red.
    fn check_grab(name: &str, image: &Bitmap, frame: Option<(u32, u32)>) -> Result<String, String> {
        let (w, h) = (image.width, image.height);
        if w < 16 || h < 16 || image.bgra.len() < (w as usize) * (h as usize) * 4 {
            return Err(format!("{name}: unusable {w}x{h} grab"));
        }
        let pixels = image.bgra.as_chunks::<4>().0;
        let Some((fw, fh)) = frame else {
            let red = pixels.iter().filter(|px| is_red(px)).count();
            if red > 0 {
                return Err(format!("{name}: {red} red pixels left after the stream ended"));
            }
            return Ok(format!("{name}: {w}x{h} grab of the ended window has no picture left"));
        };
        let (window_aspect, frame_aspect) = (f64::from(w) / f64::from(h), f64::from(fw) / f64::from(fh));
        if (window_aspect - frame_aspect).abs() > 0.06 {
            return Err(format!(
                "{name}: window aspect {window_aspect:.3} does not follow the {fw}x{fh} frame"
            ));
        }
        let scale = (f64::from(w) / f64::from(fw)).min(f64::from(h) / f64::from(fh));
        let (pw, ph) = (f64::from(fw) * scale, f64::from(fh) * scale);
        let (left, top) = ((f64::from(w) - pw) / 2.0, (f64::from(h) - ph) / 2.0);
        let (x0, x1) = ((left + pw * 0.2) as u32, (left + pw * 0.8) as u32);
        let (y0, y1) = ((top + ph * 0.2) as u32, (top + ph * 0.6) as u32);
        let mut sampled = 0usize;
        for y in y0..y1 {
            for x in x0..x1 {
                let pixel = &pixels[(y * w + x) as usize];
                if !is_red(pixel) {
                    return Err(format!("{name}: pixel ({x},{y}) is {pixel:?} (BGRA), not the red keyframe"));
                }
                sampled += 1;
            }
        }
        Ok(format!(
            "{name}: {w}x{h} grab (aspect {window_aspect:.3}) shows the coded 128x128 keyframe scaled to \
             {fw}x{fh}; {sampled}/{sampled} sampled pixels red"
        ))
    }

    fn check_grabs(grabs: &Path) -> Result<Vec<String>, String> {
        [
            ("1-portrait.png", Some(PORTRAIT)),
            ("2-landscape.png", Some(LANDSCAPE)),
            ("3-ended.png", None),
            ("4-reopened.png", Some(PORTRAIT)),
            ("5-restarted.png", Some(LANDSCAPE)),
            ("6-recording.png", Some(NATIVE)),
        ]
        .into_iter()
        .map(|(name, frame)| {
            let bytes = std::fs::read(grabs.join(name)).map_err(|e| format!("{name}: {e}"))?;
            let image = crate::win::image::decode(&bytes).map_err(|e| format!("{name}: {e}"))?;
            if frame == Some(NATIVE) {
                check_recording_grab(name, &image)
            } else {
                check_grab(name, &image, frame)
            }
        })
        .collect()
    }

    /// The recording window still shows the picture, though its toolbar,
    /// notice and hint cover too much of a small square window for
    /// `check_grab`'s sample; the grab is kept as evidence of the toolbar.
    fn check_recording_grab(name: &str, image: &Bitmap) -> Result<String, String> {
        let (w, h) = (image.width, image.height);
        if w < 16 || h < 16 || image.bgra.len() < (w as usize) * (h as usize) * 4 {
            return Err(format!("{name}: unusable {w}x{h} grab"));
        }
        if (f64::from(w) / f64::from(h) - 1.0).abs() > 0.06 {
            return Err(format!("{name}: a {w}x{h} grab does not follow the square 128x128 frame"));
        }
        let pixels = &image.bgra.as_chunks::<4>().0[..(w * h) as usize];
        let red = pixels.iter().filter(|px| is_red(px)).count();
        if red == 0 {
            return Err(format!("{name}: no pixel shows the red keyframe while recording"));
        }
        Ok(format!(
            "{name}: {w}x{h} grab while recording; {red} of {} pixels show the red keyframe",
            pixels.len()
        ))
    }

    /// Creates a `MirrorWindow` for each key in `Mirror.windows` (as App.qml
    /// does) and walks it through the lifecycle the way a user would, each
    /// step bounded; saves a grab of the window's own content at each check.
    /// Exits 0 (pass), 1 (wrong state) or 2 (not the native path, or stuck).
    const GATE_QML: &str = r#"
import QtQml
import QtQuick
import app.nectarlink

QtObject {
    id: gate
    readonly property string phone: @PHONE@
    readonly property string key: @KEY@
    readonly property string grabs: @GRABS@
    property var windows: ({})
    property int created: 0
    property int destroyed: 0
    property var first: null
    property var second: null
    property var third: null
    property int step: 0
    property double stepStarted: Date.now()
    property double closedAt: 0
    property double reclosedAt: 0
    property bool busy: false
    property bool finished: false

    readonly property Component mirrorWindow: Component {
        MirrorWindow {
            readonly property int gateApi: gateProbe.GraphicsInfo.api
            Item { id: gateProbe }
        }
    }

    readonly property Timer ticker: Timer {
        interval: 50
        repeat: true
        running: true
        onTriggered: gate.tick()
    }
    readonly property Timer watchdog: Timer {
        interval: 150000
        running: true
        onTriggered: gate.fail(2, "the sequence did not finish within 150 s: " + gate.describe())
    }

    function sync() {
        let keys = []
        try {
            keys = JSON.parse(Mirror.windows).map(w => w.key)
        } catch (e) {}
        for (const k of Object.keys(windows)) {
            if (keys.indexOf(k) < 0) {
                windows[k].destroy()
                delete windows[k]
                destroyed++
            }
        }
        for (const k of keys) {
            if (!windows[k]) {
                const w = mirrorWindow.createObject(gate, { mirrorKey: k })
                if (!w) {
                    fail(2, "MirrorWindow did not load: " + mirrorWindow.errorString())
                    return
                }
                windows[k] = w
                created++
            }
        }
    }

    function listed() {
        try {
            return JSON.parse(Mirror.windows).some(w => w.key === key)
        } catch (e) {
            return false
        }
    }

    function video(w) {
        const items = w.contentItem.children
        for (let i = 0; i < items.length; i++) {
            if (items[i].frames !== undefined && items[i].stream === w.mirrorKey)
                return items[i]
        }
        return null
    }

    function describe() {
        const w = windows[key]
        if (!w)
            return "no window (created " + created + ", destroyed " + destroyed + ")"
        const v = video(w)
        return "phase " + w.phase + ", window " + w.width + "x" + w.height + ", frame " + w.frame.width + "x"
            + w.frame.height + ", frames " + (v ? v.frames : -1) + ", visible " + w.visible
    }

    function shows(w, width, height) {
        const v = video(w)
        return w.visible && w.phase === "showing" && v !== null && v.frames > 0 && w.frame.width === width
            && w.frame.height === height && w.height > 0 && Math.abs(w.width / w.height - width / height) <= 0.05
    }

    function nativePath(w) {
        if (Qt.platform.pluginName !== "cocoa") {
            fail(2, "the Qt platform is " + Qt.platform.pluginName + ", not cocoa")
            return false
        }
        if (w.gateApi !== GraphicsInfo.Metal) {
            fail(2, "the window renders with graphics API " + w.gateApi + ", not Metal")
            return false
        }
        return true
    }

    function grab(w, name, next) {
        busy = true
        const state = describe()
        const ok = w.contentItem.grabToImage(function (result) {
            if (finished)
                return
            if (!result.saveToFile(grabs + "/" + name)) {
                fail(2, "could not save " + name)
                return
            }
            console.info("mirror-window-gate: " + name + ": " + state)
            busy = false
            advance(next)
        })
        if (!ok)
            fail(2, "the window refused grabToImage for " + name)
    }

    function advance(next) {
        step = next
        stepStarted = Date.now()
    }

    function tick() {
        if (finished)
            return
        if (Date.now() - stepStarted > 20000) {
            fail(busy ? 2 : 1, "step " + step + " did not complete within 20 s: " + describe())
            return
        }
        if (busy)
            return
        const w = windows[key] || null
        switch (step) {
        case 0:
            Mirror.start(phone)
            advance(1)
            break
        case 1:
            if (w && shows(w, 360, 640) && nativePath(w)) {
                first = w
                grab(w, "1-portrait.png", 2)
            }
            break
        case 2:
            if (w === first && shows(w, 640, 360) && nativePath(w))
                grab(w, "2-landscape.png", 3)
            break
        case 3:
            if (w === first && w.visible && w.phase === "ended" && w.frame.width <= 0)
                grab(w, "3-ended.png", 4)
            break
        case 4:
            Mirror.reopen(key)
            advance(5)
            break
        case 5:
            if (w === first && created === 1 && shows(w, 360, 640) && nativePath(w))
                grab(w, "4-reopened.png", 6)
            break
        case 6:
            first.close()
            advance(7)
            break
        case 7:
            if (!w && destroyed === 1 && !listed()) {
                if (closedAt === 0)
                    closedAt = Date.now()
                if (Date.now() - closedAt >= 1000) {
                    Mirror.start(phone)
                    advance(8)
                }
            }
            break
        case 8:
            if (w && w !== first && created === 2 && shows(w, 640, 360) && nativePath(w)) {
                second = w
                grab(w, "5-restarted.png", 9)
            }
            break
        case 9:
            second.close()
            advance(10)
            break
        case 10:
            if (!w && destroyed === 2 && !listed()) {
                if (reclosedAt === 0)
                    reclosedAt = Date.now()
                if (Date.now() - reclosedAt >= 1000) {
                    Mirror.start(phone)
                    advance(11)
                }
            }
            break
        case 11:
            if (w && w !== first && w !== second && created === 3 && shows(w, 128, 128) && nativePath(w)) {
                third = w
                Mirror.toggleRecording(key)
                advance(12)
            }
            break
        case 12:
            if (w === third && w.info.recording === true && w.info.recordingStartedMs > 0
                    && w.info.notice === "Recording started")
                advance(13)
            break
        case 13:
            if (w === third && w.info.recording === true && w.recordingElapsedSecs >= 2 && shows(w, 128, 128))
                grab(w, "6-recording.png", 14)
            break
        case 14:
            Mirror.toggleRecording(key)
            advance(15)
            break
        case 15:
            if (w !== third)
                break
            if (w.info.recording !== true && w.recordingElapsedSecs === 0 && /^Saved .+\.mp4$/.test(w.info.notice)) {
                console.info("mirror-window-gate: recording stopped: " + w.info.notice)
                third.close()
                advance(16)
            } else if ((w.info.notice || "").length > 0 && w.info.notice !== "Recording started"
                       && !w.info.notice.startsWith("Saved ")) {
                fail(1, "the recording was not saved: " + w.info.notice)
            }
            break
        case 16:
            if (!w && destroyed === 3 && !listed())
                finish(0)
            break
        }
    }

    function fail(code, why) {
        if (finished)
            return
        console.error("mirror-window-gate: " + (code === 2 ? "BLOCKED" : "FAILED") + ": " + why)
        finish(code)
    }

    function finish(code) {
        finished = true
        ticker.stop()
        watchdog.stop()
        for (const k of Object.keys(windows))
            windows[k].close()
        Qt.exit(code)
    }

    Component.onCompleted: {
        Mirror.windowsChanged.connect(gate.sync)
        gate.sync()
    }
}
"#;

    #[cfg(test)]
    mod tests {
        use std::os::unix::fs::PermissionsExt;

        use super::{SENTINEL, validate_run_dir};

        #[test]
        fn the_gate_runs_only_in_a_fresh_sentinel_scratch_directory() {
            let scratch = tempfile::tempdir().expect("scratch root");
            let root = scratch.path().canonicalize().expect("canonical scratch root");
            let run = root.join("waypair-gate").join("run");
            std::fs::create_dir_all(&run).expect("run directory");
            std::fs::set_permissions(&run, std::fs::Permissions::from_mode(0o700)).expect("owner-only run");
            let sentinel = format!("{}\n", run.display());
            std::fs::write(run.join(SENTINEL), &sentinel).expect("sentinel");
            assert_eq!(validate_run_dir(Some(run.as_path()), &root), Ok(run.clone()));

            assert!(validate_run_dir(None, &root).is_err(), "a run directory is required");
            assert!(
                validate_run_dir(Some(root.join("waypair-gate").as_path()), &root).is_err(),
                "not the wave folder itself"
            );
            let other = root.join("other").join("run");
            std::fs::create_dir_all(&other).expect("other run");
            std::fs::write(other.join(SENTINEL), format!("{}\n", other.display())).expect("other sentinel");
            assert!(
                validate_run_dir(Some(other.as_path()), &root).is_err(),
                "only waypair-* scratch folders"
            );
            std::fs::write(run.join(SENTINEL), "/elsewhere\n").expect("foreign sentinel");
            assert!(validate_run_dir(Some(run.as_path()), &root).is_err(), "the sentinel names this run");
            std::fs::write(run.join(SENTINEL), &sentinel).expect("sentinel again");
            std::fs::create_dir(run.join("receiver")).expect("left over state");
            assert!(validate_run_dir(Some(run.as_path()), &root).is_err(), "a used run directory is refused");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        App, GamepadState, LEFT_THUMB_DEADZONE, RIGHT_THUMB_DEADZONE, TRIGGER_THRESHOLD, merge_recent_apps,
        normalize_gamepad_state, parse_dropped_file_urls, save_apps, saved_apps, saved_apps_path,
    };

    #[test]
    fn recent_apps_combines_saved_and_notification_packages_without_duplicates() {
        let saved = vec!["com.android.settings".to_owned(), "org.telegram.messenger".to_owned()];
        let active = vec![
            "org.telegram.messenger".to_owned(),
            "com.google.android.apps.messaging".to_owned(),
            String::new(),
        ];
        let history = vec!["com.google.android.apps.messaging".to_owned(), "com.spotify.music".to_owned()];
        assert_eq!(
            merge_recent_apps(&saved, active, history),
            vec![
                "com.android.settings".to_owned(),
                "org.telegram.messenger".to_owned(),
                "com.google.android.apps.messaging".to_owned(),
                "com.spotify.music".to_owned(),
            ]
        );
    }

    #[test]
    fn gamepad_deadzone_filters_idle_noise_and_converts_to_mirror_input() {
        let idle = normalize_gamepad_state(GamepadState {
            buttons: 0,
            lx: LEFT_THUMB_DEADZONE - 10,
            ly: -LEFT_THUMB_DEADZONE,
            rx: RIGHT_THUMB_DEADZONE,
            ry: -(RIGHT_THUMB_DEADZONE - 1),
            lt: TRIGGER_THRESHOLD,
            rt: 5,
        });
        assert_eq!(idle, GamepadState::default());

        let active = normalize_gamepad_state(GamepadState {
            buttons: 0x1001, // A + DPadUp
            lx: 16_000,
            ly: -24_000,
            rx: -12_000,
            ry: 30_000,
            lt: 128,
            rt: 255,
        });
        assert_eq!(
            active.to_mirror_input(),
            nectarlink_core::MirrorInput::Gamepad {
                buttons: 0x1001,
                lx: 16_000,
                ly: -24_000,
                rx: -12_000,
                ry: 30_000,
                lt: 128,
                rt: 255,
            }
        );
    }

    #[test]
    fn parse_dropped_file_urls_extracts_existing_paths_and_optional_window_key() {
        let dir = tempfile::tempdir().unwrap();
        let file_a = dir.path().join("alpha.txt");
        std::fs::write(&file_a, b"hello").unwrap();
        let file_url = format!("file:///{}", file_a.to_string_lossy().replace('\\', "/"));
        let dev = nectarlink_core::DeviceId([0x11; 32]);
        let key = format!("{dev}/0");

        let (win, paths) = parse_dropped_file_urls(&[key, file_url]);
        assert_eq!(win.map(|w| w.device), Some(dev));
        assert_eq!(paths, vec![file_a]);
    }

    #[test]
    fn a_phones_apps_are_kept_for_the_next_run_with_their_icons() {
        let dir = tempfile::tempdir().unwrap();
        let phone = nectarlink_core::DeviceId([0x22; 32]);
        assert!(saved_apps(dir.path(), &phone).is_none(), "nothing saved yet");

        let png = b"\x89PNG\r\n\x1a\nicon";
        let icon = crate::icons::save(dir.path(), "org.example.maps", png).unwrap();
        let apps = vec![
            App { pkg: "org.example.maps".into(), label: "Maps".into(), icon: Some(icon.clone()) },
            App { pkg: "org.example.notes".into(), label: "Notes".into(), icon: None },
        ];
        save_apps(dir.path(), &phone, &apps);
        assert_eq!(saved_apps(dir.path(), &phone), Some(apps));

        // Another phone has its own list; a damaged file is no list.
        assert!(saved_apps(dir.path(), &nectarlink_core::DeviceId([0x33; 32])).is_none());
        std::fs::write(saved_apps_path(dir.path(), &phone), b"{not json").unwrap();
        assert!(saved_apps(dir.path(), &phone).is_none());
    }
}

/// Mac view-only mirror fixtures use the real production sink, VideoToolbox
/// decode thread, and `ffi::video_frame` publication. The composed test below
/// sends a committed H.264 frame through two authenticated Nodes over 127.0.0.1.
/// Its phone platform only auto-approves the screen request; it does not capture
/// a screen, app window, microphone, or personal device.
///
/// Runs on macOS with the nondefault `test-support` feature.
#[cfg(all(test, target_os = "macos"))]
mod mac_receive_fixture {
    use std::{
        sync::{Mutex, OnceLock},
        time::{Duration, Instant},
    };

    use nectarlink_core::{DeviceId, MIRROR_SCREEN, MirrorConfig, MirrorInput};
    #[cfg(feature = "test-support")]
    use nectarlink_core::{
        DeviceKind, MIRROR_CAPTURE, MIRROR_CORRELATION, MIRROR_VIEW, MirrorSend, MirrorStart, Node,
        NodeEvent, PacketKind, Platform,
    };
    #[cfg(feature = "test-support")]
    use std::sync::Arc;

    #[cfg(feature = "test-support")]
    use super::window_gate::{loopback_config as fixture_node_config, wait_for_link};
    use super::{Phase, Shown, Window};

    /// The two receive fixtures share the production mirror state and global
    /// core-host seam, so serialize them and start that host at most once.
    static FIXTURE_LOCK: Mutex<()> = Mutex::new(());
    static FIXTURE_HOST: OnceLock<()> = OnceLock::new();

    fn fixture_host() {
        FIXTURE_HOST.get_or_init(|| {
            let directory = tempfile::tempdir().expect("disposable core-host state");
            crate::core_host::start_receiver_fixture(directory.path().to_path_buf());
        });
    }

    /// The received-folder override is a once-per-process `OnceLock`, so its
    /// fresh 0700 run directory is held here for the whole process: it outlives
    /// every async `finish`, and no later test can see it removed.
    #[cfg(feature = "test-support")]
    static FIXTURE_RECEIVED: OnceLock<tempfile::TempDir> = OnceLock::new();

    #[cfg(feature = "test-support")]
    fn fixture_received_dir() {
        use std::os::unix::fs::PermissionsExt;

        use super::window_gate::{SENTINEL, validate_run_dir};

        FIXTURE_RECEIVED.get_or_init(|| {
            let scratch = tempfile::tempdir().expect("scratch root");
            let root = scratch.path().canonicalize().expect("canonical scratch root");
            let run = root.join("waypair-recording").join("run");
            std::fs::create_dir_all(&run).expect("run directory");
            std::fs::set_permissions(&run, std::fs::Permissions::from_mode(0o700)).expect("owner-only run");
            std::fs::write(run.join(SENTINEL), format!("{}\n", run.display())).expect("sentinel");
            let run = validate_run_dir(Some(run.as_path()), &root).expect("a fresh run directory");
            let mode = std::fs::symlink_metadata(&run).expect("run directory").permissions().mode();
            assert_eq!(mode & 0o777, 0o700, "the run directory is owner-only");
            let received = run.join("received");
            std::fs::create_dir(&received).expect("a fresh received folder");
            crate::downloads::use_window_gate_received_dir(received)
                .expect("no other test in this process chose the received folder");
            scratch
        });
    }

    /// The committed synthetic keyframe: solid red 128x128 baseline IDR.
    fn fixture_idr() -> Vec<u8> {
        include_bytes!("mac/testdata/red-128x128.annexb").to_vec()
    }

    fn fixture_config(width: u32, height: u32, generation: u64) -> MirrorConfig {
        MirrorConfig {
            codec: "h264".into(),
            width,
            height,
            session: MIRROR_SCREEN,
            request: Some(generation),
        }
    }

    /// Records a live screen request owner, as `request()` does once the
    /// node exists: the window reads Showing, as if the core had emitted
    /// `Mirroring { on: true }` for it.
    fn live_screen_request(device: DeviceId, generation: u64) -> Window {
        let window = Window::screen(device);
        super::state(|s| {
            s.windows.insert(window, Shown { phase: Phase::Showing, app: None, pkg: None });
            s.requests.insert(window, generation);
        });
        window
    }

    /// The latest published frame for `window`, waiting up to `timeout`
    /// for the decode thread (VideoToolbox, milliseconds in practice).
    fn await_bitmap(
        window: Window,
        width: u32,
        height: u32,
        timeout: Duration,
    ) -> Option<crate::win::image::Bitmap> {
        let started = Instant::now();
        while started.elapsed() < timeout {
            if let Some(bitmap) = super::latest_bitmap(&window)
                && bitmap.width == width
                && bitmap.height == height
            {
                return Some(bitmap);
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        super::latest_bitmap(&window).filter(|b| b.width == width && b.height == height)
    }

    fn assert_solid_red(bgra: &[u8]) {
        assert!(!bgra.is_empty() && bgra.len().is_multiple_of(4));
        for px in bgra.as_chunks::<4>().0.iter().step_by(257) {
            assert!(px[0] <= 10 && px[1] <= 10 && px[2] >= 244, "expected the red fixture frame, got {px:?}");
        }
    }

    fn drop_fixture_window(window: Window) {
        super::state(|s| {
            s.windows.remove(&window);
            s.requests.remove(&window);
            s.streams.remove(&window);
            s.pending_stops.remove(&window);
        });
        super::clear_window_buffers(&window);
        super::ffi::video_clear(&window.key());
    }

    #[cfg(feature = "test-support")]
    #[derive(Debug, Default)]
    struct NativeReceiverPlatform {
        sink_requests: Mutex<Vec<(DeviceId, u32)>>,
    }

    #[cfg(feature = "test-support")]
    impl Platform for NativeReceiverPlatform {
        fn mirror_sink(&self, peer: &DeviceId, session: u32) -> Option<Arc<dyn nectarlink_core::MirrorSink>> {
            self.sink_requests.lock().unwrap_or_else(|e| e.into_inner()).push((*peer, session));
            super::sink(peer, session)
        }
    }

    #[cfg(feature = "test-support")]
    #[derive(Debug)]
    struct SyntheticPhonePlatform {
        requests: tokio::sync::mpsc::UnboundedSender<(DeviceId, MirrorStart)>,
    }

    #[cfg(feature = "test-support")]
    impl Platform for SyntheticPhonePlatform {
        fn mirror_requested(&self, peer: &DeviceId, options: &MirrorStart) -> Result<(), String> {
            // Synthetic approval only: no capture, microphone, or OS prompt.
            self.requests.send((*peer, options.clone())).map_err(|_| "fixture receiver ended".into())
        }
    }

    #[cfg(feature = "test-support")]
    async fn wait_for_mirroring_event(
        events: &mut tokio::sync::broadcast::Receiver<NodeEvent>,
        phone: DeviceId,
        on: bool,
    ) -> Result<(), String> {
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                let event = events.recv().await.map_err(|error| error.to_string())?;
                if matches!(event, NodeEvent::Mirroring { device, session: MIRROR_SCREEN, on: event_on }
                    if device == phone && event_on == on)
                {
                    return Ok::<(), String>(());
                }
            }
        })
        .await
        .map_err(|_| format!("core did not emit mirror state on={on} within 5 seconds"))?
    }

    #[cfg(feature = "test-support")]
    async fn wait_for_frame_clear(window: Window) -> Result<(), String> {
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                if super::latest_bitmap(&window).is_none() {
                    return;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .map_err(|_| "native video frame was not cleared when the authenticated stream ended".to_owned())
    }

    #[cfg(feature = "test-support")]
    async fn wait_for_frame(window: Window) -> Option<crate::win::image::Bitmap> {
        tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                if let Some(frame) = super::latest_bitmap(&window)
                    && frame.width == 128
                    && frame.height == 128
                {
                    return Some(frame);
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .ok()
        .flatten()
    }

    #[cfg(feature = "test-support")]
    async fn exercise_authenticated_stream(
        receiver: &Node,
        phone: &Node,
        receiver_platform: &NativeReceiverPlatform,
        mut requests: tokio::sync::mpsc::UnboundedReceiver<(DeviceId, MirrorStart)>,
    ) -> Result<(), String> {
        let receiver_id = receiver.device_id();
        let phone_id = phone.device_id();
        wait_for_link(receiver, phone_id).await?;

        let mut events = receiver.events();
        tokio::time::timeout(Duration::from_secs(10), receiver.mirror_start(phone_id, super::OPTIONS))
            .await
            .map_err(|_| "real Node::mirror_start exceeded 10 seconds".to_owned())?
            .map_err(|error| format!("real Node::mirror_start failed: {error}"))?;
        let (asked_by, options) = tokio::time::timeout(Duration::from_secs(5), requests.recv())
            .await
            .map_err(|_| "phone platform did not receive the real mirror.start".to_owned())?
            .ok_or_else(|| "synthetic phone platform closed before mirror.start".to_owned())?;
        if asked_by != receiver_id {
            return Err("phone received mirror.start from a different authenticated peer".into());
        }
        if options.session != MIRROR_SCREEN || options.audio || options.app.is_some() {
            return Err("fixture request must be screen-only, video-only, and app-free".into());
        }
        let core_request = options.request.ok_or_else(|| "core did not stamp mirror.start".to_owned())?;

        // This is the app-owned request state that the production request()
        // path records before invoking Node::mirror_start. It lets the real
        // Platform::mirror_sink callback select production mirror::sink().
        let window = Window::screen(phone_id);
        let app_request = core_request ^ 0x9E37_79B9_7F4A_7C15;
        super::state(|state| {
            state.windows.insert(window, Shown { phase: Phase::Asking, app: None, pkg: None });
            state.requests.insert(window, app_request);
        });

        let stream = Arc::new(
            tokio::time::timeout(Duration::from_secs(10), phone.mirror_open(receiver_id))
                .await
                .map_err(|_| "real sender Node::mirror_open exceeded 10 seconds".to_owned())?
                .map_err(|error| format!("real sender Node::mirror_open failed: {error}"))?,
        );
        let send_result = async {
            let config = MirrorConfig {
                codec: "h264".into(),
                width: 128,
                height: 128,
                session: MIRROR_SCREEN,
                request: Some(core_request),
            };
            let config_stream = stream.clone();
            let config_sent = tokio::time::timeout(
                Duration::from_secs(5),
                tokio::task::spawn_blocking(move || {
                    config_stream.send(PacketKind::Config, 0, config.to_cbor())
                }),
            )
            .await
            .map_err(|_| "sending mirror config exceeded 5 seconds".to_owned())?
            .map_err(|error| format!("sending mirror config panicked: {error}"))?;
            if config_sent != MirrorSend::Queued {
                return Err(format!("real mirror config was not queued: {config_sent:?}"));
            }
            let packet_sent = stream.send(PacketKind::Keyframe, 0, fixture_idr());
            if packet_sent != MirrorSend::Queued {
                return Err(format!("real H.264 keyframe was not queued: {packet_sent:?}"));
            }

            wait_for_mirroring_event(&mut events, phone_id, true).await?;
            let frame = wait_for_frame(window).await.ok_or_else(|| {
                "production VideoToolbox path did not publish 128x128 within 10 seconds".to_owned()
            })?;
            let red = frame
                .bgra
                .as_chunks::<4>()
                .0
                .iter()
                .all(|pixel| pixel[0] <= 10 && pixel[1] <= 10 && pixel[2] >= 244);
            if !red {
                return Err("production VideoToolbox output did not match the committed red keyframe".into());
            }

            let routed = receiver_platform
                .sink_requests
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .contains(&(phone_id, MIRROR_SCREEN));
            if !routed {
                return Err("core did not route the authenticated peer to Platform::mirror_sink".into());
            }
            Ok::<(), String>(())
        }
        .await;

        // Closing the real stream makes core::mirror::receive call the
        // production sink's ended() method; the decoder then clears VideoView.
        stream.close();
        send_result?;
        wait_for_mirroring_event(&mut events, phone_id, false).await?;
        wait_for_frame_clear(window).await
    }

    #[test]
    fn mac_view_only_receive_shows_decoded_frame_and_cancels() {
        let _fixture = FIXTURE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let _ = tracing_subscriber::fmt().with_max_level(tracing::Level::DEBUG).with_test_writer().try_init();
        fixture_host();
        // The Mac receive seam is present; capture and audio stay refused.
        const {
            assert!(super::VIEW_ONLY, "mac stays view-only until native acceptance");
            assert!(!super::OPTIONS.audio, "mac never asks for the phone's sound");
        }

        // View-only gates: no app windows, no input, no control requests.
        let device = DeviceId([0x4D; 32]);
        super::start_app(device, "com.example.app".into(), "App".into());
        assert!(super::windows().iter().all(|(w, _)| w.session == MIRROR_SCREEN), "mac opens no app windows");
        let window = live_screen_request(device, 0x9E3779B9_7F4A7C15);
        super::input(window, MirrorInput::Key { key: "a".into() });
        super::request_control(&window.key());

        // Only a live screen request owns a sink (the core's `receive` asks
        // for exactly this before handing over the authenticated stream).
        let sink = super::sink(&device, MIRROR_SCREEN).expect("live screen request owns a sink");
        assert!(super::sink(&device, 7).is_none(), "mac serves no app sessions");
        assert!(super::sink(&DeviceId([0x00; 32]), MIRROR_SCREEN).is_none(), "unknown devices serve nothing");

        // Config + keyframe: the real decoder publishes the red fixture to
        // the production VideoView path (`ffi::video_frame` + latest frame).
        let idr = fixture_idr();
        sink.config(fixture_config(128, 128, 0x9E3779B9_7F4A7C15));
        sink.packet(true, 0, idr.clone());
        let frame = await_bitmap(window, 128, 128, Duration::from_secs(10))
            .expect("decoder publishes the fixture frame");
        assert_solid_red(&frame.bgra);
        let dims = super::LAST_CONFIG
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_ref()
            .and_then(|m| m.get(&window).copied());
        assert_eq!(dims, Some((128, 128)), "rotation seam tracks the live format");

        // Rotation re-negotiation on the same stream: the decoder survives
        // the new format and keeps publishing.
        if let Some(frames) = super::FRAMES.lock().unwrap_or_else(|e| e.into_inner()).as_mut() {
            frames.remove(&window);
        }
        sink.config(fixture_config(128, 128, 0x9E3779B9_7F4A7C15));
        sink.packet(true, 33_333, idr.clone());
        let rotated =
            await_bitmap(window, 128, 128, Duration::from_secs(10)).expect("stream survives re-negotiation");
        assert_solid_red(&rotated.bgra);

        // Rotation size change: flush, then the scaled picture publishes.
        sink.config(fixture_config(192, 128, 0x9E3779B9_7F4A7C15));
        sink.packet(true, 66_666, idr.clone());
        let resized = await_bitmap(window, 192, 128, Duration::from_secs(10))
            .expect("resized rotation frame publishes");
        assert_solid_red(&resized.bgra);

        // Close: `ended` with a current stream clears its buffers and blanks
        // the VideoView (`video_clear` path); the sink then serves nothing.
        sink.ended();
        let cleared = Instant::now();
        while super::latest_bitmap(&window).is_some() && cleared.elapsed() < Duration::from_secs(10) {
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(super::latest_bitmap(&window).is_none(), "ended stream clears its frame");
        drop_fixture_window(window);
        assert!(super::sink(&device, MIRROR_SCREEN).is_none(), "closed request owns no sink");

        // Replacement: a stale stream must neither publish over nor clear
        // its successor. The old thread breaks on ownership; the 192x128
        // successor frame stays exactly as published.
        let device_b = DeviceId([0x4E; 32]);
        let window_b = live_screen_request(device_b, 0xABCDEF01_23456789);
        let old = super::sink(&device_b, MIRROR_SCREEN).expect("old stream owns a sink");
        old.config(fixture_config(128, 128, 0xABCDEF01_23456789));
        old.packet(true, 0, idr.clone());
        await_bitmap(window_b, 128, 128, Duration::from_secs(10)).expect("old stream shows first");
        let old_stream = super::state(|s| s.streams[&window_b]);
        super::state(|s| {
            s.requests.insert(window_b, 0xFEDCBA98_76543210);
        });
        let next = super::sink(&device_b, MIRROR_SCREEN).expect("replacement owns the sink");
        next.config(fixture_config(192, 128, 0xFEDCBA98_76543210));
        next.packet(true, 1, idr.clone());
        let successor =
            await_bitmap(window_b, 192, 128, Duration::from_secs(10)).expect("replacement stream shows");
        old.packet(true, 99_999, idr.clone());
        old.ended();
        next.packet(true, 2, idr.clone());
        let ended = Instant::now();
        while !super::FINISHED_DECODERS
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_ref()
            .is_some_and(|ids| ids.contains(&old_stream))
        {
            assert!(ended.elapsed() < Duration::from_secs(10), "stale decoder must exit");
            std::thread::sleep(Duration::from_millis(10));
        }
        let kept = super::latest_bitmap(&window_b).expect("stale stream must not clear its successor");
        assert_eq!((kept.width, kept.height), (192, 128), "successor format survives the stale stream");
        assert_eq!(kept.bgra, successor.bgra, "successor pixels survive the stale stream");
        next.ended();
        drop_fixture_window(window_b);
    }

    #[test]
    fn production_sink_recording_writes_video_only_mp4() {
        use std::collections::HashMap;
        use std::sync::atomic::Ordering;

        let _fixture = FIXTURE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let _ = tracing_subscriber::fmt().with_max_level(tracing::Level::DEBUG).with_test_writer().try_init();
        fixture_host();
        const {
            assert!(super::VIEW_ONLY, "mac stays view-only until native acceptance");
            assert!(!super::OPTIONS.audio, "mac never asks for the phone's sound");
        }

        // Distinct device so global mirror state from the other fixtures cannot leak in.
        let device = DeviceId([0x52; 32]);
        let generation = 0x9E37_79B9_7F4A_7C16u64;
        let window = live_screen_request(device, generation);
        assert_eq!(super::phase(&window), Some(Phase::Showing), "recording begins while live");
        assert!(!super::is_recording(&window), "no recording before begin");

        // Production sink: the same `mirror::sink()` the core's `receive` selects.
        let sink = super::sink(&device, MIRROR_SCREEN).expect("live screen request owns a sink");
        let idr = fixture_idr();
        sink.config(fixture_config(128, 128, generation));
        sink.packet(true, 0, idr.clone());
        let live = await_bitmap(window, 128, 128, Duration::from_secs(10))
            .expect("decoder publishes the live fixture frame before recording");
        assert_solid_red(&live.bgra);

        // Disposable caller-owned output: temp dir, no real Downloads pollution.
        // A sibling owner file proves finish/cleanup never touches unowned paths.
        let dir = tempfile::tempdir().expect("disposable recording dir");
        let owner = dir.path().join("owner.txt");
        std::fs::write(&owner, b"owner-bytes").expect("sibling owner file");
        let path = dir.path().join("Mirror recording.mp4");
        assert!(!path.exists(), "disposable path starts free");

        // Same dims + seed the production toggle uses, without touching real Downloads/toast.
        let (width, height) = super::LAST_CONFIG
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_ref()
            .and_then(|m| m.get(&window).copied())
            .unwrap_or((128, 128));
        assert_eq!((width, height), (128, 128), "live format is the recording format");
        let mut seeded =
            crate::win::mp4::MirrorRecorder::new(path.clone(), width, height, super::OPTIONS.fps, None);
        assert!(seeded.on_audio(0, &[0u8; 480]).is_err(), "video-only recorder refuses PCM");
        if let Some(sps_pps) = super::LAST_SPS_PPS
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_ref()
            .and_then(|m| m.get(&window).cloned())
        {
            seeded.set_sps_pps(&sps_pps);
        }
        {
            let mut guard = super::RECORDERS.lock().unwrap_or_else(|e| e.into_inner());
            let map = guard.get_or_insert_with(HashMap::new);
            map.insert(window, seeded);
            super::ACTIVE_RECORDERS.store(map.len(), Ordering::Release);
        }
        assert!(super::is_recording(&window), "begin while live is visible to QML");
        assert!(super::recording_started_ms(&window) > 0, "QML elapsed timer has a start");

        // Synthetic H.264 through the existing record callbacks: config/keys to both
        // decoder and recorder. Bounded: three more IDRs (four total with live).
        sink.packet(true, 33_333, idr.clone());
        sink.packet(true, 66_666, idr.clone());
        sink.packet(true, 100_000, idr.clone());

        // Current frames survive recording: the VideoToolbox path still publishes.
        let still = await_bitmap(window, 128, 128, Duration::from_secs(10))
            .expect("live frame survives recording packets");
        assert_solid_red(&still.bgra);
        assert!(!super::has_sound(&device), "video-only fixture opens no audio output");

        // Stop/close like `stop_recording` but synchronously for a deterministic
        // assertion (production wraps the same remove + `finish` in spawn_blocking).
        let recorder = {
            let mut guard = super::RECORDERS.lock().unwrap_or_else(|e| e.into_inner());
            let rec = guard.as_mut().and_then(|m| m.remove(&window));
            super::ACTIVE_RECORDERS.store(guard.as_ref().map_or(0, HashMap::len), Ordering::Release);
            rec
        }
        .expect("recorder still owned at stop");
        assert!(!super::is_recording(&window), "stop clears the QML flag");
        let saved = recorder.finish().expect("stop writes a valid MP4");
        assert_eq!(saved, path);
        assert!(saved.is_file(), "native Open/Reveal have an existing file target");
        let bytes = std::fs::read(&saved).expect("finished MP4 is readable");
        assert!(bytes.len() > 256, "unexpectedly small MP4: {} B", bytes.len());
        assert!(bytes.len() < 8_000_000, "bounded fixture must stay small: {} B", bytes.len());
        fn contains(hay: &[u8], needle: &[u8]) -> bool {
            hay.windows(needle.len()).any(|w| w == needle)
        }
        assert_eq!(&bytes[4..8], b"ftyp", "MP4 starts with ftyp");
        assert!(contains(&bytes, b"moov"), "missing moov box");
        assert!(contains(&bytes, b"mdat"), "missing mdat box");
        assert!(contains(&bytes, b"avc1"), "missing avc1 sample entry");
        assert!(contains(&bytes, b"avcC"), "missing avcC config box");
        assert!(!contains(&bytes, b"soun"), "video-only file must not carry an audio track");
        // The finalized description must carry the real stream sets, not a fallback.
        {
            use crate::win::mp4::{nal_type, split_annexb};
            let mut sps = None;
            let mut pps = None;
            for nal in split_annexb(&idr) {
                match nal_type(nal) {
                    Some(7) => sps = Some(nal.to_vec()),
                    Some(8) => pps = Some(nal.to_vec()),
                    _ => {}
                }
            }
            let sps = sps.expect("fixture carries SPS");
            let pps = pps.expect("fixture carries PPS");
            assert!(contains(&bytes, &sps), "moov must carry the live SPS");
            assert!(contains(&bytes, &pps), "moov must carry the live PPS");
        }
        assert_eq!(
            std::fs::read(&owner).expect("owner still readable"),
            b"owner-bytes".to_vec(),
            "finish must not touch unowned sibling files"
        );

        // Close the live stream through the production `ended` path; the finished
        // file stays (owned handle already consumed), the live frame clears.
        sink.ended();
        let cleared = std::time::Instant::now();
        while super::latest_bitmap(&window).is_some() && cleared.elapsed() < Duration::from_secs(10) {
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(super::latest_bitmap(&window).is_none(), "ended stream clears its frame");
        assert!(saved.is_file(), "finished MP4 survives stream close");
        drop_fixture_window(window);
        assert!(super::sink(&device, MIRROR_SCREEN).is_none(), "closed request owns no sink");
    }

    /// A phone rotation while recording: the production sink removes that
    /// recorder and saves it through the production stop path, which keeps
    /// the frames recorded before the change and reports the usual "Saved".
    #[cfg(feature = "test-support")]
    #[test]
    fn production_sink_format_change_saves_recording_and_stops() {
        use std::collections::HashMap;
        use std::sync::atomic::Ordering;

        let _fixture = FIXTURE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        fixture_host();
        fixture_received_dir();
        assert_eq!(
            crate::downloads::user_visible_location(),
            "the gate's run directory",
            "the stop path's label must not probe the user's Downloads"
        );

        let device = DeviceId([0x53; 32]);
        let generation = 0x6A09_E667_F3BC_C908u64;
        let window = live_screen_request(device, generation);
        let sink = super::sink(&device, MIRROR_SCREEN).expect("live screen request owns a sink");
        let idr = fixture_idr();
        let rotated = include_bytes!("mac/testdata/red-32x32.annexb").to_vec();
        sink.config(fixture_config(128, 128, generation));
        sink.packet(true, 0, idr.clone());

        let dir = tempfile::tempdir().expect("disposable recording dir");
        let path = dir.path().join("Mirror rotation.mp4");
        let mut recorder =
            crate::win::mp4::MirrorRecorder::new(path.clone(), 128, 128, super::OPTIONS.fps, None);
        if let Some(sps_pps) = super::LAST_SPS_PPS
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_ref()
            .and_then(|m| m.get(&window).cloned())
        {
            recorder.set_sps_pps(&sps_pps);
        }
        {
            let mut guard = super::RECORDERS.lock().unwrap_or_else(|e| e.into_inner());
            let map = guard.get_or_insert_with(HashMap::new);
            map.insert(window, recorder);
            super::ACTIVE_RECORDERS.store(map.len(), Ordering::Release);
        }
        sink.packet(true, 33_333, idr.clone());
        sink.packet(true, 66_666, idr.clone());
        assert!(super::is_recording(&window), "same-format packets keep recording");

        // Rotation: a new size, then a keyframe encoded with a new SPS.
        sink.config(fixture_config(32, 32, generation));
        assert!(!super::is_recording(&window), "a format change stops the recording");
        sink.packet(true, 100_000, rotated.clone());

        let notice = "Saved Mirror rotation.mp4";
        let started = Instant::now();
        while super::last_notice(&window) != notice && started.elapsed() < Duration::from_secs(10) {
            std::thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(super::last_notice(&window), notice, "the stop reports the saved file");

        let bytes = std::fs::read(&path).expect("the recording before the change is saved");
        fn contains(hay: &[u8], needle: &[u8]) -> bool {
            hay.windows(needle.len()).any(|w| w == needle)
        }
        fn sps(annexb: &[u8]) -> Vec<u8> {
            use crate::win::mp4::{nal_type, split_annexb};
            split_annexb(annexb).into_iter().find(|nal| nal_type(nal) == Some(7)).expect("SPS").to_vec()
        }
        assert_eq!(&bytes[4..8], b"ftyp");
        assert!(contains(&bytes, b"moov"), "the saved file is finalized");
        assert!(!contains(&bytes, b"soun"), "video-only file must not carry an audio track");
        assert!(contains(&bytes, &sps(&idr)), "avcC keeps the recorded SPS");
        assert!(!contains(&bytes, &sps(&rotated)), "the rotated stream is not muxed in");
        let avc1 = bytes.windows(4).rposition(|w| w == b"avc1").expect("avc1 sample entry") + 4;
        assert_eq!(&bytes[avc1 + 24..avc1 + 28], &[0u8, 128, 0, 128], "avc1 keeps 128x128");
        let stsz = bytes.windows(4).rposition(|w| w == b"stsz").expect("stsz") + 4;
        assert_eq!(bytes[stsz + 8..stsz + 12], 2u32.to_be_bytes(), "the two pre-rotation frames");
        assert_eq!(std::fs::read_dir(dir.path()).expect("recording dir").count(), 1, "one file, no partials");

        sink.ended();
        drop_fixture_window(window);
        assert!(path.is_file(), "the saved recording survives the stream closing");
    }

    /// A recording that reaches its length limit: the production sink removes
    /// that recorder, so QML's recording flag and timer clear, and saves the
    /// frames recorded before the limit with a notice that says why it stopped.
    #[cfg(feature = "test-support")]
    #[test]
    fn production_sink_limit_saves_recording_and_stops() {
        use std::collections::HashMap;
        use std::sync::atomic::Ordering;

        let _fixture = FIXTURE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        fixture_host();
        fixture_received_dir();

        let device = DeviceId([0x54; 32]);
        let generation = 0x3C6E_F372_FE94_F82Bu64;
        let window = live_screen_request(device, generation);
        let sink = super::sink(&device, MIRROR_SCREEN).expect("live screen request owns a sink");
        let idr = fixture_idr();
        sink.config(fixture_config(128, 128, generation));
        sink.packet(true, 0, idr.clone());

        let dir = tempfile::tempdir().expect("disposable recording dir");
        let path = dir.path().join("Mirror limit.mp4");
        let mut recorder =
            crate::win::mp4::MirrorRecorder::new(path.clone(), 128, 128, super::OPTIONS.fps, None);
        if let Some(sps_pps) = super::LAST_SPS_PPS
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_ref()
            .and_then(|m| m.get(&window).cloned())
        {
            recorder.set_sps_pps(&sps_pps);
        }
        {
            let mut guard = super::RECORDERS.lock().unwrap_or_else(|e| e.into_inner());
            let map = guard.get_or_insert_with(HashMap::new);
            map.insert(window, recorder);
            super::ACTIVE_RECORDERS.store(map.len(), Ordering::Release);
        }
        sink.packet(true, 33_333, idr.clone());
        assert!(super::is_recording(&window), "a frame within the limit keeps recording");

        // Four hours after the first recorded frame: past the three-hour limit.
        sink.packet(true, 33_333 + 4 * 3_600 * 1_000_000, idr.clone());
        assert!(!super::is_recording(&window), "the limit stops the recording");
        assert_eq!(super::recording_started_ms(&window), 0, "QML's elapsed timer has no start");

        let notice = "Recording limit reached. Saved Mirror limit.mp4";
        let started = Instant::now();
        while super::last_notice(&window) != notice && started.elapsed() < Duration::from_secs(10) {
            std::thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(super::last_notice(&window), notice, "the stop says why and reports the saved file");

        let bytes = std::fs::read(&path).expect("the recording before the limit is saved");
        fn contains(hay: &[u8], needle: &[u8]) -> bool {
            hay.windows(needle.len()).any(|w| w == needle)
        }
        assert_eq!(&bytes[4..8], b"ftyp");
        assert!(contains(&bytes, b"moov"), "the saved file is finalized");
        assert!(!contains(&bytes, b"soun"), "video-only file must not carry an audio track");
        let stsz = bytes.windows(4).rposition(|w| w == b"stsz").expect("stsz") + 4;
        assert_eq!(bytes[stsz + 8..stsz + 12], 1u32.to_be_bytes(), "the one frame before the limit");

        sink.ended();
        drop_fixture_window(window);
    }

    #[cfg(feature = "test-support")]
    #[test]
    fn transport_to_video_frame_uses_two_authenticated_nodes() {
        let _fixture = FIXTURE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let _ = tracing_subscriber::fmt().with_max_level(tracing::Level::DEBUG).with_test_writer().try_init();
        fixture_host();
        const {
            assert!(super::VIEW_ONLY);
            assert!(!super::OPTIONS.audio);
        }

        let receiver_dir = tempfile::tempdir().expect("disposable receiver node state");
        let phone_dir = tempfile::tempdir().expect("disposable phone node state");
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .expect("fixture Tokio runtime");
        runtime.block_on(async {
            let (request_tx, request_rx) = tokio::sync::mpsc::unbounded_channel();
            let receiver_platform = Arc::new(NativeReceiverPlatform::default());
            let phone_platform = Arc::new(SyntheticPhonePlatform { requests: request_tx });
            let receiver_config = fixture_node_config(
                receiver_dir.path(),
                "Synthetic Mac",
                DeviceKind::Desktop,
                &[MIRROR_VIEW, MIRROR_CORRELATION],
            );
            let phone_config = fixture_node_config(
                phone_dir.path(),
                "Synthetic Phone",
                DeviceKind::Phone,
                &[MIRROR_CAPTURE, MIRROR_CORRELATION],
            );
            let receiver = tokio::time::timeout(
                Duration::from_secs(10),
                Node::start(receiver_config, receiver_platform.clone()),
            )
            .await
            .expect("synthetic receiver node starts within 10 seconds")
            .expect("synthetic receiver node starts");
            let phone = match tokio::time::timeout(
                Duration::from_secs(10),
                Node::start(phone_config, phone_platform),
            )
            .await
            {
                Ok(Ok(phone)) => phone,
                Ok(Err(error)) => {
                    let _ = tokio::time::timeout(Duration::from_secs(5), receiver.shutdown()).await;
                    panic!("synthetic phone node starts: {error}");
                }
                Err(_) => {
                    let _ = tokio::time::timeout(Duration::from_secs(5), receiver.shutdown()).await;
                    panic!("synthetic phone node starts within 10 seconds");
                }
            };

            let result = async {
                let invitation = tokio::time::timeout(Duration::from_secs(10), receiver.pairing_start_qr())
                    .await
                    .map_err(|_| "receiver pairing invitation exceeded 10 seconds".to_owned())?
                    .map_err(|error| format!("receiver pairing invitation failed: {error}"))?;
                if invitation.addrs.is_empty()
                    || !invitation.addrs.iter().all(|address| address.ip().is_loopback())
                {
                    return Err("loopback-bound fixture advertised a non-loopback or missing address".into());
                }
                let pairing_uri = invitation.to_uri();
                tokio::time::timeout(Duration::from_secs(10), phone.pairing_join(&pairing_uri))
                    .await
                    .map_err(|_| "synthetic loopback pairing exceeded 10 seconds".to_owned())?
                    .map_err(|error| format!("synthetic loopback pairing failed: {error}"))?;
                exercise_authenticated_stream(&receiver, &phone, &receiver_platform, request_rx).await
            }
            .await;

            let phone_shutdown = tokio::time::timeout(Duration::from_secs(5), phone.shutdown()).await.is_ok();
            let receiver_shutdown =
                tokio::time::timeout(Duration::from_secs(5), receiver.shutdown()).await.is_ok();
            drop_fixture_window(Window::screen(phone.device_id()));
            assert!(phone_shutdown, "synthetic phone node shutdown exceeded 5 seconds");
            assert!(receiver_shutdown, "synthetic receiver node shutdown exceeded 5 seconds");
            assert!(result.is_ok(), "{}", result.unwrap_err());
        });
    }
}
