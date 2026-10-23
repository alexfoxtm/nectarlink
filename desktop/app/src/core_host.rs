// SPDX-License-Identifier: GPL-3.0-or-later
//! Runs `nectarlink-core` beside Qt: a small Tokio runtime owns the [`Node`],
//! core events are folded into the [`Hub`], and Qt objects issue commands
//! through [`spawn`]. The UI starts before the node, which comes up in the
//! background (startup is never blocked on the network).

use crate::i18n::tr;
use std::{
    future::Future,
    path::PathBuf,
    sync::{Arc, OnceLock},
    time::Duration,
};

use nectarlink_core::{DeviceInfo, DeviceKind, LinkState, Node, NodeConfig, Platform};
use tokio::{
    runtime::Runtime,
    sync::{Notify, broadcast::error::RecvError},
};

use crate::state::{Changes, CoreStatus, Hub};

#[cfg(target_os = "macos")]
#[path = "mac/adb_bootstrap.rs"]
mod adb_bootstrap;

const SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(2);

#[derive(Debug)]
pub struct CoreHost {
    pub(crate) runtime: Runtime,
    node: OnceLock<Node>,
    /// Signaled once the node is up, or has failed to start.
    started: Notify,
    pub hub: Arc<Hub>,
    pub data_dir: PathBuf,
}

static HOST: OnceLock<CoreHost> = OnceLock::new();

/// The running host. Panics if called before [`start`].
pub fn host() -> &'static CoreHost {
    HOST.get().expect("core host is started in main before Qt")
}

/// The running host's data directory, if the host has been started.
#[allow(dead_code)]
pub fn try_data_dir() -> Option<&'static std::path::Path> {
    HOST.get().map(|h| h.data_dir.as_path())
}

/// The node, once it has started.
pub fn node() -> Option<Node> {
    HOST.get()?.node.get().cloned()
}

/// The node once it has started, or `None` if it couldn't.
pub async fn wait_for_node() -> Option<Node> {
    let host = host();
    loop {
        // Created before checking, so a signal in between isn't missed.
        let started = host.started.notified();
        if let Some(node) = node() {
            return Some(node);
        }
        if host.hub.read(|s| matches!(s.status, Some(CoreStatus::Failed(_)))) {
            return None;
        }
        started.await;
    }
}

/// Runs a command on the core's runtime.
pub fn spawn(fut: impl Future<Output = ()> + Send + 'static) {
    host().runtime.spawn(fut);
}

/// Creates the runtime and starts the node in the background.
pub fn start(data_dir: PathBuf, platform: Arc<dyn Platform>) -> std::io::Result<&'static CoreHost> {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .thread_name("nectarlink-core")
        .enable_all()
        .build()?;
    let host = CoreHost {
        runtime,
        node: OnceLock::new(),
        started: Notify::new(),
        hub: Hub::new(),
        data_dir: data_dir.clone(),
    };
    if HOST.set(host).is_err() {
        panic!("core host started twice");
    }
    let host = self::host();
    host.hub.update(|s| {
        s.status = Some(CoreStatus::Starting);
        Changes::STATUS
    });
    host.runtime.spawn(run(data_dir, platform));
    Ok(host)
}

/// In-memory host for code-only receiver tests: no Node, socket, identity or peer is started.
#[cfg(all(test, target_os = "macos"))]
pub(crate) fn start_receiver_fixture(data_dir: PathBuf) {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .enable_all()
        .build()
        .expect("fixture runtime");
    HOST.set(CoreHost { runtime, node: OnceLock::new(), started: Notify::new(), hub: Hub::new(), data_dir })
        .expect("receiver fixture runs in an isolated test process");
}

/// Host for `mirror::window_gate`: the caller's loopback receiver node, whose
/// events reach only the mirror windows.
#[cfg(all(target_os = "macos", feature = "test-support"))]
pub(crate) fn start_window_gate(config: NodeConfig, platform: Arc<dyn Platform>) -> Result<Node, String> {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .thread_name("nectarlink-core")
        .enable_all()
        .build()
        .map_err(|e| format!("gate runtime: {e}"))?;
    let data_dir = config.data_dir.clone();
    HOST.set(CoreHost { runtime, node: OnceLock::new(), started: Notify::new(), hub: Hub::new(), data_dir })
        .map_err(|_| "core host already started".to_owned())?;
    let host = host();
    let node = host
        .runtime
        .block_on(async {
            tokio::time::timeout(Duration::from_secs(10), Node::start(config, platform)).await
        })
        .map_err(|_| "gate receiver node did not start within 10 seconds".to_owned())?
        .map_err(|e| format!("gate receiver node failed to start: {e}"))?;
    let mut events = node.events();
    let _ = host.node.set(node.clone());
    host.started.notify_waiters();
    host.runtime.spawn(async move {
        loop {
            match events.recv().await {
                Ok(event) => crate::mirror::on_event(&event),
                Err(RecvError::Lagged(missed)) => tracing::warn!(missed, "window gate missed core events"),
                Err(RecvError::Closed) => return,
            }
        }
    });
    Ok(node)
}

/// Computes the PC's advertised capabilities, advertising capture (`mirror.capture`)
/// and input injection (`mirror.input`, `input.inject`) only when enabled for at
/// least one paired device (or actively sharing), and this PC's folders
/// (`storage.read`, `storage.write`) only while "Share this PC's folders" is on.
pub fn compute_capabilities(
    #[cfg_attr(not(windows), allow(unused_variables))] node: Option<&Node>,
) -> Vec<String> {
    let mut caps = vec![
        "pc.power".into(),
        nectarlink_core::PHOTOS_SHOW.into(),
        nectarlink_core::CAMERA_CONTINUITY.into(),
        nectarlink_core::CALLS_SHOW.into(),
        nectarlink_core::CONTACTS_SHOW.into(),
        nectarlink_core::SMS_SHOW.into(),
        nectarlink_core::TOGGLES_SHOW.into(),
    ];
    #[cfg(windows)]
    {
        caps.push(nectarlink_core::PC_WAKE.into());
        caps.push("media.control".into());
        caps.push(nectarlink_core::STORAGE_MOUNT.into());
        if crate::settings::share_pc_storage() {
            caps.push(nectarlink_core::STORAGE_READ.into());
            caps.push(nectarlink_core::STORAGE_WRITE.into());
        }
        caps.push(nectarlink_core::MIRROR_VIEW.into());
        caps.push(nectarlink_core::MIRROR_LISTEN.into());
        // Tags mirror requests and streams with a generation and refuses
        // stale-token streams where the phone negotiates it too.
        caps.push(nectarlink_core::MIRROR_CORRELATION.into());
        caps.push(nectarlink_core::MIRROR_INPUT_REQUEST.into());
        caps.push(nectarlink_core::DECK_ACTIONS.into());
        caps.push(nectarlink_core::PC_AUDIO.into());
        caps.push(nectarlink_core::RECORDER.into());
        caps.push(nectarlink_core::WEBCAM_VIRTUAL.into());
        caps.push(nectarlink_core::MEDIA_STREAM.into());
        caps.push(nectarlink_core::MIC_LISTEN.into());
    }
    // Receive the phone's video and send request-bound input to an explicitly
    // consenting phone. This does not offer input injection or capture OF this
    // Mac; audio, storage, deck and wake capabilities stay off. The owned
    // camera receiver supports preview without requiring the native extension.
    #[cfg(target_os = "macos")]
    {
        caps.push(nectarlink_core::MIRROR_VIEW.into());
        caps.push(nectarlink_core::MIRROR_CORRELATION.into());
        caps.push(nectarlink_core::MIRROR_INPUT_REQUEST.into());
        caps.push(nectarlink_core::WEBCAM_VIRTUAL.into());
        caps.push("camera.webcam.owned.v1".into());
        caps.push("phone.hotspot.auto.client.v1".into());
        // Fixed Settings route only; general app-window qualification stays off.
        caps.push("mirror.app_settings_qualification_v1".into());
    }
    #[cfg(windows)]
    let mut pc_mirror_enabled = crate::pc_mirror::is_active();
    #[cfg(windows)]
    let mut remote_input_enabled = false;
    #[cfg(windows)]
    if let Some(node) = node
        && let Ok(devices) = node.paired_devices()
    {
        for d in devices {
            if let Ok(toggles) = node.device_toggles(d.id) {
                for (name, on) in toggles {
                    if on && name == nectarlink_core::PC_MIRROR_TOGGLE {
                        pc_mirror_enabled = true;
                    }
                    if on && name == "remote_input" {
                        remote_input_enabled = true;
                    }
                }
            }
        }
    }
    #[cfg(windows)]
    if pc_mirror_enabled {
        caps.push(nectarlink_core::MIRROR_CAPTURE.into());
    }
    #[cfg(windows)]
    if remote_input_enabled {
        caps.push(nectarlink_core::MIRROR_INPUT.into());
        caps.push(nectarlink_core::INPUT_INJECT.into());
    }
    if crate::win::vcam::is_registered() {
        caps.push(nectarlink_core::WEBCAM_ADDON_VCAM.into());
    }
    caps
}

/// Recomputes and broadcasts this PC's advertised capabilities when feature toggles
/// (`pc_mirror`, `remote_input`, `share_pc_storage`) or active sessions change.
pub fn refresh_capabilities() {
    // Serialized so an older snapshot can't be broadcast after a newer one.
    static REFRESH: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
    let Some(node) = node() else { return };
    spawn(async move {
        let _turn = REFRESH.lock().await;
        let caps = compute_capabilities(Some(&node));
        node.update_power(nectarlink_core::PowerLevel::Basic, caps).await;
    });
}

async fn run(data_dir: PathBuf, platform: Arc<dyn Platform>) {
    let host = host();
    let wake = crate::win::wol::query_wake_status();
    let mut config = NodeConfig::new(&data_dir, this_device(), env!("CARGO_PKG_VERSION"));
    config.downloads_dir = Some(downloads_dir());
    // NECTARLINK_NO_ADB=1 keeps a development copy from reaching phones plugged in over USB.
    let adb_disabled = std::env::var_os("NECTARLINK_NO_ADB").is_some_and(|v| v == "1");
    config.adb_server = (!adb_disabled).then_some(nectarlink_core::DEFAULT_ADB_ADDR);
    // One background attempt on the core runtime, never the Qt thread. Wi-Fi does
    // not wait on it. The USB watcher retries if the sealed server is late or absent.
    #[cfg(target_os = "macos")]
    if !adb_disabled {
        adb_bootstrap::start_once();
    }
    config.away_mode = crate::settings::Settings::load(&data_dir).away_mode;
    // This PC's own players, for phones (crate::win::media_sessions).
    config.capabilities = compute_capabilities(None);
    let initial_caps = config.capabilities.clone();
    let node = match Node::start(config, platform).await {
        Ok(node) => node,
        Err(e) => {
            tracing::error!(error = %e, "core failed to start");
            host.hub.update(|s| {
                s.status = Some(CoreStatus::Failed(e.to_string()));
                Changes::STATUS
            });
            host.started.notify_waiters();
            return;
        }
    };
    let post_start_caps = compute_capabilities(Some(&node));
    if post_start_caps != initial_caps {
        node.update_power(nectarlink_core::PowerLevel::Basic, post_start_caps).await;
    }
    node.set_wake_info(wake.info.clone()).await;
    // Subscribe before reading the initial state so no event is missed.
    let mut events = node.events();
    let devices = node.paired_devices().unwrap_or_else(|e| {
        tracing::warn!(error = %e, "can't list paired devices");
        Vec::new()
    });
    // A device that never connected hasn't told us what it can do; its
    // features stay "unknown" (not "locked") until its first hello.
    let matrices: Vec<_> = devices
        .iter()
        .filter(|d| !matches!(d.link, LinkState::Offline { last_seen: None }))
        .filter_map(|d| node.capabilities(d.id).ok())
        .collect();
    let toggles: Vec<_> =
        devices.iter().filter_map(|d| node.phone_toggles(d.id).map(|t| (d.id, t))).collect();
    let send_queue = node.send_queue(None).unwrap_or_default();
    let status = CoreStatus::Ready { device_id: node.device_id(), name: this_device().name };
    crate::clipboard::apply_history_setting(&node);
    let mut saved_settings = crate::settings::Settings::load(&data_dir);
    let walkthrough_reconciled = saved_settings.reconcile_walkthrough_for_paired_devices(devices.len());
    if walkthrough_reconciled {
        let _ = saved_settings.save(&data_dir);
    }
    let _ = node.set_timeline_retention(nectarlink_core::TimelineRetention {
        max_days: saved_settings.timeline_retention_days,
        max_entries: nectarlink_core::DEFAULT_TIMELINE_MAX_ENTRIES,
    });
    let _ = host.node.set(node);
    // Settings changed before the node was set had no node to refresh.
    refresh_capabilities();
    host.hub.update(|s| {
        let mut changes = s.set_devices(devices);
        for m in matrices {
            changes |= s.set_matrix(m);
        }
        for (id, t) in toggles {
            changes |= s.set_toggles(id, Some(t));
        }
        changes |= s.set_send_queue(send_queue);
        s.status = Some(status);
        s.wake = wake;
        changes |= Changes::STATUS | Changes::CAPABILITIES | Changes::CLIPBOARD | Changes::TIMELINE;
        if walkthrough_reconciled {
            changes |= Changes::PREFS;
        }
        changes
    });
    host.started.notify_waiters();

    host.runtime.spawn(async {
        let mut was_active = false;
        let mut last_sent_at = std::time::Instant::now() - std::time::Duration::from_secs(60);
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(15)).await;
            let Some(node) = self::node() else { continue };
            let online_peers: Vec<_> = host.hub.read(|s| {
                s.devices
                    .iter()
                    .filter(|d| matches!(d.link, LinkState::Online { .. }))
                    .map(|d| d.id)
                    .collect()
            });
            if online_peers.is_empty() {
                was_active = false;
                continue;
            }
            let active = crate::win::input::is_user_active_within_ms(30_000);
            let should_send = active != was_active
                || (active && last_sent_at.elapsed() >= std::time::Duration::from_secs(30));
            if should_send {
                was_active = active;
                last_sent_at = std::time::Instant::now();
                for peer in online_peers {
                    let node = node.clone();
                    tokio::spawn(async move {
                        let _ = node
                            .set_phone_toggle(
                                peer,
                                "pc_active".into(),
                                nectarlink_core::PhoneToggleValue::Bool(active),
                            )
                            .await;
                    });
                }
            }
        }
    });

    loop {
        match events.recv().await {
            Ok(event) => {
                if matches!(
                    &event,
                    nectarlink_core::NodeEvent::QueueChanged
                        | nectarlink_core::NodeEvent::QueueDroppedOnUnpair { .. }
                ) && let Some(node) = self::node()
                {
                    let items = node.send_queue(None).unwrap_or_default();
                    host.hub.update(|s| s.set_send_queue(items));
                }
                if let nectarlink_core::NodeEvent::QueueDroppedOnUnpair { peer_name, dropped_count, .. } =
                    &event
                    && *dropped_count > 0
                {
                    crate::bridge::app::show_message(tr!(
                        "CoreHost",
                        "Dropped %1 queued item(s) for %2 (device unpaired).",
                        dropped_count,
                        peer_name
                    ));
                }
                crate::notifications::apply(&event);
                crate::media::on_event(&event);
                crate::notifications::update_toasts(&event);
                crate::clipboard::on_event(&event);
                crate::photos::on_event(&event);
                crate::photo_backup::on_event(&event);
                crate::continuity_camera::on_event(&event);
                crate::calls::on_event(&event);
                crate::messages::on_event(&event);
                crate::mirror::on_event(&event);
                crate::pc_mirror::on_event(&event);
                crate::remote::on_event(&event);
                crate::transfers::on_event(&event);
                crate::send_to::on_event(&event);
                crate::battery::on_event(&event);
                crate::storage::on_event(&event);
                crate::phone_browser::on_event(&event);
                crate::webcam::on_event(&event);
                crate::phone_mic::on_event(&event);
                crate::automations::on_event(&event);
                crate::bridge::app::on_home_event(&event);
            }
            Err(RecvError::Lagged(missed)) => {
                // Resynchronize what can be re-read; transient events are lost.
                tracing::warn!(missed, "UI fell behind core events; resyncing");
                let devices =
                    host.node.get().map(|n| n.paired_devices().unwrap_or_default()).unwrap_or_default();
                let toggles: Vec<_> = host
                    .node
                    .get()
                    .map(|n| {
                        devices.iter().filter_map(|d| n.phone_toggles(d.id).map(|t| (d.id, t))).collect()
                    })
                    .unwrap_or_default();
                let send_queue =
                    host.node.get().map(|n| n.send_queue(None).unwrap_or_default()).unwrap_or_default();
                host.hub.update(|s| {
                    let mut changes = s.set_devices(devices);
                    for (id, t) in toggles {
                        changes |= s.set_toggles(id, Some(t));
                    }
                    changes |= s.set_send_queue(send_queue);
                    changes
                });
            }
            Err(RecvError::Closed) => return,
        }
    }
}

/// Where files from phones go (see [`crate::downloads`]).
pub fn downloads_dir() -> PathBuf {
    crate::downloads::received_dir()
}

/// How this PC presents itself to phones.
fn this_device() -> DeviceInfo {
    let name = hostname::get().ok().and_then(|h| h.into_string().ok()).unwrap_or_else(|| "PC".into());
    DeviceInfo {
        name,
        kind: if crate::win::is_laptop() { DeviceKind::Laptop } else { DeviceKind::Desktop },
        os: std::env::consts::OS.into(),
        os_ver: crate::win::os_version(),
        model: display_model(),
        accent: None,
        screen: None,
        install_id: crate::win::machine_guid().as_deref().and_then(nectarlink_core::install_id_from),
    }
}

/// Visible device model only. Not an install id, key, or protocol fingerprint.
fn display_model() -> Option<String> {
    #[cfg(target_os = "macos")]
    {
        Some("Waypair Link".to_owned())
    }
    #[cfg(not(target_os = "macos"))]
    {
        None
    }
}

/// Re-reads physical network adapters and their Wake-on-LAN settings, updates
/// the UI state, and sends updated `pc.wake_info` to connected phones if changed.
pub async fn refresh_wake() {
    let wake = crate::win::wol::query_wake_status();
    let info = wake.info.clone();
    host().hub.update(|s| s.set_wake(wake));
    if let Some(node) = node() {
        node.set_wake_info(info).await;
    }
}

/// Stops the node (telling connected devices) and the runtime.
pub fn shutdown() {
    // No Service capture may outlive the node that would answer it.
    crate::continuity_camera::shutdown_service();
    let Some(host) = HOST.get() else { return };
    if let Some(node) = host.node.get().cloned() {
        let done =
            host.runtime.block_on(async { tokio::time::timeout(SHUTDOWN_TIMEOUT, node.shutdown()).await });
        if done.is_err() {
            tracing::warn!("core shutdown timed out");
        }
    }
}

#[cfg(all(test, any(windows, target_os = "macos")))]
mod tests {
    use super::*;

    #[cfg(windows)]
    #[test]
    fn pc_folders_are_not_offered_while_sharing_is_off() {
        crate::settings::set_share_pc_storage(false);
        let caps = compute_capabilities(None);
        assert!(!caps.contains(&nectarlink_core::STORAGE_READ.into()));
        assert!(!caps.contains(&nectarlink_core::STORAGE_WRITE.into()));
        assert!(caps.contains(&nectarlink_core::STORAGE_MOUNT.into()));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_advertises_only_supported_capabilities() {
        let caps = compute_capabilities(None);
        // Ensure unsupported features on macOS are never advertised
        // Outbound phone input is separate from capturing/injecting this Mac.
        assert!(
            caps.contains(&"mirror.view".into()),
            "macOS must advertise mirror.view so the phone's screen can be received"
        );
        assert!(
            caps.contains(&nectarlink_core::MIRROR_CORRELATION.into()),
            "macOS must advertise mirror.correlation so the request and stream generations stay tagged"
        );
        assert!(
            caps.contains(&nectarlink_core::MIRROR_INPUT_REQUEST.into()),
            "outbound phone control negotiates request ownership"
        );
        assert!(!caps.contains(&"mirror.listen".into()), "macOS must not advertise mirror.listen");
        assert!(!caps.contains(&nectarlink_core::MIRROR_CAPTURE.into()));
        assert!(!caps.contains(&nectarlink_core::MIRROR_INPUT.into()));
        assert!(!caps.contains(&nectarlink_core::INPUT_INJECT.into()));
        assert!(!caps.contains(&"media.control".into()), "macOS must not advertise media.control");
        assert!(
            !caps.contains(&nectarlink_core::STORAGE_MOUNT.into()),
            "macOS must not advertise storage.mount"
        );
        assert!(
            !caps.contains(&nectarlink_core::STORAGE_READ.into()),
            "macOS must not advertise storage.read"
        );
        assert!(
            !caps.contains(&nectarlink_core::STORAGE_WRITE.into()),
            "macOS must not advertise storage.write"
        );
        assert!(
            caps.contains(&nectarlink_core::WEBCAM_VIRTUAL.into()),
            "Mac owned camera preview receiver must not depend on native extension readiness"
        );
        assert!(caps.contains(&"camera.webcam.owned.v1".into()));
        assert!(!caps.contains(&nectarlink_core::PC_AUDIO.into()), "macOS must not advertise pc.audio");
        assert!(!caps.contains(&nectarlink_core::RECORDER.into()), "macOS must not advertise recorder");
        assert!(
            !caps.contains(&nectarlink_core::DECK_ACTIONS.into()),
            "macOS must not advertise deck.actions"
        );
        assert!(
            !caps.contains(&nectarlink_core::MEDIA_STREAM.into()),
            "macOS must not advertise media.stream"
        );
        assert!(!caps.contains(&nectarlink_core::MIC_LISTEN.into()), "macOS must not advertise mic.listen");
        assert!(!caps.contains(&nectarlink_core::PC_WAKE.into()), "macOS must not advertise pc.wake");

        // Ensure supported features on macOS are advertised
        assert!(caps.contains(&"pc.power".into()));
        assert!(caps.contains(&nectarlink_core::PHOTOS_SHOW.into()));
        assert!(caps.contains(&nectarlink_core::CAMERA_CONTINUITY.into()));
        assert!(caps.contains(&nectarlink_core::CALLS_SHOW.into()));
        assert!(caps.contains(&nectarlink_core::CONTACTS_SHOW.into()));
        assert!(caps.contains(&nectarlink_core::SMS_SHOW.into()));
        assert!(caps.contains(&nectarlink_core::TOGGLES_SHOW.into()));
    }

    /// The Mac's per-peer "Screen mirroring" permission must stay editable: the
    /// mirroring toggle is not one of the rows the Mac greys out.
    #[test]
    fn the_mac_gap_rows_do_not_disable_the_screen_mirroring_permission() {
        let page = include_str!("../qml/pages/SettingsPage.qml");
        let list = page.lines().find(|l| l.contains("macGapToggleNames: [")).expect("the Mac gap list");
        assert!(
            !list.contains("\"mirroring\""),
            "the Screen mirroring permission must stay editable on the Mac: {list}"
        );
    }
}
