# Feature parity: Android phone with this Mac port

Source read of `waypair/macos-port` at `767e3308` (parent `9b93a5c5`, the dispatched product; the extra commit is About copy only). Compared with Windows seams in `desktop/app/src/win/`, Mac seams in `desktop/app/src/mac/` (compiled as `win` on macOS), and Android handlers. Fork delta is `e71dee41..767e3308`. Live macOS PR #24 head `9c0f13d7` is owned by the integration worker; this note only uses the published diff. Linux PR #34 (`bcc2caee`) is a source reference and is not in this matrix. No build, device, credential, or Galaxy session. Alex is disconnected.

About strings and guide prose are not runtime proof. "Unit test in tree" means a test exists and this session did not run it. "Unproven" means no paired-phone run.

## What is actually a Waypair extra

Fork-only, and named in About (`767e3308`):

- Phone notification app/type filter. `android/.../notifications/NotificationFilter.kt`, applied in `NotificationListener.mirrored` before title or body is read. Default is every app and every kind. A corrupt record blocks mirroring. `NotificationFilterTest.kt` covers the parser. SMS permission is not consulted.
- Find-phone alarm volume. `RingPolicy.kt` plus `Ringer.kt`: raise the alarm stream to max, restore the captured volume once, and if Do Not Disturb is total silence, vibrate only. `RingPolicyTest.kt` is pure logic. Nobody heard it on hardware here.

Fork-only, and not a new phone feature: Mac clipboard privacy (`mac/clipboard.rs`), real idle time (`mac/input.rs`), and an honest "call audio unavailable" result (`mac/bt_hfp.rs`). Those match Windows behavior or stop a false success. Private R2 updates are fork-only and stay out of this matrix.

Not Waypair extras. Unchanged against `e71dee41`: wireless-debugging Elevated ADB (`Elevated.kt`, `InputServer.kt`, `nectarlink-adb`, USB tunnel `core/nectarlink-core/src/usb.rs`), webcam (`WebcamService.kt`, desktop `webcam.rs`, Windows virtual camera), and Continuity Camera plus the phone document scanner (`ContinuityCameraActivity.kt`, `DocumentScannerProcessor.kt`). The previous Waypair app is not a bridge target. Cloud is separate.

## Caps the Mac actually announces

`desktop/app/src/core_host.rs` `compute_capabilities`, plus core `BASE_CAPABILITIES` (`core.ping`, `device.battery`, `device.ring`, `files.transfer`, `clip.image`, `media.remote`, `link.open`, `task.notify`).

Announced on Mac: `pc.power`, `photos.show`, `camera.continuity`, `calls.show`, `contacts.show`, `sms.show`, `toggles.show`.

Withheld on Mac, and covered by `macos_advertises_only_supported_capabilities` (unit test, not a device): `mirror.view`, `mirror.listen`, `media.control`, `storage.mount` / `storage.read` / `storage.write`, `camera.virtual`, `pc.audio`, `recorder`, `deck.actions`, `media.stream`, `mic.listen`, `pc.wake`, and the conditional `mirror.capture` / `input.inject`. Windows announces those from the same function. `files.browse` is required for `remote.pc_files` in `features.rs` and is announced on neither platform.

## Matrix

| Area | Android | Windows | Mac now | Proof |
|---|---|---|---|---|
| Clipboard privacy | `PhoneClipboard.kt`; auto-read only when Elevated (`clip.read.auto`) | `win/clipboard.rs` monitor-exclusion formats | Real `NSPasteboard`. Concealed, transient, and auto-generated types stay local. File lists are stubs (`read_files` / `write_files`). | Clipboard unit tests, including a 1x1 JPEG conversion. No paired copy. |
| ADB | Elevated wireless debugging and USB loopback | Local `adb` forward unless `NECTARLINK_NO_ADB=1` | Same desktop watcher. Not a Mac-only gap and not a fork feature. | Source only. |
| Locked battery, ping, alarm | `BatteryMonitor` reports on battery broadcasts, not a timer. Ping is fixed: iroh keepalive 1s, app heartbeat 2.5s, drop after 8s idle (`session.rs`, `node.rs`). No locked-screen backoff. While mirroring with the panel off, `StreamPower.kt` holds a partial wake lock and high-perf Wi-Fi so that 8s timeout does not flap. Find-phone alarm policy is the fork extra above. | PC idle via `GetLastInputInfo` | PC idle via `CGEventSourceSecondsSinceLastEventType` (`mac/input.rs`), sent as `pc_active`. Find-my-PC rings with `afplay` of `Ping.aiff` for at most 60s (`mac/sound.rs`). | Ring policy unit test. Idle and `afplay` not run here. |
| Notification filters | Fork filter: all apps or a saved set; kinds message, email, other. No Gmail reader. | Toast actions, reply, progress (`win/toast.rs`) | `osascript` banner only (`mac/toast.rs`). Actions, reply, and progress are dropped. OTP copy is desktop-side and generic. | Filter unit test. No mirrored notification on a device. |
| SMS vs Gmail OTP | `PhoneSms.kt` reads and sends SMS/MMS from the telephony store. Caps `sms.read` / `sms.send` follow those permissions. | Shows threads because `sms.show` is announced | Same shared `messages.rs` UI, same cap. | Source. Gmail is not an API. A code is copied only if a notification passed the phone filter and `notifications.rs` `notification_otp` matches digits, then `write_private`. |
| File transfer | `files.transfer` is a base cap. Incoming files land in Downloads. | `transfers.rs` opens them with `explorer` | Same `explorer` call, so reveal/open fails on Mac even though the transfer cap is announced. | Source inspection of `open` / `show_in_folder`. |
| Folder browse | `PhoneStorage.kt` lists shared storage or a picked folder (`storage.read` / `storage.write`). | Cloud Files mount when `storage.mount` is announced (`storage.rs`, `cfg(windows)`). | Mount cap withheld. `open_in_explorer` returns not supported. Phone browse of this Mac's Downloads, Documents, Desktop, and Pictures is shared `pc_storage.rs` behind `share_pc_storage` (off by default). `files.browse` is never announced, so matrix feature `remote.pc_files` stays locked on Windows too. | `pc_storage` unit tests use temp dirs. No phone browse. |
| Photo and document capture | Continuity Camera activity and `DocumentScannerProcessor` (upstream). Cap `camera.capture` if a camera exists. | Receives on `camera.continuity`, can focus the previous window and paste | Cap is announced. Image can be written to the pasteboard. `mac/tray.rs` `focus_window` returns false, so the app must not claim it pasted. | JPEG fixture test. No shutter press. |
| Native menu / Services | Phone has no desktop menu. | Tray, jump list, share target, Send To, hotkeys (`win/tray.rs`, `jumplist.rs`, `share_target.rs`) | `Tray::create` returns an error. Jump list register is empty. `take_activated_share` returns none. Hotkeys are empty. | The error string is the implementation. |
| Messages and contacts | `PhoneSms.kt`, `PhoneContacts.kt` | Shown because `sms.show` and `contacts.show` are announced | Same. | Source. Needs the phone permissions. |
| Media | `PhoneMedia.kt` with notification access (`media.control`). `media.watch` for play-on-phone. | SMTC and per-app sessions | `media.control` withheld. `mac/media_sessions.rs` refuses commands. `mac/smtc.rs` is empty. `media.remote` is still a base cap, so this Mac can drive the phone's players. Streamed PC audio (`mac/audio_out.rs`) returns unsupported. | `audio_out` unit test expects `Unsupported`. |
| Mirroring and remote input | `MirrorService.kt`, `InputService.kt` / Elevated for `mirror.input` | Decode and inject (`win/h264.rs`, `win/input.rs`) | `mirror.view` withheld. `mac/h264.rs` errors on every codec. Pointer and key injection are no-ops and not announced. | Hidden rather than fake. |
| Camera (webcam) | `WebcamService.kt`, cap `camera.stream` with camera permission. Upstream. | `camera.virtual` plus `addon.vcam`. Matrix feature `camera.webcam` also requires Windows build 22000 (`features.rs`). | `camera.virtual` withheld. `mac/vcam.rs` does not register. The Windows build gate would still mark the feature unsupported. | Not a fork exclusive. |
| Calls, full duplex | Alerts, log, dial, answer (`PhoneCalls.kt`). In-call mute, speaker, hold, and keys via `CallCompanion.kt` (`call.incall`, Elevated, Android 12+). Voice PCM is not available to a normal app (`docs/research/hfp.md`). | Bluetooth device inspection only. Public APIs do not open an HFP SCO path. | `probe_status` is always unavailable and `connect_audio` fails with that reason. Alerts, log, and dial still use the shared calls UI. | `call_audio_is_reported_unavailable_and_connecting_fails`. Deferred. Do not implement. |

`pc.power` (lock and sleep) is announced on Mac and shells out to `CGSession -suspend` or a keystroke, then `pmset sleepnow` (`mac/shell.rs`). Those commands were not run. `pc.audio` volume functions exist (`mac/audio.rs`) but the cap stays off. In this tree, `set_speaker_volume` ignores a failed `osascript`. PR #24 at `9c0f13d7` already fails closed; do not re-apply that patch here.

Shelf and Island ask `work_area()`. On Mac it returns `None` (`mac/mod.rs`), and the QML falls back to Qt's full screen, so the Dock and menu bar are not subtracted. That is degraded, not a false success.

## Do next (two slices)

1. **Reveal a received file.** In `desktop/app/src/transfers.rs`, `open` and `show_in_folder` always spawn `explorer`. On Mac, call `open` and `open -R`. `files.transfer` is already announced. No new cap, no phone, no input injection. Add a small test that the Mac command is not `explorer`.

2. **A status item with Open and Quit only.** `mac/tray.rs` `Tray::create` should show an `NSStatusItem` that raises the main window or quits. objc2 AppKit is already linked by the pasteboard. Leave hotkeys, window tracking, share/Services, and "paste into the last app" out of this slice. Do not announce `input.inject`.

Leave these alone until a later dispatch: VideoToolbox mirroring, Now Playing, virtual camera, file-list clipboard, notification reply actions, and any call-audio route. After `9c0f13d7` is integrated, keep `pc.audio` unadvertised until that fail-closed volume path is the code in this tree; a live speaker change still needs a person at the Mac. A later small slice is `work_area()` from the visible frame so Shelf and Island clear the Dock.

## Bounded tasks, in order

1. Mac `open` / `open -R` for received files (`transfers.rs`).
2. Status item: Open and Quit (`mac/tray.rs` only).
3. Integrate PR #24 `9c0f13d7` on the integration worker's branch; do not copy its audio fix twice.
4. Visible-frame `work_area()` for Shelf and Island.
5. Phone filter and find-phone volume are already in this fork. Remaining proof is a paired phone, which this session does not have.
