// SPDX-License-Identifier: MPL-2.0
//! Copied links (`link.offer`): a web link the user copies on a phone is
//! offered to paired Macs, which show it in their Dock and open it only when
//! the user clicks. A Mac holds one offer, in memory, until its original
//! lifetime from the copy ends. It never goes on the clipboard, into history
//! or the timeline, or into a log.

use std::{
    sync::{
        Arc, Mutex, Weak,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant, SystemTime},
};

use nectarlink_protocol::{
    DeviceId, Envelope, ErrorCode,
    messages::{LinkOffer, is_offer_url, link_offer, offer_display_host, types},
};

use crate::{NodeEvent, Result, node::Shared, session::Session};

/// The copied link a Mac is offering. Carries no link: the desktop gets it
/// only from [`crate::Node::take_copied_link`], when the user clicks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CopiedLink {
    /// Never reused, so a click consumes only the offer it was shown.
    pub id: u64,
    /// The phone it came from.
    pub device: DeviceId,
    /// Plain ASCII host to show; `None`: show a generic "Web link".
    pub host: Option<String>,
}

struct Held {
    id: u64,
    peer: DeviceId,
    url: String,
    deadline: Instant,
    /// The same deadline by the wall clock: a Mac's `Instant` stops while it
    /// sleeps, so the offer ends at whichever comes first.
    wall_deadline: SystemTime,
}

impl Held {
    fn live(&self, now: Instant, wall: SystemTime) -> bool {
        now < self.deadline && wall < self.wall_deadline
    }
}

/// What the slot holds, under one lock: the offer and the admission
/// generation. The generation counts every revocation (`clear`), even one
/// that found nothing held, so an `admit` whose permission checks ran
/// before the revocation cannot write after it.
#[derive(Default)]
struct State {
    held: Option<Held>,
    generation: u64,
}

/// The one offer a Mac holds.
#[derive(Default)]
pub(crate) struct Slot {
    state: Mutex<State>,
    next_id: AtomicU64,
}

/// A revocation landed between the permission checks and the write.
#[derive(Debug, PartialEq, Eq)]
struct Stale;

impl Slot {
    fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// The current admission generation, to snapshot before any permission
    /// check. The lock is not held across the checks.
    fn generation(&self) -> u64 {
        self.lock().generation
    }

    /// Holds `url` for `ttl` from now, replacing what is held unless that is
    /// a newer copy. Both deadlines are fixed here, once. Returns the new
    /// offer's ID and deadline, or `None` when the held copy is newer.
    /// `expected` is the generation snapshotted before the permission
    /// checks: a revocation since then refuses the write with `Stale`.
    fn admit(
        &self,
        peer: DeviceId,
        url: String,
        ttl: Duration,
        now: Instant,
        wall: SystemTime,
        expected: u64,
    ) -> std::result::Result<Option<(u64, Instant)>, Stale> {
        let deadline = now + ttl;
        let mut state = self.lock();
        if state.generation != expected {
            return Err(Stale);
        }
        if state.held.as_ref().is_some_and(|h| h.live(now, wall) && h.deadline > deadline) {
            return Ok(None);
        }
        let id = self.next_id.fetch_add(1, Ordering::Relaxed) + 1;
        state.held = Some(Held { id, peer, url, deadline, wall_deadline: wall + ttl });
        Ok(Some((id, deadline)))
    }

    fn current(&self, now: Instant, wall: SystemTime) -> Option<CopiedLink> {
        let mut state = self.lock();
        if state.held.as_ref().is_some_and(|h| !h.live(now, wall)) {
            state.held = None;
        }
        state.held.as_ref().map(|h| CopiedLink {
            id: h.id,
            device: h.peer,
            host: offer_display_host(&h.url).map(str::to_owned),
        })
    }

    /// The held link if it is offer `id` and still live, leaving the slot
    /// empty. Anything else (a newer offer) stays.
    fn take(&self, id: u64, now: Instant, wall: SystemTime) -> Option<(DeviceId, String)> {
        let mut state = self.lock();
        if state.held.as_ref().is_none_or(|h| h.id != id) {
            return None;
        }
        let h = state.held.take()?;
        h.live(now, wall).then_some((h.peer, h.url))
    }

    /// Clears offer `id` if it is still the one held.
    fn expire(&self, id: u64) -> bool {
        let mut state = self.lock();
        if state.held.as_ref().is_some_and(|h| h.id == id) {
            state.held = None;
            return true;
        }
        false
    }

    /// Clears what is held (from `peer` only, when given), and always counts
    /// a new admission generation, even when nothing was held: a concurrent
    /// `admit` that snapshotted the old generation is refused. The event,
    /// if any, is emitted by the caller, outside this lock.
    fn clear(&self, peer: Option<&DeviceId>) -> bool {
        let mut state = self.lock();
        state.generation = state.generation.wrapping_add(1);
        if state.held.as_ref().is_some_and(|h| peer.is_none_or(|p| &h.peer == p)) {
            state.held = None;
            return true;
        }
        false
    }
}

fn receiving(shared: &Shared) -> bool {
    shared.local_capabilities().iter().any(|c| c == link_offer::RECEIVE)
}

fn peer_offers(shared: &Shared, peer: &DeviceId, capability: &str) -> bool {
    shared.store.get_peer(peer).ok().flatten().is_some_and(|p| p.caps.contains(capability))
}

/// Clears the held offer (from `peer` only, when given).
pub(crate) fn clear(shared: &Shared, peer: Option<&DeviceId>) {
    if shared.copied_link.clear(peer) {
        shared.emit(NodeEvent::CopiedLinkChanged);
    }
}

pub(crate) fn current(shared: &Shared) -> Option<CopiedLink> {
    shared.copied_link.current(Instant::now(), SystemTime::now())
}

/// The link of offer `id`, consuming it, if it is still held, live and
/// allowed: this device takes copied links, and the phone is paired with
/// the clipboard on.
pub(crate) fn take(shared: &Shared, id: u64) -> Option<String> {
    let (peer, url) = shared.copied_link.take(id, Instant::now(), SystemTime::now())?;
    shared.emit(NodeEvent::CopiedLinkChanged);
    let allowed = receiving(shared)
        && shared.store.is_paired(&peer).unwrap_or(false)
        && shared.toggle_on(&peer, crate::clipboard::TOGGLE);
    allowed.then_some(url)
}

/// Handles `link.offer`. Returns false for other message types.
pub(crate) async fn handle(shared: &Arc<Shared>, session: &Arc<Session>, env: &Envelope) -> Result<bool> {
    if env.t != types::LINK_OFFER {
        return Ok(false);
    }
    let reply = match admit(shared, session, env) {
        Ok(()) => Envelope::empty(types::OK),
        Err((code, message)) => Envelope::error(code, message),
    };
    if env.id.is_some() {
        session.send(reply.reply_to(env.id)).await?;
    }
    Ok(true)
}

fn admit(
    shared: &Arc<Shared>,
    session: &Arc<Session>,
    env: &Envelope,
) -> std::result::Result<(), (ErrorCode, &'static str)> {
    // Snapshot first: a revocation (`clear`) between here and the write
    // counts a new generation, and the write below is refused. The lock is
    // not held across the checks that follow.
    let generation = shared.copied_link.generation();
    let peer = session.peer;
    if !receiving(shared) {
        return Err((ErrorCode::Unsupported, "this device doesn't take copied links now"));
    }
    let current = shared.session(&peer).is_some_and(|s| Arc::ptr_eq(&s, session));
    if !current || !shared.store.is_paired(&peer).unwrap_or(false) {
        return Err((ErrorCode::Denied, "not a current paired session"));
    }
    if !shared.toggle_on(&peer, crate::clipboard::TOGGLE) {
        return Err((ErrorCode::Denied, "the clipboard is off for this device"));
    }
    if !peer_offers(shared, &peer, link_offer::SEND) {
        return Err((ErrorCode::Denied, "this device doesn't offer copied links"));
    }
    let offer = env
        .body::<LinkOffer>()
        .ok()
        .filter(LinkOffer::is_valid)
        .ok_or((ErrorCode::BadMessage, "invalid copied link"))?;
    let ttl = Duration::from_millis(offer.ttl_ms);
    let admitted =
        match shared.copied_link.admit(peer, offer.url, ttl, Instant::now(), SystemTime::now(), generation) {
            Ok(admitted) => admitted,
            Err(Stale) => return Err((ErrorCode::Denied, "the copied link was revoked while this arrived")),
        };
    if let Some((id, deadline)) = admitted {
        shared.emit(NodeEvent::CopiedLinkChanged);
        expire_at(Arc::downgrade(shared), id, deadline);
    }
    Ok(())
}

fn expire_at(shared: Weak<Shared>, id: u64, deadline: Instant) {
    tokio::spawn(async move {
        tokio::time::sleep_until(deadline.into()).await;
        if let Some(shared) = shared.upgrade()
            && shared.copied_link.expire(id)
        {
            shared.emit(NodeEvent::CopiedLinkChanged);
        }
    });
}

/// Offers a link copied `copied_age_ms` ago to every connected Mac that
/// takes copied links and has the clipboard on for this phone, while this
/// phone says it offers them. Not a link, or too old: nothing is sent.
/// Never queued or retried. Returns how many Macs took it.
pub(crate) async fn offer(shared: &Arc<Shared>, url: String, copied_age_ms: u64) -> u32 {
    let offering = shared.local_capabilities().iter().any(|c| c == link_offer::SEND);
    if !offering || copied_age_ms >= link_offer::MAX_SEND_AGE_MS || !is_offer_url(&url) {
        return 0;
    }
    let deadline = Instant::now() + Duration::from_millis(link_offer::MAX_TTL_MS - copied_age_ms);
    let mut taken = 0;
    for session in shared.live_sessions() {
        let peer = session.peer;
        if !shared.store.is_paired(&peer).unwrap_or(false)
            || !peer_offers(shared, &peer, link_offer::RECEIVE)
            || !shared.toggle_on(&peer, crate::clipboard::TOGGLE)
        {
            continue;
        }
        let left = deadline.saturating_duration_since(Instant::now());
        let ttl_ms = u64::try_from(left.as_millis()).unwrap_or(0);
        if ttl_ms == 0 {
            break;
        }
        let Ok(env) = Envelope::new(types::LINK_OFFER, &LinkOffer { url: url.clone(), ttl_ms }) else {
            continue;
        };
        let took = match session.request(env, crate::session::REQUEST_TIMEOUT).await {
            Ok(reply) => reply.expect(types::OK).is_ok(),
            Err(_) => false,
        };
        if took {
            taken += 1;
        } else {
            tracing::debug!(peer = %peer.short(), "copied link not taken");
        }
    }
    taken
}

#[cfg(test)]
mod tests {
    use super::*;

    const PEER: DeviceId = DeviceId([7; 32]);

    #[test]
    fn a_click_takes_only_the_offer_it_was_shown() {
        let slot = Slot::default();
        let (now, wall) = (Instant::now(), SystemTime::now());
        let ttl = Duration::from_secs(300);
        let generation = slot.generation();
        let (first, _) =
            slot.admit(PEER, "https://a.example/1".into(), ttl, now, wall, generation).unwrap().unwrap();
        let shown = slot.current(now, wall).unwrap();
        assert_eq!((shown.id, shown.host.as_deref()), (first, Some("a.example")));
        let (later, wall_later) = (now + Duration::from_secs(1), wall + Duration::from_secs(1));
        let (second, _) = slot
            .admit(PEER, "https://b.example/2".into(), ttl, later, wall_later, generation)
            .unwrap()
            .unwrap();
        assert_eq!(slot.take(first, later, wall_later), None);
        assert_eq!(slot.current(later, wall_later).map(|c| c.id), Some(second));
        let taken = slot.take(second, later, wall_later);
        assert_eq!(taken, Some((PEER, "https://b.example/2".into())));
        assert_eq!(slot.current(later, wall_later), None);
    }

    #[test]
    fn stale_and_expired_offers_are_never_shown_or_taken() {
        let slot = Slot::default();
        let (now, wall) = (Instant::now(), SystemTime::now());
        let ttl = Duration::from_secs(300);
        let generation = slot.generation();
        let (id, deadline) =
            slot.admit(PEER, "https://new.example/".into(), ttl, now, wall, generation).unwrap().unwrap();
        let older =
            slot.admit(PEER, "https://old.example/".into(), Duration::from_secs(10), now, wall, generation);
        assert_eq!(older, Ok(None));
        assert_eq!(slot.current(now, wall).map(|c| c.id), Some(id));
        assert_eq!(slot.current(deadline, wall), None);
        assert_eq!(slot.take(id, deadline, wall), None);
        assert!(!slot.expire(id));
        // A Mac that slept: its `Instant` barely moved, the wall clock passed the deadline.
        let (id, _) =
            slot.admit(PEER, "https://new.example/".into(), ttl, now, wall, generation).unwrap().unwrap();
        let slept = (now + Duration::from_secs(1), wall + ttl);
        assert_eq!(slot.take(id, slept.0, slept.1), None);
        assert_eq!(slot.current(slept.0, slept.1), None);
    }

    #[test]
    fn a_revoked_offer_is_not_admitted_after_reenable() {
        let slot = Slot::default();
        let (now, wall) = (Instant::now(), SystemTime::now());
        let ttl = Duration::from_secs(300);
        // An admit whose permission checks ran before the Mac locked.
        let stale = slot.generation();
        // Lock: the receiver went off and the slot was cleared, empty.
        slot.clear(None);
        // Unlock re-enables the receiver; nothing is held, nothing is shown.
        assert_eq!(slot.current(now, wall), None);
        // The in-flight admit must not write now.
        assert!(slot.admit(PEER, "https://locked.example/".into(), ttl, now, wall, stale).is_err());
        assert_eq!(slot.current(now, wall), None);
        // A fresh offer after unlock still lands.
        let fresh = slot.generation();
        let (id, _) = slot.admit(PEER, "https://a.example/1".into(), ttl, now, wall, fresh).unwrap().unwrap();
        assert_eq!(slot.current(now, wall).map(|c| c.id), Some(id));
    }
}
