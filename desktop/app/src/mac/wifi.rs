// SPDX-License-Identifier: GPL-3.0-or-later
//! Explicitly requested hotspot association using CoreWLAN and the user's Wi-Fi
//! Keychain. Joining never powers Wi-Fi on, shells out, logs credentials, or
//! changes the saved phone configuration. Location access is requested only by
//! the explicit discovery-permission setup action.
//! All unsafe CoreWLAN C ABI calls are confined here; each call documents its
//! pointer lifetime and ownership at the call site.
#![allow(unsafe_code)]

use std::ffi::{CStr, CString, c_char, c_void};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reach {
    NotInRange,
    InRange,
    Connected,
}

unsafe extern "C" {
    fn waypair_wifi_permission_error() -> *mut c_char;
    fn waypair_wifi_request_permission() -> *mut c_char;
    fn waypair_wifi_saved_networks() -> *mut c_char;
    fn waypair_wifi_reach(name: *const c_char) -> i32;
    fn waypair_wifi_connect(
        name: *const c_char,
        current: unsafe extern "C" fn(*mut c_void) -> bool,
        owner: *mut c_void,
    ) -> *mut c_char;
    fn waypair_wifi_connect_ephemeral(name: *const c_char, password: *const c_char, security: i32,
        current: unsafe extern "C" fn(*mut c_void) -> bool, owner: *mut c_void) -> *mut c_char;
    fn waypair_wifi_profile(peer: *const c_char) -> *mut c_char;
    fn waypair_wifi_set_profile(peer: *const c_char, name: *const c_char) -> i32;
    fn waypair_wifi_free(value: *mut c_char);
}

unsafe fn take(value: *mut c_char) -> Option<String> {
    if value.is_null() {
        return None;
    }
    // SAFETY: native functions return a NUL-terminated strdup, freed exactly once.
    let result = unsafe { CStr::from_ptr(value) }.to_string_lossy().into_owned();
    unsafe { waypair_wifi_free(value) };
    Some(result)
}

pub fn permission_error() -> Option<String> {
    // SAFETY: no borrowed arguments; returned allocation is consumed.
    unsafe { take(waypair_wifi_permission_error()) }
}

pub fn request_discovery_permission() -> Result<(), String> {
    // SAFETY: called on the UI thread only by the explicit setup action.
    match unsafe { take(waypair_wifi_request_permission()) } {
        None => Ok(()),
        Some(reason) => Err(reason),
    }
}

pub fn saved_networks() -> Vec<String> {
    // SAFETY: returned JSON contains SSIDs only, kept in memory.
    unsafe { take(waypair_wifi_saved_networks()) }
        .and_then(|json| serde_json::from_str(&json).ok())
        .unwrap_or_default()
}

pub fn scan() { /* reach performs a targeted scan for the chosen SSID. */
}

pub fn reach(profile: &str) -> Reach {
    let Ok(name) = CString::new(profile) else { return Reach::NotInRange };
    // SAFETY: the string remains live throughout this blocking native call.
    match unsafe { waypair_wifi_reach(name.as_ptr()) } {
        2 => Reach::Connected,
        1 => Reach::InRange,
        _ => Reach::NotInRange,
    }
}

pub fn connect_owned(profile: &str, current: &dyn Fn() -> bool) -> Result<(), String> {
    let name = CString::new(profile).map_err(|_| "Invalid saved hotspot profile")?;
    unsafe extern "C" fn is_current(owner: *mut c_void) -> bool {
        // SAFETY: native code borrows the address of this gate reference only
        // for the synchronous call, including any scan/Keychain permission UI.
        let gate = unsafe { &*owner.cast::<&dyn Fn() -> bool>() };
        gate()
    }
    let gate = current;
    let owner = (&gate as *const &dyn Fn() -> bool).cast_mut().cast::<c_void>();
    // SAFETY: name/gate outlive the synchronous call; neither is retained.
    match unsafe { take(waypair_wifi_connect(name.as_ptr(), is_current, owner)) } {
        None => Ok(()),
        Some(reason) => Err(reason),
    }
}

pub fn profile_for_peer(peer: &str) -> Option<String> {
    let peer = CString::new(peer).ok()?;
    // SAFETY: argument borrowed for this call, returned allocation consumed once.
    unsafe { take(waypair_wifi_profile(peer.as_ptr())) }
}

pub fn set_profile_for_peer(peer: &str, profile: Option<&str>) -> Result<(), String> {
    let peer = CString::new(peer).map_err(|_| "Invalid phone identity")?;
    let profile = profile.map(CString::new).transpose().map_err(|_| "Invalid saved hotspot profile")?;
    // SAFETY: both strings live until this call returns; a null name deletes only
    // the one peer's Waypair profile association, never its Wi-Fi credentials.
    let status = unsafe {
        waypair_wifi_set_profile(peer.as_ptr(), profile.as_ref().map_or(std::ptr::null(), |s| s.as_ptr()))
    };
    if status == 0 {
        Ok(())
    } else {
        Err("macOS could not save the hotspot choice in Keychain. Allow Keychain access and try again."
            .into())
    }
}

/// Credentials and owner are borrowed only during synchronous association.
pub fn connect_ephemeral_owned(network: &nectarlink_core::PhoneHotspotNetwork, current: &dyn Fn() -> bool) -> Result<(), String> {
    let name = CString::new(network.ssid.as_str()).map_err(|_| "Invalid hotspot network")?;
    let password = CString::new(network.passphrase.as_str()).map_err(|_| "Invalid hotspot credentials")?;
    let security = match network.security {
        nectarlink_core::PhoneHotspotSecurity::Wpa2 => 2,
        nectarlink_core::PhoneHotspotSecurity::Wpa3 => 3,
    };
    unsafe extern "C" fn is_current(owner: *mut c_void) -> bool {
        // SAFETY: native code borrows this gate only until the call returns.
        unsafe { (&*owner.cast::<&dyn Fn() -> bool>())() }
    }
    let gate = current;
    let owner = (&gate as *const &dyn Fn() -> bool).cast_mut().cast::<c_void>();
    // SAFETY: strings and gate remain live; native code retains none of them.
    match unsafe { take(waypair_wifi_connect_ephemeral(name.as_ptr(), password.as_ptr(), security, is_current, owner)) } {
        None => Ok(()), Some(reason) => Err(reason),
    }
}
