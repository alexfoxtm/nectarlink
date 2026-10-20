# Mirror Android next: producer lifecycle (generation fencing)

Owner: Android worker (`task_f94044173bd8`). Owned sources only:
`android/.../mirror/MirrorService.kt`, `android/.../mirror/MirrorSessionPolicy.kt` (new),
`android/.../mirror/MirrorSessionPolicyTest.kt` (new). No desktop, core,
protocol, permission, or version changes.

## Context notes

- `wave32/MIRROR-SOURCE-QUALIFICATION.json` and `wave32/MIRROR-NEXT-SOURCE.md`
  do not exist anywhere in this checkout (checked worktree plus all refs).
  Work proceeded from the actual producer code at `acb74fbb`, which already
  propagates the negotiated `request` generation into every producer config
  packet (HEAD's F1/F2 work, not repeated here).
- `desktop/app/src/...` entries visible in the worktree diff belong to the
  Mac worker / root and are untouched by this change.

## Concrete defect found (actual producer code)

`MirrorService.onStartCommand` treated *any* in-flight capture as busy and
silently dropped the incoming start (`MirrorService.kt` pre-change guard).
A Mac reconnect mints a **new** `mirror.start` generation, so the phone kept
streaming the stale generation while the Mac waited for a stream stamped
with the new one (which core `mirror_token_accepted` is correct to refuse):
reconnect could never preempt, yielding a black mirror until manual restart.

Worse, stale async capture jobs could outlive and undo a replacement:

- `startElevatedStream` ran to completion unchecked: a late socket
  overwrote the replacement's `elevatedSocket`, and its no-config fallback
  called `stopSharing()` + `MirrorRequests.show(stale request)`, reopening a
  superseded consent on top of the live session.
- `scope.launch { core.mirrorOpen ... }` failure and `ScreenEncoder.onEnded`
  called `stopSharing()` unconditionally, so a stale job's failure tore down
  the replacement session.
- `stopSharing()` never cleared `elevatedStarting`, so stopping mid-start
  left the flag set and the late completion resurrected streaming.

## Fix: trace the generation through the producer lifecycle

- New pure-Kotlin `MirrorSessionPolicy.shouldPreempt(activePc,
  activeRequest, incomingPc, incomingRequest)`: only a strictly newer
  generation from the same PC preempts. Duplicates, reordered older
  generations, other PCs, and unstamped requests against a correlated session
  keep the live capture; a correlated request supersedes a legacy capture.
- `MirrorService` tracks the live generation in `activeRequest`
  (`MirrorService.kt:60`), set synchronously in both start branches
  (`:105`, `:134`) and cleared in the extracted `tearDownCapture()` (`:390`).
- A preempting start calls `tearDownCapture()` first (`:85-90`): capture
  resources retire, the still-starting flag clears, and the replacement's
  consent/notification proceeds on the same service instance (no new
  permissions; consent/foreground-service gates unchanged).
- Every async job captures its own `(pc, request)` and re-checks liveness on
  the main thread before touching shared state; stale jobs end only their
  own stream/socket and never stop the service or re-post consent:
  non-elevated open/attach/`onEnded` (`:149-172`), elevated open/fallback/
  display-watch (`:186-218`), `streamElevated` open-failure and
  end-of-stream fallback (`:266`, `:305-317`), sound attach (`:338-356`,
  signature now takes the generation).
- Rotation behavior unchanged: `ScreenEncoder` still resends config with the
  request token on every resize loop; the elevated display watcher is now
  only registered by the live job.

## Tests

- Existing tests insufficient for the decision logic (only
  `WindowShapeTest` covers the mirror package), so one new JVM unit test:
  `MirrorSessionPolicyTest` — newer-generation preemption plus
  duplicate/older/foreign/legacy cases. No Android framework needed
  (policy object is pure Kotlin).
- No new permissions, no mic/audio/input/capture invocation or devices
  touched; `stop`/`keyframe`/`setPower` entry points unchanged (core already
  filters stale stops via `mirror_stop_applies` before they reach the
  platform).

## Root acceptance (root is sole compiler; not built here)

1. `cd android && ./gradlew :app:testDebugUnitTest --tests "app.nectarlink.android.mirror.*"`
   (expects `MirrorSessionPolicyTest` + `WindowShapeTest` green).
2. Full Android compile per repo flow (`./gradlew :app:assembleDebug` or the
   project's usual flavor) to qualify `MirrorService.kt`.
3. Manual interop when a Mac build is available: start view-only mirror from
   Mac, reconnect the Mac (new generation), confirm the phone swaps to the
   new capture with no stuck notification and no second consent for the old
   request; stop mid-"starting" (Elevated) and confirm no resurrection.
