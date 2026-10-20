# Android background connection — upstream port (wave33)

Port of upstream NectarLink **PR #22** ("Android: let users turn off the
background connection") onto `codex/feature-wave32`.

- Upstream commit: `a41502b2` (Abinanthan-CG, `Signed-off-by: abinanthan-cg`),
  plus changelog `5eab2dc6`. Both already fetched under `remotes/upstream/main`.
- Base behavior on this branch is unchanged: the toggle **defaults to ON**,
  so a fresh install keeps the always-connected behavior.
- Authorship stays upstream; this port only adapts. See the commit message
  for the full provenance trailer.

## What it does

`Settings › Background & battery › Run in the background`, plus a **Turn off**
action on the connection notification. While off, the core connects only while
the app is open: leaving the screen stops it ~5 s later, coming back starts it
again, and Home shows a card to turn it back on. A stop waits while the phone
is mirroring, transferring a file, or streaming the webcam/mic. Starts never
cancel each other; only a stop cancels an in-flight start.

## Files (Android only)

| Area | Change |
| --- | --- |
| `core/StartGuard.kt` (new) | Verbatim upstream: stop counting so in-flight starts detect a later stop |
| `core/Core.kt` | Per-run scope (`runJob`/`runScope` + `lifecycleMutex`), `busy` flow (transfers, send queue, mirror, webcam, mic), `stop()` + `shutdownAndCloseNode()`, lifecycle-guarded `enqueue*Op()` queueing, `withDevicesOffline()` on stop |
| `core/CoreState.kt` | `withDevicesOffline()` (verbatim upstream) |
| `core/SystemMonitors.kt` | `BatteryMonitor.stop()` clears `last` (verbatim upstream) |
| `NectarlinkApplication.kt` | Foreground tracking, conditional auto-start, delayed busy-aware `scheduleStop()` |
| `ui/Preferences.kt` | `background_service_enabled` (default `true`) with observable flow + static helpers |
| `service/ConnectionService.kt` | `ACTION_STOP_SERVICE` (Turn off), `start()` guard, notification cancel on destroy |
| `service/StartupReceiver.kt` | No auto-start on boot/update while off (update notice still shown) |
| `companion/PcCompanionService.kt` | No BLE wake while off |
| `ui/MainActivity.kt` | Wires the pref into Home + Settings |
| `ui/home/HomeScreen.kt` | "Background connection is off" card; `SetupCard` gains a defaulted `buttonLabel` |
| `ui/settings/SettingsScreen.kt` | "Run in the background" switch in Background & battery |
| `res/values{,-zh-rCN}/strings.xml` | 6 new strings (Waypair branding: "Waypair Link", "computer") |
| `core/CoreStateTest.kt`, `core/StartGuardTest.kt` (new) | Verbatim upstream tests |

## Deliberate deviations from upstream

- **No new dependency.** Upstream adds `lifecycle-process` (`ProcessLifecycleOwner`)
  via `android/app/build.gradle.kts` + `android/gradle/libs.versions.toml`.
  Build files are outside this task's ownership, so foreground tracking uses
  `ActivityLifecycleCallbacks` (started-activity counter) instead. Same
  observable behavior (start on first start, stop 5 s after last stop when
  eligible), no manifest/permission changes.
- **`Elevated.stop()` is `suspend` on this branch** (wave32 power work), so
  `stop()` uses `try/catch` instead of upstream's `runCatching {}` (whose
  block is not suspend).
- **Strings rebranded** to Waypair Link / computer per fork convention;
  clipboard-offer privacy, Chrome-dock and actual-WiFi-route display code is
  untouched.

## Not touched (other owners)

`mirror/MirrorService.kt` (only read via `session`), Rust `core/`,
`nectarlink-ffi`, `Cargo.*`, build versions, `AppUpdater`, signers, generated
bindings/notices. `MirrorService.session`, `WebcamService.session`,
`MicService.streaming` and `FfiQueueStatus` APIs used by `busy` were verified
present without modification.

## Validation (no builds, per task)

- `xml.dom` parse of both `strings.xml` files: OK.
- Hand-written tokenizer balance check (`{}`/`()`/`[]` outside
  strings/comments) on all 14 touched/new Kotlin files: OK.
- Line-by-line diff review against `a41502b2`: all 18 upstream hunks
  represented except the two build files (deviation above) and the manifest
  (upstream needs none).
- Unit tests (`StartGuardTest`, `CoreStateTest` addition) added but **not
  executed**: running them needs the Android Gradle build, which is out of
  scope (Root is the sole heavy compiler; no app builds on this task).
  Upstream tests are pure-JVM/JUnit + UniFFI model classes; CI should run
  `:app:testDebugUnitTest --tests "app.nectarlink.android.core.*"`.
