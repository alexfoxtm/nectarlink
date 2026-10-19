# Locked phone: keep battery, ping, and find cheap

Audit of `f6fb9b208c3d` (Waypair Link on the Nectar core). Source timers and a packet budget only. No measured mAh.

A locked Galaxy at `PowerLevel::Basic` already offers `core.ping`, `device.battery`, and `device.ring`. Nothing in this tree turns those off when the OS keyguard is up. The idle cost is that both peers keep a 1-second QUIC keepalive and a 2.5-second app heartbeat under an 8-second idle timeout, including when nobody is mirroring.

## What stays available

`BASE_CAPABILITIES` always includes `core.ping`, `device.battery`, and `device.ring` (`core/nectarlink-core/src/node.rs`). Android starts at `PowerLevel.BASIC` and leaves Basic unless Elevated or Assist is running (`android/.../core/Core.kt` `powerLevel()`). `device.ring` is also in the Basic phone matrix (`core/nectarlink-core/src/features.rs`).

| Need | While the OS screen is locked | Gate |
|---|---|---|
| Battery | `BatteryMonitor` listens for `ACTION_BATTERY_CHANGED` and emits only when the `Battery` value changes. `Node::update_battery` broadcasts `event.battery` on change. Not a timer. | No keyguard check. Signal and toggles start only while some peer is online (`Core.kt`). |
| Ping | Session task answers `ping` with `pong` (`session.rs` `handle`). `core.ping` is always offered. | No keyguard check. |
| Find | `device.ring` runs on a blocking task and calls `Ringer` (`USAGE_ALARM`, 60s cap, `RingPolicy`). | No keyguard check. Total-silence DND still blocks alarm audio; that is Zen policy, not the lock screen. |

There is no `KeyguardManager` use and no in-app lock. The only OS-screen read on this path is `BankingPause.Foreground.poll`, which returns null when `PowerManager.isInteractive` is false. That is not a privacy switch. Sensitive notifications and sensitive clipboard stay on their own flags (`notify.sensitive`, clipboard sensitive). Do not treat the Galaxy keyguard as an app lock, and do not drop Basic battery, ping, or find because the panel is off.

## Idle timers (from source)

Shared by the phone and the Mac. Both spawn `heartbeat` on every session.

| Timer | Value | Where |
|---|---|---|
| QUIC keepalive | 1s | `KEEP_ALIVE_INTERVAL` in `node.rs`, passed to `keep_alive_interval` |
| QUIC idle timeout | 8s | `IDLE_TIMEOUT` in `node.rs`, `max_idle_timeout` |
| App heartbeat | 2.5s | `HEARTBEAT_INTERVAL` in `session.rs` |
| Heartbeat RPC timeout | 3s, then one retry of 1.5s | `HEARTBEAT_TIMEOUT`, `HEARTBEAT_RETRY_TIMEOUT` |
| Protocol text | Ping after 45s with no traffic | `docs/protocol/v0.md` §7.1. The code does not do this. |

QUIC's effective idle timeout is the minimum of the two peers' advertised values. A build that stops sending for 8s is closed by a peer that still advertises 8s. Slowing only the phone does not help: the Mac's 1s keepalive still wakes the phone to ACK, and if the phone stops answering, the Mac closes at 8s. `StreamPower.kt` already documents that failure for mirroring.

Other timers, not the radio budget:

- Reconnect backoff 1s–30s, dial defer 750ms, USB dial grace 6s (`node.rs` `maintain`). Idle while a session is up.
- Mac `pc_active`: sleep 15s, input window 30s, resend at most every 30s while active (`desktop/app/src/core_host.rs`, `mac/input.rs` via the `win` path alias). One small toggle, not the 1s path.
- FGS update check every 6h. Banking poll 800ms when pause-for-banking is configured, else 15s (`ConnectionService.kt`).
- `StreamPower` partial wake lock and `WIFI_MODE_FULL_HIGH_PERF` only while screen or app streaming is on. 4h acquire, renewed on each `set`. Released in `MirrorService.stopSharing` (also `onDestroy`). Not held for a locked idle phone.

Worst-case app dead-peer close is about one heartbeat period plus 3s plus 1.5s (7s). The 8s QUIC idle timeout is the backstop. Those two bounds are why a one-sided slowdown drops the link.

## Packet budget, one idle session, both peers on this core

Not a capture. Counts follow the timers.

Each 2.5s, each side sends its own `ping` and the `pong` for the peer's `ping`: 4 control frames / 2.5s, about 96 frames/min. A control frame is a QUIC packet, so it also refreshes the idle timer.

`keep_alive_interval` is 1s and the heartbeat period is 2.5s, so each side still emits a QUIC keepalive in the gap. Order of magnitude: up to two keepalive datagrams per second, minus the seconds covered by `ping`/`pong`. Together, roughly two to four UDP datagrams per second (on the order of 150–200 packets/min) for a locked phone that is only sitting on the link.

`publish_online` runs when RTT moves by 5ms or more. `LinkState::Online` includes `rtt_ms` and is compared with `PartialEq`, so each such sample emits `LinkChanged`. That is extra fan-out tied to the 2.5s ping, not extra radio packets.

`event.battery` is not in this budget. It fires on change.

## What is actually waste

Definite:

- Bidirectional 2.5s `ping` on top of a 1s QUIC keepalive. The keepalive already resets an 8s idle timer. The protocol's own rule is a ping after 45s of silence. Both directions are redundant: `Session::rx_since` can see that the peer already delivered traffic.
- The 5ms RTT gate republishes `LinkChanged` for jitter that does not mean a new path.
- When banking pause is configured, `ConnectionService` still `delay`s 800ms while `poll()` has already returned because the screen is not interactive. The screen-off check avoids usage-stat work and does not avoid the wakeup.

Not a leak:

- `StreamPower` is not held on the idle locked path, and mirroring releases it in `stopSharing`.
- The discovery `MulticastLock` is held for the whole paired foreground service and released in `ConnectionService.onDestroy`. It is not an orphaned lock. Leave it; dropping it while locked is a discovery change, not this task.

Do not remove `StreamPower` during an active mirror with the panel off. That lock is what keeps the 8s timeout from flapping.

## One change

Do this in `nectarlink-core`, which both the Galaxy and the Mac run. Do not change Android timers alone.

Pick the send period from the negotiated idle timeout (minimum of the two advertisements), and never send slower than that timeout:

- If the negotiated idle timeout is still 8s, set QUIC keepalive to 3s and the app heartbeat to 6s. 3s survives one lost keepalive inside 8s (send at 0s and 3s; a single loss still delivers the next before 8s). 6s stays under 8s if a keepalive is skipped. Do not use 15s, and do not use the spec's 45s, against an 8s peer.
- Skip this side's `ping` when `rx_since` shows the peer already sent traffic in the interval. Still answer the peer's `ping`. `ping` / `pong` bodies stay as in `docs/protocol/v0.md` §7.1.
- While a mirror, mic, webcam, or transfer is open, keep today's 1s / 2.5s. `StreamPower` stays as it is.
- A later build may advertise a larger `max_idle_timeout` (for example 30s) only as a QUIC transport parameter. The effective timeout stays `min(local, peer)`. An old 8s peer keeps the 3s / 6s floor. No new app message.

Leave `device.battery`, `core.ping`, and `device.ring` on at Basic with the screen locked. Do not add a keyguard gate.

Rough budget after the floor, still against an 8s peer: keepalive about every 3s per side, and one `ping`/`pong` pair about every 6s instead of four control frames every 2.5s. That is well under one datagram per second average, and the peer's 8s timer still sees traffic. This is a source budget, not a mAh claim.

Separate and smaller, not a substitute: in `ConnectionService`, when `poll()` sees a non-interactive screen, sleep `BANKING_IDLE_MS` (15s) instead of `BANKING_POLL_MS` (800ms). That does not touch the QUIC timeout.

Secondary, same heartbeat edit if it stays small: publish `LinkChanged` when the path changes, not when `rtt_ms` alone moves by 5ms during this idle profile.

## Tests already here

No test asserts 1s or 2.5s. `session.rs` tests cover LAN address checks only. `ring_and_battery_flow_between_devices` in `core/nectarlink-core/tests/two_devices.rs` shows ring and `event.battery` once a session exists. `RingPolicyTest` covers alarm volume and Zen, not the lock screen.

Main acceptance, when someone implements the helper: negotiated idle 8s selects keepalive 3s and heartbeat 6s, both strictly under 8s, and `ring_and_battery_flow_between_devices` still passes with no keyguard gate added.

Failure acceptance: a requested keepalive of 15s with negotiated idle 8s is clamped under 8s and is not returned as 15s.
