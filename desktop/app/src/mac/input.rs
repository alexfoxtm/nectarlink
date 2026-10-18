// SPDX-License-Identifier: GPL-3.0-or-later
//! macOS synthetic input stubs.

use nectarlink_core::{ButtonAction, KeyMod, MouseButton, SlideAction, TouchAction};

pub fn move_relative(_dx: f32, _dy: f32) {}
pub fn move_absolute(_x: f32, _y: f32) {}
pub fn mirror_touch(_action: TouchAction, _x: f32, _y: f32) {}
pub fn mouse_button(_button: MouseButton, _action: ButtonAction) {}
pub fn release_held_buttons() {}
pub fn scroll(_dx: f32, _dy: f32) {}
pub fn type_text(_text: &str) {}
pub fn press_key(_key: &str, _mods: &[KeyMod]) {}
pub fn slide(_action: SlideAction) {}
pub fn is_user_active_within_ms(_max_idle_ms: u32) -> bool {
    true
}
