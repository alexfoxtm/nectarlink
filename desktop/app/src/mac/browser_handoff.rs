// SPDX-License-Identifier: GPL-3.0-or-later
//! Copied-link Dock offer: a web link copied on a paired phone shows as the
//! Mac default browser's icon with a phone badge on Waypair's own Dock tile,
//! and only a click on that tile by the signed Dock opens it.
//!
//! The core holds the offer (`nectarlink_core::CopiedLink`, RAM only, at
//! most 300 s from the copy, never extended) and hands out its link only
//! from `take_copied_link(id)`. This side keeps just the shown offer's id:
//!
//! - It shows only while the local receiver is on and this Mac's session is
//!   unlocked, awake and on console. Lock, sleep or a session switch clears
//!   the core's offer and stops advertising the receiver; unlocking never
//!   brings an offer back.
//! - A click takes only the offer id it showed, never a newer arrival, and
//!   opens it only as a validated, byte-identical HTTP(S) URL, in the app
//!   registered for the scheme, forced through `NSWorkspace` so another app
//!   claiming the domain cannot take it. Nothing is opened automatically,
//!   written to the clipboard or logged; the default browser is never
//!   changed and Waypair registers for nothing.
//! - Clearing restores the normal app icon. Dock clicks reach this code
//!   through a guarded adapter on Qt's private reflection delegate, set only
//!   while an offer shows and restored exactly afterwards; Qt stays the app
//!   delegate. This is Waypair's own Dock tile, not Apple Handoff.
//!
//! Threading: offer changes arrive on the core's event thread; every AppKit
//! call runs on the main thread (dispatched to the main queue, which Qt's
//! Cocoa run loop services).

#![allow(unsafe_code)]

use std::{
    cell::RefCell,
    ffi::c_void,
    sync::{
        Mutex, OnceLock,
        atomic::{AtomicBool, Ordering},
    },
};

use dispatch2::DispatchQueue;
use nectarlink_core::DeviceId;
use objc2::{
    DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send,
    rc::{Allocated, Retained},
    runtime::{AnyClass, AnyObject, Sel},
    sel,
};
use objc2_app_kit::{
    NSApplication, NSRunningApplication, NSWorkspace, NSWorkspaceDidWakeNotification,
    NSWorkspaceScreensDidSleepNotification, NSWorkspaceScreensDidWakeNotification,
    NSWorkspaceSessionDidBecomeActiveNotification, NSWorkspaceSessionDidResignActiveNotification,
    NSWorkspaceWillSleepNotification,
};
use objc2_foundation::{
    NSArray, NSBundle, NSNumber, NSObject, NSObjectProtocol, NSPoint, NSRect, NSSize, NSString, NSURL,
};

const MAX_URL_BYTES: usize = 8_192;
const DOCK_BUNDLE_ID: &str = "com.apple.dock";

// ---------------------------------------------------------------------------
// Pure policy (no AppKit), unit-tested below.
// ---------------------------------------------------------------------------

/// The frozen wire rule, checked again before opening: absolute lowercase
/// `http`/`https` with a host, at most 8192 UTF-8 bytes, no userinfo, no
/// control, format or whitespace characters, and an explicit port only in
/// 1..=65535.
fn is_valid_offer_url(url: &str) -> bool {
    if url.is_empty() || url.len() > MAX_URL_BYTES || url.chars().any(is_rejected_char) {
        return false;
    }
    let Some(rest) = url.strip_prefix("https://").or_else(|| url.strip_prefix("http://")) else {
        return false;
    };
    let authority = &rest[..rest.find(['/', '?', '#']).unwrap_or(rest.len())];
    if authority.is_empty() || authority.contains(['@', '\\']) {
        return false;
    }
    let (host, port) = if let Some(bracketed) = authority.strip_prefix('[') {
        let Some(close) = bracketed.find(']') else { return false };
        let port = match &bracketed[close + 1..] {
            "" => None,
            after => match after.strip_prefix(':') {
                Some(port) => Some(port),
                None => return false,
            },
        };
        (&bracketed[..close], port)
    } else {
        match authority.rsplit_once(':') {
            Some((host, port)) if !host.contains(':') => (host, Some(port)),
            Some(_) => return false,
            None => (authority, None),
        }
    };
    if host.is_empty() {
        return false;
    }
    port.is_none_or(|port| {
        !port.is_empty()
            && port.len() <= 5
            && port.bytes().all(|b| b.is_ascii_digit())
            && matches!(port.parse::<u32>(), Ok(1..=65_535))
    })
}

fn is_rejected_char(c: char) -> bool {
    let v = c as u32;
    v <= 0x20 || v == 0x7F || (0x80..=0x9F).contains(&v) || c.is_whitespace() || is_format_char(v)
}

/// Unicode general category Cf.
fn is_format_char(v: u32) -> bool {
    matches!(
        v,
        0xAD | 0x600..=0x605
            | 0x61C
            | 0x6DD
            | 0x70F
            | 0x890..=0x891
            | 0x8E2
            | 0x180E
            | 0x200B..=0x200F
            | 0x202A..=0x202E
            | 0x2060..=0x2064
            | 0x2066..=0x206F
            | 0xFEFF
            | 0xFFF9..=0xFFFB
            | 0x110BD
            | 0x110CD
            | 0x13430..=0x1343F
            | 0x1BCA0..=0x1BCA3
            | 0x1D173..=0x1D17A
            | 0xE0001
            | 0xE0020..=0xE007F
    )
}

/// The offer the tile may show: the core's current one, only while the
/// receiver is on and this Mac is not blocked.
fn shown_offer(current: Option<(DeviceId, u64)>, receiving: bool, blocked: bool) -> Option<(DeviceId, u64)> {
    current.filter(|_| receiving && !blocked)
}

// ---------------------------------------------------------------------------
// Receiver state (any thread).
// ---------------------------------------------------------------------------

/// The core's side of the offer and the app's window, supplied at [`start`].
pub struct AppHooks {
    /// Consumes offer `id` and returns its link, if it is still the held one.
    pub take: Box<dyn Fn(u64) -> Option<String> + Send + Sync>,
    /// Clears the held offer (Mac locked, asleep or switched away).
    pub clear: Box<dyn Fn() + Send + Sync>,
    /// Advertises (or stops advertising) that this Mac takes offers.
    pub set_receiver: Box<dyn Fn(bool) + Send + Sync>,
    /// Shows the main window, for a Dock click that opens no link.
    pub show_window: Box<dyn Fn() + Send + Sync>,
}

impl std::fmt::Debug for AppHooks {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AppHooks").finish_non_exhaustive()
    }
}

static HOOKS: OnceLock<AppHooks> = OnceLock::new();
/// The core's current offer as last reported: device and offer id, no link.
static CURRENT: Mutex<Option<(DeviceId, u64)>> = Mutex::new(None);
/// The local opt-in ("Copied links in the Dock").
static RECEIVING: AtomicBool = AtomicBool::new(false);
/// This Mac is locked, asleep or switched away. Starts blocked until the
/// first session snapshot says otherwise.
static BLOCKED: AtomicBool = AtomicBool::new(true);

fn current() -> Option<(DeviceId, u64)> {
    shown_offer(
        *CURRENT.lock().unwrap_or_else(|e| e.into_inner()),
        RECEIVING.load(Ordering::Acquire),
        BLOCKED.load(Ordering::Acquire),
    )
}

/// Whether the core should advertise the receiver now.
pub fn receiver_wanted() -> bool {
    RECEIVING.load(Ordering::Acquire) && !BLOCKED.load(Ordering::Acquire)
}

fn apply_receiver() {
    if let Some(hooks) = HOOKS.get() {
        (hooks.set_receiver)(receiver_wanted());
    }
}

/// The core's offer changed: `current` is its device and id, or `None`.
pub fn offer_changed(current: Option<(DeviceId, u64)>) {
    *CURRENT.lock().unwrap_or_else(|e| e.into_inner()) = current;
    if let Some((device, id)) = current {
        tracing::debug!(device = %device.short(), id, "copied link offer");
    }
    schedule_refresh();
}

/// Turns the local receiver on or off.
pub fn set_receiving(on: bool) {
    RECEIVING.store(on, Ordering::Release);
    apply_receiver();
    schedule_refresh();
}

fn block_session() {
    BLOCKED.store(true, Ordering::Release);
    *CURRENT.lock().unwrap_or_else(|e| e.into_inner()) = None;
    if let Some(hooks) = HOOKS.get() {
        (hooks.clear)();
    }
    apply_receiver();
}

fn unblock_session() {
    if BLOCKED.swap(false, Ordering::AcqRel) {
        apply_receiver();
    }
}

/// Re-renders the Dock tile on the main queue, always asynchronously so it
/// never runs inside a delegate callback it may uninstall.
fn schedule_refresh() {
    DispatchQueue::main().exec_async(|| {
        if let Some(mtm) = MainThreadMarker::new() {
            refresh(mtm);
        }
    });
}

// ---------------------------------------------------------------------------
// Main-thread UI: Dock tile, reopen hook, session observers.
// ---------------------------------------------------------------------------

struct Ui {
    observers: Retained<SessionObserver>,
    /// The scheme handler app whose icon the tile shows, while it shows one.
    shown: Option<Retained<NSURL>>,
    /// The offer id the tile was last rendered for; the only one a click takes.
    shown_id: Option<u64>,
    hook: Option<Hook>,
}

thread_local! {
    static UI: RefCell<Option<Ui>> = const { RefCell::new(None) };
}

/// Starts watching this Mac's session. Call on the main thread once the Qt
/// application exists.
pub fn start(receiving: bool, hooks: AppHooks) {
    let Some(mtm) = MainThreadMarker::new() else { return };
    RECEIVING.store(receiving, Ordering::Release);
    if HOOKS.set(hooks).is_err() || UI.with(|ui| ui.borrow().is_some()) {
        return;
    }
    let observers = SessionObserver::new(mtm);
    observers.register();
    UI.with(|ui| *ui.borrow_mut() = Some(Ui { observers, shown: None, shown_id: None, hook: None }));
    refresh_session();
    refresh(mtm);
}

/// Restores the app icon and Qt's delegate, and stops watching. Call on the
/// main thread before the Qt application is dropped.
pub fn stop() {
    let Some(mtm) = MainThreadMarker::new() else { return };
    *CURRENT.lock().unwrap_or_else(|e| e.into_inner()) = None;
    hide(mtm);
    if let Some(ui) = UI.with(|ui| ui.borrow_mut().take()) {
        ui.observers.unregister();
    }
}

fn refresh(mtm: MainThreadMarker) {
    if UI.with(|ui| ui.borrow().is_none()) {
        return;
    }
    // The link itself stays in the core until a click, so the icon is the
    // HTTPS handler's; the click opens with the link's own scheme handler.
    let offer = current();
    let browser = offer.and_then(|_| scheme_handler("https"));
    match browser {
        Some(app) => show(mtm, app),
        None => hide(mtm),
    }
    let shown_id = offer
        .map(|(_, id)| id)
        .filter(|_| UI.with(|ui| ui.borrow().as_ref().is_some_and(|ui| ui.shown.is_some())));
    UI.with(|ui| {
        if let Some(ui) = ui.borrow_mut().as_mut() {
            ui.shown_id = shown_id;
        }
    });
}

fn show(mtm: MainThreadMarker, app: Retained<NSURL>) {
    let app_id = app.absoluteString().map(|s| s.to_string());
    let same = UI.with(|ui| {
        ui.borrow()
            .as_ref()
            .and_then(|ui| ui.shown.as_ref())
            .is_some_and(|shown| shown.absoluteString().map(|s| s.to_string()) == app_id)
    });
    if same {
        return;
    }
    let Some(view) = app.path().and_then(|path| badged_tile_view(mtm, &path)) else {
        hide(mtm);
        return;
    };
    let tile = dock_tile(mtm);
    // SAFETY: `dockTile` is the app's NSDockTile; `setContentView:` takes an
    // NSView (here an NSImageView) and `display` takes no arguments.
    unsafe {
        let _: () = msg_send![&*tile, setContentView: &*view];
        let _: () = msg_send![&*tile, display];
    }
    install_hook(mtm);
    UI.with(|ui| {
        if let Some(ui) = ui.borrow_mut().as_mut() {
            ui.shown = Some(app);
        }
    });
}

fn hide(mtm: MainThreadMarker) {
    let was_shown = UI.with(|ui| ui.borrow_mut().as_mut().and_then(|ui| ui.shown.take()).is_some());
    if was_shown {
        let tile = dock_tile(mtm);
        // SAFETY: a nil content view returns the tile to the application
        // icon (Waypair's white W).
        unsafe {
            let _: () = msg_send![&*tile, setContentView: Option::<&AnyObject>::None];
            let _: () = msg_send![&*tile, display];
        }
    }
    uninstall_hook();
}

fn dock_tile(mtm: MainThreadMarker) -> Retained<AnyObject> {
    let app = NSApplication::sharedApplication(mtm);
    // SAFETY: `-[NSApplication dockTile]` returns the app's non-nil NSDockTile.
    unsafe { msg_send![&*app, dockTile] }
}

/// An `NSURL` whose absolute string is byte-identical to a valid offer URL.
fn exact_open_url(url: &str) -> Option<Retained<NSURL>> {
    if !is_valid_offer_url(url) {
        return None;
    }
    let parsed = NSURL::URLWithString(&NSString::from_str(url))?;
    let scheme = parsed.scheme()?.to_string();
    let host = parsed.host()?;
    let exact = parsed.absoluteString().is_some_and(|s| s.to_string() == url);
    (exact && (scheme == "http" || scheme == "https") && host.length() > 0).then_some(parsed)
}

/// The app registered for the URL scheme (not for this URL, so an app
/// claiming the domain is never chosen), unless it is Waypair itself.
fn scheme_handler(scheme: &str) -> Option<Retained<NSURL>> {
    if scheme != "http" && scheme != "https" {
        return None;
    }
    let probe = NSURL::URLWithString(&NSString::from_str(&format!("{scheme}://localhost/")))?;
    let app = NSWorkspace::sharedWorkspace().URLForApplicationToOpenURL(&probe)?;
    let handler_id = NSBundle::bundleWithURL(&app)?.bundleIdentifier()?.to_string();
    let own_id = NSBundle::mainBundle().bundleIdentifier().map(|id| id.to_string());
    (own_id.is_none_or(|own| !own.eq_ignore_ascii_case(&handler_id))).then_some(app)
}

/// An image view of the handler app's own icon (never bundled) with a phone
/// badge, sized for the Dock tile.
fn badged_tile_view(mtm: MainThreadMarker, app_path: &NSString) -> Option<Retained<AnyObject>> {
    let icon = NSWorkspace::sharedWorkspace();
    // SAFETY: `iconForFile:` takes an NSString path and returns a non-nil NSImage.
    let browser_icon: Retained<AnyObject> = unsafe { msg_send![&*icon, iconForFile: app_path] };
    let image = badged_icon(&browser_icon)?;
    let view_class = AnyClass::get(c"NSImageView")?;
    let tile = dock_tile(mtm);
    // SAFETY: `+[NSImageView imageViewWithImage:]` takes an NSImage;
    // `-[NSDockTile size]` returns an NSSize; `setFrame:` takes an NSRect and
    // `setImageScaling:` an NSImageScaling (3 = proportionally up or down).
    unsafe {
        let view: Option<Retained<AnyObject>> = msg_send![view_class, imageViewWithImage: &*image];
        let view = view?;
        let size: NSSize = msg_send![&*tile, size];
        let _: () = msg_send![&*view, setFrame: NSRect::new(NSPoint::ZERO, size)];
        let _: () = msg_send![&*view, setImageScaling: 3usize];
        Some(view)
    }
}

fn badged_icon(browser_icon: &AnyObject) -> Option<Retained<AnyObject>> {
    const SIDE: f64 = 256.0;
    const SOURCE_OVER: usize = 2;
    const WEIGHT_SEMIBOLD: f64 = 0.3;
    let image_class = AnyClass::get(c"NSImage")?;
    let color_class = AnyClass::get(c"NSColor")?;
    let path_class = AnyClass::get(c"NSBezierPath")?;
    let full = NSRect::new(NSPoint::ZERO, NSSize::new(SIDE, SIDE));
    let badge = NSRect::new(NSPoint::new(SIDE * 0.6, 0.0), NSSize::new(SIDE * 0.4, SIDE * 0.4));
    let ring = NSRect::new(
        NSPoint::new(badge.origin.x + 3.0, badge.origin.y + 3.0),
        NSSize::new(badge.size.width - 6.0, badge.size.height - 6.0),
    );
    // SAFETY: every message below is public AppKit API with the argument and
    // return types written here; drawing happens between `lockFocus` and
    // `unlockFocus` on the main thread.
    unsafe {
        let allocated: Allocated<AnyObject> = msg_send![image_class, alloc];
        let image: Retained<AnyObject> = msg_send![allocated, initWithSize: NSSize::new(SIDE, SIDE)];
        let _: () = msg_send![&*image, lockFocus];
        let _: () = msg_send![
            browser_icon,
            drawInRect: full,
            fromRect: NSRect::ZERO,
            operation: SOURCE_OVER,
            fraction: 1.0f64
        ];
        let accent: Retained<AnyObject> = msg_send![color_class, controlAccentColor];
        let white: Retained<AnyObject> = msg_send![color_class, whiteColor];
        let _: () = msg_send![&*accent, setFill];
        let disc: Retained<AnyObject> = msg_send![path_class, bezierPathWithOvalInRect: badge];
        let _: () = msg_send![&*disc, fill];
        let _: () = msg_send![&*white, setStroke];
        let outline: Retained<AnyObject> = msg_send![path_class, bezierPathWithOvalInRect: ring];
        let _: () = msg_send![&*outline, setLineWidth: 6.0f64];
        let _: () = msg_send![&*outline, stroke];
        if let Some(glyph) = phone_glyph(image_class, &white, SIDE * 0.2, WEIGHT_SEMIBOLD) {
            let size: NSSize = msg_send![&*glyph, size];
            let at = NSRect::new(
                NSPoint::new(
                    badge.origin.x + (badge.size.width - size.width) / 2.0,
                    badge.origin.y + (badge.size.height - size.height) / 2.0,
                ),
                size,
            );
            let _: () = msg_send![&*glyph, drawInRect: at, fromRect: NSRect::ZERO, operation: SOURCE_OVER, fraction: 1.0f64];
        }
        let _: () = msg_send![&*image, unlockFocus];
        Some(image)
    }
}

/// The system phone symbol in white, or `None` where SF Symbols are missing.
///
/// # Safety
/// Main thread, inside the image's `lockFocus`.
unsafe fn phone_glyph(
    image_class: &AnyClass,
    white: &AnyObject,
    point_size: f64,
    weight: f64,
) -> Option<Retained<AnyObject>> {
    let config_class = AnyClass::get(c"NSImageSymbolConfiguration")?;
    // SAFETY: public AppKit API (macOS 11+, palette colors macOS 12+, checked).
    unsafe {
        let mut symbol: Option<Retained<AnyObject>> = None;
        for name in ["smartphone", "iphone"] {
            symbol = msg_send![
                image_class,
                imageWithSystemSymbolName: &*NSString::from_str(name),
                accessibilityDescription: Option::<&NSString>::None
            ];
            if symbol.is_some() {
                break;
            }
        }
        let symbol = symbol?;
        let mut config: Retained<AnyObject> =
            msg_send![config_class, configurationWithPointSize: point_size, weight: weight];
        let palette = sel!(configurationWithPaletteColors:);
        let has_palette: bool = msg_send![config_class, respondsToSelector: palette];
        if has_palette {
            let colors = NSArray::from_slice(&[white]);
            let palette: Retained<AnyObject> =
                msg_send![config_class, configurationWithPaletteColors: &*colors];
            config = msg_send![&*config, configurationByApplyingConfiguration: &*palette];
        }
        msg_send![&*symbol, imageWithSymbolConfiguration: &*config]
    }
}

// --- Reopen hook -----------------------------------------------------------
//
// A guarded adapter on a private Qt detail, not an Apple API and not Apple
// Handoff. Qt's `QCocoaApplicationDelegate` hands
// `applicationShouldHandleReopen:hasVisibleWindows:` to its "reflection
// delegate" when that object responds to it. While the tile shows, the
// reflector below is that reflection delegate; Qt stays `NSApp.delegate`.
// The previous reflection delegate is read from Qt's ivar (Qt has a setter
// but no getter), forwarded everything else, and restored exactly. If the
// delegate is not Qt's class, the setter or ivar is missing, or the ivar is
// not an object, nothing is installed and a click gets the default reopen.

#[derive(Debug)]
struct ReflectorIvars {
    /// Qt's reflection delegate before ours (normally nil), forwarded every
    /// message the reflector does not handle and restored on removal.
    prior: Option<Retained<AnyObject>>,
}

define_class!(
    // SAFETY: `NSObject` has no subclassing requirements and the reflector
    // does not implement `Drop`.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "WaypairLinkDockReopenReflector"]
    #[ivars = ReflectorIvars]
    struct DockReflector;

    // SAFETY: `NSObjectProtocol` has no safety requirements.
    unsafe impl NSObjectProtocol for DockReflector {}

    impl DockReflector {
        #[unsafe(method(applicationShouldHandleReopen:hasVisibleWindows:))]
        fn should_handle_reopen(&self, app: &AnyObject, has_visible_windows: bool) -> bool {
            let consumed =
                std::panic::catch_unwind(std::panic::AssertUnwindSafe(open_offer_from_dock)).unwrap_or(false);
            if consumed {
                return false;
            }
            if let Some(prior) = self.ivars().prior.as_deref() {
                let selector = sel!(applicationShouldHandleReopen:hasVisibleWindows:);
                // SAFETY: the prior delegate is an NSObject; the reopen message
                // is sent with its documented signature only when it responds.
                unsafe {
                    let responds: bool = msg_send![prior, respondsToSelector: selector];
                    if responds {
                        return msg_send![prior, applicationShouldHandleReopen: app, hasVisibleWindows: has_visible_windows];
                    }
                }
            }
            // Qt skips its own reopen handling once a reflection delegate
            // answers, so bring the app forward as the tray's Open does.
            if let Some(hooks) = HOOKS.get() {
                (hooks.show_window)();
            }
            true
        }

        #[unsafe(method(respondsToSelector:))]
        fn responds_to_selector(&self, selector: Sel) -> bool {
            // SAFETY: `respondsToSelector:` takes a selector and returns BOOL.
            unsafe {
                let own: bool = msg_send![super(self), respondsToSelector: selector];
                own || self.ivars().prior.as_deref().is_some_and(|prior| msg_send![prior, respondsToSelector: selector])
            }
        }

        #[unsafe(method(forwardingTargetForSelector:))]
        fn forwarding_target(&self, selector: Sel) -> *mut AnyObject {
            let Some(prior) = self.ivars().prior.as_deref() else { return std::ptr::null_mut() };
            // SAFETY: as above; the returned object is unretained (+0), as
            // `forwardingTargetForSelector:` requires, and the reflector keeps it alive.
            let responds: bool = unsafe { msg_send![prior, respondsToSelector: selector] };
            if responds { (prior as *const AnyObject).cast_mut() } else { std::ptr::null_mut() }
        }
    }
);

impl DockReflector {
    fn new(mtm: MainThreadMarker, prior: Option<Retained<AnyObject>>) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(ReflectorIvars { prior });
        // SAFETY: `NSObject`'s `init` signature.
        unsafe { msg_send![super(this), init] }
    }
}

/// The installed reflector and the Qt delegate it is installed on.
struct Hook {
    qt: Retained<AnyObject>,
    reflector: Retained<DockReflector>,
}

fn app_delegate(mtm: MainThreadMarker) -> Option<Retained<AnyObject>> {
    let app = NSApplication::sharedApplication(mtm);
    // SAFETY: `-[NSApplication delegate]` returns the (possibly nil) delegate.
    unsafe { msg_send![&*app, delegate] }
}

/// Qt's current reflection delegate, or `None` when `qt` is not a
/// `QCocoaApplicationDelegate` with the expected setter and object ivar.
fn qt_reflection_delegate(qt: &AnyObject) -> Option<Option<Retained<AnyObject>>> {
    let class = qt.class();
    if class.name() != c"QCocoaApplicationDelegate" {
        return None;
    }
    // SAFETY: `respondsToSelector:` takes a selector and returns BOOL.
    let settable: bool = unsafe { msg_send![qt, respondsToSelector: sel!(setReflectionDelegate:)] };
    let ivar = class.instance_variable(c"reflectionDelegate")?;
    if !settable || ivar.type_encoding().to_bytes().first() != Some(&b'@') {
        return None;
    }
    // SAFETY: the ivar exists on `qt`'s class and holds an object pointer
    // (checked above); it is read on the main thread, where Qt writes it.
    unsafe {
        let raw = *ivar.load::<*mut AnyObject>(qt);
        Some(Retained::retain(raw))
    }
}

fn set_reflection_delegate(qt: &AnyObject, delegate: Option<&AnyObject>) {
    // SAFETY: `-[QCocoaApplicationDelegate setReflectionDelegate:]` takes a
    // (nullable) NSObject and retains it; checked present before any call.
    unsafe {
        let _: () = msg_send![qt, setReflectionDelegate: delegate];
    }
}

fn install_hook(mtm: MainThreadMarker) {
    if UI.with(|ui| ui.borrow().as_ref().is_some_and(|ui| ui.hook.is_some())) {
        return;
    }
    let Some(qt) = app_delegate(mtm) else { return };
    let Some(prior) = qt_reflection_delegate(&qt) else {
        tracing::debug!("Qt reopen adapter unavailable; Dock clicks get the default reopen");
        return;
    };
    let reflector = DockReflector::new(mtm, prior);
    let as_object: &AnyObject = &reflector;
    set_reflection_delegate(&qt, Some(as_object));
    UI.with(|ui| {
        if let Some(ui) = ui.borrow_mut().as_mut() {
            ui.hook = Some(Hook { qt, reflector });
        }
    });
}

fn uninstall_hook() {
    let Some(hook) = UI.with(|ui| ui.borrow_mut().as_mut().and_then(|ui| ui.hook.take())) else { return };
    let ours = qt_reflection_delegate(&hook.qt).flatten().is_some_and(|current| {
        std::ptr::eq(Retained::as_ptr(&current), Retained::as_ptr(&hook.reflector).cast::<AnyObject>())
    });
    if ours {
        set_reflection_delegate(&hook.qt, hook.reflector.ivars().prior.as_deref());
    } else {
        // Someone replaced it after us; leave theirs and keep ours alive in
        // case it is still referenced.
        std::mem::forget(hook.reflector);
    }
}

/// Opens the offer for a Dock click. `true` when consumed; any other reopen
/// (plain launch, `open -a`, scripts, helpers) returns `false` and just
/// brings the app forward.
fn open_offer_from_dock() -> bool {
    if MainThreadMarker::new().is_none() {
        return false;
    }
    let Some(id) =
        UI.with(|ui| ui.borrow().as_ref().filter(|ui| ui.shown.is_some()).and_then(|ui| ui.shown_id))
    else {
        return false;
    };
    if current().is_none() {
        schedule_refresh();
        return false;
    }
    let Some(hooks) = HOOKS.get() else { return false };
    if !reopen_is_from_signed_dock() {
        return false;
    }
    // Only the id this tile showed; a newer offer stays with the core.
    let Some(url) = (hooks.take)(id) else {
        schedule_refresh();
        return false;
    };
    {
        let mut shown = CURRENT.lock().unwrap_or_else(|e| e.into_inner());
        if shown.is_some_and(|(_, held)| held == id) {
            *shown = None;
        }
    }
    schedule_refresh();
    let target = exact_open_url(&url);
    let app = target.as_ref().and_then(|target| target.scheme()).and_then(|s| scheme_handler(&s.to_string()));
    let opened = match (target, app) {
        (Some(target), Some(app)) => open_with(&target, &app),
        _ => false,
    };
    tracing::debug!(id, opened, "copied link clicked in the Dock");
    true
}

/// Opens `url` with `app` only, activating it, without prompting or adding
/// to recent items. Never `-[NSWorkspace openURL:]`.
fn open_with(url: &NSURL, app: &NSURL) -> bool {
    let Some(config_class) = AnyClass::get(c"NSWorkspaceOpenConfiguration") else { return false };
    let workspace = NSWorkspace::sharedWorkspace();
    let urls = NSArray::from_slice(&[url]);
    // SAFETY: public NSWorkspace API (macOS 10.15+); a nil completion handler
    // is allowed.
    unsafe {
        let config: Retained<AnyObject> = msg_send![config_class, configuration];
        let _: () = msg_send![&*config, setActivates: true];
        let _: () = msg_send![&*config, setPromptsUserIfNeeded: false];
        let _: () = msg_send![&*config, setAddsToRecentItems: false];
        let _: () = msg_send![
            &*workspace,
            openURLs: &*urls,
            withApplicationAtURL: app,
            configuration: &*config,
            completionHandler: Option::<&block2::DynBlock<dyn Fn(*mut AnyObject, *mut AnyObject)>>::None
        ];
    }
    true
}

#[link(name = "Security", kind = "framework")]
unsafe extern "C" {
    static kSecGuestAttributePid: *const c_void;
    fn SecCodeCopyGuestWithAttributes(
        host: *const c_void,
        attributes: *const c_void,
        flags: u32,
        guest: *mut *const c_void,
    ) -> i32;
    fn SecRequirementCreateWithString(
        text: *const c_void,
        flags: u32,
        requirement: *mut *const c_void,
    ) -> i32;
    fn SecCodeCheckValidity(code: *const c_void, flags: u32, requirement: *const c_void) -> i32;
}

#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    fn CFRelease(cf: *const c_void);
}

#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" {
    fn CGSessionCopyCurrentDictionary() -> *const c_void;
    fn CGMainDisplayID() -> u32;
    fn CGDisplayIsAsleep(display: u32) -> u32;
}

/// Releases a +1 Core Foundation object on drop.
struct CfOwned(*const c_void);

impl Drop for CfOwned {
    fn drop(&mut self) {
        if !self.0.is_null() {
            // SAFETY: the pointer came from a Create/Copy call that returned +1.
            unsafe { CFRelease(self.0) };
        }
    }
}

/// True only for a reopen (`aevt`/`rapp`) Apple event whose sender process
/// is `com.apple.dock` and satisfies `identifier "com.apple.dock" and anchor
/// apple`. Missing or unverifiable sender information fails closed.
fn reopen_is_from_signed_dock() -> bool {
    const CLASS_AEVT: u32 = 0x6165_7674;
    const ID_RAPP: u32 = 0x7261_7070;
    const KEY_SENDER_PID: u32 = 0x7370_6964;
    let Some(manager_class) = AnyClass::get(c"NSAppleEventManager") else { return false };
    // SAFETY: public NSAppleEventManager / NSAppleEventDescriptor API; the
    // current event is only valid during the delegate callback this runs in.
    let pid: i32 = unsafe {
        let manager: Retained<AnyObject> = msg_send![manager_class, sharedAppleEventManager];
        let event: Option<Retained<AnyObject>> = msg_send![&*manager, currentAppleEvent];
        let Some(event) = event else { return false };
        let class: u32 = msg_send![&*event, eventClass];
        let id: u32 = msg_send![&*event, eventID];
        if class != CLASS_AEVT || id != ID_RAPP {
            return false;
        }
        let sender: Option<Retained<AnyObject>> =
            msg_send![&*event, attributeDescriptorForKeyword: KEY_SENDER_PID];
        let Some(sender) = sender else { return false };
        msg_send![&*sender, int32Value]
    };
    if pid <= 0 {
        return false;
    }
    let is_dock = NSRunningApplication::runningApplicationWithProcessIdentifier(pid)
        .and_then(|app| app.bundleIdentifier())
        .is_some_and(|id| id.to_string() == DOCK_BUNDLE_ID);
    is_dock && is_signed_dock(pid)
}

fn is_signed_dock(pid: i32) -> bool {
    let number = NSNumber::new_i32(pid);
    // SAFETY: NSDictionary and NSString are toll-free bridged to
    // CFDictionary and CFString; `kSecGuestAttributePid` is a CFString key.
    // Created objects are released by `CfOwned`.
    unsafe {
        let key = kSecGuestAttributePid as *const AnyObject;
        let Some(key) = key.as_ref() else { return false };
        let Some(dictionary_class) = AnyClass::get(c"NSDictionary") else { return false };
        let attributes: Retained<AnyObject> =
            msg_send![dictionary_class, dictionaryWithObject: &*number, forKey: key];
        let mut code: *const c_void = std::ptr::null();
        let attributes_ref = (&*attributes as *const AnyObject).cast::<c_void>();
        if SecCodeCopyGuestWithAttributes(std::ptr::null(), attributes_ref, 0, &mut code) != 0
            || code.is_null()
        {
            return false;
        }
        let code = CfOwned(code);
        let text = NSString::from_str(&format!("identifier \"{DOCK_BUNDLE_ID}\" and anchor apple"));
        let mut requirement: *const c_void = std::ptr::null();
        let text_ref = (&*text as *const NSString).cast::<c_void>();
        if SecRequirementCreateWithString(text_ref, 0, &mut requirement) != 0 || requirement.is_null() {
            return false;
        }
        let requirement = CfOwned(requirement);
        SecCodeCheckValidity(code.0, 0, requirement.0) == 0
    }
}

// --- Session observers ------------------------------------------------------

/// Re-reads whether this Mac's session is on console, unlocked and awake;
/// a missing snapshot counts as blocked. Unblocking never restores an offer.
fn refresh_session() {
    // SAFETY: `CGSessionCopyCurrentDictionary` returns a +1 CFDictionary (or
    // NULL), toll-free bridged to NSDictionary and released by `CfOwned`.
    let blocked = unsafe {
        let raw = CGSessionCopyCurrentDictionary();
        let owned = CfOwned(raw);
        match (owned.0 as *const AnyObject).as_ref() {
            None => true,
            Some(session) => {
                let flag = |key: &str| -> bool {
                    let value: Option<Retained<AnyObject>> =
                        msg_send![session, objectForKey: &*NSString::from_str(key)];
                    value.is_some_and(|value| msg_send![&*value, boolValue])
                };
                !flag("kCGSSessionOnConsoleKey")
                    || flag("CGSSessionScreenIsLocked")
                    || CGDisplayIsAsleep(CGMainDisplayID()) != 0
            }
        }
    };
    if blocked {
        block_session();
    } else {
        unblock_session();
    }
    schedule_refresh();
}

#[derive(Debug)]
struct ObserverIvars {
    _unused: u8,
}

define_class!(
    // SAFETY: `NSObject` has no subclassing requirements and the observer
    // does not implement `Drop`.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "WaypairLinkCopiedLinkSessionObserver"]
    #[ivars = ObserverIvars]
    struct SessionObserver;

    // SAFETY: `NSObjectProtocol` has no safety requirements.
    unsafe impl NSObjectProtocol for SessionObserver {}

    impl SessionObserver {
        #[unsafe(method(waypairSessionBlocked:))]
        fn session_blocked(&self, _notification: &AnyObject) {
            block_session();
            schedule_refresh();
        }

        #[unsafe(method(waypairSessionMaybeResumed:))]
        fn session_maybe_resumed(&self, _notification: &AnyObject) {
            refresh_session();
        }
    }
);

const SCREEN_LOCKED: &str = "com.apple.screenIsLocked";
const SCREEN_UNLOCKED: &str = "com.apple.screenIsUnlocked";

impl SessionObserver {
    fn new(mtm: MainThreadMarker) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(ObserverIvars { _unused: 0 });
        // SAFETY: `NSObject`'s `init` signature.
        unsafe { msg_send![super(this), init] }
    }

    fn centers() -> (Retained<AnyObject>, Option<Retained<AnyObject>>) {
        let workspace = NSWorkspace::sharedWorkspace();
        // SAFETY: both centers are public singletons.
        unsafe {
            let local: Retained<AnyObject> = msg_send![&*workspace, notificationCenter];
            let distributed = AnyClass::get(c"NSDistributedNotificationCenter")
                .map(|class| -> Retained<AnyObject> { msg_send![class, defaultCenter] });
            (local, distributed)
        }
    }

    /// Registers before the startup snapshot, so no transition between them
    /// is lost.
    fn register(&self) {
        let (local, distributed) = Self::centers();
        let blocked = sel!(waypairSessionBlocked:);
        let resumed = sel!(waypairSessionMaybeResumed:);
        // SAFETY: the NSWorkspace notification names are AppKit's constants;
        // `addObserver:selector:name:object:` takes the observer, a selector
        // it implements, a name and a nil object. `stop` removes the
        // observer before it is released.
        unsafe {
            for (name, selector) in [
                (NSWorkspaceSessionDidResignActiveNotification, blocked),
                (NSWorkspaceWillSleepNotification, blocked),
                (NSWorkspaceScreensDidSleepNotification, blocked),
                (NSWorkspaceSessionDidBecomeActiveNotification, resumed),
                (NSWorkspaceDidWakeNotification, resumed),
                (NSWorkspaceScreensDidWakeNotification, resumed),
            ] {
                let _: () = msg_send![
                    &*local,
                    addObserver: self,
                    selector: selector,
                    name: name,
                    object: Option::<&AnyObject>::None
                ];
            }
            if let Some(distributed) = distributed {
                for (name, selector) in [(SCREEN_LOCKED, blocked), (SCREEN_UNLOCKED, resumed)] {
                    let _: () = msg_send![
                        &*distributed,
                        addObserver: self,
                        selector: selector,
                        name: &*NSString::from_str(name),
                        object: Option::<&AnyObject>::None
                    ];
                }
            }
        }
    }

    fn unregister(&self) {
        let (local, distributed) = Self::centers();
        // SAFETY: `removeObserver:` takes any previously registered observer.
        unsafe {
            let _: () = msg_send![&*local, removeObserver: self];
            if let Some(distributed) = distributed {
                let _: () = msg_send![&*distributed, removeObserver: self];
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_tile_shows_the_core_offer_only_while_receiving_and_unlocked() {
        let offer = Some((DeviceId([7; 32]), 3));
        assert_eq!(shown_offer(offer, true, false), offer);
        assert_eq!(shown_offer(offer, false, false), None);
        assert_eq!(shown_offer(offer, true, true), None);
        assert_eq!(shown_offer(None, true, false), None);
        for url in [
            "https://example.com/a?b=c#d",
            "http://example.com:8080",
            "http://[::1]:8080/x",
            "https://xn--bcher-kva.example/",
        ] {
            assert!(is_valid_offer_url(url), "{url:?}");
        }
    }

    #[test]
    fn links_that_break_the_wire_rule_are_never_opened() {
        for url in [
            "",
            "HTTPS://example.com/",
            "ftp://example.com/",
            "javascript:alert(1)",
            "https://",
            "https://user@example.com/",
            "https://example.com\\@evil.example/",
            "https://example.com:0/",
            "https://example.com:65536/",
            "https://example.com:/",
            "https://[::1/",
            "https://exa mple.com/",
            "https://example.com/\u{200B}",
            "https://example.com/\u{85}",
        ] {
            assert!(!is_valid_offer_url(url), "{url:?}");
        }
        assert!(!is_valid_offer_url(&format!("https://example.com/{}", "a".repeat(MAX_URL_BYTES))));
    }
}
