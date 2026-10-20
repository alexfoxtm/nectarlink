// SPDX-License-Identifier: GPL-3.0-or-later
//! Pure decisions for macOS local input. No CoreGraphics calls live here, so
//! tests can check bounds, chords, and which host an event belongs to without
//! posting anything.

use nectarlink_core::{ButtonAction, KeyMod, MouseButton, SlideAction, TouchAction};

/// Largest relative pointer step, in points. Matches `remote::MAX_MOVE_DELTA`.
pub const MAX_MOVE_DELTA: f32 = 4000.0;
/// Ids `CGGetOnlineDisplayList` may store here. A larger reported count is an
/// incomplete union and must not be clamped.
pub const DISPLAY_LIST_CAP: u32 = 16;
/// Largest scroll step, in wheel notches. Matches `remote::MAX_SCROLL_DELTA`.
pub const MAX_SCROLL_DELTA: f32 = 200.0;
/// Largest text blob this backend will post. Remote text is 256 bytes;
/// reverse-mirror text is [`nectarlink_core::MIRROR_MAX_TEXT_BYTES`].
pub const MAX_TEXT_BYTES: usize = nectarlink_core::MIRROR_MAX_TEXT_BYTES;
/// Protocol cap on modifiers attached to one key (`remote::MAX_MODS`).
pub const MAX_MODS: usize = 4;
/// Longest key name `win/input.rs` resolves. Anything longer is refused.
pub const MAX_KEY_LEN: usize = 32;

/// A modifier key this backend can hold down and later release.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Modifier {
    Control,
    Option,
    Shift,
    Command,
}

impl Modifier {
    /// Virtual keycode from HIToolbox `Events.h`.
    pub const fn keycode(self) -> u16 {
        match self {
            Self::Control => 0x3B, // kVK_Control
            Self::Option => 0x3A,  // kVK_Option
            Self::Shift => 0x38,   // kVK_Shift
            Self::Command => 0x37, // kVK_Command
        }
    }

    /// `CGEventFlags` bit from `IOLLEvent.h` (`NX_*MASK`).
    pub const fn flag(self) -> u64 {
        match self {
            Self::Control => 0x0004_0000, // kCGEventFlagMaskControl
            Self::Option => 0x0008_0000,  // kCGEventFlagMaskAlternate
            Self::Shift => 0x0002_0000,   // kCGEventFlagMaskShift
            Self::Command => 0x0010_0000, // kCGEventFlagMaskCommand
        }
    }

    fn from_protocol(m: KeyMod) -> Self {
        match m {
            // Explicit "ctrl" stays Control. Named edit shortcuts add Command
            // themselves; they do not reuse this mapping.
            KeyMod::Ctrl => Self::Control,
            KeyMod::Alt => Self::Option,
            KeyMod::Shift => Self::Shift,
            // The protocol's OS key. On the Mac that key is Command.
            KeyMod::Win => Self::Command,
        }
    }
}

/// One keyboard or media step, already ordered for posting.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyStep {
    Modifier {
        kind: Modifier,
        down: bool,
        flags: u64,
    },
    Key {
        code: u16,
        down: bool,
        flags: u64,
    },
    /// `NX_KEYTYPE_*` from IOKit `ev_keymap.h` (play, next, previous).
    Media {
        usage: i32,
        down: bool,
    },
    /// `press_key("lock")` goes through the existing power path, not CGEvent.
    Lock,
}

/// Mouse button this backend tracks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PointerButton {
    Left,
    Right,
    Middle,
}

impl From<MouseButton> for PointerButton {
    fn from(button: MouseButton) -> Self {
        match button {
            MouseButton::Left => Self::Left,
            MouseButton::Right => Self::Right,
            MouseButton::Middle => Self::Middle,
        }
    }
}

/// What a pointer event should do. Drag is chosen from held buttons so a
/// Mac move while a button is down is a drag, which is what Windows does
/// implicitly by leaving the button down in the input stream.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MoveKind {
    Move,
    LeftDrag,
    RightDrag,
    MiddleDrag,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PointerStep {
    Down(PointerButton),
    Up(PointerButton),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Place {
    Move,
    LeftDown,
    LeftUp,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Remainders {
    pub x: f32,
    pub y: f32,
}

/// Buttons, modifiers, and a non-modifier key we have posted down.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Held {
    pub left: bool,
    pub right: bool,
    pub middle: bool,
    pub control: bool,
    pub option: bool,
    pub shift: bool,
    pub command: bool,
    pub key: Option<u16>,
    /// UTF-16 units of a unicode key this backend posted down. Teardown posts
    /// these units up. Keycode 0 is `kVK_ANSI_A` and is not a stand-in.
    pub unicode: Option<([u16; 2], u8)>,
    pub media: Option<i32>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Release {
    Button(PointerButton),
    Modifier(Modifier),
    Key(u16),
    /// Matching unicode up for [`Held::unicode`]. Not a keycode-0 key-up.
    Unicode(([u16; 2], u8)),
    Media(i32),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TextPiece {
    Return,
    /// UTF-16 code units of one scalar, length 1 or 2. Never a lone surrogate.
    Units([u16; 2], u8),
}

/// `kVK_ANSI_*` for `a`..=`z`, from HIToolbox `Events.h`.
fn letter_code(b: u8) -> Option<u16> {
    Some(match b {
        b'a' => 0x00,
        b'b' => 0x0B,
        b'c' => 0x08,
        b'd' => 0x02,
        b'e' => 0x0E,
        b'f' => 0x03,
        b'g' => 0x05,
        b'h' => 0x04,
        b'i' => 0x22,
        b'j' => 0x26,
        b'k' => 0x28,
        b'l' => 0x25,
        b'm' => 0x2E,
        b'n' => 0x2D,
        b'o' => 0x1F,
        b'p' => 0x23,
        b'q' => 0x0C,
        b'r' => 0x0F,
        b's' => 0x01,
        b't' => 0x11,
        b'u' => 0x20,
        b'v' => 0x09,
        b'w' => 0x0D,
        b'x' => 0x07,
        b'y' => 0x10,
        b'z' => 0x06,
        _ => return None,
    })
}

/// `kVK_ANSI_0`..=`kVK_ANSI_9`.
fn digit_code(b: u8) -> Option<u16> {
    Some(match b {
        b'0' => 0x1D,
        b'1' => 0x12,
        b'2' => 0x13,
        b'3' => 0x14,
        b'4' => 0x15,
        b'5' => 0x17,
        b'6' => 0x16,
        b'7' => 0x1A,
        b'8' => 0x1C,
        b'9' => 0x19,
        _ => return None,
    })
}

/// `kVK_F1`..=`kVK_F12`. The numbers are not contiguous in `Events.h`.
fn function_code(n: u8) -> Option<u16> {
    Some(match n {
        1 => 0x7A,
        2 => 0x78,
        3 => 0x63,
        4 => 0x76,
        5 => 0x60,
        6 => 0x61,
        7 => 0x62,
        8 => 0x64,
        9 => 0x65,
        10 => 0x6D,
        11 => 0x67,
        12 => 0x6F,
        _ => return None,
    })
}

fn named_key(key: &str) -> Option<u16> {
    Some(match key {
        "enter" => 0x24,       // kVK_Return
        "backspace" => 0x33,   // kVK_Delete (backspace)
        "tab" => 0x30,         // kVK_Tab
        "escape" => 0x35,      // kVK_Escape
        "space" => 0x31,       // kVK_Space
        "delete" => 0x75,      // kVK_ForwardDelete
        "insert" => 0x72,      // kVK_Help, the Apple extended-keyboard insert key
        "up" => 0x7E,          // kVK_UpArrow
        "down" => 0x7D,        // kVK_DownArrow
        "left" => 0x7B,        // kVK_LeftArrow
        "right" => 0x7C,       // kVK_RightArrow
        "home" => 0x73,        // kVK_Home
        "end" => 0x77,         // kVK_End
        "page_up" => 0x74,     // kVK_PageUp
        "page_down" => 0x79,   // kVK_PageDown
        "volume_up" => 0x48,   // kVK_VolumeUp
        "volume_down" => 0x49, // kVK_VolumeDown
        "volume_mute" => 0x4A, // kVK_Mute
        other => {
            let bytes = other.as_bytes();
            if let Some(n) = other.strip_prefix('f').and_then(|rest| rest.parse::<u8>().ok()) {
                return function_code(n);
            }
            if bytes.len() == 1 {
                return letter_code(bytes[0]).or_else(|| digit_code(bytes[0]));
            }
            return None;
        }
    })
}

/// Consumer-control usages. These are not ANSI virtual keycodes.
fn media_usage(key: &str) -> Option<i32> {
    Some(match key {
        "play_pause" => 16, // NX_KEYTYPE_PLAY
        "next_track" => 17, // NX_KEYTYPE_NEXT
        "prev_track" => 18, // NX_KEYTYPE_PREVIOUS
        _ => return None,
    })
}

/// Named shortcuts use the Mac chord that performs the same action.
/// Caller-supplied modifiers are not rewritten: explicit `ctrl` stays Control.
fn shortcut(key: &str) -> Option<(u16, &'static [Modifier])> {
    const COMMAND: &[Modifier] = &[Modifier::Command];
    const COMMAND_SHIFT: &[Modifier] = &[Modifier::Command, Modifier::Shift];
    const CONTROL: &[Modifier] = &[Modifier::Control];
    Some(match key {
        "copy" => (letter_code(b'c')?, COMMAND),
        "paste" => (letter_code(b'v')?, COMMAND),
        "cut" => (letter_code(b'x')?, COMMAND),
        "undo" => (letter_code(b'z')?, COMMAND),
        // Windows redo is Ctrl+Y. On the Mac the redo action is Command-Shift-Z.
        "redo" => (letter_code(b'z')?, COMMAND_SHIFT),
        "select_all" => (letter_code(b'a')?, COMMAND),
        // Windows task view is Win+Tab. Mission Control is Control-Up.
        "task_view" => (named_key("up")?, CONTROL),
        _ => return None,
    })
}

fn dedup(preset: &[Modifier], extra: &[KeyMod]) -> Option<Vec<Modifier>> {
    if extra.len() > MAX_MODS {
        return None;
    }
    let mut out = Vec::with_capacity(preset.len() + extra.len());
    for kind in preset.iter().copied().chain(extra.iter().copied().map(Modifier::from_protocol)) {
        if !out.contains(&kind) {
            out.push(kind);
        }
        if out.len() > MAX_MODS {
            return None;
        }
    }
    Some(out)
}

fn chord(code: u16, mods: &[Modifier]) -> Vec<KeyStep> {
    let mut running = 0u64;
    let mut steps = Vec::with_capacity(mods.len() * 2 + 2);
    for kind in mods {
        running |= kind.flag();
        steps.push(KeyStep::Modifier { kind: *kind, down: true, flags: running });
    }
    steps.push(KeyStep::Key { code, down: true, flags: running });
    steps.push(KeyStep::Key { code, down: false, flags: running });
    for kind in mods.iter().rev() {
        running &= !kind.flag();
        steps.push(KeyStep::Modifier { kind: *kind, down: false, flags: running });
    }
    steps
}

/// Rewrite a planned chord against one `CGEventSourceFlagsState` snapshot.
///
/// A modifier bit already set in `baseline` is physically held. Those steps
/// are dropped, so this backend does not press, release, or store them.
/// Every remaining key and modifier flag word is `baseline` plus the synthetic
/// bits the chord still owns. A physical press or release after this snapshot
/// is not visible here. That race is an acceptance gate, not a proof.
pub fn adopt_physical_baseline(steps: &[KeyStep], baseline: u64) -> Vec<KeyStep> {
    let mut out = Vec::with_capacity(steps.len());
    for step in steps {
        match *step {
            KeyStep::Modifier { kind, down, flags } => {
                if kind.flag() & baseline != 0 {
                    continue;
                }
                out.push(KeyStep::Modifier { kind, down, flags: baseline | flags });
            }
            KeyStep::Key { code, down, flags } => {
                out.push(KeyStep::Key { code, down, flags: baseline | flags });
            }
            other => out.push(other),
        }
    }
    out
}

/// Modifier bits this backend has stored as its own.
pub fn owned_modifier_flags(held: Held) -> u64 {
    let mut flags = 0u64;
    if held.control {
        flags |= Modifier::Control.flag();
    }
    if held.option {
        flags |= Modifier::Option.flag();
    }
    if held.shift {
        flags |= Modifier::Shift.flag();
    }
    if held.command {
        flags |= Modifier::Command.flag();
    }
    flags
}

/// Bits in one flags snapshot that this backend does not own.
///
/// Owned bits are removed so a later release lifts only those. Bits the user
/// was already holding stay. A modifier that changes after the snapshot is
/// the same acceptance gate as [`adopt_physical_baseline`].
pub fn physical_baseline(snapshot: u64, owned: Held) -> u64 {
    snapshot & !owned_modifier_flags(owned)
}

/// Flag word for one release: the physical baseline plus owned bits that are
/// still down after this release. Releasing one owned modifier does not clear
/// a bit that was only in the baseline.
pub fn release_event_flags(owned: Held, baseline: u64, release: Release) -> u64 {
    let mut synthetic = owned_modifier_flags(owned);
    if let Release::Modifier(kind) = release {
        synthetic &= !kind.flag();
    }
    baseline | synthetic
}

/// `reported` fits in `capacity`. Zero is not a list. A count above `capacity`
/// is a truncated display union and must not be clamped.
pub fn display_list_fits(reported: u32, capacity: u32) -> bool {
    reported > 0 && reported <= capacity
}

fn media_chord(usage: i32, mods: &[Modifier]) -> Vec<KeyStep> {
    let mut running = 0u64;
    let mut steps = Vec::with_capacity(mods.len() * 2 + 2);
    for kind in mods {
        running |= kind.flag();
        steps.push(KeyStep::Modifier { kind: *kind, down: true, flags: running });
    }
    steps.push(KeyStep::Media { usage, down: true });
    steps.push(KeyStep::Media { usage, down: false });
    for kind in mods.iter().rev() {
        running &= !kind.flag();
        steps.push(KeyStep::Modifier { kind: *kind, down: false, flags: running });
    }
    steps
}

/// The key sequence for `press_key`, or `None` when the name or modifiers
/// are outside the bounds this backend will post.
pub fn plan_key(key: &str, mods: &[KeyMod]) -> Option<Vec<KeyStep>> {
    if key.is_empty() || key.len() > MAX_KEY_LEN {
        return None;
    }
    if key == "lock" {
        return Some(vec![KeyStep::Lock]);
    }
    if let Some((code, preset)) = shortcut(key) {
        // Windows only applies a shortcut alias when the caller did not also
        // pass modifiers for the protocol aliases. Extra modifiers are still
        // merged, matching `press_key` in `win/input.rs`.
        return Some(chord(code, &dedup(preset, mods)?));
    }
    if let Some(usage) = media_usage(key) {
        return Some(media_chord(usage, &dedup(&[], mods)?));
    }
    let code = named_key(key)?;
    Some(chord(code, &dedup(&[], mods)?))
}

/// Presentation keys. Same actions as Windows: Page Down, Page Up, F5, Escape, B.
pub fn plan_slide(action: SlideAction) -> Vec<KeyStep> {
    let code = match action {
        SlideAction::Next => 0x79,     // kVK_PageDown
        SlideAction::Previous => 0x74, // kVK_PageUp
        SlideAction::Start => 0x60,    // kVK_F5
        SlideAction::Stop => 0x35,     // kVK_Escape
        SlideAction::Black => 0x0B,    // kVK_ANSI_B
    };
    chord(code, &[])
}

/// Steps still down, as ups, last-down first.
pub fn unwind_key_steps(posted: &[KeyStep]) -> Vec<KeyStep> {
    let mut open: Vec<KeyStep> = Vec::new();
    for step in posted {
        match *step {
            KeyStep::Modifier { down: true, .. }
            | KeyStep::Key { down: true, .. }
            | KeyStep::Media { down: true, .. } => open.push(*step),
            KeyStep::Modifier { kind, down: false, .. } => {
                if let Some(i) = open
                    .iter()
                    .rposition(|s| matches!(*s, KeyStep::Modifier { kind: k, down: true, .. } if k == kind))
                {
                    open.remove(i);
                }
            }
            KeyStep::Key { code, down: false, .. } => {
                if let Some(i) = open
                    .iter()
                    .rposition(|s| matches!(*s, KeyStep::Key { code: c, down: true, .. } if c == code))
                {
                    open.remove(i);
                }
            }
            KeyStep::Media { usage, down: false, .. } => {
                if let Some(i) = open
                    .iter()
                    .rposition(|s| matches!(*s, KeyStep::Media { usage: u, down: true } if u == usage))
                {
                    open.remove(i);
                }
            }
            KeyStep::Lock => {}
        }
    }
    open.iter()
        .rev()
        .map(|step| match *step {
            KeyStep::Modifier { kind, flags, .. } => {
                KeyStep::Modifier { kind, down: false, flags: flags & !kind.flag() }
            }
            KeyStep::Key { code, flags, .. } => KeyStep::Key { code, down: false, flags },
            KeyStep::Media { usage, .. } => KeyStep::Media { usage, down: false },
            KeyStep::Lock => KeyStep::Lock,
        })
        .collect()
}

fn step_delta(rem: f32, delta: f32, limit: f32) -> Option<(i32, f32)> {
    if !delta.is_finite() {
        return None;
    }
    let rem = if rem.is_finite() { rem } else { 0.0 };
    let total = rem + delta.clamp(-limit, limit);
    if !total.is_finite() {
        return None;
    }
    let mut stepped = total.round();
    if !stepped.is_finite() {
        return None;
    }
    stepped = stepped.clamp(-limit, limit);
    let ix = stepped as i32;
    let new_rem = (total - stepped).clamp(-1.0, 1.0);
    Some((ix, new_rem))
}

/// Integer point deltas plus the fractional remainder to keep.
/// `None` refuses a non-finite sample and leaves the caller's remainder alone.
pub fn plan_relative(rem: Remainders, dx: f32, dy: f32) -> Option<(i32, i32, Remainders)> {
    let (ix, rx) = step_delta(rem.x, dx, MAX_MOVE_DELTA)?;
    let (iy, ry) = step_delta(rem.y, dy, MAX_MOVE_DELTA)?;
    Some((ix, iy, Remainders { x: rx, y: ry }))
}

/// Line counts for a scroll. Vertical is CoreGraphics axis 1: positive scrolls
/// up, so protocol `dy > 0` (scroll down, the Windows wheel sign) is negated.
/// Horizontal axis 2 stays positive to the right.
pub fn plan_scroll(rem: Remainders, dx: f32, dy: f32) -> Option<(i32, i32, Remainders)> {
    if !dx.is_finite() || !dy.is_finite() {
        return None;
    }
    let dx = dx.clamp(-MAX_SCROLL_DELTA, MAX_SCROLL_DELTA);
    let dy = dy.clamp(-MAX_SCROLL_DELTA, MAX_SCROLL_DELTA);
    let (horizontal, rx) = step_delta(rem.x, dx, MAX_SCROLL_DELTA)?;
    let (vertical, ry) = step_delta(rem.y, -dy, MAX_SCROLL_DELTA)?;
    Some((vertical, horizontal, Remainders { x: rx, y: ry }))
}

pub fn plan_unit(x: f32, y: f32) -> Option<(f32, f32)> {
    if !x.is_finite() || !y.is_finite() {
        return None;
    }
    Some((x.clamp(0.0, 1.0), y.clamp(0.0, 1.0)))
}

/// Map a unit point into a display rect in top-left global coordinates.
/// `1.0` lands on the last point of the rect, not on the next display.
pub fn map_primary(
    x: f32,
    y: f32,
    origin_x: f64,
    origin_y: f64,
    width: f64,
    height: f64,
) -> Option<(f64, f64)> {
    let (x, y) = plan_unit(x, y)?;
    if !origin_x.is_finite() || !origin_y.is_finite() || !width.is_finite() || !height.is_finite() {
        return None;
    }
    if width <= 1.0 || height <= 1.0 {
        return None;
    }
    let max_x = origin_x + width - 1.0;
    let max_y = origin_y + height - 1.0;
    let px = origin_x + f64::from(x) * (width - 1.0);
    let py = origin_y + f64::from(y) * (height - 1.0);
    Some((px.clamp(origin_x, max_x), py.clamp(origin_y, max_y)))
}

pub fn clamp_point(x: f64, y: f64, min_x: f64, min_y: f64, max_x: f64, max_y: f64) -> Option<(f64, f64)> {
    if ![x, y, min_x, min_y, max_x, max_y].into_iter().all(f64::is_finite) {
        return None;
    }
    if max_x < min_x || max_y < min_y {
        return None;
    }
    Some((x.clamp(min_x, max_x), y.clamp(min_y, max_y)))
}

pub fn move_kind(held: Held) -> MoveKind {
    if held.left {
        MoveKind::LeftDrag
    } else if held.right {
        MoveKind::RightDrag
    } else if held.middle {
        MoveKind::MiddleDrag
    } else {
        MoveKind::Move
    }
}

pub fn plan_button(button: MouseButton, action: ButtonAction) -> Vec<PointerStep> {
    let button = PointerButton::from(button);
    match action {
        ButtonAction::Down => vec![PointerStep::Down(button)],
        ButtonAction::Up => vec![PointerStep::Up(button)],
        ButtonAction::Click => vec![PointerStep::Down(button), PointerStep::Up(button)],
    }
}

/// Reverse-mirror touch on this Mac. Phone-screen input has its separate
/// `mirror::input` transport and never reaches this platform adapter.
pub fn plan_local_touch(action: TouchAction, x: f32, y: f32) -> Option<Place> {
    let _ = plan_unit(x, y)?;
    Some(match action {
        TouchAction::Down => Place::LeftDown,
        TouchAction::Move => Place::Move,
        TouchAction::Up => Place::LeftUp,
    })
}

pub fn set_button(mut held: Held, button: PointerButton, down: bool) -> Held {
    match button {
        PointerButton::Left => held.left = down,
        PointerButton::Right => held.right = down,
        PointerButton::Middle => held.middle = down,
    }
    held
}

/// Releases in an order that lifts a stuck key before its modifiers, then buttons.
pub fn release_plan(held: Held) -> Vec<Release> {
    let mut out = Vec::new();
    if let Some(units) = held.unicode {
        out.push(Release::Unicode(units));
    }
    if let Some(code) = held.key {
        out.push(Release::Key(code));
    }
    if let Some(usage) = held.media {
        out.push(Release::Media(usage));
    }
    if held.command {
        out.push(Release::Modifier(Modifier::Command));
    }
    if held.shift {
        out.push(Release::Modifier(Modifier::Shift));
    }
    if held.option {
        out.push(Release::Modifier(Modifier::Option));
    }
    if held.control {
        out.push(Release::Modifier(Modifier::Control));
    }
    if held.left {
        out.push(Release::Button(PointerButton::Left));
    }
    if held.right {
        out.push(Release::Button(PointerButton::Right));
    }
    if held.middle {
        out.push(Release::Button(PointerButton::Middle));
    }
    out
}

pub fn clear_release(mut held: Held, release: Release) -> Held {
    match release {
        Release::Button(PointerButton::Left) => held.left = false,
        Release::Button(PointerButton::Right) => held.right = false,
        Release::Button(PointerButton::Middle) => held.middle = false,
        Release::Modifier(Modifier::Command) => held.command = false,
        Release::Modifier(Modifier::Shift) => held.shift = false,
        Release::Modifier(Modifier::Option) => held.option = false,
        Release::Modifier(Modifier::Control) => held.control = false,
        Release::Key(code) if held.key == Some(code) => held.key = None,
        Release::Key(_) => {}
        Release::Unicode(_) => held.unicode = None,
        Release::Media(usage) if held.media == Some(usage) => held.media = None,
        Release::Media(_) => {}
    }
    held
}

pub fn note_key_step(mut held: Held, step: KeyStep) -> Held {
    match step {
        KeyStep::Modifier { kind, down, .. } => match kind {
            Modifier::Control => held.control = down,
            Modifier::Option => held.option = down,
            Modifier::Shift => held.shift = down,
            Modifier::Command => held.command = down,
        },
        KeyStep::Key { code, down: true, .. } => held.key = Some(code),
        KeyStep::Key { code, down: false, .. } if held.key == Some(code) => held.key = None,
        KeyStep::Key { .. } => {}
        KeyStep::Media { usage, down: true } => held.media = Some(usage),
        KeyStep::Media { usage, down: false } if held.media == Some(usage) => held.media = None,
        KeyStep::Media { .. } => {}
        KeyStep::Lock => {}
    }
    held
}

/// Bounded text. Over-long input is refused whole. `\\r` is skipped, `\\n`
/// becomes Return, and other control characters are dropped. Each remaining
/// scalar is one UTF-16 sequence so a surrogate pair is never split.
pub fn plan_text(text: &str) -> Option<Vec<TextPiece>> {
    if text.len() > MAX_TEXT_BYTES {
        return None;
    }
    let mut out = Vec::new();
    for ch in text.chars() {
        if ch == '\r' || (ch.is_control() && ch != '\n') {
            continue;
        }
        if ch == '\n' {
            out.push(TextPiece::Return);
            continue;
        }
        let mut buf = [0u16; 2];
        let units = ch.encode_utf16(&mut buf);
        let n = u8::try_from(units.len()).unwrap_or(0);
        if n == 0 || n > 2 {
            continue;
        }
        out.push(TextPiece::Units(buf, n));
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_named_keys_and_shortcuts() {
        let enter = plan_key("enter", &[]).unwrap();
        assert_eq!(
            enter,
            vec![
                KeyStep::Key { code: 0x24, down: true, flags: 0 },
                KeyStep::Key { code: 0x24, down: false, flags: 0 },
            ]
        );
        let up = plan_key("up", &[]).unwrap();
        assert_eq!(up[0], KeyStep::Key { code: 0x7E, down: true, flags: 0 });

        let copy = plan_key("copy", &[]).unwrap();
        assert_eq!(
            copy,
            vec![
                KeyStep::Modifier { kind: Modifier::Command, down: true, flags: Modifier::Command.flag() },
                KeyStep::Key { code: 0x08, down: true, flags: Modifier::Command.flag() },
                KeyStep::Key { code: 0x08, down: false, flags: Modifier::Command.flag() },
                KeyStep::Modifier { kind: Modifier::Command, down: false, flags: 0 },
            ]
        );
        let paste = plan_key("paste", &[]).unwrap();
        assert!(matches!(
            paste[1],
            KeyStep::Key { code: 0x09, down: true, flags } if flags == Modifier::Command.flag()
        ));

        let task = plan_key("task_view", &[]).unwrap();
        assert_eq!(
            task[0],
            KeyStep::Modifier { kind: Modifier::Control, down: true, flags: Modifier::Control.flag() }
        );
        assert_eq!(task[1], KeyStep::Key { code: 0x7E, down: true, flags: Modifier::Control.flag() });

        let redo = plan_key("redo", &[]).unwrap();
        assert_eq!(
            redo[2],
            KeyStep::Key { code: 0x06, down: true, flags: Modifier::Command.flag() | Modifier::Shift.flag() }
        );

        assert_eq!(plan_key("a", &[]).unwrap()[0], KeyStep::Key { code: 0x00, down: true, flags: 0 });
        assert_eq!(plan_key("f5", &[]).unwrap()[0], KeyStep::Key { code: 0x60, down: true, flags: 0 });
        assert_eq!(plan_slide(SlideAction::Next)[0], KeyStep::Key { code: 0x79, down: true, flags: 0 });
        assert_eq!(
            plan_key("play_pause", &[]).unwrap(),
            vec![KeyStep::Media { usage: 16, down: true }, KeyStep::Media { usage: 16, down: false },]
        );

        // Explicit ctrl stays Control, even next to the Command that "copy" adds.
        let mixed = plan_key("copy", &[KeyMod::Ctrl]).unwrap();
        assert!(
            mixed
                .iter()
                .any(|s| { matches!(*s, KeyStep::Modifier { kind: Modifier::Control, down: true, .. }) })
        );

        assert!(plan_key("unknown_key", &[]).is_none());
        assert!(plan_key("win", &[]).is_none());
        assert!(plan_key("A", &[]).is_none());
        assert!(plan_key(&"k".repeat(MAX_KEY_LEN + 1), &[]).is_none());
        let too_many = [KeyMod::Ctrl, KeyMod::Alt, KeyMod::Shift, KeyMod::Win, KeyMod::Ctrl];
        assert!(plan_key("a", &too_many).is_none());
        assert_eq!(plan_key("lock", &[KeyMod::Shift]).unwrap(), vec![KeyStep::Lock]);

        // A zero baseline leaves the planned chord alone.
        assert_eq!(adopt_physical_baseline(&copy, 0), copy);

        // Physical Command is not pressed, released, or stored. The key still
        // carries that bit because it is already down.
        let physical_command = adopt_physical_baseline(&copy, Modifier::Command.flag());
        assert!(physical_command.iter().all(|step| !matches!(*step, KeyStep::Modifier { .. })));
        assert_eq!(
            physical_command,
            vec![
                KeyStep::Key { code: 0x08, down: true, flags: Modifier::Command.flag() },
                KeyStep::Key { code: 0x08, down: false, flags: Modifier::Command.flag() },
            ]
        );
        let mut owned = Held::default();
        for step in &physical_command {
            owned = note_key_step(owned, *step);
        }
        assert!(!owned.command);

        // Physical Shift stays in the flag word. This backend does not press or
        // release it, and the Command-up that it does own does not clear it.
        let redo_steps = plan_key("redo", &[]).unwrap();
        let physical_shift = adopt_physical_baseline(&redo_steps, Modifier::Shift.flag());
        assert!(
            physical_shift
                .iter()
                .all(|step| { !matches!(*step, KeyStep::Modifier { kind: Modifier::Shift, .. }) })
        );
        assert!(physical_shift.iter().any(|step| {
            matches!(
                *step,
                KeyStep::Key { code: 0x06, down: true, flags }
                    if flags == Modifier::Command.flag() | Modifier::Shift.flag()
            )
        }));
        assert!(physical_shift.iter().any(|step| {
            matches!(
                *step,
                KeyStep::Modifier { kind: Modifier::Command, down: false, flags }
                    if flags == Modifier::Shift.flag()
            )
        }));
        owned = Held::default();
        for step in &physical_shift {
            owned = note_key_step(owned, *step);
        }
        assert!(!owned.shift);
        assert!(!owned.command);
    }

    #[test]
    fn bounds_pointer_text_and_releases_without_crossing_hosts() {
        assert!(plan_relative(Remainders::default(), f32::NAN, 1.0).is_none());
        let (ix, iy, rem) = plan_relative(Remainders::default(), 10_000.0, 0.4).unwrap();
        assert_eq!((ix, iy), (4000, 0));
        assert!(rem.y > 0.0 && rem.y < 1.0);
        let (ix2, _, _) = plan_relative(Remainders { x: 0.6, y: 0.0 }, 4000.0, 0.0).unwrap();
        assert_eq!(ix2, 4000);

        assert!(plan_scroll(Remainders::default(), f32::INFINITY, 0.0).is_none());
        let (vertical, horizontal, _) = plan_scroll(Remainders::default(), 1.0, 1.0).unwrap();
        assert_eq!((vertical, horizontal), (-1, 1));
        let (vertical, _, _) = plan_scroll(Remainders::default(), 0.0, 10_000.0).unwrap();
        assert_eq!(vertical, -200);

        assert!(plan_unit(f32::NAN, 0.0).is_none());
        assert_eq!(plan_unit(-1.0, 2.0), Some((0.0, 1.0)));
        assert_eq!(map_primary(1.0, 1.0, 0.0, 0.0, 1000.0, 800.0), Some((999.0, 799.0)));
        assert!(map_primary(0.0, 0.0, 0.0, 0.0, 1.0, 800.0).is_none());

        assert_eq!(plan_local_touch(TouchAction::Down, 2.0, -1.0), Some(Place::LeftDown));
        assert!(plan_local_touch(TouchAction::Move, f32::NAN, 0.0).is_none());
        assert_eq!(move_kind(Held { left: true, ..Held::default() }), MoveKind::LeftDrag);
        assert_eq!(move_kind(Held::default()), MoveKind::Move);

        let mut held = set_button(Held::default(), PointerButton::Left, true);
        held.command = true;
        held.key = Some(0x08);
        assert_eq!(
            release_plan(held),
            vec![
                Release::Key(0x08),
                Release::Modifier(Modifier::Command),
                Release::Button(PointerButton::Left),
            ]
        );
        assert_eq!(clear_release(held, Release::Key(0x08)).key, None);

        let partial = [
            KeyStep::Modifier { kind: Modifier::Command, down: true, flags: Modifier::Command.flag() },
            KeyStep::Key { code: 0x08, down: true, flags: Modifier::Command.flag() },
        ];
        assert_eq!(
            unwind_key_steps(&partial),
            vec![
                KeyStep::Key { code: 0x08, down: false, flags: Modifier::Command.flag() },
                KeyStep::Modifier { kind: Modifier::Command, down: false, flags: 0 },
            ]
        );
        assert!(unwind_key_steps(&plan_key("copy", &[]).unwrap()).is_empty());

        assert!(plan_text(&"a".repeat(MAX_TEXT_BYTES + 1)).is_none());
        let pieces = plan_text("a\r\nb").unwrap();
        assert_eq!(pieces.len(), 3);
        assert_eq!(pieces[0], TextPiece::Units([u16::from(b'a'), 0], 1));
        assert_eq!(pieces[1], TextPiece::Return);
        assert_eq!(pieces[2], TextPiece::Units([u16::from(b'b'), 0], 1));
        let emoji = plan_text("😀").unwrap();
        match &emoji[0] {
            TextPiece::Units(buf, 2) => {
                assert_eq!(*buf, [0xD83D, 0xDE00]);
            }
            other => panic!("emoji must stay one pair, got {other:?}"),
        }
        assert!(plan_text("\u{0001}").unwrap().is_empty());

        // A failed unicode up is released with the same UTF-16 units, not as a
        // bare keycode-0 (`kVK_ANSI_A`) substitute.
        let stuck = Held { unicode: Some(([0xD83D, 0xDE00], 2)), ..Held::default() };
        assert_eq!(release_plan(stuck), vec![Release::Unicode(([0xD83D, 0xDE00], 2))]);
        let cleared = clear_release(stuck, Release::Unicode(([0xD83D, 0xDE00], 2)));
        assert!(cleared.unicode.is_none());
        assert!(cleared.key.is_none());
        let letter_a = Held { key: Some(0), ..Held::default() };
        assert_eq!(release_plan(letter_a), vec![Release::Key(0)]);

        // Owned Command is lifted. Physical Shift in the snapshot stays set.
        let owned_command = Held { command: true, ..Held::default() };
        let snapshot = Modifier::Shift.flag() | Modifier::Command.flag();
        let baseline = physical_baseline(snapshot, owned_command);
        assert_eq!(baseline, Modifier::Shift.flag());
        assert_eq!(
            release_event_flags(owned_command, baseline, Release::Modifier(Modifier::Command)),
            Modifier::Shift.flag()
        );
        assert_eq!(
            release_event_flags(owned_command, baseline, Release::Key(0x08)),
            Modifier::Shift.flag() | Modifier::Command.flag()
        );
        // Stopping after Command-down still releases only Command, and keeps Shift.
        let partial_owned = [KeyStep::Modifier {
            kind: Modifier::Command,
            down: true,
            flags: Modifier::Shift.flag() | Modifier::Command.flag(),
        }];
        assert_eq!(
            unwind_key_steps(&partial_owned),
            vec![KeyStep::Modifier { kind: Modifier::Command, down: false, flags: Modifier::Shift.flag() }]
        );
        let after = note_key_step(
            note_key_step(Held::default(), partial_owned[0]),
            unwind_key_steps(&partial_owned)[0],
        );
        assert!(!after.command);
        assert!(!after.shift);

        assert!(display_list_fits(1, DISPLAY_LIST_CAP));
        assert!(display_list_fits(DISPLAY_LIST_CAP, DISPLAY_LIST_CAP));
        assert!(!display_list_fits(DISPLAY_LIST_CAP + 1, DISPLAY_LIST_CAP));
        assert!(!display_list_fits(0, DISPLAY_LIST_CAP));
    }
}
