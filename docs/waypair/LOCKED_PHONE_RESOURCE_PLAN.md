# Locked phone: keep battery, ping, and find cheap

Audit of `f6fb9b208c3d` (Waypair Link on the Nectar core). Source timers and a control-frame count only. Packet counts and power are unknown until a capture. No measured mAh.

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

QUIC's effective idle timeout is the minimum of the two peers' advertised values. A peer that still advertises 8s can close a session that goes quiet for 8s. Changing only one side's send period does not change the other side's advertised timeout. `StreamPower.kt` holds a partial wake lock during mirroring so that 8s idle timeout does not flap. That lock is not evidence about idle-locked packet cost.

Other timers, not the radio budget:

- Reconnect backoff 1s–30s, dial defer 750ms, USB dial grace 6s (`node.rs` `maintain`). Idle while a session is up.
- Mac `pc_active`: sleep 15s, input window 30s, resend at most every 30s while active (`desktop/app/src/core_host.rs`, `mac/input.rs` via the `win` path alias). One small toggle, not the 1s path.
- FGS update check every 6h. Banking poll 800ms when pause-for-banking is configured, else 15s (`ConnectionService.kt`).
- `StreamPower` partial wake lock and `WIFI_MODE_FULL_HIGH_PERF` only while screen or app streaming is on. 4h acquire, renewed on each `set`. Released in `MirrorService.stopSharing` (also `onDestroy`). Not held for a locked idle phone.

Worst-case app dead-peer close is about one heartbeat period plus 3s plus 1.5s (7s). The 8s QUIC idle timeout is the backstop. Those two bounds are why a one-sided slowdown drops the link.

## Control-frame count, one idle session, both peers on this core

Not a capture. The count follows the heartbeat timers only.

Each 2.5s, each side sends its own `ping` and the `pong` for the peer's `ping`: 4 control frames / 2.5s, about 96 control frames/min. One app control frame is not one UDP datagram. QUIC may coalesce frames, piggyback ACKs, and batch. Keepalive datagrams are a separate timer (`KEEP_ALIVE_INTERVAL`, 1s) and are not added into this 96. Actual UDP packets per minute, and any battery effect, are unknown until a capture.

`publish_online` runs when RTT moves by 5ms or more. `LinkState::Online` includes `rtt_ms` and is compared with `PartialEq`, so each such sample emits `LinkChanged`. That is UI fan-out tied to the 2.5s ping. It is not a packet count.

`event.battery` is not in this count. It fires on change.

## What is actually waste

Definite:

- Bidirectional 2.5s `ping` on top of a 1s QUIC keepalive. The keepalive already resets an 8s idle timer. The protocol's own rule is a ping after 45s of silence. Both directions are redundant: `Session::rx_since` can see that the peer already delivered traffic.
- The 5ms RTT gate republishes `LinkChanged` for jitter that does not mean a new path.
- When banking pause is configured, `ConnectionService` still `delay`s 800ms while `poll()` has already returned because the screen is not interactive. The screen-off check avoids usage-stat work and does not avoid the wakeup.

Not a leak:

- `StreamPower` is not held on the idle locked path, and mirroring releases it in `stopSharing`.
- The discovery `MulticastLock` is held for the whole paired foreground service and released in `ConnectionService.onDestroy`. It is not an orphaned lock. Leave it; dropping it while locked is a discovery change, not this task.

Do not remove `StreamPower` during an active mirror with the panel off. That lock is what keeps the 8s timeout from flapping.

## Unevaluated candidate (do not implement from this note)

A slower shared cadence — 3s QUIC keepalive and 6s app heartbeat while the negotiated idle timeout is still 8s — is an unevaluated candidate, not a selected design. Both peers run `nectarlink-core`, so any later edit has to land in that crate rather than in Android alone. Sitting strictly under 8s is not proof the candidate is safe. A lost datagram, jitter, or an old peer that still advertises 8s can still close the session. The protocol's 45s ping text does not override that peer timeout.

Before anyone changes a timer, confirm all of the following. This audit did not:

- The transport API can actually change keepalive and idle timeout, and whether that is per live connection or only when the endpoint is bound.
- Behavior under jitter and loss, not only the nominal period.
- Interop with a peer that still advertises the current 8s timeout.

Leave `device.battery`, `core.ping`, and `device.ring` on at Basic with the screen locked. Do not add a keyguard gate. No timer change belongs in the correction that records this candidate.

Separate and smaller, and not a substitute for a measured cadence: in `ConnectionService`, when `poll()` sees a non-interactive screen, the loop still waits 800ms (`BANKING_POLL_MS`) even though the poll already returned. `BANKING_IDLE_MS` is 15s. That observation does not touch the QUIC timeout.

## Tests already here

No test asserts 1s or 2.5s. `session.rs` tests cover LAN address checks only. `ring_and_battery_flow_between_devices` in `core/nectarlink-core/tests/two_devices.rs` shows ring and `event.battery` once a session exists. `RingPolicyTest` covers alarm volume and Zen, not the lock screen.

There is no acceptance test for the 3s/6s candidate. A helper that only checks "period < 8s" would not prove it.
