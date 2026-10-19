// SPDX-License-Identifier: GPL-3.0-or-later
//! macOS synthetic input.
//!
//! Nothing is injected on macOS yet (posting events needs Accessibility
//! consent and isn't implemented), and `mirror.input` / `input.inject` aren't
//! advertised, so the injection functions are no-ops. Reading how long the
//! person has been idle needs no consent and is real, so the PC's "in use"
//! signal to phones and the updater's quiet-time check aren't fabricated.

use nectarlink_core::{ButtonAction, KeyMod, MouseButton, SlideAction, TouchAction};

/// `kCGEventSourceStateCombinedSessionState`: events in this login session.
const COMBINED_SESSION_STATE: i32 = 0;
/// `kCGAnyInputEventType`: any keyboard, mouse or other HID input.
const ANY_INPUT_EVENT_TYPE: u32 = u32::MAX;

// One read-only query of a public CoreGraphics function: no events are
// posted, and it needs no Accessibility or Input Monitoring permission.
#[allow(unsafe_code)]
#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" {
    /// `CGEventSourceSecondsSinceLastEventType`; pure, no side effects.
    #[link_name = "CGEventSourceSecondsSinceLastEventType"]
    safe fn seconds_since_last_event(state_id: i32, event_type: u32) -> f64;
}

pub fn move_relative(_dx: f32, _dy: f32) {}
#[allow(dead_code)]
pub fn move_absolute(_x: f32, _y: f32) {}
#[allow(dead_code)]
pub fn mirror_touch(_action: TouchAction, _x: f32, _y: f32) {}
pub fn mouse_button(_button: MouseButton, _action: ButtonAction) {}
pub fn release_held_buttons() {}
pub fn scroll(_dx: f32, _dy: f32) {}
pub fn type_text(_text: &str) {}
pub fn press_key(_key: &str, _mods: &[KeyMod]) {}
pub fn slide(_action: SlideAction) {}

/// Whether someone used the keyboard, mouse or trackpad in the last
/// `max_idle_ms`, per the session's event source.
pub fn is_user_active_within_ms(max_idle_ms: u32) -> bool {
    let idle_secs = seconds_since_last_event(COMBINED_SESSION_STATE, ANY_INPUT_EVENT_TYPE);
    idle_secs.is_finite() && idle_secs >= 0.0 && idle_secs * 1000.0 <= f64::from(max_idle_ms)
}
