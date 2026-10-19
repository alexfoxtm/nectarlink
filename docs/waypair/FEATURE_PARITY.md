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
| ADB | Elevated wireless debugging and USB loopback | Local `adb` forward unless `NECTARLINK_NO_ADB=1` | Desktop watcher plus fork startup of a seal-checked bundled ADB server when localhost5037 is closed. No runtime download or PATH/Homebrew fallback; existing servers are not restarted. | Source/pure fixture tests and packaging seal checks; fresh-host USB hardware acceptance pending. |
| Locked battery, ping, alarm | `BatteryMonitor` reports on battery broadcasts, not a timer. Ping is fixed: iroh keepalive 1s, app heartbeat 2.5s, drop after 8s idle (`session.rs`, `node.rs`). No locked-screen backoff. While mirroring with the panel off, `StreamPower.kt` holds a partial wake lock and high-perf Wi-Fi so that 8s timeout does not flap. Find-phone alarm policy is the fork extra above. | PC idle via `GetLastInputInfo` | PC idle via `CGEventSourceSecondsSinceLastEventType` (`mac/input.rs`), sent as `pc_active`. Find-my-PC rings with `afplay` of `Ping.aiff` for at most 60s (`mac/sound.rs`). | Ring policy unit test. Idle and `afplay` not run here. |
| Notification filters | Fork filter: all apps or a saved set; kinds message, email, other. No Gmail reader. | Toast actions, reply, progress (`win/toast.rs`) | `osascript` banner only (`mac/toast.rs`). Actions, reply, and progress are dropped. OTP copy is desktop-side and generic. | Filter unit test. No mirrored notification on a device. |
| SMS vs Gmail OTP | `PhoneSms.kt` reads and sends SMS/MMS from the telephony store. Caps `sms.read` / `sms.send` follow those permissions. | Shows threads because `sms.show` is announced | Same shared `messages.rs` UI, same cap. | Source. Gmail is not an API. A code is copied only if a notification passed the phone filter and `notifications.rs` `notification_otp` matches digits, then `write_private`. |
| File transfer | `files.transfer` is a base cap. Incoming files land in Downloads. | `transfers.rs` opens them with `explorer` | Native `/usr/bin/open` and Finder Reveal (`-R`) now replace the Windows-only command. | Three synthetic command-construction tests; owner peer-transfer acceptance remains pending. |
| Folder browse | `PhoneStorage.kt` lists shared storage or a picked folder (`storage.read` / `storage.write`). | Cloud Files mount when `storage.mount` is announced (`storage.rs`, `cfg(windows)`). | Mount cap withheld. `open_in_explorer` returns not supported. Phone browse of this Mac's Downloads, Documents, Desktop, and Pictures is shared `pc_storage.rs` behind `share_pc_storage` (off by default). `files.browse` is never announced, so matrix feature `remote.pc_files` stays locked on Windows too. | `pc_storage` unit tests use temp dirs. No phone browse. |
| Photo and document capture | Already implemented in source, upstream of this fork: `ContinuityCameraActivity.kt` captures, `DocumentScannerProcessor.kt` finds and straightens a page (`DocumentScannerProcessorTest.kt`). Cap `camera.capture` if a camera exists. This row is not a QA result. | Receives on `camera.continuity`, can focus the previous window and paste | Cap is announced. Image can be written to the pasteboard. `mac/tray.rs` `focus_window` returns false, so the app must not claim it pasted. | Phone scan logic has a unit test in tree. Desktop JPEG fixture test converts bytes. No shutter press and no paired photo. |
| Native menu / Services | Phone has no desktop menu. | Tray, jump list, share target, Send To, hotkeys (`win/tray.rs`, `jumplist.rs`, `share_target.rs`) | Native status item now has Open and Quit, wired to existing actions. Close-to-tray remains disabled pending runtime proof. Public capture Services are designed, not implemented; share target/hotkeys remain unavailable. | Pure event-tag tests plus Root build/runtime gates; see `MAC_CAPTURE_SERVICES.md`. |
| Messages and contacts | `PhoneSms.kt`, `PhoneContacts.kt` | Shown because `sms.show` and `contacts.show` are announced | Same. | Source. Needs the phone permissions. |
| Media | `PhoneMedia.kt` with notification access (`media.control`). `media.watch` for play-on-phone. | SMTC and per-app sessions | `media.control` withheld. `mac/media_sessions.rs` refuses commands. `mac/smtc.rs` is empty. `media.remote` is still a base cap, so this Mac can drive the phone's players. Streamed PC audio (`mac/audio_out.rs`) returns unsupported. | `audio_out` unit test expects `Unsupported`. |
| Mirroring and remote input | `MirrorService.kt`, `InputService.kt` / Elevated for `mirror.input` | Decode and inject (`win/h264.rs`, `win/input.rs`) | `mirror.view` withheld. `mac/h264.rs` errors on every codec. Pointer and key injection are no-ops and not announced. | Hidden rather than fake. |
| Camera (webcam) | `WebcamService.kt`, cap `camera.stream` with camera permission. Upstream. | `camera.virtual` plus `addon.vcam`. Matrix feature `camera.webcam` also requires Windows build 22000 (`features.rs`). | `camera.virtual` withheld. `mac/vcam.rs` does not register. The Windows build gate would still mark the feature unsupported. | Not a fork exclusive. |
| Calls, full duplex | Alerts, log, dial, answer (`PhoneCalls.kt`). In-call mute, speaker, hold, and keys via `CallCompanion.kt` (`call.incall`, Elevated, Android 12+). Voice PCM is not available to a normal app (`docs/research/hfp.md`). | Bluetooth device inspection only. Public APIs do not open an HFP SCO path. | `probe_status` is always unavailable and `connect_audio` fails with that reason. Alerts, log, and dial still use the shared calls UI. | `call_audio_is_reported_unavailable_and_connecting_fails`. **BLOCKED/deferred.** Still the highest roadmap priority when a later session is authorized. This round does not start it. |

`pc.power` (lock and sleep) is announced on Mac and shells out to `CGSession -suspend` or a keystroke, then `pmset sleepnow` (`mac/shell.rs`). Those commands were not run. `pc.audio` volume functions exist (`mac/audio.rs`) but the cap stays off. In this tree, `set_speaker_volume` ignores a failed `osascript`. PR #24 at `9c0f13d7` already fails closed; do not re-apply that patch here.

Shelf and Island ask `work_area()`. On Mac it returns `None` (`mac/mod.rs`), and the QML falls back to Qt's full screen, so the Dock and menu bar are not subtracted. That is degraded, not a false success.

## Do next (two slices)

1. **Reveal a received file.** In `desktop/app/src/transfers.rs`, `open` and `show_in_folder` always spawn `explorer`. On Mac, call `open` and `open -R`. `files.transfer` is already announced. No new cap, no phone, no input injection. Add a small test that the Mac command is not `explorer`.

2. **A status item with Open and Quit only.** `mac/tray.rs` `Tray::create` should show an `NSStatusItem` that raises the main window or quits. objc2 AppKit is already linked by the pasteboard. Leave hotkeys, window tracking, share/Services, and "paste into the last app" out of this slice. Do not announce `input.inject`.

Not in the next two slices: VideoToolbox mirroring, Now Playing, virtual camera, file-list clipboard, and notification reply actions. Call audio stays **BLOCKED/deferred** and remains the highest roadmap priority when a later session is authorized; this round does not start it. After `9c0f13d7` is integrated, keep `pc.audio` unadvertised until that fail-closed volume path is the code in this tree; a live speaker change still needs a person at the Mac. A later small slice is `work_area()` from the visible frame so Shelf and Island clear the Dock.

## Bounded tasks, in order

1. Mac `open` / `open -R` for received files (`transfers.rs`).
2. Status item: Open and Quit (`mac/tray.rs` only).
3. Integrate PR #24 `9c0f13d7` on the integration worker's branch; do not copy its audio fix twice.
4. Visible-frame `work_area()` for Shelf and Island.
5. Phone filter and find-phone volume are already in this fork. Remaining proof is a paired phone, which this session does not have.


## Round 23 implementation and verification follow-up

The snapshot above records the audit before implementation. Product merge `184aca79` now contains the reviewed `9c0f13d7` author update through history-preserving integration `435ffa2a`/`0383623f`; native Rust clipboard and the fork Mac Sparkle/R2 route are preserved. Root passed 19 pure Mac backend cases, eight updater cases, eighteen upstream-checker cases, and two real loopback download scenarios (resume and oversized chunked refusal). No audio command, personal call or microphone was executed.

Received-file Open/Reveal is fixed by `058f5c88`: macOS uses `/usr/bin/open` and Finder `-R`, preserving path arguments and request activation. Three focused transfer cases pass on the merged product, including a Unicode/spaces path and leading-dash safety. Actual phone-to-Mac transfer and native Finder interaction remain runtime acceptance gates. A status item is the next native desktop slice; none is claimed by this fix.

Android `6943bc8c` fixes numeric local-build ordering (`local.10` after `local.9`) and refuses malformed signed or non-ASCII identifiers. Root passed all nine existing updater/filter/alarm cases and `lintDebug`. `767e3308`/`231f40fe` make actual Waypair additions discoverable in both settings UIs and correct generic pairing wording for Macs as well as PCs. UI rendering and new release qualification are separate from these source/test results. That intermediate snapshot was superseded by the completed release below.


## Current published baseline after Round 23

Waypair Link Android `0.1.4-local.3`/code96 is published from `40525651`; macOS `0.1.4`/build31 is published and installed from `f6fb9b20`. Both carry the monochrome Waypair W and independently maintained fork wording, with original credits/full licenses behind Open-source Licenses. Root verified all eight R2 objects by full HTTP SHA/size readback; both Mac and disposable emulator fresh checks show current. The Mac's existing encrypted identity, Ocean theme and saved pair entry survived; obsolete NectarLink.app was removed. Cloud remains `0.1.9`/code10, with four legacy/Cloud feeds unchanged. Alex owns Galaxy installation and later pairing; no physical phone acceptance is claimed.

Wave24 continues the next native status-item slice, automatic Mac ADB bootstrap audit and public capture Services design. The completed Open/Finder Reveal fix and upstream `9c0f13d7` integration above are not pending work. Full-duplex call audio still needs an attended supported-route proof; it remains the highest feature priority without blocking independent progress.

### Wave24 scope

Native status-menu and sealed ADB bootstrap are the current bounded Mac changes;
qualification and publication are recorded separately by Root. The resource audit
in `LOCKED_PHONE_RESOURCE_PLAN.md` makes no measured battery claim and changes no
transport cadence. Native capture Services remain a design only. Cloud candidate
`0e64fa5b` is retained separately and is not the production Cloud10 source.
