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
| Native menu / Services | Phone has no desktop menu. | Tray, jump list, share target, Send To, hotkeys (`win/tray.rs`, `jumplist.rs`, `share_target.rs`) | Native status item has Open and Quit. The opt-in Keep in menu bar setting now persists on macOS; closing unloads the window only when a real status item was created, and Dock/Finder activation reopens it. Default remains off. Source review and desktop tests passed; signed normal-app close/Open/Quit acceptance remains pending. Public capture Services are designed, not implemented; share target/hotkeys remain unavailable. | Pure event-tag tests plus Root build/runtime gates; see `MAC_CAPTURE_SERVICES.md`. |
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

## Windows parity execution queue — 2026-10-19 Wave25

Alex explicitly wants the disabled Mac rows implemented, with the current Rust/Qt
and Kotlin/Compose stack retained. Grey controls describe today's build, not a
product decision to abandon those features. Do not announce a capability merely
because its UI, an SDK symbol or a Windows implementation exists.

Upstream rechecked at 2026-10-20 00:41 UTC: PR24 remains open at `9c0f13d7`,
canonical main is `5bdcadbf` (not yet integrated; photo-backup/hotspot changes
remain queued separately). Reuse upstream protocol/Android improvements and
prepare focused generic contributions after the maintainer's PR24 merge gate;
private R2 code stays exclusively in this fork.

| Priority/slice | Real Mac implementation work | Admission / current status |
|---|---|---|
| P0 Galaxy SIM voice continuity | Supported full-duplex call route, Mac microphone/speaker transport, safe handback | Hardware route unproven; next attended probe deferred. Ringing/answer/decline is not voice acceptance. No personal-call construction/capture/routing without fresh explicit authorization. |
| First video slice: H.264 screen receive | VideoToolbox/CoreMedia/CoreVideo decoder; Annex-B SPS/PPS/access units; bounded NV12 stride/color conversion | Cursor Auto source implementation dispatched. Keep `mirror.view` and UI disabled until Root synthetic decode and real transport/render gates pass. No codec-library binary bundling. |
| Mac-to-phone screen share | ScreenCaptureKit capture plus compatible negotiated encoding/transport | Separate source and synthetic route after decoder; explicit owner screen-capture approval before real capture. |
| Input / keyboard / pointer / local Deck actions | CoreGraphics event adapter, modifier/button cleanup and coordinate mapping | Needs explicit Accessibility approval for actual input; no permission bypass. Phone-side control continues through its existing approval/ADB path. |
| PC audio / phone speaker | Public macOS capture/playback backend, formats, bounded queues and lifecycle | Independent of SIM-call audio. Explicit permission gate for real system-audio capture; synthetic buffers first. |
| Phone as microphone / recordings | Native playback and encoding/container backend plus cancellation and device selection | Android mic consent remains authoritative; no microphone execution in this wave. |
| Virtual webcam | Native CoreMediaIO Camera Extension and frame transport | Separate extension, signing, install approval and distribution qualification; Windows driver cannot be reused as a Mac driver. |
| Phone file browser / Finder integration | First a bounded in-app browser; Finder/FileProvider integration separately | Preserve Android SAF/file permissions and explicit transfer destinations. Existing transfer Open/Reveal is already real; it is not a mount. |
| Global Mac media control | Public own-app media APIs or approved app-specific integration | Do not substitute private MediaRemote APIs for Windows' global media-session API. Generic control of arbitrary apps needs a supported-route assessment. |
| Focus / DND integration | Supported owner-controlled macOS integration | Public API/permission limitations require an honest supported alternative; no private Focus database writes. |
| Wake / global shortcuts / clipboard files | Native adapters with actual capability/permission checks | Source stubs remain off until verified; no enabling by flag alone. |
| Native photo/scan menu | Public AppKit Services and caller-pasteboard contract | Source exists; exact declarations verified in registry. Synthetic host menu gate currently HOLD (empty menu, no invocation). Fixture diagnosis active; never count this as native acceptance. |
| Tray / close-to-tray | Native menu/lifecycle | Explicitly deferred by Alex. Existing Open/Quit source is not permission to enable close-to-tray. |

Release work continues independently: Android98 is already published; installed
Mac32 remains current. Mac33 UI fixes are committed (`431d4111`, `2ee6ac36`),
independently source-reviewed and locally compiled. New publication is on hold
for demonstrated third-party notice/source-offer packaging gaps; repair workers
own disjoint Mac-packaging and Android-resource scopes. This is not a claim that
GPL forbids our open-source, independently branded fork. Cloud remains separate.

Liquid Glass / GPUI work is deferred: Alex asked to spend time on roadmap
implementation instead. No framework rewrite or new visual-effect dependency
was introduced.

## Current development checkpoint — 2026-10-20

Use one normal installed Waypair Link app per platform. Alex retired separate
QA/test app bundles and alternate installed identities. Code checks and synthetic
data fixtures continue; existing signing, data and pairing remain in place.

Android `0.1.4-local.6` / code99 is published from `b521d6c5`. Its normal
`com.waypair` APK keeps signer B and the same permissions. Third-party notices
and the exact source archive are accessible in About. Independent artifact
review, lint, alignment, installed-byte checks and a direct 98-to-99 emulator
upgrade passed; the fresh public update check reports current. All four R2
objects matched full-download hashes and five other feeds stayed unchanged.
This is not a new OTA cycle or a Galaxy installation. Cloud10 remains separate.

The following Mac source now compiles together and passes strict all-target
Clippy. Sixteen focused checks passed, two per component. This records source
and bounded fixture evidence, not native feature acceptance:

| Component | Implemented source | Remaining admission |
| --- | --- | --- |
| H.264 receive | VideoToolbox decode, bounded Annex-B parameters, NV12 stride/color conversion and format recovery | Original red-frame native fixture passed; actual peer transport and Qt painting remain unverified. `mirror.view` stays off. |
| H.264 send | VideoToolbox encoder, bounded AVCC conversion, SPS/PPS before IDR, keyframe requests and bitrate changes | Pure packet cases and a real synthetic VideoToolbox encode/decode roundtrip passed in an isolated code-only fixture containing the exact product modules. IDR/P/requested-IDR, dimensions and luma were checked. Full integrated compilation, peer transport and Android rendering remain. |
| Mac screen share | Public per-session ScreenCaptureKit picker, retained frame bounds and a view-only sender with generation-safe teardown | Capture decision/cancellation cases passed. No picker, personal capture or stop-race runtime test ran. `READY` and capture capability remain off. |
| Local input | Public CoreGraphics adapter, existing-trust check, owned modifier/button/unicode cleanup | Pure planning cases passed. No Accessibility grant or input event was performed; capabilities remain off. |
| PCM playback | AudioQueue output, explicit device choice, bounded queue/jitter and callback lifetime | Pure format/device/buffer cases passed. No playback or microphone use; audio capabilities remain off. |
| Clipboard files | Native local file URLs for explicit Send clipboard; private and torn lists refused; filenames do not auto-sync as text | Pure parsing and a synthetic named-pasteboard fixture passed. Normal Finder-to-peer acceptance remains; received-file auto-copy is still off on Mac. |
| Phone folders | Bounded in-app list/save, path identity, cancellation/revocation and exclusive destination publication | Pure cases passed. Explicit Mac opt-in, paired SAF/provider and filesystem acceptance remain; browser flag stays off and Finder mount is not implemented. |
| Usable display area | Public primary-screen metadata, menu-bar/Dock subtraction and physical-pixel conversion | Two coordinate cases passed. Native Retina/Dock placement still needs the normal app check. |

Mac33 is installed and published from `e90860f4` (private channel
`0.1.4-local.5`). The normal app, DMG and pinned source archive passed an
independent artifact review; four full R2 downloads matched and five other
feeds stayed unchanged. The normal updater reports current. Identity and
settings survived the direct in-place update; this is not a new OTA cycle.
Keep using that same installation. Native permissions and personal
call/audio/capture actions still need the owner's specific approval.

Native notification repair is the next Mac slice. The existing PR24
`osascript` poster uses the script process's system identity, drops click
routing and cannot use Waypair's bundle icon. Replace it with public
UserNotifications from the app, retain the phone title/body, and request
authorization only from an explicit settings action. This is not yet
implemented or accepted in the published Mac33 build.

The old browser Dock offer uses `NSApp.dockTile.contentView` in the same app;
it does not require a helper app or Apple's cross-device Handoff transport.
Porting it still needs a negotiated privacy/session/lifetime offer and an
explicit-click route that preserves Qt's application delegate. Ordinary
`clip.set` text alone is insufficient. Tray expansion stays deferred.


### Canonical main integration, Wave26

The fork now incorporates main `5bdcadbf` with original history, including photo backup, hotspot joining infrastructure, locale completeness and the UI-check entry point. Mac photo backup uses the existing native folder opener and a Waypair Link default folder; it stays opt-in and needs Photos and Files sharing. Claims use only known backup transfer IDs so unrelated incoming files, recordings and handoffs remain visible. Unknown backup transfers can briefly appear before their reply identifies them; no name/size heuristic claims user files. Mac hotspot joining stays visibly unavailable, with explicit unsupported adapters. The phone hotspot toggle is separate.

Twenty-five focused desktop checks passed; the twelve backup checks passed again after removing the now-write-only asking state. Chinese catalogue gaps were repaired rather than weakening the new completeness check. Peer compatibility is a source verdict: no wire, capability or version changes were introduced, so neither app needs to update first. Paired backup, permissions and final release qualification remain separate. Private R2 code remains fork-only.


### Published Wave26 outcome — 2026-10-20

Mac `0.1.4`/build34 (channel `0.1.4-local.6`) and Android `0.1.4-local.7`/code100 are published from `93cab164`. Mac34 is installed in place; its encrypted identity, saved peer IDs/pair dates and selected settings match the pre-update snapshot. Native notification authorization was already allowed, with no prompt or new grant. Home, notification settings and the opt-in backup row render; backup remains off. Alert icon/click/background-quiet confirmation is owner-pending.

All eight full R2 objects match local hashes. Each publication protected the other five feeds; Cloud10 and the three legacy feeds are unchanged. Both normal clients report current. The existing normal Android emulator upgraded99→100 directly with the same UID/first-install time and exact installed APK, then its network state was restored and it shut down without wiping. This is not a new OTA cycle or a Galaxy installation; Galaxy remains owner-led. No QA app bundles or alternate identities were installed.

Checks:32 focused desktop cases, the localhost gallery integration case, strict Clippy, format and canonical486-string checks; Android7 selected locale/updater cases, zero lint errors, same requested permissions/signer B and nine16KiB-aligned native libraries. A later two-case permission recheck covers the mapping now used by production. Known early-backup UI timing and all native transport/capture/audio/input admission gaps remain above; full-duplex SIM calls remain deferred and unproven.

Finished Orca workers were released and the merged, remotely preserved integration worktree was removed. Build outputs from different worktrees must not share a CXX-Qt target cache: the observed stale build-script bridge list was corrected by consolidating source and refreshing only generated workspace packages. Qualified archives, notices, signatures, source and receipts remain under `~/orca/tmp/waypair-wave26-20261019/release/`; post-publication outcomes are sidecars, not edits to frozen receipts.


### Wave27 source progress — 2026-10-20

Phone-folder browsing now requires an explicit session-local action; page activation alone never lists files. Closing the page, switching devices, disconnecting or losing admitted access clears names and cancels saves; generation checks reject late replies. Four browser tests, the Mac capability gate and four catalogue checks pass; strict Clippy, format and486-string generation checks pass. The browser remains disabled. Before enabling, close the matrix-silent unset-to-false backend-toggle case, phone-side revocation of an already-open read, path-transition admission and actual native/provider acceptance. A state-only test is not event-delivery proof.

Android SAF source fixes remove the empty persisted-grant bypass, derive read length from the opened descriptor instead of nullable provider metadata, refuse unknown lengths before handoff, validate directory components, and map provider security refusals to Denied. All-files paths, stored folders and permissions stay unchanged. Independent source review found no release blocker; both existing storage tests pass. These tests cover path rules, not DocumentsContract provider behavior. Release qualification and publication are separate from this source checkpoint.

The copied-link Dock report is a protocol design delta, not implemented capability: session/lock/expiry privacy and safe Qt reopen handling still gate it. The code-only VideoView paint fixture uses the production renderer and an offscreen synthetic scene; its runtime result is recorded outside the checkout. No additional app bundle or identity is created.


Wave27 publication outcome: Android `0.1.4-local.8`/code101 is signed and published from `9367660d`. Four full HTTP bodies match the qualified APK, corresponding source, sums and manifest; the other five feeds are unchanged. Normal emulator100→101 kept UID10224 and the first-install date, rendered About/licenses with the exact source offer, and its fresh public check reports current. This is not a new OTA cycle or a Galaxy install. Mac34/local.6 and Cloud10 stay unchanged.

The production VideoView passed both actual offscreen software paint-device cases (known BGRX color, then stale switch/clear); this is distinct from Metal/window and phone transport acceptance. Nine focused desktop, one core path/symlink and two existing Android storage cases pass, plus strict Clippy/format/string checks and zero release lint errors. All eight Orca attempts settled, the five created terminals released, and the owned emulator was restored to its radio snapshot and shut down without wiping. No new app bundle/identity or source worktree was created. Frozen qualification/source/publication evidence stays under `~/orca/tmp/waypair-wave27-20261020/release/`; future browser enablement and full-duplex calls remain gated above.


### Wave28 source — storage read revocation

Core reads now retain their originating session and a per-peer cancellation generation. Explicit sharing revocation and local storage-read capability loss cancel admitted reads, including stalled request, platform-open, metadata and body waits; late blocking results are dropped when they return. Path changes publish independently of small RTT differences, and entering Relay with remote file access off cancels active served reads. No heartbeat timer, wire opcode or per-chunk database polling was added.

Local raw storage/remote-files changes force the existing capability event even when the matrix is identical; routine refreshes remain quiet. Five core cases and six desktop cases pass, including a deliberate regression run that fails when forced delivery is disabled. Independent source cross-reviews and strict Clippy/format/string checks passed. Browser enablement stays off: phone-side consent push, actual native/provider acceptance and host folder-sharing opt-out remain separate gates. The code-only provider harness passed its repaired target/cleanup safety recheck for a controlled run and six local checks. It has not been run on a device/provider, creates no extra installed app, and its code101 guard must be repinned and reviewed before code102 use.


### Published Wave28 outcome — 2026-10-20

Source `0b5b7063` is pushed. Mac `0.1.4`/build35 (channel `0.1.4-local.7`) and Android `0.1.4-local.9`/code102 are signed and published in the existing personal R2 channels. All eight full HTTP bodies match; each publication preserves five other feeds. Cloud10 and the three legacy feeds remain unchanged. Both fresh manual update checks report current.

The normal Mac34→35 upgrade preserved the encrypted identity, saved pair IDs/dates and selected settings. About/licenses and exact corresponding-source offer passed; signing remains Apple Development, without notarization or Developer ID distribution proof. The normal emulator101→102 upgrade preserved UID10224, first-install time and the exact APK, rendered About/licenses and was restored to radio1/0 before shutdown without wiping. These are direct upgrades, not new OTA cycles; the Galaxy was not accessed or installed.

All Wave28 Orca workers settled and released, with no reclaimable terminals or child worktrees. Completed staging and unused generated outputs moved recoverably to Trash after dependency/open-file checks; qualified artifacts, valid notices and signing/compiler dependencies remain. Frozen receipts and post-publication sidecars are under `~/orca/tmp/waypair-wave28-20261020/release/`. Phone browsing remains disabled; provider/peer acceptance, native notification presentation acceptance and full-duplex SIM calling remain separate gates.


### Wave29 provider acceptance — 2026-10-20

The normal Android102 app passed 17 actual system ExternalStorageProvider checks through the production DocumentsContract path, paired to a code-only core peer over the explicit emulator USB tunnel. All-files access stayed off; only two synthetic SAF folders were granted through DocumentsUI. The checks cover consent refusal before approval, directory listing, descriptor-sized and ranged reads, growth after a stale listing, file/directory errors, revoking one folder while keeping the other usable, and provider failures after the second tree is deleted.

All six cleanup checks passed. The temporary peer, SAF grants, owned synthetic files, helper forward and temporary peer keys were removed. The emulator retained its package UID, first-install date, permissions, appops, pre-existing forward and radio snapshot, then shut down without wiping. Home again showed no paired computer and no selected run folders remained. No Galaxy or personal Mac pairing was accessed. Evidence: `~/orca/tmp/waypair-wave29-20261020/PROVIDER102-ACCEPTANCE.json` and the run receipt beneath it.

This proves the ordinary provider path for the already-published Android102 source `0b5b7063`; it does not prove an adversarial unknown descriptor size, a deterministic SecurityException race, the native Qt browser, or the new Wave29 per-peer consent push. The remaining browser gates are reviewed phone-side access propagation, host opt-out, path transitions and actual Qt browser acceptance; the ordinary provider gate is closed. Publication remains Mac35/Android102/Cloud10 until a later qualified cohort.

### Actual connection transport and menu-bar candidate (2026-10-20)

The connection badge now uses a separate display path following the selected Iroh route. A physical USB label requires the selected current bridge and a positive host USB descriptor. Wireless ADB carries a validated outer endpoint for matching the phone’s RAM-only interface table; native routes match their selected local interface. Only Wi-Fi/cellular/Ethernet/USB classifications travel in battery hints, with old or unknown hints falling back to Unavailable. Signal bars do not determine transport. Auth, routing and capability admission are unchanged. Actual peer transport acceptance is separate from source and localhost tests.

The native menu-bar option supersedes the earlier tray deferral at Alex’s request. It requires native normal-app verification before release. R2 publication and Galaxy installation are separate gates; personal call probes remain deferred.
