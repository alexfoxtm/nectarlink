# Wave 24 — locked-phone idle cadence

Source: `docs/waypair/LOCKED_PHONE_RESOURCE_PLAN.md` at `f6fb9b208c3d`. Docs only so far. Do not edit the core in this wave's audit commit.

## Task

In `core/nectarlink-core`, drive QUIC keepalive and the session heartbeat from the negotiated idle timeout (the minimum of the two peers). Android and Mac share this crate. Do not slow only one side.

Against a peer that still advertises 8s (`IDLE_TIMEOUT` in `core/nectarlink-core/src/node.rs`):

- Keepalive 3s, heartbeat 6s, both strictly under 8s. A 15s or 45s period is illegal on that peer (`docs/protocol/v0.md` §7.1's 45s ping applies only after the negotiated idle timeout is longer than the send period).
- Skip a local `ping` when `Session::rx_since` shows the peer already sent this interval. Keep answering `ping` with `pong`. Do not change the `ping` / `pong` / `event.battery` / `device.ring` shapes.
- Keep 1s keepalive and 2.5s heartbeat (`HEARTBEAT_INTERVAL` in `session.rs`) while mirror, mic, webcam, or a transfer is open. Leave `StreamPower.kt` in place for that case.

Leave `core.ping`, `device.battery`, and `device.ring` offered at `PowerLevel::Basic` (`BASE_CAPABILITIES` in `node.rs`). Do not add a keyguard or app-lock gate. OS lock is not the sensitive-notification or clipboard flag.

Optional in the same heartbeat change: emit `LinkChanged` on path change, not on a 5ms `rtt_ms` wobble, while the idle profile is active (`publish_online` / `LinkState::Online`).

Out of this task: `ConnectionService` banking poll (800ms while the screen is not interactive) and the paired-lifetime multicast lock. See the plan. Do not drop `StreamPower` to chase battery.

## Acceptance

- Main: helper returns keepalive 3s and heartbeat 6s when negotiated idle is 8s, and `ring_and_battery_flow_between_devices` still passes.
- Failure: requested keepalive 15s with negotiated idle 8s is clamped under 8s, not returned as 15s.

No existing test pins 1000ms or 2500ms.
