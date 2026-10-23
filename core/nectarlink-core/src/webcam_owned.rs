// SPDX-License-Identifier: MPL-2.0
//! Additive webcam ownership. Never reacquires a replacement Node/Session.
use crate::{Error, MirrorStream, NodeEvent, Result, node::Shared, session::Session};
use iroh::endpoint::{RecvStream, SendStream, VarInt};
use nectarlink_protocol::{
    DeviceId, Envelope, ErrorCode, PacketKind,
    messages::{
        StreamHeader, WebcamOwnedConfig, WebcamOwnedStart, WebcamOwnedStop, WebcamStart, types, webcam,
    },
    read_video_packet, write_frame,
};
use std::{
    collections::HashMap,
    sync::{
        Arc, Mutex, Weak,
        atomic::{AtomicBool, AtomicU8, Ordering},
    },
};

fn lock<T>(value: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    value.lock().unwrap_or_else(|e| e.into_inner())
}

struct Owner {
    shared: Weak<Shared>,
    session: Weak<Session>,
    request: u64,
    active: AtomicBool,
    closed: Arc<AtomicBool>,
    stop: Arc<tokio::sync::Notify>,
    opened: AtomicBool,
    // 0 not requested, 1 cleaning, 2 terminal success, 3 failed (exact retry allowed).
    cleanup: AtomicU8,
    terminal: tokio::sync::Notify,
}
impl Owner {
    fn new(shared: &Arc<Shared>, session: &Arc<Session>, request: u64) -> Arc<Self> {
        Arc::new(Self {
            shared: Arc::downgrade(shared),
            session: Arc::downgrade(session),
            request,
            active: AtomicBool::new(true),
            closed: Arc::default(),
            stop: Arc::default(),
            opened: AtomicBool::new(false),
            cleanup: AtomicU8::new(0),
            terminal: Default::default(),
        })
    }
    fn revoke(&self) {
        self.active.store(false, Ordering::Release);
        self.closed.store(true, Ordering::Release);
        self.stop.notify_waiters();
        self.stop.notify_one();
    }
    fn capture(self: &Arc<Self>) -> Option<WebcamCaptureContext> {
        Some(WebcamCaptureContext {
            shared: self.shared.upgrade()?,
            session: self.session.upgrade()?,
            owner: self.clone(),
        })
    }
}
#[derive(Default)]
struct Owners {
    outgoing: HashMap<DeviceId, Arc<Owner>>,
    incoming: HashMap<DeviceId, Arc<Owner>>,
    // One bounded watermark per peer: actual Core Session identity + durable request.
    seen: HashMap<DeviceId, (u64, u64)>,
}
#[derive(Default)]
pub(crate) struct State {
    owners: Mutex<Owners>,
}
impl State {
    pub(crate) fn revoke_peer(&self, peer: &DeviceId) -> Vec<WebcamCaptureContext> {
        let mut owners = lock(&self.owners);
        if let Some(owner) = owners.outgoing.remove(peer) {
            owner.revoke();
        }
        let mut contexts = Vec::new();
        if let Some(owner) = owners.incoming.get(peer) {
            owner.revoke();
            if let Some(context) = owner.capture() {
                contexts.push(context);
            }
        }
        contexts
    }
    pub(crate) fn write_toggle(
        &self,
        shared: &Shared,
        peer: &DeviceId,
        enabled: bool,
    ) -> Result<(Option<WebcamView>, Option<WebcamCaptureContext>)> {
        // Same fence as actual owner allocation: OFF/ON never recaptures a
        // successor from a later peer lookup, matching notification authority.
        let owners = lock(&self.owners);
        if enabled {
            shared.store.set_toggle(peer, crate::webcam::TOGGLE, true)?;
            return Ok((None, None));
        }
        let view = owners.outgoing.get(peer).and_then(|owner| {
            owner.revoke();
            Some(WebcamView {
                shared: owner.shared.upgrade()?,
                session: owner.session.upgrade()?,
                owner: owner.clone(),
            })
        });
        let capture = owners.incoming.get(peer).and_then(|owner| {
            owner.revoke();
            owner.capture()
        });
        shared.store.set_toggle(peer, crate::webcam::TOGGLE, false)?;
        Ok((view, capture))
    }
    pub(crate) fn capability_update<T>(
        &self,
        update: impl FnOnce() -> (bool, T),
    ) -> (T, Vec<WebcamView>, Vec<WebcamCaptureContext>) {
        let owners = lock(&self.owners);
        let (lost, result) = update();
        let mut views = Vec::new();
        let mut captures = Vec::new();
        if lost {
            for owner in owners.outgoing.values() {
                owner.revoke();
                if let (Some(shared), Some(session)) = (owner.shared.upgrade(), owner.session.upgrade()) {
                    views.push(WebcamView { shared, session, owner: owner.clone() });
                }
            }
            for owner in owners.incoming.values() {
                owner.revoke();
                if let Some(context) = owner.capture() {
                    captures.push(context);
                }
            }
        }
        (result, views, captures)
    }
    pub(crate) fn revoke_all(&self) -> Vec<WebcamCaptureContext> {
        let peers: Vec<_> = {
            let owners = lock(&self.owners);
            owners.outgoing.keys().chain(owners.incoming.keys()).copied().collect()
        };
        peers.into_iter().flat_map(|peer| self.revoke_peer(&peer)).collect()
    }
}

fn original_session_current(shared: &Shared, session: &Arc<Session>) -> bool {
    !shared.cancel.is_cancelled()
        && !session.cancel.is_cancelled()
        && shared.session(&session.peer).is_some_and(|live| Arc::ptr_eq(&live, session))
}
fn session_current(shared: &Shared, session: &Arc<Session>) -> bool {
    original_session_current(shared, session)
        && shared.store.is_paired(&session.peer).unwrap_or(false)
        && shared.toggle_on(&session.peer, crate::webcam::TOGGLE)
}
fn sender_ready(shared: &Shared, peer: &DeviceId) -> bool {
    shared.local_capabilities().iter().any(|cap| cap == webcam::VIRTUAL)
        && shared.peer_supports(peer, webcam::STREAM)
}
fn capture_ready(shared: &Shared, peer: &DeviceId) -> bool {
    shared.local_capabilities().iter().any(|cap| cap == webcam::STREAM)
        && shared.peer_supports(peer, webcam::VIRTUAL)
}
fn bilateral(shared: &Shared, peer: &DeviceId) -> bool {
    shared.local_capabilities().iter().any(|cap| cap == webcam::OWNED_V1)
        && shared.peer_supports(peer, webcam::OWNED_V1)
}

/// Original sender authority, published before Start waits for the phone reply.
#[derive(Clone)]
pub struct WebcamView {
    shared: Arc<Shared>,
    session: Arc<Session>,
    owner: Arc<Owner>,
}
impl std::fmt::Debug for WebcamView {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WebcamView").field("request", &self.owner.request).finish_non_exhaustive()
    }
}
impl WebcamView {
    pub fn request(&self) -> u64 {
        self.owner.request
    }
    pub async fn closed(&self) {
        loop {
            let notified = self.owner.stop.notified();
            if !self.owner.active.load(Ordering::Acquire) {
                return;
            }
            notified.await;
        }
    }
    pub fn same_owner(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.shared, &other.shared)
            && Arc::ptr_eq(&self.session, &other.session)
            && Arc::ptr_eq(&self.owner, &other.owner)
    }
    fn retained(&self) -> bool {
        lock(&self.shared.webcam_owned.owners)
            .outgoing
            .get(&self.session.peer)
            .is_some_and(|owner| Arc::ptr_eq(owner, &self.owner))
    }
    pub fn is_current(&self) -> bool {
        session_current(&self.shared, &self.session)
            && bilateral(&self.shared, &self.session.peer)
            && sender_ready(&self.shared, &self.session.peer)
            && self.retained()
            && self.owner.active.load(Ordering::Acquire)
    }
    pub fn peer(&self) -> DeviceId {
        self.session.peer
    }
    pub fn original_session_current(&self) -> bool {
        original_session_current(&self.shared, &self.session)
    }
    pub async fn session_ended(&self) {
        tokio::select! { _ = self.shared.cancel.cancelled() => {}, _ = self.session.cancel.cancelled() => {} }
    }
    pub async fn keyframe(&self) -> Result<()> {
        let env = Envelope::new(types::WEBCAM_KEYFRAME_OWNED, &WebcamOwnedStop { request: self.request() })?;
        let permit = self.session.reserve().await?;
        if !self.is_current() {
            return Err(Error::Denied);
        }
        permit.send(env);
        Ok(())
    }
    pub async fn configure(&self, options: WebcamStart) -> Result<()> {
        if !options.is_valid() || !self.is_current() {
            return Err(Error::Denied);
        }
        let body = WebcamOwnedStart { request: self.request(), options };
        let env = Envelope::new(types::WEBCAM_START_OWNED, &body)?;
        self.session
            .request_if_current(env, crate::session::REQUEST_TIMEOUT, || self.is_current())
            .await?
            .expect(types::WEBCAM_OK)?;
        if !self.is_current() {
            return Err(Error::Denied);
        }
        Ok(())
    }
    /// Retires only this original operation; a failed cleanup remains retained,
    /// so a new Start cannot inherit an unfinished cancellation transaction.
    pub async fn stop_if_current(&self) -> Result<bool> {
        let original = self
            .shared
            .with_webcam_session(&self.session, || {
                let owners = lock(&self.shared.webcam_owned.owners);
                let applies = owners
                    .outgoing
                    .get(&self.session.peer)
                    .is_some_and(|owner| Arc::ptr_eq(owner, &self.owner));
                if applies {
                    self.owner.revoke();
                }
                applies
            })
            .unwrap_or(false);
        if !original {
            let mut owners = lock(&self.shared.webcam_owned.owners);
            if owners.outgoing.get(&self.session.peer).is_some_and(|owner| Arc::ptr_eq(owner, &self.owner)) {
                owners.outgoing.remove(&self.session.peer);
            }
            self.owner.revoke();
            return Ok(false);
        }
        let env = Envelope::new(types::WEBCAM_STOP_OWNED, &WebcamOwnedStop { request: self.request() })?;
        self.session
            .request_if_current(env, crate::session::REQUEST_TIMEOUT, || {
                original_session_current(&self.shared, &self.session) && self.retained()
            })
            .await?
            .expect(types::OK)?;
        let mut owners = lock(&self.shared.webcam_owned.owners);
        if owners.outgoing.get(&self.session.peer).is_some_and(|owner| Arc::ptr_eq(owner, &self.owner)) {
            owners.outgoing.remove(&self.session.peer);
        }
        Ok(true)
    }
}

pub(crate) async fn start(
    shared: &Arc<Shared>,
    session: &Arc<Session>,
    options: WebcamStart,
    ready: impl FnOnce(WebcamView) -> bool,
) -> Result<()> {
    if !bilateral(shared, &session.peer) || !sender_ready(shared, &session.peer) {
        return Err(Error::Unsupported);
    }
    if !options.is_valid() || !session_current(shared, session) {
        return Err(Error::Denied);
    }
    // This is the existing durable Core capture allocator used by MirrorView.
    // Its wire operation namespace is separate; no host/UI ID is repurposed.
    let request = shared.store.next_mirror_request()?;
    let owner = Owner::new(shared, session, request);
    shared
        .with_webcam_session(session, || {
            let mut owners = lock(&shared.webcam_owned.owners);
            if !shared.toggle_on(&session.peer, crate::webcam::TOGGLE)
                || !shared.store.is_paired(&session.peer).unwrap_or(false)
                || !sender_ready(shared, &session.peer)
                || !bilateral(shared, &session.peer)
            {
                return Err(Error::Denied);
            }
            if owners.outgoing.contains_key(&session.peer) {
                return Err(Error::Denied);
            }
            owners.outgoing.insert(session.peer, owner.clone());
            Ok(())
        })
        .ok_or(Error::Offline)??;
    let view = WebcamView { shared: shared.clone(), session: session.clone(), owner };
    if !ready(view.clone()) || !view.is_current() {
        let mut owners = lock(&shared.webcam_owned.owners);
        if owners.outgoing.get(&session.peer).is_some_and(|owner| Arc::ptr_eq(owner, &view.owner)) {
            owners.outgoing.remove(&session.peer);
        }
        view.owner.revoke();
        return Err(Error::Denied);
    }
    // The caller retains this same view on errors for exact cleanup/retry.
    view.configure(options).await
}

/// Actual receiving Core/Session/capture authority, retained through Android posts.
#[derive(Clone)]
pub struct WebcamCaptureContext {
    shared: Arc<Shared>,
    session: Arc<Session>,
    owner: Arc<Owner>,
}
impl std::fmt::Debug for WebcamCaptureContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("WebcamCaptureContext")
    }
}
impl WebcamCaptureContext {
    pub(crate) fn belongs_to(&self, shared: &Arc<Shared>) -> bool {
        Arc::ptr_eq(shared, &self.shared)
    }
    pub fn peer(&self) -> DeviceId {
        self.session.peer
    }
    pub fn request(&self) -> u64 {
        self.owner.request
    }
    pub async fn closed(&self) {
        loop {
            let notified = self.owner.stop.notified();
            if !self.owner.active.load(Ordering::Acquire) {
                return;
            }
            notified.await;
        }
    }
    pub fn same_owner(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.shared, &other.shared)
            && Arc::ptr_eq(&self.session, &other.session)
            && Arc::ptr_eq(&self.owner, &other.owner)
    }
    pub fn is_current(&self) -> bool {
        session_current(&self.shared, &self.session)
            && bilateral(&self.shared, &self.session.peer)
            && capture_ready(&self.shared, &self.session.peer)
            && lock(&self.shared.webcam_owned.owners)
                .incoming
                .get(&self.session.peer)
                .is_some_and(|owner| Arc::ptr_eq(owner, &self.owner))
            && self.owner.active.load(Ordering::Acquire)
    }
    pub fn finish(&self) {
        request_cleanup(self.clone());
    }
    pub async fn open(&self) -> Result<MirrorStream> {
        let context = self.clone();
        self.shared
            .runtime
            .spawn(async move { context.open_inner().await })
            .await
            .map_err(|_| Error::Offline)?
    }
    async fn open_inner(&self) -> Result<MirrorStream> {
        if !self.is_current()
            || self.owner.opened.compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire).is_err()
        {
            return Err(Error::Denied);
        }
        let result = async {
            let (mut send, recv) = self.session.conn.open_bi().await.map_err(crate::error::net)?;
            if !self.is_current() {
                return Err(Error::Denied);
            }
            let header = StreamHeader {
                svc: webcam::SERVICE.into(),
                op: webcam::OP_VIDEO_OWNED.into(),
                v: webcam::VERSION,
            };
            write_frame(&mut send, &Envelope::new(types::STREAM, &header)?.to_cbor()).await?;
            if !self.is_current() {
                return Err(Error::Denied);
            }
            Ok(MirrorStream::spawn_stream_with_lifetime(
                send,
                recv,
                false,
                None,
                self.owner.closed.clone(),
                self.owner.stop.clone(),
            ))
        }
        .await;
        if result.is_err() {
            self.finish();
        }
        result
    }
    pub async fn wait_terminal(&self) -> Result<()> {
        loop {
            let notified = self.owner.terminal.notified();
            match self.owner.cleanup.load(Ordering::Acquire) {
                2 => return Ok(()),
                3 => return Err(Error::Internal("original camera cleanup did not complete".into())),
                _ => notified.await,
            }
        }
    }
}

/// Retire immediately; only terminal platform completion permits a success ACK.
pub(crate) fn request_cleanup(context: WebcamCaptureContext) {
    context.owner.revoke();
    let status = context.owner.cleanup.load(Ordering::Acquire);
    if status == 1 || status == 2 {
        return;
    }
    if context.owner.cleanup.compare_exchange(status, 1, Ordering::AcqRel, Ordering::Acquire).is_err() {
        return;
    }
    let runtime = context.shared.runtime.clone();
    runtime.spawn(async move {
        let platform = context.shared.platform.clone();
        let original = context.clone();
        let completed = tokio::task::spawn_blocking(move || platform.webcam_stop_owned(original))
            .await
            .ok()
            .is_some_and(|result| result.is_ok());
        if completed {
            let mut owners = lock(&context.shared.webcam_owned.owners);
            if owners
                .incoming
                .get(&context.session.peer)
                .is_some_and(|owner| Arc::ptr_eq(owner, &context.owner))
            {
                owners.incoming.remove(&context.session.peer);
            }
        }
        context.owner.cleanup.store(if completed { 2 } else { 3 }, Ordering::Release);
        context.owner.terminal.notify_waiters();
        if completed && original_session_current(&context.shared, &context.session) {
            let env =
                Envelope::new(types::WEBCAM_STOP_OWNED, &WebcamOwnedStop { request: context.request() });
            if let Ok(env) = env {
                let _ = context
                    .session
                    .request_if_current(env, crate::session::REQUEST_TIMEOUT, || {
                        original_session_current(&context.shared, &context.session)
                    })
                    .await;
            }
        }
    });
}

/// Owned messages are admitted/retired synchronously in the original reader;
/// only platform work and its completion reply are detached afterward.
pub(crate) async fn handle(shared: &Arc<Shared>, session: &Arc<Session>, env: &Envelope) -> Result<bool> {
    if env.t != types::WEBCAM_START_OWNED
        && env.t != types::WEBCAM_STOP_OWNED
        && env.t != types::WEBCAM_KEYFRAME_OWNED
    {
        return Ok(false);
    }
    if !bilateral(shared, &session.peer) || !original_session_current(shared, session) {
        session
            .send(
                Envelope::error(ErrorCode::Denied, "owned camera session is no longer current")
                    .reply_to(env.id),
            )
            .await?;
        return Ok(true);
    }
    if env.t == types::WEBCAM_KEYFRAME_OWNED {
        let body: WebcamOwnedStop = env.body()?;
        let context = shared
            .with_webcam_session(session, || {
                lock(&shared.webcam_owned.owners)
                    .incoming
                    .get(&session.peer)
                    .filter(|owner| {
                        owner.request == body.request
                            && owner.active.load(Ordering::Acquire)
                            && owner.session.upgrade().is_some_and(|original| Arc::ptr_eq(&original, session))
                    })
                    .and_then(|owner| owner.capture())
            })
            .flatten();
        if let Some(context) = context {
            let platform = shared.platform.clone();
            tokio::task::spawn_blocking(move || {
                if context.is_current() {
                    platform.webcam_keyframe_owned(context);
                }
            });
        }
        return Ok(true);
    }
    if env.t == types::WEBCAM_START_OWNED {
        let body: WebcamOwnedStart = env.body()?;
        if !session_current(shared, session)
            || !capture_ready(shared, &session.peer)
            || body.request == 0
            || !body.options.is_valid()
            || !shared.local_capabilities().iter().any(|cap| cap == webcam::STREAM)
        {
            session
                .send(Envelope::error(ErrorCode::Unsupported, "owned camera is unavailable").reply_to(env.id))
                .await?;
            return Ok(true);
        }
        let context = shared
            .with_webcam_session(session, || {
                let mut owners = lock(&shared.webcam_owned.owners);
                if !shared.toggle_on(&session.peer, crate::webcam::TOGGLE)
                    || !shared.store.is_paired(&session.peer).unwrap_or(false)
                    || !capture_ready(shared, &session.peer)
                    || !bilateral(shared, &session.peer)
                {
                    return Err(Error::Denied);
                }
                if let Some(owner) = owners.incoming.get(&session.peer) {
                    if owner.request == body.request
                        && owner.session.upgrade().is_some_and(|original| Arc::ptr_eq(&original, session))
                        && owner.active.load(Ordering::Acquire)
                    {
                        return owner.capture().ok_or(Error::Offline);
                    }
                    return Err(Error::Denied);
                }
                let seen = owners.seen.entry(session.peer).or_insert((session.mirror_control_id, 0));
                if seen.0 != session.mirror_control_id {
                    *seen = (session.mirror_control_id, 0);
                }
                if body.request <= seen.1 {
                    return Err(Error::Denied);
                }
                seen.1 = body.request;
                let owner = Owner::new(shared, session, body.request);
                owners.incoming.insert(session.peer, owner.clone());
                owner.capture().ok_or(Error::Offline)
            })
            .ok_or(Error::Offline)?;
        let context = match context {
            Ok(context) => context,
            Err(_) => {
                session
                    .send(
                        Envelope::error(ErrorCode::Denied, "original camera owner is busy or retired")
                            .reply_to(env.id),
                    )
                    .await?;
                return Ok(true);
            }
        };
        let original_session = session.clone();
        let reply_to = env.id;
        let platform = shared.platform.clone();
        tokio::spawn(async move {
            let original = context.clone();
            let options = body.options;
            let accepted = tokio::task::spawn_blocking(move || {
                if !original.is_current() {
                    return Err("retired camera request".into());
                }
                platform.webcam_requested_owned(original, &options)
            })
            .await
            .ok()
            .is_some_and(|result| result.is_ok());
            let reply = if accepted && context.is_current() {
                Envelope::empty(types::WEBCAM_OK)
            } else {
                request_cleanup(context);
                Envelope::error(ErrorCode::Denied, "owned camera request was not admitted")
            };
            let _ = original_session.send(reply.reply_to(reply_to)).await;
        });
    } else {
        let body: WebcamOwnedStop = env.body()?;
        if body.request == 0 {
            return Err(Error::Protocol("empty owned webcam stop".into()));
        }
        let outgoing_closed = shared
            .with_webcam_session(session, || {
                let mut owners = lock(&shared.webcam_owned.owners);
                let applies = owners.outgoing.get(&session.peer).is_some_and(|owner| {
                    owner.request == body.request
                        && owner.session.upgrade().is_some_and(|original| Arc::ptr_eq(&original, session))
                });
                if applies && let Some(owner) = owners.outgoing.remove(&session.peer) {
                    owner.revoke();
                }
                applies
            })
            .unwrap_or(false);
        if outgoing_closed {
            session.send(Envelope::empty(types::OK).reply_to(env.id)).await?;
            return Ok(true);
        }
        let context = shared
            .with_webcam_session(session, || {
                let mut owners = lock(&shared.webcam_owned.owners);
                let seen = owners.seen.entry(session.peer).or_insert((session.mirror_control_id, 0));
                if seen.0 != session.mirror_control_id {
                    *seen = (session.mirror_control_id, 0);
                }
                seen.1 = seen.1.max(body.request); // unknown Stop also fences a later Start.
                owners
                    .incoming
                    .get(&session.peer)
                    .filter(|owner| {
                        owner.request == body.request
                            && owner.session.upgrade().is_some_and(|original| Arc::ptr_eq(&original, session))
                    })
                    .and_then(|owner| {
                        owner.revoke();
                        owner.capture()
                    })
            })
            .ok_or(Error::Offline)?;
        if let Some(context) = context {
            request_cleanup(context.clone());
            let original_session = session.clone();
            let reply_to = env.id;
            tokio::spawn(async move {
                let reply = if context.wait_terminal().await.is_ok() {
                    Envelope::empty(types::OK)
                } else {
                    Envelope::error(ErrorCode::Internal, "original camera cleanup has not completed")
                };
                let _ = original_session.send(reply.reply_to(reply_to)).await;
            });
        } else {
            // No matching Entry can appear later: reader admission + watermark
            // already retired this request. An unrelated live owner is untouched.
            session.send(Envelope::empty(types::OK).reply_to(env.id)).await?;
        }
    }
    Ok(true)
}

pub(crate) async fn receive(
    shared: Arc<Shared>,
    session: Arc<Session>,
    mut send: SendStream,
    mut recv: RecvStream,
) {
    let refuse = |mut send: SendStream, mut recv: RecvStream| {
        let _ = send.reset(VarInt::from_u32(webcam::STOPPED));
        let _ = recv.stop(VarInt::from_u32(webcam::STOPPED));
    };
    if !bilateral(&shared, &session.peer)
        || !sender_ready(&shared, &session.peer)
        || !session_current(&shared, &session)
    {
        return refuse(send, recv);
    }
    let first = match read_video_packet(&mut recv).await {
        Ok(Some(packet)) if packet.kind == PacketKind::Config => {
            WebcamOwnedConfig::from_cbor(&packet.data).ok()
        }
        _ => None,
    };
    let Some(first) = first.filter(|config| config.request != 0 && config.config.is_valid()) else {
        return refuse(send, recv);
    };
    let view = shared
        .with_webcam_session(&session, || {
            let owners = lock(&shared.webcam_owned.owners);
            let owner = owners.outgoing.get(&session.peer)?.clone();
            if owner.request != first.request
                || !owner.active.load(Ordering::Acquire)
                || !owner.session.upgrade().is_some_and(|original| Arc::ptr_eq(&original, &session))
            {
                return None;
            }
            Some(WebcamView { shared: shared.clone(), session: session.clone(), owner })
        })
        .flatten();
    let Some(view) = view else {
        return refuse(send, recv);
    };
    if view.owner.opened.compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire).is_err() {
        return refuse(send, recv);
    }
    let Some(sink) = shared.platform.webcam_owned_sink(&session.peer, &view) else {
        return refuse(send, recv);
    };
    let sink_stop = sink.stop_signal();
    let sink_stopped = async {
        match sink_stop {
            Some(signal) => signal.notified().await,
            None => std::future::pending::<()>().await,
        }
    };
    tokio::pin!(sink_stopped);
    shared.emit(NodeEvent::Webcam { device: session.peer, on: true });
    sink.config(first.config);
    loop {
        let packet = tokio::select! {
            _ = session.cancel.cancelled() => break,
            _ = view.owner.stop.notified() => break,
            _ = &mut sink_stopped => break,
            packet = read_video_packet(&mut recv) => packet,
        };
        if !view.is_current() {
            break;
        }
        match packet {
            Ok(Some(packet)) if packet.kind == PacketKind::Config => {
                let config = WebcamOwnedConfig::from_cbor(&packet.data).ok();
                match config.filter(|config| config.request == view.request() && config.config.is_valid()) {
                    Some(config) => sink.config(config.config),
                    None => break,
                }
            }
            Ok(Some(packet)) => sink.packet(packet.kind == PacketKind::Keyframe, packet.time_us, packet.data),
            _ => break,
        }
    }
    let _ = recv.stop(VarInt::from_u32(webcam::STOPPED));
    let _ = send.finish();
    sink.ended();
    shared.emit(NodeEvent::Webcam { device: session.peer, on: false });
}
