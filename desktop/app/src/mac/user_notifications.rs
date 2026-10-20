// SPDX-License-Identifier: GPL-3.0-or-later
//! Public UserNotifications posting for the running app.
//!
//! The delegate is `UNUserNotificationCenter.delegate` only. This file does
//! not replace `NSApplication`'s delegate and does not run its own run loop.
//! Completion handlers are invoked before any user callback, and no lock is
//! held across a framework call or that callback.

#![allow(unsafe_code)]

use std::sync::{Once, OnceLock};

use block2::RcBlock;
use dispatch2::DispatchQueue;
use objc2::MainThreadMarker;
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, Bool, ProtocolObject};
use objc2::{AnyThread, define_class, msg_send};
use objc2_foundation::{NSArray, NSBundle, NSDictionary, NSError, NSObject, NSObjectProtocol, NSString};
use objc2_user_notifications::{
    UNAuthorizationOptions, UNMutableNotificationContent, UNNotification, UNNotificationInterruptionLevel,
    UNNotificationPresentationOptions, UNNotificationRequest, UNNotificationResponse, UNNotificationSettings,
    UNNotificationSound, UNUserNotificationCenter, UNUserNotificationCenterDelegate,
};

use super::{
    BannerText, NotificationAuthorization, Toast, ToastEvent, admit_post, app_bundle_executable, banner_text,
    begin_authorization_read, begin_authorization_request, emit, end_authorization_request,
    finish_authorization_read, notification_identifier, pending,
};

const MAX_REMOVALS: usize = 256;
const USER_INFO_DEVICE: &str = "device";
const USER_INFO_KEY: &str = "key";

struct DelegateIvars {
    _unused: u8,
}

impl std::fmt::Debug for DelegateIvars {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DelegateIvars").finish_non_exhaustive()
    }
}

define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = AnyThread]
    #[name = "DesktopUserNotificationDelegate"]
    #[ivars = DelegateIvars]
    struct CenterDelegate;

    unsafe impl NSObjectProtocol for CenterDelegate {}

    unsafe impl UNUserNotificationCenterDelegate for CenterDelegate {
        #[unsafe(method(userNotificationCenter:willPresentNotification:withCompletionHandler:))]
        fn user_notification_center_will_present_notification(
            &self,
            _center: &UNUserNotificationCenter,
            notification: &UNNotification,
            completion_handler: &block2::DynBlock<dyn Fn(UNNotificationPresentationOptions)>,
        ) {
            let options =
                std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| presentation_options(notification)))
                    .unwrap_or(
                        UNNotificationPresentationOptions::Banner | UNNotificationPresentationOptions::List,
                    );
            completion_handler.call((options,));
        }

        #[unsafe(method(userNotificationCenter:didReceiveNotificationResponse:withCompletionHandler:))]
        fn user_notification_center_did_receive_response(
            &self,
            _center: &UNUserNotificationCenter,
            response: &UNNotificationResponse,
            completion_handler: &block2::DynBlock<dyn Fn()>,
        ) {
            let event = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| response_event(response)))
                .ok()
                .flatten();
            completion_handler.call(());
            // Delegate callbacks are not guaranteed to be on Qt's thread.
            // `on_toast` runs on the main queue after this handler returns.
            if let Some(event) = event {
                on_main(move || {
                    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| emit(event)));
                });
            }
        }
    }
);

impl CenterDelegate {
    fn new() -> Retained<Self> {
        let this = Self::alloc().set_ivars(DelegateIvars { _unused: 0 });
        unsafe { msg_send![super(this), init] }
    }
}

static DELEGATE: OnceLock<Retained<CenterDelegate>> = OnceLock::new();

/// Every entry point checks this before touching `UNUserNotificationCenter`.
/// Unbundled, nothing is posted and authorization stays unknown.
fn bundled() -> bool {
    static BUNDLED: OnceLock<bool> = OnceLock::new();
    *BUNDLED.get_or_init(|| {
        let exe = std::env::current_exe().is_ok_and(|exe| app_bundle_executable(&exe));
        let bundle = NSBundle::mainBundle();
        let app = bundle.bundlePath().to_string().ends_with(".app");
        let identity = bundle.bundleIdentifier().is_some_and(|id| id.length() > 0);
        let bundled = exe && app && identity;
        if !bundled {
            tracing::info!("not running from an app bundle; system notifications are off");
        }
        bundled
    })
}

pub(super) fn install_delegate() {
    if !bundled() {
        return;
    }
    on_main(install_delegate_now);
}

pub(super) fn refresh_authorization() {
    if !bundled() {
        return;
    }
    let generation = begin_authorization_read();
    on_main(move || {
        let center = UNUserNotificationCenter::currentNotificationCenter();
        let block = RcBlock::new(move |settings: std::ptr::NonNull<UNNotificationSettings>| {
            // SAFETY: the framework passes a live settings object for this call.
            let settings = unsafe { settings.as_ref() };
            finish_authorization_read(generation, authorization_from_settings(settings));
        });
        center.getNotificationSettingsWithCompletionHandler(&block);
    });
}

pub(super) fn request_authorization() {
    if !bundled() || begin_authorization_request() {
        return;
    }
    on_main(|| {
        let center = UNUserNotificationCenter::currentNotificationCenter();
        let block = RcBlock::new(move |_granted: Bool, error: *mut NSError| {
            if !error.is_null() {
                // SAFETY: a non-null error is borrowed for this callback. Only the
                // numeric code is read; the description can contain a name.
                let code = unsafe { &*error }.code();
                tracing::warn!(code, "notification authorization was not changed");
            }
            end_authorization_request();
            refresh_authorization();
        });
        center.requestAuthorizationWithOptions_completionHandler(
            UNAuthorizationOptions::Alert | UNAuthorizationOptions::Sound,
            &block,
        );
    });
}

pub(super) fn post(toast: Toast) {
    if !bundled() {
        return;
    }
    let banner = banner_text(&toast);
    let silent = toast.silent;
    let ticket = pending().begin(&banner.device, &banner.identifier);
    let Some(ticket) = ticket else {
        tracing::warn!("notification was not posted; too many are waiting");
        return;
    };
    let generation = begin_authorization_read();
    on_main(move || {
        install_delegate_now();
        let center = UNUserNotificationCenter::currentNotificationCenter();
        let block = RcBlock::new(move |settings: std::ptr::NonNull<UNNotificationSettings>| {
            // SAFETY: the framework passes a live settings object for this call.
            let settings = unsafe { settings.as_ref() };
            let auth = authorization_from_settings(settings);
            finish_authorization_read(generation, auth);
            if !admit_post(auth) {
                pending().refuse(&banner.identifier);
                tracing::debug!(status = auth.label(), "notification was not posted");
                return;
            }
            let banner = banner.clone();
            on_main(move || deliver(&banner, silent, ticket));
        });
        center.getNotificationSettingsWithCompletionHandler(&block);
    });
}

pub(super) fn remove(device: &str, key: &str) {
    if !bundled() {
        return;
    }
    let identifier = notification_identifier(device, key);
    pending().cancel(&identifier);
    on_main(move || withdraw(&[NSString::from_str(&identifier)]));
}

pub(super) fn remove_device(device: &str) {
    if !bundled() {
        return;
    }
    let device = device.to_string();
    let cut = pending().cancel_device(&device, 2);
    on_main(move || {
        let center = UNUserNotificationCenter::currentNotificationCenter();
        let delivered_device = device.clone();
        let delivered = RcBlock::new(move |notes: std::ptr::NonNull<NSArray<UNNotification>>| {
            // SAFETY: the framework passes a live array for this call.
            let notes = unsafe { notes.as_ref() };
            let ids = delivered_identifiers(notes, &delivered_device);
            on_main(move || sweep(&ids, cut));
        });
        center.getDeliveredNotificationsWithCompletionHandler(&delivered);
        let pending_device = device;
        let queued = RcBlock::new(move |notes: std::ptr::NonNull<NSArray<UNNotificationRequest>>| {
            // SAFETY: the framework passes a live array for this call.
            let notes = unsafe { notes.as_ref() };
            let ids = pending_identifiers(notes, &pending_device);
            on_main(move || sweep(&ids, cut));
        });
        center.getPendingNotificationRequestsWithCompletionHandler(&queued);
    });
}

/// Runs on the main queue, so no add can land between the check and the withdraw.
fn sweep(identifiers: &[String], cut: u64) {
    let ids: Vec<String> = {
        let state = pending();
        identifiers.iter().filter(|id| state.sweep_may_withdraw(id, cut)).cloned().collect()
    };
    let ids: Vec<Retained<NSString>> = ids.iter().map(|id| NSString::from_str(id)).collect();
    withdraw(&ids);
    pending().end_sweep();
}

pub(super) fn unregister() {
    if !bundled() {
        return;
    }
    on_main(|| {
        let center = UNUserNotificationCenter::currentNotificationCenter();
        center.removeAllDeliveredNotifications();
        center.removeAllPendingNotificationRequests();
    });
}

fn install_delegate_now() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        let center = UNUserNotificationCenter::currentNotificationCenter();
        let delegate = CenterDelegate::new();
        let as_delegate = ProtocolObject::<dyn UNUserNotificationCenterDelegate>::from_ref(&*delegate);
        center.setDelegate(Some(as_delegate));
        let _ = DELEGATE.set(delegate);
    });
}

fn on_main(work: impl FnOnce() + Send + 'static) {
    if MainThreadMarker::new().is_some() {
        work();
        return;
    }
    DispatchQueue::main().exec_async(work);
}

fn authorization_from_settings(settings: &UNNotificationSettings) -> NotificationAuthorization {
    NotificationAuthorization::from_system(settings.authorizationStatus().0)
}

/// Runs on the main queue, so admitting and adding cannot interleave with a withdraw.
fn deliver(banner: &BannerText, silent: bool, ticket: u64) {
    if !pending().admit(&banner.identifier, ticket) {
        tracing::debug!("notification was replaced or removed before it was shown");
        return;
    }
    let content = UNMutableNotificationContent::new();
    content.setTitle(&NSString::from_str(&banner.title));
    content.setSubtitle(&NSString::from_str(&banner.subtitle));
    content.setBody(&NSString::from_str(&banner.body));
    if !silent {
        content.setSound(Some(&UNNotificationSound::defaultSound()));
    }
    if banner.quiet {
        content.setInterruptionLevel(UNNotificationInterruptionLevel::Passive);
    }
    let device_key = NSString::from_str(USER_INFO_DEVICE);
    let key_key = NSString::from_str(USER_INFO_KEY);
    let device_value = NSString::from_str(&banner.device);
    let key_value = NSString::from_str(&banner.key);
    let info = NSDictionary::<NSString, NSString>::from_slices(
        &[&*device_key, &*key_key],
        &[&*device_value, &*key_value],
    );
    // SAFETY: both entries are strings, which is the userInfo type this poster writes.
    unsafe { content.setUserInfo(&*(Retained::as_ptr(&info).cast::<NSDictionary>())) };
    let request = UNNotificationRequest::requestWithIdentifier_content_trigger(
        &NSString::from_str(&banner.identifier),
        &content,
        None,
    );
    let center = UNUserNotificationCenter::currentNotificationCenter();
    let identifier = banner.identifier.clone();
    let done = RcBlock::new(move |error: *mut NSError| {
        let shown = error.is_null();
        if !shown {
            // SAFETY: a non-null error is borrowed for this callback. Only the code is read.
            let code = unsafe { &*error }.code();
            tracing::warn!(code, "notification wasn't shown");
        }
        let identifier = identifier.clone();
        on_main(move || {
            let withdraw_now = pending().added(&identifier, ticket, shown);
            if withdraw_now {
                withdraw(&[NSString::from_str(&identifier)]);
            }
        });
    });
    center.addNotificationRequest_withCompletionHandler(&request, Some(&done));
}

fn presentation_options(notification: &UNNotification) -> UNNotificationPresentationOptions {
    let content = notification.request().content();
    if content.interruptionLevel() == UNNotificationInterruptionLevel::Passive {
        return UNNotificationPresentationOptions::List;
    }
    let mut options = UNNotificationPresentationOptions::Banner | UNNotificationPresentationOptions::List;
    if content.sound().is_some() {
        options |= UNNotificationPresentationOptions::Sound;
    }
    options
}

fn response_event(response: &UNNotificationResponse) -> Option<ToastEvent> {
    let action = response.actionIdentifier();
    let request = response.notification().request();
    let device = user_info_field(&request.content().userInfo(), USER_INFO_DEVICE)?;
    let key = user_info_field(&request.content().userInfo(), USER_INFO_KEY)?;
    if action.isEqualToString(default_action_identifier()) {
        return Some(ToastEvent::Opened { device, key });
    }
    if action.isEqualToString(dismiss_action_identifier()) {
        return Some(ToastEvent::Dismissed { device, key });
    }
    None
}

fn default_action_identifier() -> &'static NSString {
    // SAFETY: the framework exports this `NSString` for the life of the process.
    unsafe { objc2_user_notifications::UNNotificationDefaultActionIdentifier }
}

fn dismiss_action_identifier() -> &'static NSString {
    // SAFETY: the framework exports this `NSString` for the life of the process.
    unsafe { objc2_user_notifications::UNNotificationDismissActionIdentifier }
}

fn user_info_field(info: &NSDictionary, field: &str) -> Option<String> {
    let key = NSString::from_str(field);
    // SAFETY: `key` is an Objective-C object, which is what `objectForKey:` accepts.
    let key_obj = unsafe { &*Retained::as_ptr(&key).cast::<AnyObject>() };
    let value = info.objectForKey(key_obj)?;
    let text = value.downcast::<NSString>().ok()?.to_string();
    (!text.is_empty()).then_some(text)
}

fn delivered_identifiers(notes: &NSArray<UNNotification>, device: &str) -> Vec<String> {
    matching_identifiers(notes.count(), |index| {
        matching_identifier(&notes.objectAtIndex(index).request(), device)
    })
}

fn pending_identifiers(notes: &NSArray<UNNotificationRequest>, device: &str) -> Vec<String> {
    matching_identifiers(notes.count(), |index| matching_identifier(&notes.objectAtIndex(index), device))
}

/// Scans the whole system list and keeps at most `MAX_REMOVALS` matches.
fn matching_identifiers(count: usize, matching: impl Fn(usize) -> Option<String>) -> Vec<String> {
    let mut ids = Vec::new();
    for index in 0..count {
        if ids.len() == MAX_REMOVALS {
            tracing::warn!("notification removal stopped at the bounded identifier list");
            break;
        }
        if let Some(identifier) = matching(index) {
            ids.push(identifier);
        }
    }
    ids
}

fn matching_identifier(request: &UNNotificationRequest, device: &str) -> Option<String> {
    let owner = user_info_field(&request.content().userInfo(), USER_INFO_DEVICE)?;
    (owner == device).then(|| request.identifier().to_string())
}

fn withdraw(identifiers: &[Retained<NSString>]) {
    if identifiers.is_empty() {
        return;
    }
    let array = NSArray::from_retained_slice(identifiers);
    let center = UNUserNotificationCenter::currentNotificationCenter();
    center.removeDeliveredNotificationsWithIdentifiers(&array);
    center.removePendingNotificationRequestsWithIdentifiers(&array);
}
