// SPDX-License-Identifier: GPL-3.0-or-later
//! macOS synthetic input for a paired phone acting as a touchpad, keyboard,
//! or presentation remote, and for reverse mirroring (the phone's finger
//! drives this Mac's pointer).
//!
//! The public functions match `win/input.rs`. Events are posted with public
//! CoreGraphics calls and only after `AXIsProcessTrusted` returns true. This
//! module never prompts for Accessibility and never grants it. Touch aimed at
//! a mirrored Android phone is not posted here; that path is `mirror::input`.
//!
//! `mirror.input` and `input.inject` stay unadvertised. Callers that already
//! refuse those features on Mac keep refusing them.

mod cg;
mod plan;

use std::sync::{Mutex, MutexGuard};

use nectarlink_core::{ButtonAction, KeyMod, MouseButton, PowerAction, SlideAction, TouchAction};

use plan::{Held, KeyStep, MoveKind, Place, PointerButton, PointerStep, Release, TextPiece};

/// Fractional remainders plus every button, modifier, and key this backend
/// has posted down and not yet released.
struct InputState {
    move_rem_x: f32,
    move_rem_y: f32,
    scroll_rem_x: f32,
    scroll_rem_y: f32,
    held: Held,
}

static STATE: Mutex<InputState> = Mutex::new(InputState {
    move_rem_x: 0.0,
    move_rem_y: 0.0,
    scroll_rem_x: 0.0,
    scroll_rem_y: 0.0,
    held: Held {
        left: false,
        right: false,
        middle: false,
        control: false,
        option: false,
        shift: false,
        command: false,
        key: None,
        unicode: None,
        media: None,
    },
});

fn lock() -> MutexGuard<'static, InputState> {
    STATE.lock().unwrap_or_else(|err| err.into_inner())
}

fn ensure_hook() {
    cg::arm_teardown(release_held_buttons);
}

fn move_event(kind: MoveKind) -> (u32, u32) {
    match kind {
        MoveKind::Move => (cg::MOVED, cg::BUTTON_LEFT),
        MoveKind::LeftDrag => (cg::LEFT_DRAGGED, cg::BUTTON_LEFT),
        MoveKind::RightDrag => (cg::RIGHT_DRAGGED, cg::BUTTON_RIGHT),
        MoveKind::MiddleDrag => (cg::OTHER_DRAGGED, cg::BUTTON_CENTER),
    }
}

fn button_event(button: PointerButton, down: bool) -> (u32, u32) {
    match (button, down) {
        (PointerButton::Left, true) => (cg::LEFT_DOWN, cg::BUTTON_LEFT),
        (PointerButton::Left, false) => (cg::LEFT_UP, cg::BUTTON_LEFT),
        (PointerButton::Right, true) => (cg::RIGHT_DOWN, cg::BUTTON_RIGHT),
        (PointerButton::Right, false) => (cg::RIGHT_UP, cg::BUTTON_RIGHT),
        (PointerButton::Middle, true) => (cg::OTHER_DOWN, cg::BUTTON_CENTER),
        (PointerButton::Middle, false) => (cg::OTHER_UP, cg::BUTTON_CENTER),
    }
}

fn post_button(session: &cg::Session, button: PointerButton, down: bool) -> bool {
    let Some((x, y)) = cg::cursor() else { return false };
    let (event_type, cg_button) = button_event(button, down);
    session.post_mouse(event_type, cg_button, x, y, 0, 0)
}

fn post_key_step(session: &cg::Session, step: KeyStep) -> bool {
    match step {
        KeyStep::Modifier { kind, down, flags } => session.post_key(kind.keycode(), down, flags),
        KeyStep::Key { code, down, flags } => session.post_key(code, down, flags),
        KeyStep::Media { usage, down } => session.post_media(usage, down),
        KeyStep::Lock => false,
    }
}

/// If a chord stops halfway, `Drop` posts the missing ups.
struct Unwind<'a> {
    session: &'a cg::Session,
    posted: Vec<KeyStep>,
    armed: bool,
}

impl Drop for Unwind<'_> {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        for undo in plan::unwind_key_steps(&self.posted) {
            if post_key_step(self.session, undo) {
                let mut state = lock();
                state.held = plan::note_key_step(state.held, undo);
            }
        }
    }
}

fn post_steps(steps: &[KeyStep]) {
    if !cg::trusted() {
        return;
    }
    let Some(session) = cg::Session::new() else { return };
    ensure_hook();
    // One flags snapshot for the chord. Physically held modifiers are not
    // pressed or stored. A change after this read is an acceptance gate.
    let steps = plan::adopt_physical_baseline(steps, cg::flags_state());
    let mut guard = Unwind { session: &session, posted: Vec::new(), armed: true };
    for step in &steps {
        if !post_key_step(&session, *step) {
            return;
        }
        guard.posted.push(*step);
        let mut state = lock();
        state.held = plan::note_key_step(state.held, *step);
    }
    guard.armed = false;
}

fn post_release(session: &cg::Session, release: Release, baseline: u64, owned: Held) -> bool {
    let flags = plan::release_event_flags(owned, baseline, release);
    match release {
        Release::Button(button) => post_button(session, button, false),
        Release::Modifier(kind) => session.post_key(kind.keycode(), false, flags),
        Release::Key(code) => session.post_key(code, false, flags),
        Release::Unicode((buf, n)) => {
            let n = usize::from(n).min(buf.len());
            if n == 0 {
                return false;
            }
            session.post_unicode(&buf[..n], false, flags)
        }
        Release::Media(usage) => session.post_media(usage, false),
    }
}

/// Moves the pointer by `(dx, dy)` points. Fractions are kept for the next
/// move. Deltas outside `[-4000, 4000]` are clamped. Nothing is posted when
/// this process is not a trusted accessibility client.
pub fn move_relative(dx: f32, dy: f32) {
    let current = {
        let state = lock();
        plan::Remainders { x: state.move_rem_x, y: state.move_rem_y }
    };
    let Some((ix, iy, next)) = plan::plan_relative(current, dx, dy) else { return };
    if ix == 0 && iy == 0 {
        let mut state = lock();
        state.move_rem_x = next.x;
        state.move_rem_y = next.y;
        return;
    }
    if !cg::trusted() {
        return;
    }
    let Some(session) = cg::Session::new() else { return };
    let Some((x, y)) = cg::cursor() else { return };
    let Some(edges) = cg::desktop_edges() else { return };
    let Some((px, py)) = plan::clamp_point(
        x + f64::from(ix),
        y + f64::from(iy),
        edges.min_x,
        edges.min_y,
        edges.max_x,
        edges.max_y,
    ) else {
        return;
    };
    let (event_type, button) = move_event(plan::move_kind(lock().held));
    if session.post_mouse(event_type, button, px, py, i64::from(ix), i64::from(iy)) {
        ensure_hook();
        let mut state = lock();
        state.move_rem_x = next.x;
        state.move_rem_y = next.y;
    }
}

/// Moves the pointer to a normalized point on the primary display (`0.0..=1.0`).
#[allow(dead_code)]
pub fn move_absolute(x: f32, y: f32) {
    if plan::plan_unit(x, y).is_none() || !cg::trusted() {
        return;
    }
    let Some(rect) = cg::primary_bounds() else { return };
    let Some((px, py)) = plan::map_primary(x, y, rect.origin_x, rect.origin_y, rect.width, rect.height)
    else {
        return;
    };
    let Some(session) = cg::Session::new() else { return };
    let (event_type, button) = move_event(plan::move_kind(lock().held));
    if session.post_mouse(event_type, button, px, py, 0, 0) {
        ensure_hook();
    }
}

/// Reverse-mirror touch on this Mac: absolute pointer motion plus the left
/// button. Phone-screen touches use `mirror::input`, not this platform adapter.
#[allow(dead_code)]
pub fn mirror_touch(action: TouchAction, x: f32, y: f32) {
    let Some(place) = plan::plan_local_touch(action, x, y) else {
        return;
    };
    if !cg::trusted() {
        return;
    }
    let Some(rect) = cg::primary_bounds() else { return };
    let Some((px, py)) = plan::map_primary(x, y, rect.origin_x, rect.origin_y, rect.width, rect.height)
    else {
        return;
    };
    let Some(session) = cg::Session::new() else { return };
    let (event_type, button, down) = match place {
        Place::Move => {
            let (event_type, button) = move_event(plan::move_kind(lock().held));
            (event_type, button, None)
        }
        Place::LeftDown => (cg::LEFT_DOWN, cg::BUTTON_LEFT, Some(true)),
        Place::LeftUp => (cg::LEFT_UP, cg::BUTTON_LEFT, Some(false)),
    };
    if !session.post_mouse(event_type, button, px, py, 0, 0) {
        return;
    }
    ensure_hook();
    if let Some(down) = down {
        let mut state = lock();
        state.held = plan::set_button(state.held, PointerButton::Left, down);
    }
}

/// Presses, releases, or clicks a mouse button without moving the pointer.
pub fn mouse_button(button: MouseButton, action: ButtonAction) {
    if !cg::trusted() {
        return;
    }
    let Some(session) = cg::Session::new() else { return };
    let pointer = PointerButton::from(button);
    for step in plan::plan_button(button, action) {
        let down = matches!(step, PointerStep::Down(_));
        if !post_button(&session, pointer, down) {
            return;
        }
        ensure_hook();
        let mut state = lock();
        state.held = plan::set_button(state.held, pointer, down);
    }
}

/// Releases buttons, modifiers, and keys this backend still holds.
///
/// When the process is not trusted the posts are skipped and the bookkeeping
/// is kept, so a later trusted call can still lift them. A failed post leaves
/// that one item held.
pub fn release_held_buttons() {
    let held = lock().held;
    if held == Held::default() {
        let mut state = lock();
        state.move_rem_x = 0.0;
        state.move_rem_y = 0.0;
        state.scroll_rem_x = 0.0;
        state.scroll_rem_y = 0.0;
        return;
    }
    if !cg::trusted() {
        return;
    }
    let Some(session) = cg::Session::new() else { return };
    ensure_hook();
    // `held` is already an owned copy, so this loop does not keep the mutex
    // guard alive across a post. The baseline is one flags snapshot with this
    // backend's bits removed. A physical change after the snapshot is an
    // acceptance gate.
    let baseline = plan::physical_baseline(cg::flags_state(), held);
    let mut owned = held;
    for step in plan::release_plan(held) {
        if post_release(&session, step, baseline, owned) {
            owned = plan::clear_release(owned, step);
            let mut state = lock();
            state.held = plan::clear_release(state.held, step);
        }
    }
    if lock().held == Held::default() {
        let mut state = lock();
        state.move_rem_x = 0.0;
        state.move_rem_y = 0.0;
        state.scroll_rem_x = 0.0;
        state.scroll_rem_y = 0.0;
    }
}

/// Scrolls by wheel notches. `dy > 0` scrolls down (negative CoreGraphics axis
/// 1). `dx > 0` scrolls right. Each axis is clamped to `[-200, 200]`.
pub fn scroll(dx: f32, dy: f32) {
    let current = {
        let state = lock();
        plan::Remainders { x: state.scroll_rem_x, y: state.scroll_rem_y }
    };
    let Some((vertical, horizontal, next)) = plan::plan_scroll(current, dx, dy) else { return };
    if vertical == 0 && horizontal == 0 {
        let mut state = lock();
        state.scroll_rem_x = next.x;
        state.scroll_rem_y = next.y;
        return;
    }
    if !cg::trusted() {
        return;
    }
    let Some(session) = cg::Session::new() else { return };
    if session.post_scroll(vertical, horizontal) {
        let mut state = lock();
        state.scroll_rem_x = next.x;
        state.scroll_rem_y = next.y;
    }
}

/// Types Unicode text. Input longer than the mirror text cap is refused whole.
/// `\n` is Return, `\r` and other controls are skipped. Each scalar is one
/// down/up pair so a surrogate pair is not split.
pub fn type_text(text: &str) {
    let Some(pieces) = plan::plan_text(text) else { return };
    if pieces.is_empty() || !cg::trusted() {
        return;
    }
    let Some(session) = cg::Session::new() else { return };
    ensure_hook();
    let baseline = cg::flags_state();
    for piece in pieces {
        let posted = match piece {
            TextPiece::Return => tap_key(&session, 0x24, baseline),
            TextPiece::Units(buf, n) => tap_unicode(&session, &buf[..usize::from(n)], baseline),
        };
        if !posted {
            return;
        }
    }
}

fn tap_key(session: &cg::Session, code: u16, flags: u64) -> bool {
    if !session.post_key(code, true, flags) {
        return false;
    }
    {
        let mut state = lock();
        state.held.key = Some(code);
    }
    let up = session.post_key(code, false, flags) || session.post_key(code, false, flags);
    if up {
        let mut state = lock();
        if state.held.key == Some(code) {
            state.held.key = None;
        }
    }
    up
}

fn tap_unicode(session: &cg::Session, units: &[u16], flags: u64) -> bool {
    if units.is_empty() || units.len() > 2 {
        return false;
    }
    let mut buf = [0u16; 2];
    buf[..units.len()].copy_from_slice(units);
    let n = u8::try_from(units.len()).unwrap_or(0);
    if n == 0 || !session.post_unicode(units, true, flags) {
        return false;
    }
    {
        let mut state = lock();
        state.held.unicode = Some((buf, n));
    }
    let up = session.post_unicode(units, false, flags) || session.post_unicode(units, false, flags);
    if up {
        let mut state = lock();
        state.held.unicode = None;
    }
    up
}

/// Presses a named key or shortcut. Unknown names, over-long names, and more
/// than four modifiers are ignored. `"lock"` uses the existing power path.
pub fn press_key(key: &str, mods: &[KeyMod]) {
    let Some(steps) = plan::plan_key(key, mods) else { return };
    if steps.iter().any(|step| matches!(step, KeyStep::Lock)) {
        let _ = crate::links::power(PowerAction::Lock);
        return;
    }
    post_steps(&steps);
}

/// Presentation remote: next, previous, start, stop, or black.
pub fn slide(action: SlideAction) {
    post_steps(&plan::plan_slide(action));
}

/// Whether someone used the keyboard, mouse, or trackpad in the last
/// `max_idle_ms`, per the session event source. This is a read of
/// `CGEventSourceSecondsSinceLastEventType` and does not need Accessibility.
pub fn is_user_active_within_ms(max_idle_ms: u32) -> bool {
    let idle_secs = cg::idle_seconds();
    idle_secs.is_finite() && idle_secs >= 0.0 && idle_secs * 1000.0 <= f64::from(max_idle_ms)
}
