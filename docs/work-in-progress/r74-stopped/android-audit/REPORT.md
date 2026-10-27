# Alex — Android Link R74 source gap audit

Source: immutable `../link-source`, snapshot pin `563bb1784d4c8eec81b8ae6c87b62eb110685f16`. Roadmap: immutable `../ROADMAP-BASELINE.json` (44 existing cards). Read-only audit; no Git, compiler/JVM/SDK, app, ADB, phone, network, pairing, account, permission, call, capture, or playback operation occurred. Only this report directory was written.

## Recommendation

Select the `folder-browser` / `consent-interop` card for a real Android source increment: support same-name moves between existing directories inside one explicitly granted SAF tree. This fills an explicit implementation hole without altering shared core, consent, routes, capability flags, Apps, mirror, webcam, or any previous UI work. Keep cross-tree moves and simultaneous move+rename unsupported in the first bounded increment. Root must confirm the Mac client path exposes the operation before claiming an end-to-end feature; the Android source gap itself is definite.

## Ranked real gaps

1. **Move files inside a selected phone folder.** `android/app/src/main/kotlin/app/nectarlink/android/storage/PhoneStorage.kt:523-535` allows SAF rename only when source and destination have the same parent. The explicit comparison at 529-530 rejects a move from `Shared/Documents/a.txt` to `Shared/Archive/a.txt` even when both directories are inside the same granted tree. All-files mode already implements moving (`rename`, 177-209); the Android Platform already forwards the general operation (`core/PhonePlatform.kt:354`); the protocol promises rename or move (`docs/protocol/storage.md:82-84`). This is missing implementation, not an unperformed device gate.

   Bounded behavior: retain the existing same-parent rename; for same-tree/same-basename moves resolve both existing parent directories (require directories, no implicit target-folder creation), reject destination collision and moving a directory beneath itself, verify current read/write grants and provider move support, perform one provider-native move and accept only its actual result. Unsupported/refused providers must fail honestly; no copy/delete fallback and no request for broader storage permission. Verify the offline SDK contract during Root qualification before choosing the exact provider API call. Ownership: `PhoneStorage.kt` and its existing test file only; no shared core or new endpoint.

2. **Automatic refresh for an already-open SAF folder.** `PhoneStorage.kt:54-58` registers a `FileObserver` only in all-files mode. The SAF branch at 81-83 calls `listSaf`; `listSaf` at 408-424 only returns a list. `observers` at 47 and `stopWatching`/`watchDirectory` at 239-270 contain only `FileObserver`, with no `ContentObserver` for tree documents. Consequently selected-folder changes have no Android invalidation producer, while protocol 93-96 explicitly describes `storage.changed`. Core already routes the callback through `storageChanged(path)` (`core/Core.kt:230-232`). This is a source gap in live browse behavior; concrete provider notification support remains provider dependent.

   Bounded behavior: watch only directories actually listed under currently granted SAF trees; map provider notifications back to that exact relative directory; cap subscriptions as current all-files watchers do; unregister on Stop, grant removal, provider refusal, and ownership retirement. Late callbacks after unregister must do nothing. Root needs a deliberate lifecycle adjustment because `Core.kt:974` currently calls `storage.stopWatching()` whenever all-files access is absent; do not add a watcher in `listSaf` and leave that unconditional teardown unchanged. Ownership initially `PhoneStorage.kt`, narrowly scoped `Core.kt` lifecycle glue, and tests. This is a slightly larger increment than #1.

## Present implementation, awaiting acceptance

- Ordinary independent Apps are implemented, not Settings-only: `mirror/AppWindows.kt:227-231`, 260-373 open the selected package on an owned Elevated display and stream through the original Node/capture context. `input` at 497-521 retains full ordinary app input and scroll-only legacy Settings qualification. Do not recreate Apps or broaden grant admission to fill a roadmap checkbox.
- Existing lifecycle fences include exact window map identity + `current()` (376-377), admission work/retirement (94-157), app packet-pump owned cleanup (289-295), and guarded resize/input; the roadmap's physical scroll/fold/reopen/revoke matrix is acceptance work, not evidence that these features lack code.
- Optional background connection has real preference-controlled boot/update start (`service/StartupReceiver.kt:24-40`), Start/Stop generation guard (`core/StartGuard.kt`), and OEM settings guidance (`core/BackgroundAccess.kt`). Replacing it would duplicate implemented work.
- Phone media publishing/commands are present (`media/PhoneMedia.kt`); Photos list/open/full vs selected-access gates are present (`photos/RecentPhotos.kt:145-182`, 242-342). The photo-backup roadmap card is already source/test/build/published done; its Galaxy backup gate is unperformed. No backup rewrite is justified by this source audit.
- Per-peer Disconnect/Reconnect is an explicit planned source gap in `next-link-five`, but its roadmap requires shared-core/store/session registration policy and is unsuitable for this Android-only ownership lane.

## Root validation plan (not run here)

Existing command from disposable source-copy `android/`: `./gradlew --offline --no-daemon :app:testDebugUnitTest --tests app.nectarlink.android.storage.PhoneStorageTest`, then related `:app:testDebugUnitTest` and `:app:lintDebug`. Respect Root's established generated-bindings/cache recipe; do not invoke a fresh Rust/SDK build merely to run the focused checks.

Existing test coverage is only pure relative-path/canonical-root checks (`storage/PhoneStorageTest.kt:13-137`); it does not exercise a SAF provider. Add at most one main and one failure test for changed behavior. For #1, make the production move decision/dispatch callable with local fake document operations (small seam only if needed): main verifies exact original grant/tree and one move with no copy/delete; failure verifies revoked/unsupported/colliding target invokes no mutation and does not expand access. Preserve the same-parent rename existing behavior. A source-only unit test cannot prove a real provider implements native move. Real current provider + original phone consent + normal Mac browse/refresh/move acceptance remain separate, and browser flags remain unchanged.

## Audit exclusions

Calls stayed read-only; no call endpoints/routes/roles/grants/probes. Original Stop/decline/new-phone approval, CAMERA permission, real CameraX SurfaceRequest and true 2160 admission were untouched. No neutral/colorful icon or Inter/Compose changes; no vendor assets. No source was edited, and no broad feature/release/device acceptance is claimed.
