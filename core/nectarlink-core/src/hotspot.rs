// SPDX-License-Identifier: MPL-2.0
//! User-requested hotspot preparation, owned by one peer, one live protocol
//! connection and one monotonic request. The expected saved SSID is carried
//! only over the authenticated private peer channel and redacted from Debug.
//! Automatic credentials are ephemeral and travel only over the owned private RPC.

use crate::{Error, Result, node::Shared, session::Session};
use nectarlink_protocol::{
    DeviceId, Envelope, ErrorCode,
    messages::{
        PHONE_HOTSPOT_V3, PHONE_HOTSPOT_AUTO_V1, PHONE_HOTSPOT_AUTO_CLIENT_V1, PhoneHotspotNetwork, PhoneHotspotCancel, PhoneHotspotPrepare, PhoneHotspotResult, PhoneHotspotState,
        types,
    },
};
use std::{
    collections::HashMap,
    sync::{
        Arc, Mutex, Weak,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};

const PREPARE_WITHIN: Duration = Duration::from_secs(40);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(45);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PhoneHotspotOwner {
    pub peer: DeviceId,
    pub connection: u64,
    pub request: u64,
}

#[derive(Debug, Clone)]
pub struct PhoneHotspotPreparation {
    pub state: PhoneHotspotState,
    pub network: Option<PhoneHotspotNetwork>,
}

struct Pending {
    auto_join: bool,
    owner: PhoneHotspotOwner,
    session: Weak<Session>,
    cancelled: AtomicBool,
    cancellation_running: AtomicBool,
    deadline: Instant,
}

#[derive(Default)]
pub(crate) struct Current {
    next: AtomicU64,
    outgoing: Mutex<HashMap<DeviceId, Arc<Pending>>>,
    incoming: Mutex<HashMap<DeviceId, Arc<Pending>>>,
    // Prevent an old queued prepare/cancel from acquiring a fresh operation on
    // the same connection after its earlier operation has completed.
    last_incoming: Mutex<HashMap<DeviceId, (u64, u64)>>,
    last_cancelled: Mutex<HashMap<DeviceId, (u64, u64)>>,
}

fn allowed(shared: &Shared, session: &Arc<Session>) -> bool {
    !shared.cancel.is_cancelled()
        && shared.store.is_paired(&session.peer).unwrap_or(false)
        && shared.toggle_on(&session.peer, "toggles")
        && shared.session(&session.peer).is_some_and(|live| Arc::ptr_eq(&live, session))
}

fn matches(pending: &Pending, owner: PhoneHotspotOwner) -> bool {
    pending.owner == owner
}

fn pending_current(shared: &Shared, pending: &Pending) -> bool {
    !pending.cancelled.load(Ordering::Acquire)
        && Instant::now() < pending.deadline
        && pending.session.upgrade().is_some_and(|session| allowed(shared, &session))
}

fn remove(map: &Mutex<HashMap<DeviceId, Arc<Pending>>>, owner: PhoneHotspotOwner) {
    let mut map = map.lock().unwrap_or_else(|e| e.into_inner());
    if map.get(&owner.peer).is_some_and(|pending| matches(pending, owner)) {
        map.remove(&owner.peer);
    }
}

struct Retirement {
    shared: Arc<Shared>,
    owner: PhoneHotspotOwner,
    incoming: bool,
    cancel_on_drop: bool,
}
impl Drop for Retirement {
    fn drop(&mut self) {
        if !self.incoming && self.cancel_on_drop {
            cancel(&self.shared, self.owner);
        }
        let map = if self.incoming { &self.shared.hotspot.incoming } else { &self.shared.hotspot.outgoing };
        let cancellation_running = self.incoming
            && map.lock().unwrap_or_else(|e| e.into_inner()).get(&self.owner.peer).is_some_and(|pending| {
                matches(pending, self.owner) && pending.cancellation_running.load(Ordering::Acquire)
            });
        if !cancellation_running {
            remove(map, self.owner);
        }
    }
}

pub(crate) fn create(shared: &Arc<Shared>, session: &Arc<Session>) -> Result<PhoneHotspotOwner> {
    create_mode(shared, session, false)
}
pub(crate) fn create_auto(shared: &Arc<Shared>, session: &Arc<Session>) -> Result<PhoneHotspotOwner> {
    create_mode(shared, session, true)
}
fn create_mode(shared: &Arc<Shared>, session: &Arc<Session>, auto_join: bool) -> Result<PhoneHotspotOwner> {
    if !allowed(shared, session) {
        return Err(Error::Denied);
    }
    let caps = shared.store.get_peer(&session.peer)?.map(|peer| peer.caps).unwrap_or_default();
    if !caps.contains(PHONE_HOTSPOT_V3) || (auto_join && !caps.contains(PHONE_HOTSPOT_AUTO_V1)) {
        return Err(Error::Unsupported);
    }
    let request = shared
        .hotspot
        .next
        .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |n| n.checked_add(1))
        .map_err(|_| Error::Denied)?
        + 1;
    let owner = PhoneHotspotOwner { peer: session.peer, connection: session.mirror_control_id, request };
    let mut outgoing = shared.hotspot.outgoing.lock().unwrap_or_else(|e| e.into_inner());
    if outgoing
        .get(&owner.peer)
        .is_some_and(|pending| Instant::now() >= pending.deadline || pending.session.upgrade().is_none())
    {
        outgoing.remove(&owner.peer);
    }
    if outgoing.contains_key(&owner.peer) {
        return Err(Error::Denied);
    }
    outgoing.insert(
        owner.peer,
        Arc::new(Pending {
            auto_join,
            owner,
            session: Arc::downgrade(session),
            cancelled: AtomicBool::new(false),
            cancellation_running: AtomicBool::new(false),
            deadline: Instant::now() + REQUEST_TIMEOUT,
        }),
    );
    Ok(owner)
}

/// Platform gate, rechecked before the helper's final GO and throughout readback.
pub(crate) fn incoming_current(shared: &Shared, owner: PhoneHotspotOwner) -> bool {
    let pending = shared.hotspot.incoming.lock().unwrap_or_else(|e| e.into_inner()).get(&owner.peer).cloned();
    shared.local_capabilities().iter().any(|cap| cap == PHONE_HOTSPOT_V3)
        && pending.is_some_and(|pending| matches(&pending, owner) && pending_current(shared, &pending)
            && (!pending.auto_join || (shared.local_capabilities().iter().any(|cap| cap == PHONE_HOTSPOT_AUTO_V1)
                && shared.store.get_peer(&owner.peer).ok().flatten().is_some_and(|peer| peer.caps.contains(PHONE_HOTSPOT_AUTO_CLIENT_V1)))))
}

pub(crate) fn cancellation_current(shared: &Shared, owner: PhoneHotspotOwner) -> bool {
    let pending = shared.hotspot.incoming.lock().unwrap_or_else(|e| e.into_inner()).get(&owner.peer).cloned();
    pending.is_some_and(|pending| {
        matches(&pending, owner)
            && pending.cancelled.load(Ordering::Acquire)
            && pending.cancellation_running.load(Ordering::Acquire)
            && Instant::now() < pending.deadline
            && pending.session.upgrade().is_some_and(|session| allowed(shared, &session))
    })
}

pub(crate) fn cancel(shared: &Arc<Shared>, owner: PhoneHotspotOwner) {
    let pending = shared.hotspot.outgoing.lock().unwrap_or_else(|e| e.into_inner()).get(&owner.peer).cloned();
    let Some(pending) = pending.filter(|pending| matches(pending, owner)) else { return };
    pending.cancelled.store(true, Ordering::Release);
    let Some(session) = pending.session.upgrade() else { return };
    // Use only the original connection, even if a replacement is now live.
    shared.runtime.spawn(async move {
        if let Ok(env) =
            Envelope::new(types::PHONE_HOTSPOT_CANCEL, &PhoneHotspotCancel { request: owner.request })
        {
            let _ = session.send(env).await;
        }
    });
}

pub(crate) async fn prepare(
    shared: &Arc<Shared>,
    owner: PhoneHotspotOwner,
    network: String,
) -> Result<PhoneHotspotState> {
    Ok(prepare_mode(shared, owner, network, false).await?.state)
}
pub(crate) async fn prepare_auto(shared: &Arc<Shared>, owner: PhoneHotspotOwner) -> Result<PhoneHotspotPreparation> {
    prepare_mode(shared, owner, String::new(), true).await
}
async fn prepare_mode(shared: &Arc<Shared>, owner: PhoneHotspotOwner, network: String, auto_join: bool) -> Result<PhoneHotspotPreparation> {
    let pending = shared.hotspot.outgoing.lock().unwrap_or_else(|e| e.into_inner()).get(&owner.peer).cloned();
    let pending = pending.filter(|pending| matches(pending, owner)).ok_or(Error::Denied)?;
    let mut retire = Retirement { shared: shared.clone(), owner, incoming: false, cancel_on_drop: true };
    if pending.auto_join != auto_join || !pending_current(shared, &pending) {
        return Err(Error::Denied);
    }
    if (!auto_join && network.is_empty()) || network.len() > 32 || network.contains('\0') {
        return Err(Error::Protocol("invalid saved hotspot profile".into()));
    }
    let session = pending.session.upgrade().ok_or(Error::Denied)?;
    if auto_join && !shared.store.get_peer(&owner.peer)?.is_some_and(|peer| peer.caps.contains(PHONE_HOTSPOT_AUTO_V1) && peer.caps.contains(PHONE_HOTSPOT_V3)) { return Err(Error::Unsupported); }
    let env = Envelope::new(
        types::PHONE_HOTSPOT_PREPARE,
        &PhoneHotspotPrepare { request: owner.request, network, auto_join },
    )?;
    // Keep polling the underlying correlated request until its normal reply or
    // timeout so Session::request always removes its pending response slot.
    // Cancel sends a separate frame and causes the phone's live owner gate to
    // fail; it does not drop a pending generic request future.
    let reply = session.request(env, REQUEST_TIMEOUT).await?;
    reply.expect(types::PHONE_HOTSPOT_RESULT)?;
    let result: PhoneHotspotResult = reply.body()?;
    if result.request != owner.request || !pending_current(shared, &pending)
        || (auto_join && !shared.store.get_peer(&owner.peer)?.is_some_and(|peer| peer.caps.contains(PHONE_HOTSPOT_AUTO_V1) && peer.caps.contains(PHONE_HOTSPOT_V3))) {
        return Ok(PhoneHotspotPreparation { state: PhoneHotspotState::Cancelled, network: None });
    }
    // Successful completion is not a Stop. Keep the phone hotspot available
    // while the explicitly requested Mac network association runs.
    retire.cancel_on_drop = false;
    let network = result.network.filter(|network| auto_join && result.state == PhoneHotspotState::Active && network.valid());
    let state = if auto_join && result.state == PhoneHotspotState::Active && network.is_none() { PhoneHotspotState::Failed } else { result.state };
    Ok(PhoneHotspotPreparation { state, network })
}

/// Preparation is dispatched concurrently so a cancellation frame on the same
/// authenticated stream can be processed while the OS/helper readback waits.
pub(crate) async fn handle(shared: &Arc<Shared>, session: &Arc<Session>, env: &Envelope) -> Result<bool> {
    match env.t.as_str() {
        types::PHONE_HOTSPOT_CANCEL => {
            let cancel: PhoneHotspotCancel = env.body()?;
            let owner = PhoneHotspotOwner {
                peer: session.peer,
                connection: session.mirror_control_id,
                request: cancel.request,
            };
            let dispatch = {
                let incoming = shared.hotspot.incoming.lock().unwrap_or_else(|e| e.into_inner());
                if owner.request != 0 && allowed(shared, session) {
                    let mut last = shared.hotspot.last_cancelled.lock().unwrap_or_else(|e| e.into_inner());
                    let previous = last
                        .get(&owner.peer)
                        .filter(|(connection, _)| *connection == owner.connection)
                        .map_or(0, |(_, request)| *request);
                    last.insert(owner.peer, (owner.connection, previous.max(owner.request)));
                }
                incoming.get(&owner.peer).filter(|p| matches(p, owner)).and_then(|pending| {
                    pending.cancelled.store(true, Ordering::Release);
                    (allowed(shared, session) && !pending.cancellation_running.swap(true, Ordering::AcqRel))
                        .then(|| pending.clone())
                })
            };
            if let Some(pending) = dispatch {
                let shared = shared.clone();
                let runtime = shared.runtime.clone();
                runtime.spawn(async move {
                    let platform = shared.platform.clone();
                    let _ = tokio::task::spawn_blocking(move || {
                        platform.cancel_phone_hotspot_owned(&owner.peer, owner.connection, owner.request)
                    })
                    .await;
                    pending.cancellation_running.store(false, Ordering::Release);
                    remove(&shared.hotspot.incoming, owner);
                });
            }
        }
        types::PHONE_HOTSPOT_PREPARE => {
            let request: PhoneHotspotPrepare = env.body()?;
            if request.request == 0
                || (!request.auto_join && request.network.is_empty())
                || request.network.len() > 32
                || request.network.contains('\0')
                || env.id.is_none()
                || !allowed(shared, session)
            {
                session
                    .send(
                        Envelope::error(ErrorCode::Denied, "hotspot request is not current").reply_to(env.id),
                    )
                    .await?;
                return Ok(true);
            }
            if !shared.local_capabilities().iter().any(|cap| cap == PHONE_HOTSPOT_V3)
                || (request.auto_join && (!shared.local_capabilities().iter().any(|cap| cap == PHONE_HOTSPOT_AUTO_V1)
                    || !shared.store.get_peer(&session.peer)?.is_some_and(|peer| peer.caps.contains(PHONE_HOTSPOT_AUTO_CLIENT_V1)))) {
                let result =
                    PhoneHotspotResult { request: request.request, state: PhoneHotspotState::NeedsSettings, network: None };
                session.send(Envelope::new(types::PHONE_HOTSPOT_RESULT, &result)?.reply_to(env.id)).await?;
                return Ok(true);
            }
            let owner = PhoneHotspotOwner {
                peer: session.peer,
                connection: session.mirror_control_id,
                request: request.request,
            };
            let admitted = {
                let mut incoming = shared.hotspot.incoming.lock().unwrap_or_else(|e| e.into_inner());
                let cancelled = shared.hotspot.last_cancelled.lock().unwrap_or_else(|e| e.into_inner());
                let was_cancelled = cancelled.get(&owner.peer).is_some_and(|(connection, request)| {
                    *connection == owner.connection && *request >= owner.request
                });
                let mut last = shared.hotspot.last_incoming.lock().unwrap_or_else(|e| e.into_inner());
                if !allowed(shared, session)
                    || was_cancelled
                    || incoming.contains_key(&owner.peer)
                    || last.get(&owner.peer).is_some_and(|(connection, request)| {
                        *connection == owner.connection && *request >= owner.request
                    })
                {
                    false
                } else {
                    last.insert(owner.peer, (owner.connection, owner.request));
                    incoming.insert(
                        owner.peer,
                        Arc::new(Pending {
                            auto_join: request.auto_join,
                            owner,
                            session: Arc::downgrade(session),
                            cancelled: AtomicBool::new(false),
                            cancellation_running: AtomicBool::new(false),
                            deadline: Instant::now() + PREPARE_WITHIN,
                        }),
                    );
                    true
                }
            };
            if !admitted {
                session
                    .send(
                        Envelope::error(ErrorCode::Denied, "hotspot request is busy or stale")
                            .reply_to(env.id),
                    )
                    .await?;
                return Ok(true);
            }
            let shared = shared.clone();
            let session = session.clone();
            let reply_id = env.id;
            let runtime = shared.runtime.clone();
            runtime.spawn(async move {
                let _retire =
                    Retirement { shared: shared.clone(), owner, incoming: true, cancel_on_drop: false };
                let platform = shared.platform.clone();
                let auto_join = request.auto_join;
                let outcome = if incoming_current(&shared, owner) {
                    tokio::task::spawn_blocking(move || {
                        if auto_join {
                            platform.prepare_phone_hotspot_auto_owned(&owner.peer, owner.connection, owner.request)
                        } else {
                            PhoneHotspotPreparation { state: platform.prepare_phone_hotspot_owned(
                                &owner.peer, owner.connection, owner.request, &request.network), network: None }
                        }
                    }).await.unwrap_or(PhoneHotspotPreparation { state: PhoneHotspotState::Failed, network: None })
                } else {
                    PhoneHotspotPreparation { state: PhoneHotspotState::Cancelled, network: None }
                };
                let current = incoming_current(&shared, owner);
                let network = outcome.network.filter(|network| current && auto_join && outcome.state == PhoneHotspotState::Active && network.valid());
                let state = if !current { PhoneHotspotState::Cancelled }
                    else if auto_join && outcome.state == PhoneHotspotState::Active && network.is_none() { PhoneHotspotState::Failed }
                    else { outcome.state };
                let result = PhoneHotspotResult { request: owner.request, state, network };
                if let Ok(env) = Envelope::new(types::PHONE_HOTSPOT_RESULT, &result) {
                    let _ = session.send(env.reply_to(reply_id)).await;
                }
            });
        }
        _ => return Ok(false),
    }
    Ok(true)
}
