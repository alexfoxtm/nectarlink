// SPDX-License-Identifier: GPL-3.0-or-later
//! A user-tapped, owned phone-hotspot request followed by OS Wi-Fi joining.
//! The chosen network is a profile the user saved once through system Wi-Fi.
//! On macOS, Keychain stores the per-phone SSID and supplies Wi-Fi credentials;
//! no SSID/password is logged or written to hotspot.json. Only the expected
//! SSID crosses the private paired channel; the password remains in Keychain.

use crate::i18n::tr;
use std::{
    collections::HashMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

#[cfg(any(not(target_os = "macos"), test))]
use std::path::Path;
#[cfg(not(target_os = "macos"))]
use std::path::PathBuf;

use nectarlink_core::{DeviceId, PhoneHotspotOwner, PhoneHotspotState};

use crate::{bridge::app::show_message, core_host, state::Changes, win::wifi::Reach};

/// How long the phone's hotspot has to show up.
const APPEAR_WITHIN: Duration = Duration::from_secs(45);
/// How long Windows has to finish joining it.
const JOIN_WITHIN: Duration = Duration::from_secs(20);
const LOOK_EVERY: Duration = Duration::from_secs(2);

#[cfg(not(target_os = "macos"))]
static NETWORKS: Mutex<Option<HashMap<DeviceId, String>>> = Mutex::new(None);
/// Phones whose hotspot the PC is joining right now.
struct JoinRequest {
    cancelled: Arc<AtomicBool>,
    remote: Option<PhoneHotspotOwner>,
}
static JOINING: Mutex<Option<HashMap<DeviceId, JoinRequest>>> = Mutex::new(None);

#[cfg(not(target_os = "macos"))]
fn file() -> PathBuf {
    core_host::host().data_dir.join("hotspot.json")
}

#[cfg(any(not(target_os = "macos"), test))]
fn load(path: &Path) -> HashMap<DeviceId, String> {
    std::fs::read(path)
        .ok()
        .and_then(|b| serde_json::from_slice::<HashMap<String, String>>(&b).ok())
        .map(|m| m.into_iter().filter_map(|(k, v)| Some((k.parse().ok()?, v))).collect())
        .unwrap_or_default()
}

#[cfg(any(not(target_os = "macos"), test))]
fn save(path: &Path, networks: &HashMap<DeviceId, String>) -> std::io::Result<()> {
    let plain: HashMap<String, &String> = networks.iter().map(|(k, v)| (k.to_string(), v)).collect();
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_vec(&plain).map_err(std::io::Error::other)?)?;
    std::fs::rename(&tmp, path)
}

#[cfg(not(target_os = "macos"))]
fn with_networks<T>(f: impl FnOnce(&mut HashMap<DeviceId, String>) -> T) -> T {
    let mut guard = NETWORKS.lock().unwrap_or_else(|e| e.into_inner());
    let networks = guard.get_or_insert_with(|| load(&file()));
    f(networks)
}

/// The Wi-Fi network the PC joins when `device` turns its hotspot on, if one is set.
pub fn network_for(device: DeviceId) -> Option<String> {
    #[cfg(target_os = "macos")]
    {
        crate::win::wifi::profile_for_peer(&device.to_string())
    }
    #[cfg(not(target_os = "macos"))]
    {
        with_networks(|n| n.get(&device).cloned())
    }
}

/// Sets (or, with `None`, clears) the network for `device`'s hotspot.
pub fn set_network(device: DeviceId, network: Option<String>) {
    cancel(device);
    #[cfg(target_os = "macos")]
    {
        if let Err(reason) = crate::win::wifi::set_profile_for_peer(&device.to_string(), network.as_deref()) {
            show_message(reason);
        }
    }
    #[cfg(not(target_os = "macos"))]
    let snapshot = with_networks(|n| {
        match network.filter(|s| !s.trim().is_empty()) {
            Some(name) => n.insert(device, name),
            None => n.remove(&device),
        };
        n.clone()
    });
    #[cfg(not(target_os = "macos"))]
    if let Err(e) = save(&file(), &snapshot) {
        tracing::warn!(error = %e, "can't save the hotspot networks");
    }
    core_host::host().hub.changed(Changes::DEVICES);
}

/// What the PC needs from Wi-Fi, so joining can be tested without changing networks.
pub(crate) trait Wifi {
    fn scan(&self);
    fn reach(&self, profile: &str) -> Reach;
    fn connect(&self, profile: &str) -> Result<(), String>;
    fn wait(&self, d: Duration);
    fn current(&self) -> bool {
        true
    }
    fn permission_error(&self) -> Option<String> {
        None
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum JoinOutcome {
    Already,
    Joined,
    /// The hotspot never showed up in time.
    NotFound,
    Failed(String),
    Cancelled,
}

/// Waits for `profile` to come into range, joins it, and waits for Windows to finish.
pub(crate) fn join(
    wifi: &impl Wifi,
    profile: &str,
    appear_within: Duration,
    join_within: Duration,
) -> JoinOutcome {
    if !wifi.current() {
        return JoinOutcome::Cancelled;
    }
    if let Some(reason) = wifi.permission_error() {
        return JoinOutcome::Failed(reason);
    }
    if wifi.reach(profile) == Reach::Connected {
        return JoinOutcome::Already;
    }
    let started = Instant::now();
    let mut waited = Duration::ZERO;
    loop {
        if !wifi.current() {
            return JoinOutcome::Cancelled;
        }
        if let Some(reason) = wifi.permission_error() {
            return JoinOutcome::Failed(reason);
        }
        wifi.scan();
        wifi.wait(LOOK_EVERY);
        waited += LOOK_EVERY;
        if !wifi.current() {
            return JoinOutcome::Cancelled;
        }
        match wifi.reach(profile) {
            Reach::Connected => return JoinOutcome::Joined,
            Reach::InRange => break,
            Reach::NotInRange if waited >= appear_within || started.elapsed() > appear_within * 2 => {
                return JoinOutcome::NotFound;
            }
            Reach::NotInRange => {}
        }
    }
    if !wifi.current() {
        return JoinOutcome::Cancelled;
    }
    if let Err(e) = wifi.connect(profile) {
        return JoinOutcome::Failed(e);
    }
    let mut joining = Duration::ZERO;
    while joining < join_within {
        wifi.wait(LOOK_EVERY);
        joining += LOOK_EVERY;
        if !wifi.current() {
            return JoinOutcome::Cancelled;
        }
        if wifi.reach(profile) == Reach::Connected {
            return JoinOutcome::Joined;
        }
    }
    JoinOutcome::Failed("The operating system did not finish joining the hotspot".into())
}

struct OwnedWifi {
    device: DeviceId,
    connection: u64,
    cancelled: Arc<AtomicBool>,
}

impl Wifi for OwnedWifi {
    fn scan(&self) {
        crate::win::wifi::scan();
    }
    fn reach(&self, profile: &str) -> Reach {
        crate::win::wifi::reach(profile)
    }
    fn connect(&self, profile: &str) -> Result<(), String> {
        if !self.current() {
            return Err("Hotspot join cancelled".into());
        }
        #[cfg(target_os = "macos")]
        {
            crate::win::wifi::connect_owned(profile, &|| self.current())
        }
        #[cfg(not(target_os = "macos"))]
        {
            crate::win::wifi::connect(profile)
        }
    }
    fn wait(&self, d: Duration) {
        // Stop/revocation is noticed promptly without stopping the source link.
        let until = Instant::now() + d;
        while self.current() && Instant::now() < until {
            std::thread::sleep(Duration::from_millis(100));
        }
    }
    fn current(&self) -> bool {
        !self.cancelled.load(Ordering::Acquire)
            && core_host::node()
                .is_some_and(|node| node.phone_hotspot_connection_current(self.device, self.connection))
    }
    fn permission_error(&self) -> Option<String> {
        #[cfg(target_os = "macos")]
        {
            crate::win::wifi::permission_error()
        }
        #[cfg(not(target_os = "macos"))]
        {
            None
        }
    }
}

fn reserve(device: DeviceId) -> Option<Arc<AtomicBool>> {
    let mut joining = JOINING.lock().unwrap_or_else(|e| e.into_inner());
    let joining = joining.get_or_insert_with(HashMap::new);
    if joining.contains_key(&device) {
        return None;
    }
    let cancelled = Arc::new(AtomicBool::new(false));
    joining.insert(device, JoinRequest { cancelled: cancelled.clone(), remote: None });
    Some(cancelled)
}

fn retire(device: DeviceId, owner: &Arc<AtomicBool>) {
    let mut joining = JOINING.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(joining) = joining.as_mut()
        && joining.get(&device).is_some_and(|current| Arc::ptr_eq(&current.cancelled, owner))
    {
        joining.remove(&device);
    }
}

fn set_remote_owner(device: DeviceId, local: &Arc<AtomicBool>, remote: PhoneHotspotOwner) {
    let mut joining = JOINING.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(owner) =
        joining.as_mut().and_then(|m| m.get_mut(&device)).filter(|owner| Arc::ptr_eq(&owner.cancelled, local))
    {
        owner.remote = Some(remote);
        if owner.cancelled.load(Ordering::Acquire)
            && let Some(node) = core_host::node()
        {
            node.cancel_phone_hotspot_request(remote);
        }
    }
}

pub fn is_joining(device: DeviceId) -> bool {
    JOINING
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .as_ref()
        .is_some_and(|joining| joining.contains_key(&device))
}

/// Stop only this phone's explicit join. No phone-wide hotspot-off, source
/// connection close, Wi-Fi power change or speculative network rollback.
/// CoreWLAN has no cancellation API once association has been submitted.
pub fn cancel(device: DeviceId) {
    let joining = JOINING.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(owner) = joining.as_ref().and_then(|m| m.get(&device)) {
        owner.cancelled.store(true, Ordering::Release);
        if let (Some(node), Some(remote)) = (core_host::node(), owner.remote) {
            node.cancel_phone_hotspot_request(remote);
        }
    }
}

/// Explicit user action: prepare the phone on this exact paired connection,
/// then join only after verified remote readback and current ownership checks.
pub fn join_phone(device: DeviceId) {
    let Some(profile) = network_for(device) else {
        show_message("Choose the phone's saved hotspot in Settings > Devices first.");
        return;
    };
    #[cfg(target_os = "macos")]
    if let Some(reason) = crate::win::wifi::permission_error() {
        show_message(reason);
        return;
    }
    let Some(node) = core_host::node() else { return };
    let Some(connection) = node.phone_hotspot_connection(device) else {
        show_message("Reconnect to the paired phone and allow Phone controls before joining its hotspot.");
        return;
    };
    let Some(cancelled) = reserve(device) else { return };
    let remote = match node.create_phone_hotspot_request(device) {
        Ok(owner) if owner.connection == connection => owner,
        Ok(owner) => {
            node.cancel_phone_hotspot_request(owner);
            // Poll the owned request once so its retirement guard releases it.
            core_host::spawn(async move {
                let _ = node.prepare_phone_hotspot(owner, profile).await;
            });
            retire(device, &cancelled);
            return;
        }
        Err(_) => {
            retire(device, &cancelled);
            show_message(
                "Waypair cannot verify automatic hotspot control on this connection. On Galaxy, open Settings > Connections > Mobile Hotspot and Tethering > Mobile Hotspot. You can join it manually in this computer's Wi-Fi settings.",
            );
            return;
        }
    };
    set_remote_owner(device, &cancelled, remote);
    core_host::spawn(async move {
        let result = node.prepare_phone_hotspot(remote, profile.clone()).await;
        if cancelled.load(Ordering::Acquire) {
            retire(device, &cancelled);
            return;
        }
        if matches!(result, Ok(PhoneHotspotState::ConfigChanged)) {
            retire(device, &cancelled);
            show_message(
                "The phone hotspot settings changed or the saved Wi-Fi network does not match. Choose this phone's current saved hotspot and retry. The Mac has not switched networks.",
            );
            return;
        }
        if !matches!(result, Ok(PhoneHotspotState::Active))
            || !node.phone_hotspot_connection_current(device, connection)
        {
            retire(device, &cancelled);
            show_message(
                "Waypair could not verify that the phone hotspot is on. On Galaxy, open Settings > Connections > Mobile Hotspot and Tethering > Mobile Hotspot. The Mac has not switched networks.",
            );
            return;
        }
        join_prepared(device, connection, profile, cancelled);
    });
}

fn join_prepared(device: DeviceId, connection: u64, profile: String, cancelled: Arc<AtomicBool>) {
    let failure_owner = cancelled.clone();
    let result = std::thread::Builder::new().name("hotspot-join".into()).spawn(move || {
        let wifi = OwnedWifi { device, connection, cancelled: cancelled.clone() };
        let outcome = join(&wifi, &profile, APPEAR_WITHIN, JOIN_WITHIN);
        let current = wifi.current();
        retire(device, &cancelled);
        if !current { return; }
        match outcome {
            JoinOutcome::Already | JoinOutcome::Joined => show_message(tr!("Hotspot", "Connected to the phone hotspot. Internet access depends on the phone's mobile connection.")),
            JoinOutcome::NotFound => show_message("The phone hotspot did not appear. Check Mobile Hotspot on the phone and the saved network in Wi-Fi settings."),
            JoinOutcome::Failed(reason) => show_message(reason),
            JoinOutcome::Cancelled => {}
        }
    });
    if result.is_err() {
        retire(device, &failure_owner);
        show_message("Could not start the hotspot join. Try again.");
    }
}

/// Legacy toggle caller still uses a verified paired RPC, but never infers a
/// join owner from an unrelated peer or a replacement connection on macOS.
pub fn hotspot_turned_on(device: DeviceId) {
    #[cfg(target_os = "macos")]
    {
        let _ = device;
    }
    #[cfg(not(target_os = "macos"))]
    {
        let Some(profile) = network_for(device) else { return };
        let Some(node) = core_host::node() else { return };
        let Some(connection) = node.phone_hotspot_connection(device) else { return };
        let Some(cancelled) = reserve(device) else { return };
        join_prepared(device, connection, profile, cancelled);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::{Cell, RefCell};

    /// Wi-Fi that shows the hotspot after `appears` looks and joins it on connect.
    struct Fake {
        appears_after: u32,
        looks: Cell<u32>,
        connected: Cell<bool>,
        connect_works: bool,
        log: RefCell<Vec<&'static str>>,
    }

    impl Fake {
        fn new(appears_after: u32, connect_works: bool) -> Fake {
            Fake {
                appears_after,
                looks: Cell::new(0),
                connected: Cell::new(false),
                connect_works,
                log: RefCell::new(Vec::new()),
            }
        }
    }

    impl Wifi for Fake {
        fn scan(&self) {
            self.looks.set(self.looks.get() + 1);
        }
        fn reach(&self, _profile: &str) -> Reach {
            if self.connected.get() {
                Reach::Connected
            } else if self.looks.get() >= self.appears_after {
                Reach::InRange
            } else {
                Reach::NotInRange
            }
        }
        fn connect(&self, _profile: &str) -> Result<(), String> {
            self.log.borrow_mut().push("connect");
            if self.connect_works {
                self.connected.set(true);
                Ok(())
            } else {
                Err("refused".into())
            }
        }
        fn wait(&self, _d: Duration) {}
    }

    const APPEAR: Duration = Duration::from_secs(10);
    const JOIN: Duration = Duration::from_secs(6);

    #[test]
    fn joins_once_the_hotspot_shows_up() {
        let wifi = Fake::new(3, true);
        assert_eq!(join(&wifi, "Pixel", APPEAR, JOIN), JoinOutcome::Joined);
        assert_eq!(*wifi.log.borrow(), ["connect"], "connects exactly once");
        assert_eq!(wifi.looks.get(), 3);
    }

    #[test]
    fn already_on_it_does_nothing() {
        let wifi = Fake::new(0, true);
        wifi.connected.set(true);
        assert_eq!(join(&wifi, "Pixel", APPEAR, JOIN), JoinOutcome::Already);
        assert!(wifi.log.borrow().is_empty());
    }

    #[test]
    fn gives_up_when_it_never_shows_up_without_connecting() {
        let wifi = Fake::new(u32::MAX, true);
        assert_eq!(join(&wifi, "Pixel", APPEAR, JOIN), JoinOutcome::NotFound);
        assert!(wifi.log.borrow().is_empty(), "never connects to a network that isn't there");
        assert_eq!(wifi.looks.get(), 5, "looked for 10 s, every 2 s");
    }

    #[test]
    fn says_so_when_windows_refuses() {
        assert!(matches!(join(&Fake::new(1, false), "Pixel", APPEAR, JOIN), JoinOutcome::Failed(_)));
    }

    #[test]
    fn networks_round_trip_by_phone() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("hotspot.json");
        let mut networks = HashMap::new();
        networks.insert(DeviceId([3; 32]), "Pixel 9 hotspot".to_owned());
        save(&path, &networks).unwrap();
        assert_eq!(load(&path), networks);
        assert!(load(&dir.path().join("missing.json")).is_empty());
    }
}
