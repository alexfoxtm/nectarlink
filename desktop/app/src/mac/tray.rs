// SPDX-License-Identifier: GPL-3.0-or-later
//! macOS status bar / tray integration.
//!
//! This slice is deliberately narrow: one real `NSStatusItem` whose menu
//! contains only the translated Open and Quit items, both wired to the
//! existing [`TrayEvent`] handler. There is still no global hotkey, no
//! window tracking and no hide-on-close on macOS (callers keep
//! `close_to_tray` false until a running app proves this menu), and
//! window focusing still reports failure so Continuity Camera never
//! claims it pasted.
//!
//! Threading and ownership: AppKit menu actions are delivered by the
//! main run loop, so creating the item requires the GUI main thread and
//! the handler is invoked there. The menu items' target is weak (the
//! AppKit convention), so [`Tray`] owns the target object — and, through
//! the retained menu, the items — for exactly its own lifetime. Dropping
//! the tray removes the status item from the system status bar. `Tray`
//! holds main-thread-only AppKit objects and is therefore not `Send`,
//! mirroring the Windows tray's "create and drop on the GUI thread"
//! contract.

use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2::{MainThreadMarker, sel};
use objc2_app_kit::{NSMenu, NSMenuItem, NSStatusBar, NSStatusItem, NSVariableStatusItemLength};
use objc2_foundation::NSString;

#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrayEvent {
    Open,
    CommandPalette,
    Shelf,
    TakePhoto,
    ScanDocument,
    FindPhone,
    OpenLinkOnPhone,
    Quit,
    Resumed,
    AppearanceChanged,
}

#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct MenuLabels {
    pub open: String,
    pub command_palette: String,
    pub shelf: String,
    pub take_photo: String,
    pub scan_document: String,
    pub find_phone: String,
    pub open_link: String,
    pub quit: String,
}

/// `NSMenuItem.tag` values for the two items in this slice. Pure mapping
/// (see [`event_for_tag`]) so the dispatch is testable without AppKit.
const TAG_OPEN: isize = 1;
const TAG_QUIT: isize = 2;

/// Which [`TrayEvent`] a clicked menu item sends, by its tag. Unknown
/// tags (any other item, should one ever appear) send nothing.
fn event_for_tag(tag: isize) -> Option<TrayEvent> {
    match tag {
        TAG_OPEN => Some(TrayEvent::Open),
        TAG_QUIT => Some(TrayEvent::Quit),
        _ => None,
    }
}

type Handler = Box<dyn Fn(TrayEvent)>;

/// The menu items' target. AppKit holds item targets weakly, so `Tray`
/// retains this object; while it lives, its handler is the app's
/// existing tray-event handler, called on the main thread because
/// status-item menu actions run in the main run loop.
#[allow(unsafe_code)]
mod target {
    use objc2::rc::Retained;
    use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send};
    use objc2_app_kit::NSMenuItem;
    use objc2_foundation::{NSObject, NSObjectProtocol};

    use super::{Handler, event_for_tag};

    pub struct TargetIvars {
        pub handler: Handler,
    }

    impl std::fmt::Debug for TargetIvars {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.debug_struct("TargetIvars").finish_non_exhaustive()
        }
    }

    define_class!(
        // SAFETY:
        // - The superclass `NSObject` has no subclassing requirements.
        // - `MenuTarget` does not implement `Drop`.
        #[name = "WaypairLinkTrayMenuTarget"]
        #[unsafe(super = NSObject)]
        #[thread_kind = MainThreadOnly]
        #[ivars = TargetIvars]
        pub struct MenuTarget;

        // SAFETY: `NSObjectProtocol` has no safety requirements.
        unsafe impl NSObjectProtocol for MenuTarget {}

        impl MenuTarget {
            /// Both menu items share this one action; the clicked item's
            /// tag selects the event. Runs on the main thread (AppKit
            /// delivers menu actions there).
            #[unsafe(method(waypairTrayItemClicked:))]
            fn item_clicked(&self, sender: &NSMenuItem) {
                if let Some(event) = event_for_tag(sender.tag()) {
                    (self.ivars().handler)(event);
                }
            }
        }
    );

    impl MenuTarget {
        pub fn new(mtm: MainThreadMarker, handler: Handler) -> Retained<Self> {
            let this = Self::alloc(mtm).set_ivars(TargetIvars { handler });
            // SAFETY: The signature of `NSObject`'s `init` is correct.
            unsafe { msg_send![super(this), init] }
        }
    }
}

use target::MenuTarget;

/// One menu item pointing at `target` with the shared click action.
#[allow(unsafe_code)]
fn menu_item(
    mtm: MainThreadMarker,
    title: &str,
    tag: isize,
    target: &Retained<MenuTarget>,
) -> Retained<NSMenuItem> {
    let item = NSMenuItem::new(mtm);
    item.setTitle(&NSString::from_str(title));
    item.setTag(tag);
    // The item holds its target weakly and its action as a selector; the
    // target must (and, via `Tray`, does) outlive the item.
    let target_object: Retained<AnyObject> = target.clone().into();
    // SAFETY: `target_object` is the `MenuTarget` that implements
    // `waypairTrayItemClicked:`, and the selector names that method.
    unsafe {
        item.setTarget(Some(&target_object));
        item.setAction(Some(sel!(waypairTrayItemClicked:)));
    }
    item
}

/// The owned status item. Removed from the status bar on drop; create
/// and drop on the GUI main thread.
pub struct Tray {
    item: Retained<NSStatusItem>,
    // Retained so the menu (and its items) cannot be deallocated while
    // the status item displays them; the item also retains its menu.
    _menu: Retained<NSMenu>,
    // Retained because menu items hold their target weakly.
    _target: Retained<MenuTarget>,
}

impl std::fmt::Debug for Tray {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Tray").finish_non_exhaustive()
    }
}

impl Tray {
    pub fn create(
        tooltip: &str,
        labels: MenuLabels,
        handler: impl Fn(TrayEvent) + 'static,
    ) -> Result<Tray, String> {
        // Status items may only be created on the main thread; the Qt
        // GUI thread that calls this is the main thread.
        let Some(mtm) = MainThreadMarker::new() else {
            return Err("status bar item must be created on the GUI main thread".into());
        };

        let target = MenuTarget::new(mtm, Box::new(handler));
        let menu = NSMenu::new(mtm);
        menu.addItem(&menu_item(mtm, &labels.open, TAG_OPEN, &target));
        menu.addItem(&menu_item(mtm, &labels.quit, TAG_QUIT, &target));

        let status_bar = NSStatusBar::systemStatusBar();
        let item = status_bar.statusItemWithLength(NSVariableStatusItemLength);
        item.setMenu(Some(&menu));

        // A neutral glyph-only "W" in the button title renders in the
        // menu bar's own foreground colour (black in light mode, white
        // in dark mode); the tooltip names the app for sighted and
        // assistive users alike. Deliberately not a template image over
        // the opaque black app tile, which AppKit would tint solid and
        // hide the W; no image asset is needed for this slice.
        let Some(button) = item.button(mtm) else {
            status_bar.removeStatusItem(&item);
            return Err("status bar item has no button".into());
        };
        button.setTitle(&NSString::from_str("W"));
        button.setToolTip(Some(&NSString::from_str(tooltip)));

        Ok(Tray { item, _menu: menu, _target: target })
    }

    pub fn set_tooltip(&self, text: &str) {
        // Tooltip updates arrive on the GUI thread; off the main thread
        // there is nothing safe to touch, so they are dropped rather
        // than risking an AppKit call from the wrong thread.
        let Some(mtm) = MainThreadMarker::new() else { return };
        if let Some(button) = self.item.button(mtm) {
            button.setToolTip(Some(&NSString::from_str(text)));
        }
    }

    pub fn set_find_phone_enabled(&self, _enabled: bool) {}
}

impl Drop for Tray {
    fn drop(&mut self) {
        // `Tray` is not `Send` and is created on the main thread, so in
        // practice this runs there too; guard anyway and never call
        // AppKit off-main (the retained item's own deallocation then
        // removes it instead).
        if MainThreadMarker::new().is_some() {
            NSStatusBar::systemStatusBar().removeStatusItem(&self.item);
        }
    }
}

pub fn update_hotkeys(_photo_spec: &str, _scan_spec: &str, _palette_spec: &str, _shelf_spec: &str) {}

#[allow(dead_code)]
pub fn record_foreground_window() {}

pub fn apply_no_activate_style_by_title(_title: &str) {}

/// No window is ever focused from here; `false` keeps the caller on its
/// "image is on your clipboard" path instead of a false "pasted" message.
pub fn focus_window(_hwnd: isize) -> bool {
    false
}

pub fn is_window_alive_and_visible(_hwnd: isize) -> bool {
    false
}

pub fn snapshot_target_window() -> Option<(isize, String)> {
    None
}

pub fn send_paste_fallback_if_not_foreground(_hwnd: isize) {}

#[allow(dead_code)]
pub fn parse_hotkey(_spec: &str) -> Option<(u32, u32)> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn menu_item_tags_map_to_open_and_quit() {
        assert_eq!(event_for_tag(TAG_OPEN), Some(TrayEvent::Open));
        assert_eq!(event_for_tag(TAG_QUIT), Some(TrayEvent::Quit));
    }

    #[test]
    fn unknown_menu_item_tags_send_no_event() {
        assert_eq!(event_for_tag(0), None);
        assert_eq!(event_for_tag(-1), None);
        assert_eq!(event_for_tag(isize::MAX), None);
    }
}
