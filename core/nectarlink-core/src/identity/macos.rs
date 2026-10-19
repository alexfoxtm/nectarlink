// SPDX-License-Identifier: MPL-2.0
//! macOS Keychain-backed [`KeyProtector`]: a random 32-byte wrapping
//! key lives in this app's own login-Keychain generic-password item
//! (stable service `com.waypair.link.nectar.key-wrapping.v1`), and the
//! device key is sealed with it using ring's AES-256-GCM.
//!
//! Trust and failure rules:
//!
//! - The Keychain item relies on the normal macOS ACL for generic
//!   passwords: only the signed application identity that created it
//!   (this app, under its stable signing identity across releases)
//!   reads it without a prompt. No access-control (user-presence)
//!   entries, ACL edits, or authentication bypasses are used.
//! - Creation happens only on the protect path when the item is
//!   genuinely absent (`errSecItemNotFound`), via an atomic
//!   create-only `SecItemAdd`: a racing creator gets
//!   `errSecDuplicateItem`, re-reads the winner's key, and never
//!   updates or deletes existing wrapping material (the previous
//!   upsert via `set_generic_password` could overwrite a winner's
//!   key and orphan everything sealed with it). A process-wide
//!   creation lock serialises protector instances locally, but
//!   cross-process correctness rests on the atomic add alone. Every
//!   other Keychain error — access denied, interaction not allowed,
//!   a malformed (non-32-byte) item — fails closed. The unprotect path never
//!   creates, regenerates, or overwrites the wrapping key, so a
//!   corrupt sealed file or a refused Keychain can never trick the
//!   app into sealing a new identity over the old one.
//! - Sealed format (the bytes after `identity.rs`' MAGIC + method
//!   byte): version 1, then the 12-byte random nonce, then
//!   AES-256-GCM ciphertext with its appended tag, with a fixed AAD
//!   binding the blob to this protector. Authentication failure is an
//!   error, never a fallback.

use std::{io, sync::Mutex};

use core_foundation::data::CFData;
use ring::{
    aead::{AES_256_GCM, Aad, LessSafeKey, Nonce, UnboundKey},
    rand::{SecureRandom, SystemRandom},
};
use security_framework::{
    item::{ItemAddOptions, ItemAddValue, ItemClass},
    passwords::{PasswordOptions, generic_password},
};

use super::KeyProtector;

/// Stable Keychain service for the wrapping key. Deliberately the
/// app's own reverse-DNS service (not shared with any other app), and
/// versioned so a future format can add a new service instead of
/// migrating this one in place.
const SERVICE: &str = "com.waypair.link.nectar.key-wrapping.v1";
/// Account name of the wrapping-key item within [`SERVICE`].
const ACCOUNT: &str = "device-key-wrapping";
/// Sealed-blob format version written by [`MacosKeyProtector::protect`].
const FORMAT_VERSION: u8 = 1;
/// AAD binding sealed blobs to this protector and format.
const AAD: &[u8] = b"nectarlink/macos-key-wrapping/v1";
/// `errSecItemNotFound` from Security.framework.
const ERR_SEC_ITEM_NOT_FOUND: i32 = -25300;
/// `errSecDuplicateItem` from Security.framework.
const ERR_SEC_DUPLICATE_ITEM: i32 = -25299;

/// Serialises fetch-or-create across protector instances in this
/// process. This is only an optimisation: cross-process correctness
/// comes from the atomic create-only `SecItemAdd` below, never from
/// this mutex.
static CREATION_LOCK: Mutex<()> = Mutex::new(());

const WRAPPING_KEY_LEN: usize = 32;
const NONCE_LEN: usize = 12;

/// macOS Keychain protector (method id 2; DPAPI is 1, plain is 0).
pub struct MacosKeyProtector {
    /// The wrapping key, fetched (or, on first protect, created) at
    /// most once per instance so frequent users such as clipboard
    /// history do not hit the Keychain on every operation.
    cached_key: Mutex<Option<[u8; WRAPPING_KEY_LEN]>>,
}

impl std::fmt::Debug for MacosKeyProtector {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Never print key material (the cache is intentionally omitted).
        f.debug_struct("MacosKeyProtector").field("service", &SERVICE).finish_non_exhaustive()
    }
}

impl Default for MacosKeyProtector {
    fn default() -> Self {
        Self::new()
    }
}

impl MacosKeyProtector {
    pub fn new() -> Self {
        Self { cached_key: Mutex::new(None) }
    }

    /// The stored wrapping key, or `Ok(None)` only when the Keychain
    /// item is genuinely absent. Any other outcome fails closed.
    fn fetch_key(&self) -> io::Result<Option<[u8; WRAPPING_KEY_LEN]>> {
        match generic_password(PasswordOptions::new_generic_password(SERVICE, ACCOUNT)) {
            Ok(bytes) => {
                let key: [u8; WRAPPING_KEY_LEN] = bytes.as_slice().try_into().map_err(|_| {
                    io::Error::new(
                        io::ErrorKind::InvalidData,
                        "the Keychain wrapping key has an unexpected length; refusing to use or replace it",
                    )
                })?;
                Ok(Some(key))
            }
            Err(e) if e.code() == ERR_SEC_ITEM_NOT_FOUND => Ok(None),
            Err(e) => Err(io::Error::other(format!(
                "cannot read the Keychain wrapping key (OSStatus {}); refusing to regenerate it",
                e.code()
            ))),
        }
    }

    /// Atomically creates the wrapping-key item with `candidate`
    /// (`SecItemAdd`): `Ok(true)` when this call created it,
    /// `Ok(false)` when an item already existed — in which case the
    /// existing item is left exactly as-is (never updated, never
    /// deleted) and `candidate` must be discarded.
    fn add_key_create_only(&self, candidate: &[u8; WRAPPING_KEY_LEN]) -> io::Result<bool> {
        let mut options = ItemAddOptions::new(ItemAddValue::Data {
            class: ItemClass::generic_password(),
            data: CFData::from_buffer(candidate),
        });
        options.set_service(SERVICE);
        options.set_account_name(ACCOUNT);
        match options.add() {
            Ok(()) => Ok(true),
            Err(e) if e.code() == ERR_SEC_DUPLICATE_ITEM => Ok(false),
            Err(e) => Err(io::Error::other(format!(
                "cannot store the Keychain wrapping key (OSStatus {})",
                e.code()
            ))),
        }
    }

    /// The wrapping key, creating a random one only when `create` is
    /// set (the protect path) and the item is absent.
    fn wrapping_key(&self, create: bool) -> io::Result<[u8; WRAPPING_KEY_LEN]> {
        if let Some(key) = *self.cached_key.lock().unwrap_or_else(|e| e.into_inner()) {
            return Ok(key);
        }
        // One fetch-or-create at a time per process; another instance
        // may have filled our cache-relevant Keychain item meanwhile,
        // which the fetch inside the lock then observes.
        let _creation_guard = CREATION_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(key) = *self.cached_key.lock().unwrap_or_else(|e| e.into_inner()) {
            return Ok(key);
        }
        let key = match self.fetch_key()? {
            Some(key) => key,
            None if !create => {
                return Err(io::Error::new(
                    io::ErrorKind::NotFound,
                    "the Keychain wrapping key is missing; refusing to regenerate it while unlocking",
                ));
            }
            None => {
                let mut candidate = [0u8; WRAPPING_KEY_LEN];
                SystemRandom::new()
                    .fill(&mut candidate)
                    .map_err(|_| io::Error::other("cannot generate a random Keychain wrapping key"))?;
                get_or_create_with(|| self.fetch_key(), |c| self.add_key_create_only(c), &candidate)?
            }
        };
        *self.cached_key.lock().unwrap_or_else(|e| e.into_inner()) = Some(key);
        Ok(key)
    }
}

/// Create-if-absent resolution: returns the stored key when one
/// exists; otherwise offers `candidate` to the atomic `add`. When
/// `add` reports the item already existed (a racing creator won),
/// the winner's key is re-read and returned — `candidate` is never
/// used and the existing item is never overwritten. Kept generic
/// over the store operations so the race semantics are testable
/// with a fake store and synthetic keys.
fn get_or_create_with(
    fetch: impl Fn() -> io::Result<Option<[u8; WRAPPING_KEY_LEN]>>,
    add: impl Fn(&[u8; WRAPPING_KEY_LEN]) -> io::Result<bool>,
    candidate: &[u8; WRAPPING_KEY_LEN],
) -> io::Result<[u8; WRAPPING_KEY_LEN]> {
    if let Some(key) = fetch()? {
        return Ok(key);
    }
    if add(candidate)? {
        return Ok(*candidate);
    }
    fetch()?.ok_or_else(|| {
        io::Error::other("the Keychain wrapping key appeared during creation but cannot be read back")
    })
}

/// Seals `plaintext` under `key` with `nonce`:
/// `[FORMAT_VERSION, nonce.., ciphertext || tag]`. Pure (no Keychain),
/// so tests use synthetic keys only.
fn seal_with_key(
    key: &[u8; WRAPPING_KEY_LEN],
    nonce: [u8; NONCE_LEN],
    plaintext: &[u8],
) -> io::Result<Vec<u8>> {
    let key = LessSafeKey::new(
        UnboundKey::new(&AES_256_GCM, key).map_err(|_| io::Error::other("invalid wrapping key"))?,
    );
    let mut sealed = Vec::with_capacity(1 + NONCE_LEN + plaintext.len() + AES_256_GCM.tag_len());
    sealed.push(FORMAT_VERSION);
    sealed.extend_from_slice(&nonce);
    let mut body = plaintext.to_vec();
    key.seal_in_place_append_tag(Nonce::assume_unique_for_key(nonce), Aad::from(AAD), &mut body)
        .map_err(|_| io::Error::other("cannot seal the device key"))?;
    sealed.extend_from_slice(&body);
    Ok(sealed)
}

/// Opens a blob from [`seal_with_key`]. Authentication failure,
/// truncation, and unknown versions are errors (fail closed).
fn open_with_key(key: &[u8; WRAPPING_KEY_LEN], sealed: &[u8]) -> io::Result<Vec<u8>> {
    let invalid = |msg: &'static str| io::Error::new(io::ErrorKind::InvalidData, msg);
    let (&version, rest) = sealed.split_first().ok_or_else(|| invalid("the sealed device key is empty"))?;
    if version != FORMAT_VERSION {
        return Err(invalid("the sealed device key has an unknown format version"));
    }
    if rest.len() < NONCE_LEN + AES_256_GCM.tag_len() {
        return Err(invalid("the sealed device key is truncated"));
    }
    let (nonce, body) = rest.split_at(NONCE_LEN);
    let nonce: [u8; NONCE_LEN] = nonce.try_into().expect("nonce length checked above");
    let key = LessSafeKey::new(
        UnboundKey::new(&AES_256_GCM, key).map_err(|_| io::Error::other("invalid wrapping key"))?,
    );
    let mut body = body.to_vec();
    let plain = key
        .open_in_place(Nonce::assume_unique_for_key(nonce), Aad::from(AAD), &mut body)
        .map_err(|_| invalid("the sealed device key does not authenticate"))?;
    Ok(plain.to_vec())
}

impl KeyProtector for MacosKeyProtector {
    fn id(&self) -> u8 {
        2
    }

    fn protect(&self, plaintext: &[u8]) -> io::Result<Vec<u8>> {
        let key = self.wrapping_key(true)?;
        let mut nonce = [0u8; NONCE_LEN];
        SystemRandom::new()
            .fill(&mut nonce)
            .map_err(|_| io::Error::other("cannot generate a random seal nonce"))?;
        seal_with_key(&key, nonce, plaintext)
    }

    fn unprotect(&self, ciphertext: &[u8]) -> io::Result<Vec<u8>> {
        // Never creates a wrapping key: a missing one is a hard error.
        let key = self.wrapping_key(false)?;
        open_with_key(&key, ciphertext)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Synthetic key only: these tests never touch a real Keychain.
    const SYNTHETIC_KEY: [u8; WRAPPING_KEY_LEN] = [0x5a; WRAPPING_KEY_LEN];

    #[test]
    fn seal_open_round_trips_with_a_synthetic_key() {
        let sealed = seal_with_key(&SYNTHETIC_KEY, [1; NONCE_LEN], b"device key bytes").unwrap();
        assert_eq!(sealed[0], FORMAT_VERSION);
        assert!(!sealed.windows(16).any(|w| w == &b"device key bytes"[..]), "plaintext must not appear");
        assert_eq!(open_with_key(&SYNTHETIC_KEY, &sealed).unwrap(), b"device key bytes");
    }

    #[test]
    fn corrupted_or_foreign_sealed_data_fails_closed() {
        let sealed = seal_with_key(&SYNTHETIC_KEY, [2; NONCE_LEN], b"device key bytes").unwrap();
        // Wrong key, tampered ciphertext, truncation, bad version.
        assert!(open_with_key(&[0x11; WRAPPING_KEY_LEN], &sealed).is_err());
        let mut tampered = sealed.clone();
        *tampered.last_mut().unwrap() ^= 1;
        assert!(open_with_key(&SYNTHETIC_KEY, &tampered).is_err());
        assert!(open_with_key(&SYNTHETIC_KEY, &sealed[..8]).is_err());
        let mut future = sealed.clone();
        future[0] = FORMAT_VERSION + 1;
        assert!(open_with_key(&SYNTHETIC_KEY, &future).is_err());

        // Create-if-absent race against a fake store: the first
        // creator's key wins, and a racing creator's different
        // candidate is neither stored nor used (never an upsert).
        let store: Mutex<Option<[u8; WRAPPING_KEY_LEN]>> = Mutex::new(None);
        let attempt = |candidate: &[u8; WRAPPING_KEY_LEN]| {
            get_or_create_with(
                || Ok(*store.lock().unwrap()),
                |c| {
                    let mut slot = store.lock().unwrap();
                    if slot.is_some() {
                        Ok(false)
                    } else {
                        *slot = Some(*c);
                        Ok(true)
                    }
                },
                candidate,
            )
        };
        assert_eq!(attempt(&[0xa1; WRAPPING_KEY_LEN]).unwrap(), [0xa1; WRAPPING_KEY_LEN]);
        assert_eq!(attempt(&[0xb2; WRAPPING_KEY_LEN]).unwrap(), [0xa1; WRAPPING_KEY_LEN]);
        assert_eq!(*store.lock().unwrap(), Some([0xa1; WRAPPING_KEY_LEN]));
    }
}
