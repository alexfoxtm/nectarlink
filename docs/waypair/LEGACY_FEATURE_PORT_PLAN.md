# Legacy Link gaps worth a later port

This note compares the Waypair Link fork at `f97bd213` with the old Waypair Link in `/Users/AleksandrSlobodanyuk/Projects/icloud-galaxy`. Later branding commits such as `8b7e7638` are another worker's labels and are not part of this comparison. It does not replace `FEATURE_PARITY.md`. That file's closing snapshot still describes Android 96 and Mac 31; publication evidence after it is cited below and is not edited here.

No compiler, device, phone, fetch, or upstream message was used for this note. Symbol and API presence is source evidence only. A paired call, a Dock click, a shutter press, and a Galaxy session are not claimed.

## Baselines

| Tree | Commit | What it is |
| --- | --- | --- |
| This fork | `f97bd213` | Qt/Rust Android and Mac source used for the comparison. |
| Installed Mac | `8b48af3d` | Waypair Link 0.1.4 build 32, installed by the normal 31-to-32 update. Identity and Ocean theme retained. Receipt: `waypair-wave24-20261019/release/MAC32-POST-PUBLICATION.json`. |
| Published Android | `bc101cd2` | 0.1.4-local.4, version code 97, on the private R2 feed. Emulator fresh check said up to date. No Galaxy install is claimed. Receipt: `waypair-wave25-20261019/release/ANDROID97-POST-PUBLICATION.json`. |
| Unpublished Android build | scratch `android98-adb-only/` | Release assemble log ends `BUILD SUCCESSFUL`. `AdbKeyTest` recorded 3 tests, 0 failures. The APK is named `unpublished-adb-only.apk`. Not a feed update. A later final 98 can be rebuilt. |
| Old published Android | `67236218` | Link 0.46.2 selected-folder docs, on `alex.s/wp-folder-android-20261019`. |
| Old published Mac | `faf5dfea` | 0.46.2 build 26, on `alex.s/wp-folder-mac-20261019`. |
| Old feature tips | `b7cf6eea`, `2a42c764` | `origin/alex.s/wp-link-features-20261018` and `origin/alex.s/wp-mac-features-20261018`. Preserved on origin. Not merged. Worktrees stay deleted. |
| Cloud | `d7bfa09e` | Separate product. Not a Link source. |
| Upstream main | `e4b279a` | Local `upstream/main`. Its parent is `f8f72547`. |
| Upstream PR 24 | `9c0f13d7` | Local `upstream/pr/24`. Still the open macOS port. The fork already contains this commit. That is not an upstream merge. |

thtbee's comment on PR 24 (`issuecomment-6278184327`, 2026-10-19) welcomes small PRs, asks for macOS follow-ups after that PR merges, and asks for an Android issue before Android implementation. The unsupported Mac work named there is screen mirroring, H.264, PC audio streaming, and the tray. Those four are maintainer-planned. They are not tasks in the list below. This note creates no issue and no PR.

Original copyright, license, and authorship stay in the old repository and in this tree's Open-source Licenses. Nothing here is a license to paste the old Swift or Kotlin into the Qt app. A later port reimplements the cited behavior. The old Swift app does not run beside Waypair Link.

## Already in this fork

These are not next ports. Evidence is the class named, not a device pass.

- **Notification app and type filter.** `NotificationFilter.kt` is applied in `NotificationListener.mirrored` before the title or body is read. SMS permission is not consulted. There is no Gmail reader. Unit test exists; this note did not run it. No mirrored notification on a phone is recorded here.
- **In-app notification reply and dismiss.** `NotificationItem.qml` calls `NotificationList.reply`. Core `Node::run_notification_action` sends `NOTIFY_ACTION`. `PhonePlatform.runNotificationAction` delivers `RemoteInput` or the action intent. The Mac banner in `mac/toast.rs` `show` drops actions, reply, and progress. No paired reply is recorded.
- **Incoming file accept, decline, open, and reveal.** `TransferItem.qml` shows Accept and Decline while status is `requested`. Mac open uses `/usr/bin/open` and Finder `-R` (`058f5c88`). Command-construction tests passed on the merged product. No phone-to-Mac transfer is recorded.
- **Continuity capture with phone review.** Upstream `ContinuityCameraActivity` still requires the person on the phone to review and send. `request_camera_capture_now` does not queue (`5b1d40ba`, synthetic loopback, 4 camera tests recorded by Root). Native Services (`mac/services.rs`, `NSServices` in `package-app.sh`) are source after Mac 32. They are not in the installed build. Native QA is still pending. The old Link design did not paste into another Mac app, so Services are a fork slice, not a legacy port.
- **Status item.** `mac/tray.rs` at Mac 32 is Open and Quit only, with tag tests. Tray work is deferred. Do not add phone actions to it.
- **Find-phone volume policy.** `RingPolicy` / `Ringer` raise the alarm stream, restore it, and vibrate when Do Not Disturb is total silence. `RingPolicyTest` is pure logic. Nobody heard it in the evidence used here.
- **Battery, ping, and ring while the screen is locked.** `LOCKED_PHONE_RESOURCE_PLAN.md`: no keyguard gate on `device.battery`, `core.ping`, or `device.ring`. No milliamp-hour or packet capture. Home already shows `Offline ·` plus a relative time when `lastSeen > 0` (`HomePage.qml`).
- **Sensitive notification app-op, via existing ADB.** `NotificationListener.canReadSensitive` checks `android:receive_sensitive_notifications` on Android 15 and later. `InputServer` runs `appops set … RECEIVE_SENSITIVE_NOTIFICATIONS allow` through the existing elevated ADB path. Shizuku is not required. No Galaxy app-op check is recorded.
- **Mac UI copy already committed, not in the installed Mac.** Mica is hidden and a desktop `updateApp` chip says "Unavailable on this Mac" (`3eadd5bf`, `a8d8997c`). Those commits are in Android 97's history and in `HEAD`. They are not in Mac 32. `qmlformat` only. No visual pass. Generic Mac UI and tray stay held; do not start another UI slice from this note.
- **Private R2 updates.** Android 97 is published. Mac 32 is installed. Cloud 0.1.9 / code 10 is unchanged. Android 98 is built and unpublished.

`links.rs` `open_copied_link_on_phone` is the other direction: this Mac's clipboard opens on the phone. The same function is in PR 24 at `9c0f13d7` and in upstream main at `e4b279a`. `Platform::open_link` in `main.rs` calls `links::open_here`, which opens a phone-sent link immediately. Neither tree has `NSDockTile`, a browser-icon Dock tile, or a signed-Dock reopen check.

## Deferred, not one of the eight

Full-duplex Galaxy SIM audio through the Mac microphone and speakers stays the highest priority and stays unstarted. The old `CallAudioGate` commits on the Mac feature branch are marked non-shipping. This fork's `mac/bt_hfp.rs` reports call audio unavailable. PC audio streaming is also on the maintainer's after-merge list. A later proof needs an attended supported route. This note does not design that route.

## Eight later tasks

Order is usefulness of old Link behavior that this fork does not already have, minus anything PR 24 or upstream main already does, minus tray, mirroring, H.264, and PC audio. Branding files stay with the branding worker. Nothing below is implemented by this note.

### 1. Copied-URL Dock handoff, after branding

**Implementation gate verified 2026-10-19.** The current `clip.set` path writes the pasteboard and history and carries no original copy deadline or copied-link metadata. A desktop-only interception cannot reproduce the published Waypair consent/lifetime/privacy contract. The next slice must first define a separately negotiated NectarLink-native optional copied-link offer, sender-side metadata-before-read checks, authenticated pair/session admission, and default-off consent on both sides. It must leave ordinary clipboard behavior and older peers unchanged. This is a feature implementation dependency, not a reason to weaken sender verification or auto-open a URL. The official signed-Dock reopen APIs are present; no Dock feature code or runtime acceptance has landed. Evidence: `~/orca/tmp/waypair-wave25-20261019/DOCK-HANDOFF-IMPLEMENTATION.md`.

**Benefit.** A web link copied on the paired phone shows the Mac's default-browser icon, with a small phone badge, on the Waypair Link Dock tile. The page opens only when the owner clicks that tile. It does not open by itself and it is not written to the Mac pasteboard.

**Old behavior to reuse, from published `faf5dfea`, not from a new design.**

- Phone: `CopiedLinkRelayService` sends one `link.offer`. `CopiedLinkPolicy.acceptedUrl` allows only an absolute `http` or `https` URL, at most 8192 UTF-8 bytes, with a host, no userinfo, and a port only in 1…65535. Sensitive or no-sync clipboard metadata is `Plan.Reject` before the text is read. A clip older than 60 seconds is not sent. Original lifetime is 300000 ms and is never extended. OTP is a different wire type (`otp.copyCandidate` / `otp.candidate` in `PairingSetupModel`). A numeric code is not a copied-link offer because it is not an `http(s)` URL. There is no extra digit filter inside `isValidURL`.
- Mac store: `PreviewStore.receiveCopiedLink` holds the latest offer in RAM for the trusted pair id. It does not read the pair id from the wire body. Opt-in off, local lock, pause, inactive session, stale peer lease, session mismatch, or pair mismatch drops the receive. `dropCopiedLinkOffer` runs on opt-out, unpair, pair change, and successful open. A normal disconnect only resets the peer lease (`resetPeerCopiedLinkLease`). The accepted offer stays openable until its original deadline. Do not "fix" that to clear on disconnect.
- Dock: `CopiedLinkDockController` plus `DefaultBrowserLinkOpener`. The icon is `LSCopyDefaultHandlerForURLScheme` for `http` or `https`, then `NSWorkspace` icon of that app. If the handler is Waypair Link itself, or there is no handler, the tile is cleared. The browser icon is not bundled. `CopiedLinkDockPolicy.shouldOpenOnReopen` is true only when an offer is showing and the sender is a signed `com.apple.dock` (`kAEReopenApplication`, bundle id, `anchor apple`). Anything unverifiable fails closed. `applicationShouldHandleReopen` in `WaypairApp.swift` returns false when that open consumes the offer. `openCopiedLink` revalidates pair, consent, lock, expiry, and URL, then opens with `NSWorkspace` `addsToRecentItems = false`. Logs under `--qa-link-metadata` record flags only, never the URL, query, or fragment. The UI may show the host label from `CopiedLinkPolicy.displayHost`.

**Do not reuse as the arrival path.** `links::open_here` opens immediately. General `ClipboardReceived` handling in `desktop/app/src/clipboard.rs` presents the clip on this Mac. The old offer never touches the Mac pasteboard. PR 24 and main do not contain this Dock behavior, so it is not duplicate upstream work.

**Files for a later desktop worker, after branding settles.** Do not edit branding QML, `mark.rs`, `tray.rs`, the package script, or version files. Proposed new file: `desktop/app/src/mac/copied_link_dock.rs` (GPL-3.0-or-later, objc2, same narrow `unsafe` pattern as `mac/services.rs`). Wiring: a macOS-only branch where a phone text clip would otherwise be written locally, plus the app reopen hook. No second process and no Swift helper. The old code did this inside the Mac app. If suppressing one clip requires a core or wire change, stop. Do not add `link.offer`.

**Consent.** Separate from general clipboard sync, default off, matching `copiedLinksEnabled`.

**Risks.** A shared clipboard edit can change Windows. Keep the branch `cfg`'d to macOS. A wrong reopen check would open a page from an ordinary activation. Fail closed. Do not log the URL.

**Minimum evidence.** Pure tests only: URL accept and reject (userinfo, whitespace, bad port, non-web scheme), sensitive metadata rejected before a read, expiry purge, pair mismatch, and the reopen decision table. No pasteboard, no browser launch, no personal clipboard, no phone. A later Root pass can use a qualification bundle and a synthetic offer. This note does not ask for that yet.

**Fork or upstream.** Fork-only until a small Mac PR is possible after PR 24 merges. It is not the tray.

### 2. One chosen phone folder on the Mac

**Benefit.** The Mac lists and reads a single folder the person picked on the phone, and loses that access when the grant is revoked.

**Old source.** Published `67236218`: `PhoneFolderSelection` answers `folder.probe` without opening the picker, and `PhoneFolderBrowseController` / `FolderBrowseSessionAdapter` release the read grant on close or root change. The feature tip adds composed-root lifetime and grant-loss cleanup (`38b4773b`, `b499e819`). Those two commits are not in the published 0.46.2 tip. No paired browse is claimed from them.

**This fork.** `PhoneStorage.addSafFolder` already persists an `ACTION_OPEN_DOCUMENT_TREE` grant. `files.storage` still requires desktop `storage.mount`. The Mac matrix test leaves that feature locked as "update the desktop" even when the phone has `storage.read` and `storage.write`. `storage.mount` is the Windows mount, and it stays withheld. Do not announce it.

**Files.** A later Android/desktop slice: phone listing of one persisted SAF tree through the existing storage list/read calls, and a Mac page that does not depend on `storage.mount`. Do not copy `FolderBrowseProtocol.kt`.

**Risks.** All-files access is a wider grant than the old one-tree session. Do not make it the default. Grant loss must stop an in-flight listing.

**Minimum evidence.** A unit test with a temporary tree: list succeeds, then a revoked grant returns empty and does not retain names. No phone.

**Fork or upstream.** The missing Mac cap is a product choice. Android storage APIs already exist upstream. Propose the Mac browsing rule in an issue before any upstream Android change. A fork-only Mac page can proceed without that issue if it does not change the shared capability set.

### 3. Capture and selected-file consent that already exists, plus grant-loss cleanup

**Benefit.** A photo, scan, or selected file moves only after the person on the receiving side accepts, and a withdrawn folder grant cancels the offer.

**Already ported.** Receiver Accept and Decline. Phone-side capture review. Mac Open and Reveal as user actions, not an automatic open.

**Genuine remainder.** The Link feature branch abandons a capture when the storage-error screen closes (`274cba07`) and retires the request when the capability is withdrawn (`c69f3fc1`). Those commits are ahead of published `67236218`. This fork's continuity path cancels on user cancel and on Service deadlines. It does not have the old SAF grant-loss hook.

**Files.** `ContinuityCameraActivity` / `continuity_camera.rs` cancel path, and the SAF grant listener next to `PhoneStorage`. No new capture protocol.

**Risks.** Do not queue a Service capture. `request_camera_capture_now` must stay unqueued.

**Minimum evidence.** A unit test: grant removed or activity finished without Send produces a cancel and no file in the incoming directory. No camera and no phone.

**Fork or upstream.** Cancel-on-missing-grant is a reasonable small Android fix, but only after an issue, per the maintainer note. Until then it stays fork-only.

### 4. Notification actions: prove the in-app path, do not add a banner

**Benefit.** Reply and dismiss work only for an action the current phone notification still has.

**Already in source,** on both PR 24 and this fork, as cited above. The old notification-actions branch (`origin/alex.s/wp-notification-actions-20261018`, tip `ffc8472e`) has synthetic reply, dismiss, and withdrawal tests. Its own roadmap still left native and phone gates open. That is not a device pass.

**Do not build** a Mac banner with actions while tray and the other post-merge Mac integrations are held. `mac/toast.rs` staying banner-only is acceptable for this round.

**Minimum evidence, when a phone is authorized later.** One current notification that offers reply, one that does not, and one that is removed before the click. The removed key must fail. No message text in logs.

**Fork or upstream.** The in-app path is already upstream-shaped. A banner is a macOS follow-up after PR 24, in the same waiting list as the tray, not a fork feature to start now.

### 5. Keep SMS, the notification filter, and Gmail apart

**Benefit.** Turning off email notifications does not turn an inbox reader on or off, and the reverse.

**Old source.** `CodeSourceAccessSection` at `67236218` states that inbox readers keep their own opt-ins. `GmailAuthorizationAdapter` returns `NotConfigured` unless the Gradle property `waypairGmailApiConfigured` is set. This note did not find a default of true. Do not treat Gmail as a shipped reader.

**This fork.** The filter already ignores SMS permission and has no Gmail client. `PhoneSms` is the full SMS inbox behind `sms.read` / `sms.send`, which is a different feature from a code-only reader.

**Next change, fork-only copy, not a client.** If the notification filter screen can be read as a Gmail switch, add one sentence that Messages, Email, and Other do not read the SMS store or Gmail. Do not register an OAuth client. Do not port `GmailInboxClient`.

**Minimum evidence.** The sentence is visible in the filter page source. No account and no network.

**Fork or upstream.** Copy only. An Android behavior change waits for an issue.

### 6. Protected codes stay on the existing ADB app-op

**Benefit.** A one-time code inside a notification Android marks sensitive is not readable until that app-op is allowed, and the notification filter still runs first.

**Old source.** `protectedCodeStatus` in `HandoffSetupSteps.kt` is `READY` only when notification access is granted and companion association is ready. That association is a different mechanism from this fork's BLE companion (`PcCompanion`), which is a wake path.

**This fork.** Elevated ADB already sets `RECEIVE_SENSITIVE_NOTIFICATIONS`. Shizuku shows up in the old clipboard-probe files. It is not the supported route here.

**Next.** Do not add Shizuku, and do not add a second association dialog. When a phone is authorized, record whether the existing app-op command changes `canReadSensitive`, and whether a blocked filter still drops the notification before the body is read.

**Minimum evidence until then.** A unit test that `mirrored` returns null when the filter disallows the package, without reading the body. The filter test file is the place. This note did not run it.

**Fork or upstream.** The app-op path is already in the fork's elevated helper. Upstream Android changes still need an issue.

### 7. Find-phone: accepted is not the same as heard

**Benefit.** The Mac does not say the phone is ringing unless the phone has started the alarm, and a locked phone can still be found without unlocking private content.

**Old source.** Mac `7a1c3828` on the quality line distinguishes alarm acknowledgment from audible confirmation. Android `c9ee1546` / `eedc2fc8` on the feature branch play a bounded alarm and add locked-phone admission. Those commits are not ancestors of published `67236218`. The other line `794f6cd0` (0.45.8) has the direct alarm and is not an ancestor of `67236218` either. Both are preserved history, not one merged release.

**This fork.** `Ringer.startRingingNow` restores the volume if no player starts, which is the right failure. The UI proof that the desktop waits for that outcome is missing. Battery and ping already stay available while locked. Do not change the 1-second keepalive, the 2.5-second heartbeat, or the 8-second idle timeout. The slower cadence in `LOCKED_PHONE_RESOURCE_PLAN.md` stays unevaluated.

**Files.** The existing ring result from `Ringer` to the desktop find-phone status. No new opcode.

**Minimum evidence.** A pure test that a failed player does not report ringing. A locked-phone hearing test waits for an attended Galaxy session.

**Fork or upstream.** Honest status for an existing `device.ring` is a small fix. Android wording still needs an issue before it is offered upstream.

### 8. Settings backup and the old app stay out

**Benefit.** Link updates and pairings are not replaced by a Cloud backup import or a second desktop runtime.

**Old Cloud** `SettingsBackupEngine` at `d7bfa09e` is Cloud. Do not port it into Link.

**This fork** already updates from the private R2 feeds: Android 97 published, Mac 32 installed, Cloud 10 left as it is. Android 98's ADB-only build is unpublished. Branding of visible names, including the wireless-debugging host label (`bb64496f`), is a separate worker. Final 98 can be rebuilt from that source. This plan does not publish it.

**Fork or upstream.** R2, Sparkle, and feed files stay fork-only (`FORK_POLICY.md`).

## Not in these eight

Tray, menu-bar phone actions, screen mirroring, H.264, and PC audio streaming wait for the maintainer path after PR 24. Native capture Services wait for Root's QA of the source already in `HEAD`; they are not a restoration of old Link. Full-duplex call audio stays deferred. Shizuku is not a requirement. No worktree of the retired feature branches is recreated.
