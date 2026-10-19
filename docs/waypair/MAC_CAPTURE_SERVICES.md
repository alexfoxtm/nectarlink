# Mac capture Services: Take Photo and Scan Document from other apps

Design contract for two macOS Services, **Take Photo** and **Scan Document**.
They let another Mac app ask the paired Galaxy (Waypair Link Android) for an
image and receive it in place. This is a source-bound design against
`f6fb9b20`. Nothing here is implemented, built, or tested on a device yet.
Calls and voice are unrelated and deferred. The status item and ADB are
separate slices owned by other work and are not changed by this contract.

## What macOS allows, stated honestly

- **Services are the only public way into other apps' menus.** An app
  declares `NSServices` in its `Info.plist` and sets
  `NSApplication.servicesProvider`. AppKit lists the service in the app
  menu's **Services** submenu of every app where it applies. AppKit text views
  usually also show a Services submenu in their contextual menu, but the
  sources below don't document that. It is checked per app, not promised.
- **Where an image-returning service is enabled.** For each service, AppKit
  asks the host's key-window responder chain
  `validRequestor(forSendType:returnType:)`. A service that only returns
  images (`NSReturnTypes` PNG/TIFF, no `NSSendTypes`) is enabled only where
  some responder accepts that return type: rich-text editors, Mail compose,
  and any app that adopted Apple's Continuity Camera through the same
  method. It stays disabled in plain-text fields, Terminal, and custom views
  that don't opt in. That is the correct behaviour, not a defect.
- **No injection into every Mac menu.** There is no public API to add items
  to arbitrary apps' menus, toolbars, or contextual menus outside Services.
  Finder Sync's `menu(for:)` adds Finder menus only for folders the extension
  monitors, and needs a sandboxed app extension. It inserts nothing into a
  host document, so it is out of scope.
- **Not Apple Continuity Camera.** `NSMenuItem.importFromDeviceIdentifier`
  and the system "Take Photo / Scan Documents" items are filled in by macOS
  for the user's iPhone or iPad. A third-party app can't register an Android
  phone there, and this design uses no private API or `com.apple.*`
  entitlement. The signing stays as in `MACOS_PACKAGING.md`: hardened runtime
  with no extra entitlements.
- **Our own menus** (the status item, the Dock menu through
  `applicationDockMenu(_:)`, and the app window) can start the same capture.
  It then lands on the clipboard only, as described under Insertion below.

## Current pipeline (at `f6fb9b20`)

- Core (`core/nectarlink-core/src/continuity_camera.rs`, MPL-2.0):
  - `request()` checks pairing and the per-device `photos` toggle, then sends
    `camera.capture.request`. When there is no live session, or the send
    fails transiently, it **enqueues** a `CameraRequest` that expires after
    2 minutes (`store.rs`) and still returns `Ok`.
  - The phone answers `camera.capture.ok`. The core validates it and drops
    it: no event reaches the desktop.
  - `receive_result()` takes JPEG or PNG up to 32 MiB
    (`MAX_IMAGE_BYTES`) within a 60 s transfer timeout. It **always saves the
    image to the downloads folder** and the timeline, then emits
    `CameraCaptureReceived` with the bytes.
- Desktop coordinator (`desktop/app/src/continuity_camera.rs`, GPL-3.0) is
  shaped for Windows:
  - It snapshots an `HWND`, writes the clipboard, and synthesizes Ctrl+V.
  - It matches a result by `request_id` **or device**, so a late result from
    an older request counts as the current one.
  - Starting a new capture cancels the previous one with reason `replaced`.
  - It maps cancel reasons `busy` and `permission_denied`, but Android sends
    localized sentences, so the Mac always shows the generic "cancelled"
    text.
- Mac stubs:
  - `mac/tray.rs`: `snapshot_target_window` returns `None` and
    `focus_window` returns `false`, so Mac captures today are clipboard-only
    and never claim they pasted.
  - `mac/share_target.rs`: no activation source.
  - `mac/clipboard.rs` writes PNG on the main thread through GCD, with a
    2 s timeout that reports outcomes honestly.
  - `mac/toast.rs` is an `osascript` banner with no actions.
- Packaging (`scripts/macos/package-app.sh`): `CFBundleName`
  `Waypair Link`, bundle id `com.waypair.mac` for releases. No `NSServices`
  key today.
- Android (`ContinuityCameraActivity.kt`, `ContinuityCameraRequest.kt`):
  - The request opens the activity, or a high-priority notification that
    times out after 2 minutes.
  - Camera permission is asked on the phone.
  - The user presses the shutter, reviews, and taps **Use photo** or
    **Send scan** (with **Retake** available). Leaving the screen sends
    cancel.
  - A cancel from the PC closes the screen.

## Contract

### 1. Declaration (packager)

`package-app.sh` writes two `NSServices` entries:

| Key | Take Photo | Scan Document |
| --- | --- | --- |
| `NSMenuItem.default` | `Waypair Link: Take Photo with Phone` | `Waypair Link: Scan Document with Phone` |
| `NSMessage` | `waypairCapture` | `waypairCapture` |
| `NSUserData` | `photo` | `scan` |
| `NSPortName` | the `CFBundleName` value (`Waypair Link`) | same |
| `NSReturnTypes` | `public.png`, `public.tiff` | same |
| `NSSendTypes` | absent | absent |
| `NSRequiredContext` | `{}` (empty, required for the item to show) | same |
| `NSTimeout` | `"95000"` | same |
| `NSServiceDescription` | token, localized in `ServicesMenu.strings` | same |

- **No `NSKeyEquivalent`.** Apple warns that shortcuts collide and that the
  host app's own shortcut wins. Users can assign one under System Settings
  > Keyboard > Keyboard Shortcuts > Services.
- **No `/` in menu text**, because AppKit drops everything before a slash.
- `ServicesMenu.strings` for `en` and `zh_CN`, matching the app's
  translations.
- **Validation:** `NSPortName` must equal `CFBundleName`, and the plist must
  pass `plutil -lint`. A qualification bundle id must also change the menu
  text (for example a `(Qualification)` suffix), so two installed builds
  don't register the same names.

### 2. Provider object, thread, and lifetime

- **New file `desktop/app/src/mac/services.rs`:**
  - One Objective-C class defined with objc2 `define_class!`, with a narrow
    `#[allow(unsafe_code)]` as in `mac/input.rs`. The workspace stays
    `unsafe_code = "deny"`.
  - One method, `waypairCapture:userData:error:`, which receives
    `(NSPasteboard*, NSString*, NSString**)`.
- **Registration:**
  - In `main.rs` under `cfg(target_os = "macos")`, after
    `QGuiApplication::new()`, `core_host::start`, and settings load, and
    just before `app.exec()`. Apple says requests can arrive as soon as the
    provider is set.
  - If `servicesProvider` is already non-nil (set by Qt or a plugin), log
    it and don't override.
  - The instance lives in a process-lifetime `static`, so the provider is
    never deallocated while AppKit holds it.
  - At shutdown, set it to nil first, before `drop(engine)` and
    `core_host::shutdown()`.
- **Thread:**
  - AppKit delivers the call through the main run loop. The method checks
    `MainThreadMarker`; if it's not on the main thread it fails closed with
    an error and starts no request.
  - The method stays synchronous until it writes the pasteboard or gives up.
    Apple's model: the requester waits for the response until `NSTimeout`
    and then continues without it.
- **Waiting without freezing Waypair:**
  - Run the current `NSRunLoop` in `NSDefaultRunLoopMode` in slices of at
    most 100 ms. This keeps Qt and the main GCD queue running (which
    `mac/clipboard.rs` needs).
  - Between slices, poll a `std::sync::mpsc` receiver. The result comes from
    the core event thread and needs no main-thread hop.
- **Re-entrancy guard:** only one service request at a time. A second
  invocation, including one delivered during the nested loop, returns at
  once with the error `busy`. While a service request is open, the app
  window can Cancel it (and so can the status item, if it later gets a
  Cancel item; the first slice has only Open and Quit), but nothing can
  start a new capture.
- **Quit:** an app-quit flag ends the wait with `cancelled` and sends a
  phone cancel with reason `cancelled`.

### 3. Readiness, request, expiry, cancel

The deadlines run from the moment the method is entered:

| Phase | Limit | Result when it fails |
| --- | --- | --- |
| Preflight | ≤ 1.5 s, local state only | error and banner, nothing sent |
| Phone acknowledgement (`camera.capture.ok`) | 10 s | phone cancel `no_ack`, error and banner |
| Capture and transfer | until 90 s | switch to clipboard fallback (no cancel) |
| Hard expiry | 120 s | phone cancel `expired`, state cleared |

- **Preflight**, all from hub state, with no network wait:
  - A paired device exists.
  - The chosen device is `LinkState::Online`. The choice is the first online
    phone, or the preferred index when that one is online. There is no
    picker, because the requester is blocked.
  - The `photos` toggle is on.
  - The phone announced `camera.capture`, when capabilities are known.
  - If any check fails, return the error and show a banner such as
    "Pair your phone first", "<phone> isn't connected",
    "Allow Photos for <phone> in Waypair Link", or
    "<phone> can't capture".
- **Never queue from a Service.** A queued request returns `Ok` while
  nothing reached the phone, and the requester would wait for nothing.
  - Add core `Node::request_camera_capture_now(peer, id, mode)`. It checks
    pairing and the toggle, then calls `request_on_session` and returns
    `Offline` when there's no session. It never calls `enqueue_send_item`.
    The existing `request()` and its 2-minute queue stay unchanged for the
    window and tray paths.
- **Acknowledgement event.** Emit the dropped `camera.capture.ok` as
  `NodeEvent::CameraCaptureAccepted { device, request_id }`. This is the
  only truthful signal that the phone showed the request.
  - Update every exhaustive `NodeEvent` match: desktop `state.rs`, and
    `core/nectarlink-ffi` if it doesn't use a wildcard.
- **Request id:** `mac-svc-<unix_ms>-<seq>`, within `MAX_ID_BYTES` (64).
- **Matching:** strictly by `request_id`. The device-based fallback in
  `continuity_camera::on_event` must not apply to service-bound requests.
- **Cancel paths:**
  - The phone cancels (back button, Cancel, permission denied, `photos`
    off): return at once with an error, and show a banner with the phone's
    reason. Strip control characters from the reason and cut it to 120
    characters, since it is free text from the peer.
  - Waypair Cancel (window or status item): `continuity_camera::cancel()`
    sends the phone cancel, and the waiter returns an error.
  - The user presses Esc or ⌘. in the requesting app. Apple documents this
    as cancelling the request, but **it does not say whether the provider is
    told**, so assume it isn't. The provider keeps waiting until a result or
    its deadline. A result that arrives then goes to a pasteboard nobody
    reads, so the banner wording below never claims insertion, and the
    downloads copy exists anyway.
  - The link drops (`LinkChanged` to offline): error and banner. No queue
    fallback.

### 4. Android user confirmation

No Android change is required, and none may weaken confirmation. A Service
never captures silently:

- The phone shows the request (an activity, or the 2-minute heads-up
  notification).
- Android asks for camera permission if it isn't granted.
- The user presses the shutter, reviews, and taps **Use photo** or
  **Send scan**.

Background activity limits on Android can reduce the request to a
notification tap, so the 10 s acknowledgement measures delivery, not the
user's action. An optional later Android cleanup: send the machine reason
codes `busy` and `permission_denied` alongside the localized text.

### 5. Insertion and clipboard fallback

- **Delivery before 90 s:**
  - Decode the phone's JPEG with the existing `mac/image.rs`, which is
    Qt-backed and takes ≤ 32 MiB.
  - Encode PNG, and TIFF through `NSBitmapImageRep`. Qt's TIFF plugin isn't
    deployed.
  - Write both to the **service pasteboard passed to the method**. Then
    return without an error, and AppKit sends the requester
    `readSelection(from:)`.
  - Don't touch the general pasteboard.
- **Fallback when the result arrives after 90 s and before 120 s,** or when
  writing the service pasteboard fails:
  - Write PNG to the general pasteboard with `mac::clipboard::write_image`.
    This also suppresses the self-echo.
  - Show a banner: "Photo from <phone> is on the clipboard — press ⌘V
    (<app> stopped waiting)".
- **Unknown request id**, or a result after 120 s: keep today's
  clipboard-only behaviour with its existing wording.
- **Every path:** the core has already saved the file in the downloads
  folder and the timeline, so no capture is lost.
- **No synthesized keystrokes and no focus changes on macOS.** Waypair
  doesn't activate itself during a Service; that could steal the requester's
  key window. `mac/tray.rs` keeps returning `false` from `focus_window`.

### 6. Progress and error UX (truthful states only)

The banners go through `mac/toast.rs`. The `continuity_camera::view()`
fields (`busy`, `mode`, `status`) are the shared state the window and status
item read. Allowed messages:

- At entry: "Asking <phone>…"
- After acknowledgement: "Take the photo on <phone>" or "Scan the document
  on <phone>", plus "(<app> waits up to 90 s; Esc stops waiting)".
- On return: "Photo from <phone> returned to <app>". The `<app>` value is
  `NSWorkspace.frontmostApplication` read at entry and labelled as the
  requesting app. Never say "pasted" or "inserted".
- Fallback and errors as in sections 3 and 5.

The `error:` string the method returns is **only logged to the console**
(per Apple), so every user-facing failure also needs a banner. Strings use
`tr!("MacServices", …)` and are regenerated with the canonical extractor.

### 7. Licensing and provenance

- New desktop Rust is GPL-3.0-or-later with SPDX headers. Core changes keep
  MPL-2.0. Upstream notices are kept, and commits use DCO sign-off
  (`FORK_POLICY.md`).
- Apple documentation is a reference only. No Apple sample code is copied:
  the archived guide's ROT13 and plist samples are illustrations, and the
  plist keys are facts.
- No new crates. Only feature flags on the existing objc2 crates, which are
  MIT-licensed:
  - `objc2-app-kit`: `NSApplication`, `NSResponder`, `NSRunningApplication`,
    `NSWorkspace`, `NSBitmapImageRep`, `NSImageRep`.
  - `objc2-foundation`: `NSRunLoop`, `NSDate`.
  - `Cargo.lock` should not change. Root confirms that when compiling.
- No private API, no new entitlement, and no camera or microphone usage key
  on the Mac. The camera is on the phone.

## File ownership (proposed)

| File | Change | Owner note |
| --- | --- | --- |
| `desktop/app/src/mac/services.rs` (new) | provider class, wait loop, pasteboard write, pure state machine plus unit tests | capture-services implementer |
| `desktop/app/src/mac/mod.rs` | `pub mod services;` | one line; coordinate with the status-item slice |
| `desktop/app/src/continuity_camera.rs` | `start_for_service(mode) -> Ticket`, strict id routing, busy guard; Windows behaviour unchanged | shared file; Windows tests must stay green |
| `desktop/app/src/main.rs` | macOS-only register and unregister | small `cfg` block |
| `desktop/app/src/state.rs` | map `CameraCaptureAccepted` | one arm |
| `desktop/app/Cargo.toml` | objc2 feature flags | the status-item slice also edits the `objc2-app-kit` list; merge the feature strings, Root compiles |
| `core/nectarlink-core/src/{continuity_camera,node,events}.rs` | `request_camera_capture_now`, `CameraCaptureAccepted` | MPL core; ffi match if exhaustive |
| `scripts/macos/package-app.sh` | `NSServices`, `ServicesMenu.strings`, lint | packaging |
| `desktop/app/i18n/*` | regenerated strings | canonical extractor |
| `docs/waypair/FEATURE_PARITY.md`, `MACOS_PACKAGING.md` | row and plist keys after it lands | docs |

Not touched: `mac/tray.rs` (status item), the ADB paths, Android, calls and
voice.

## Focused synthetic acceptance (no camera, microphone, or phone)

1. **Unit, pure:**
   - Preflight decisions (no pair, offline, toggle off, no capability).
   - Deadline transitions at 10, 90, and 120 s with an injected clock.
   - Strict id matching, so a late result from another id never resolves
     the waiter.
   - Busy guard.
   - Reason sanitizing.
   - The PNG/TIFF encoding pair from the existing JPEG fixture.
2. **Core:**
   - `request_camera_capture_now` without a session returns `Offline` and
     leaves `send_queue` empty.
   - `CameraCaptureAccepted` is emitted in the existing
     `continuity_camera_photo_and_scan_flow` harness
     (`core/nectarlink-core/tests/two_devices.rs`).
3. **Packaging:**
   - `plutil -lint`.
   - `plutil -extract NSServices xml1` has two entries with
     `NSPortName == CFBundleName`, `NSRequiredContext` present, no `/` in
     menu text, and `NSTimeout` set to `95000`.
4. **Runtime, by Root on the Mac, with a synthetic phone:**
   - Setup: a qualification bundle paired with `nectarlink-cli --as-phone`.
     It answers requests with a generated 800×600 JPEG after
     `NECTARLINK_CLI_CAMERA_DELAY_MS`.
   - Registration: `/System/Library/CoreServices/pbs -dump_pboard` lists
     both services (Apple marks `pbs` as a debug tool only).
     `TextEdit -NSDebugServices <bundle id>` explains enablement.
   - In TextEdit: the items are enabled in a rich-text document and disabled
     in a plain-text one.
   - Delivery: delay 1200 ms. A Swift scratch requester calls
     `NSPerformService("Waypair Link: Take Photo with Phone", pb)`. It
     returns true with an 800×600 PNG on `pb`. The general pasteboard's
     `changeCount` is unchanged, and the file exists in downloads.
   - Timeout: delay 100000 ms. The requester gets false by about 95 s. The
     image then appears on the general pasteboard with the fallback banner.
   - Offline: stop the CLI. The requester gets false within 2 s, nothing is
     queued, and the error banner shows.
   - Cancel: Waypair Cancel during the wait. The requester gets false
     promptly, and the CLI logs the cancel.
   - Busy: a second invocation during the wait returns at once.
   - Not in scope here: a physical Galaxy, a real shutter press, and
     per-app context-menu coverage beyond TextEdit. Those stay separate
     acceptance gates for Alex.

## Primary sources (read 2026-10-19)

Apple Developer documentation, current:

- NSServices: <https://developer.apple.com/documentation/bundleresources/information-property-list/nsservices>
  - Sub-keys: `…/nsservices/nsmenuitem`, `…/nsmessage`, `…/nsportname`,
    `…/nsreturntypes`, `…/nssendtypes`, `…/nstimeout`, `…/nsuserdata`,
    `…/nskeyequivalent`. These pages give one-line definitions; the
    detailed semantics are in the archived guide below.
- NSApplication.servicesProvider: <https://developer.apple.com/documentation/appkit/nsapplication/servicesprovider>
- NSApplication.servicesMenu: <https://developer.apple.com/documentation/appkit/nsapplication/servicesmenu>
- registerServicesMenuSendTypes(_:returnTypes:): <https://developer.apple.com/documentation/appkit/nsapplication/registerservicesmenusendtypes(_:returntypes:)>
- NSResponder.validRequestor(forSendType:returnType:): <https://developer.apple.com/documentation/appkit/nsresponder/validrequestor(forsendtype:returntype:)>
- NSServicesMenuRequestor: <https://developer.apple.com/documentation/appkit/nsservicesmenurequestor>
  - readSelection(from:): <https://developer.apple.com/documentation/appkit/nsservicesmenurequestor/readselection(from:)>
- NSPerformService(_:_:): <https://developer.apple.com/documentation/appkit/nsperformservice(_:_:)>
- NSUpdateDynamicServices(): <https://developer.apple.com/documentation/appkit/nsupdatedynamicservices()>
- NSPasteboard.PasteboardType: `png` <https://developer.apple.com/documentation/appkit/nspasteboard/pasteboardtype/png>, `tiff` <https://developer.apple.com/documentation/appkit/nspasteboard/pasteboardtype/tiff>
- NSMenuItem.importFromDeviceIdentifier: <https://developer.apple.com/documentation/appkit/nsmenuitem/importfromdeviceidentifier>
- Supporting Continuity Camera in Your Mac App: <https://developer.apple.com/documentation/appkit/supporting-continuity-camera-in-your-mac-app>
- NSApplicationDelegate.applicationDockMenu(_:): <https://developer.apple.com/documentation/appkit/nsapplicationdelegate/applicationdockmenu(_:)>
- Finder Sync: <https://developer.apple.com/documentation/findersync>
  - FIFinderSyncProtocol.menu(for:): <https://developer.apple.com/documentation/findersync/fifindersyncprotocol/menu(for:)>
  - FIFinderSyncController: <https://developer.apple.com/documentation/findersync/fifindersynccontroller>

Apple Services Implementation Guide (archived, last updated 2016-09-13; Apple
no longer updates it, but it is the only official source for `NSTimeout`'s
30 s default, Esc/⌘. cancel, `NSRequiredContext`, the slash rule,
console-only errors, and `pbs`/`NSDebugServices`):

- Introduction: <https://developer.apple.com/library/archive/documentation/Cocoa/Conceptual/SysServices/introduction.html>
- Services Properties: <https://developer.apple.com/library/archive/documentation/Cocoa/Conceptual/SysServices/Articles/properties.html>
- Providing a Service: <https://developer.apple.com/library/archive/documentation/Cocoa/Conceptual/SysServices/Articles/providing.html>
- Using Services: <https://developer.apple.com/library/archive/documentation/Cocoa/Conceptual/SysServices/Articles/using.html>

Apple Support:

- Use services in apps on Mac: <https://support.apple.com/guide/mac-help/use-macos-services-mchlp1012/mac>

Undocumented, and therefore verified at runtime instead of assumed:

- whether a requester's Esc cancel notifies the provider;
- whether macOS caps `NSTimeout`;
- the requester's UI while it waits;
- whether a given third-party host shows Services in its contextual menu.
