# Wave 33 composed Mac mirror window gate

Date: 2026-10-20

## Status

Source only. The gate is written; nothing has been built or run. Root owns the build, the run and the native review. The shipping Mac still advertises no `mirror.*` capability; `compute_capabilities` and its macOS test are unchanged.

## Why a binary mode and not a `#[test]`

libtest runs every test on a spawned thread. AppKit refuses to create an `NSWindow` off the main thread, so a cargo test cannot open the real `MirrorWindow`. The gate therefore runs as a mode of the production binary on its main thread. That mode is compiled only on macOS with the non-default `test-support` feature, which a normal app build does not include. The coordinator approved the `main.rs` and `core_host.rs` additions below.

## Changes

- `desktop/app/src/main.rs`: under `cfg(all(target_os = "macos", feature = "test-support"))`, `--mirror-window-gate` returns `mirror::window_gate::run(--data-dir)`. This happens before setup, logging, single-instance, the core host, BLE, the tray and services.
- `desktop/app/src/core_host.rs`: `start_window_gate(config, platform)`, under the same cfg, installs the host around a caller-built receiver node and forwards that node's events only to `mirror::on_event`.
- `desktop/app/src/mirror.rs`: a new `window_gate` module under the same cfg. The existing composed test now uses the gate's `loopback_config` and `wait_for_link`, with identical settings, instead of its own copies. `rustfmt` also reflowed two existing lines of that test. No production path changed.
- `desktop/app/qml/MirrorWindow.qml`: unchanged. The gate instantiates this production component.

## What the gate does

1. **Fail-closed start.** The arguments must be exactly `--mirror-window-gate --data-dir <dir>`, and `QT_QPA_PLATFORM`, `QT_QUICK_BACKEND` and `QSG_RHI_BACKEND` must be unset. `<dir>` must meet all of these conditions:
   - it is canonical and absolute;
   - it sits at least two levels under `~/orca/tmp`, inside a `waypair-*` folder;
   - it is not a symlink, belongs to the owner of `~/orca/tmp`, and is not group or world writable;
   - it holds only a regular file `.mirror-window-gate` whose content is the directory's own path.

   Any failure exits 2 before anything starts.
2. **Two throwaway nodes.** A receiver (`Synthetic Mac`, capabilities `mirror.view` and `mirror.correlation`) and a phone (`Synthetic Phone`, capabilities `mirror.capture` and `mirror.correlation`) each use `enable_loopback_mirror_fixture`: bound to 127.0.0.1 with port mapping disabled. Relay, mDNS, ADB and USB are off, and each node gets a `PlainKeyProtector` key in `<dir>/receiver` or `<dir>/phone`. No keychain, user profile or app data is touched.
3. **Pairing.** Real QR pairing runs: the receiver's invitation must be loopback-only, then `pairing_join` and an online authenticated link are required. The secret stays in memory and is never printed. The gate logs at INFO, and core does not log URIs.
4. **The real UI on the main thread.** The gate sets up Qt as `--ui-check` does: Qt messages go to stderr and QML problems from `qrc:/` are counted. It then creates `QGuiApplication` and loads a small driver QML. The driver creates the production `MirrorWindow` for each key in `Mirror.windows`, as `App.qml` does, and destroys it when the key goes away. It adds only an invisible probe `Item` to read `GraphicsInfo.api`.
5. **Sequence.** Each step has a 20 s bound, the driver has a 150 s watchdog, and the process hard-exits at 180 s.

   | Grab | Step |
   | --- | --- |
   | `1-portrait.png` | `Mirror.start(phone)` opens the window. The phone answers the real `mirror.start` (screen only, video only, live token) with format 360x640 and the 128x128 red IDR. The window must reach phase `showing` with `VideoView.frames > 0`, frame 360x640 and a window aspect within 0.05 of the frame. |
   | `2-landscape.png` | Rotation. Once that grab exists, the phone sends 640x360 and the IDR on the same stream. The window must re-aspect to landscape. |
   | `3-ended.png` | Phone-side stop. Once that grab exists, the phone closes its stream. The same window must stay open with phase `ended` and an empty `VideoView` frame. |
   | `4-reopened.png` | `Mirror.reopen(key)`, the "Try again" action, reuses the same window object. A new request yields portrait again. |
   | none | `close()` on the window runs the real `onClosing`, which calls `Mirror.stop` and `mirror::stop`. The window is destroyed, and the stop must reach the phone through `mirror_stop`. |
   | `5-restarted.png` | After a 1 s dwell, `Mirror.start(phone)` creates a new window object. A third request yields landscape. Closing that window must also destroy it and stop the phone. |

   At every showing grab the driver requires the Qt platform `cocoa` and the graphics API `GraphicsInfo.Metal`; otherwise it exits 2.
6. **Rust checks after Qt exits.** Each PNG is decoded through the production `decode_image`:
   - Showing grabs must keep the frame's aspect within 0.06. Every pixel of the picture's central band must be red (R ≥ 230, G and B ≤ 24). The band spans 20–80% of the picture's width and 20–60% of its height, which keeps the toolbar and the control hint out.
   - The ended grab must contain no red pixel.
   - The phone must see exactly 3 starts and 2 stops, with formats `[(360,640), (640,360), (360,640), (640,360)]`.
   - The app's QML must produce 0 problems.
   - Both nodes must shut down within 5 s.
7. **Exit codes.** 0 means pass (PASS lifecycle, five PASS render lines and PASS phone on stdout). 1 means wrong state or pixels. 2 means a blocker: not Cocoa/Metal, a stuck step, a bad run directory or a node that did not start. On failure the driver closes its windows, which stops the mirroring, and the gate still shuts both nodes down.

**Coded frame versus rendered geometry.** Every picture is the committed 128x128 baseline IDR, `desktop/app/src/mac/testdata/red-128x128.annexb`. The production decoder scales it to the announced 360x640 or 640x360, nearest neighbour as for any phone format. Portrait and landscape here are announced formats, not a rotated encode.

## Root commands

Run in a GUI login session. The runner refuses if the owned files are uncommitted. Each attempt writes `R<n>/` and never overwrites:

```sh
python3 /Users/AleksandrSlobodanyuk/orca/tmp/waypair-wave33-20261020/link1-mirror-window/root-mirror-window-gate.py
```

It runs these commands with `CARGO_TARGET_DIR=/Users/AleksandrSlobodanyuk/orca/tmp/waypair-wave21-nectar-port-20261019/cargo-target`. The debug profile is used, so the release app build is untouched.

```sh
cargo build --locked -p nectarlink-desktop --features test-support --bin nectarlink-desktop --message-format=json-render-diagnostics
cargo test --locked -p nectarlink-desktop --features test-support --bin nectarlink-desktop mirror:: -- --test-threads=1
# fresh owner-only run directory with its sentinel, then:
<debug nectarlink-desktop> --mirror-window-gate --data-dir <R<n>/gate-run>
```

The test step must show these three tests passing, with 0 failed:

- the new run-directory guard test, `mirror::window_gate::tests::the_gate_runs_only_in_a_fresh_sentinel_scratch_directory`;
- the default-feature receiver fixture, `mirror::mac_receive_fixture::mac_view_only_receive_shows_decoded_frame_and_cancels`;
- R4's composed transport test, `mirror::mac_receive_fixture::transport_to_video_frame_uses_two_authenticated_nodes`.

`RESULT.json` records:

- the source SHA and the executable's sha256;
- each step's exit code and the gate's exit code;
- the PASS lines and the grab sha256s;
- whether the process ended;
- whether the logs contain a `nectarlink://pair` URI. The pairing secret would be in it, so this must be false.

Format check for these files: `rustfmt --edition 2024 --check desktop/app/src/mirror.rs desktop/app/src/core_host.rs` passes. `desktop/app/src/main.rs` is clean too; a crate-wide check still reports another worker's `phone_browser.rs`.

## Known side effects and limits

- `Theme.qml` binds `AppController`. Its initializer installs the production menu-bar status item ("W", with Open and Quit) for the life of the gate process. No notification permission is requested, and no hotkeys, login item or services are registered.
- `grabToImage` renders the window's own scene-graph content into an offscreen texture with Metal. It is not a desktop capture and not the swapchain; wave 32 separately proved the swapchain paint of `VideoView`. The macOS title bar is not included.
- `mirror.view` and `mirror.correlation` are granted to the synthetic receiver only by the gate. A pass is native window and lifecycle proof for synthetic frames. It does not prove phone capture, permission prompts, a real Galaxy, sound, input, or that the shipping Mac should advertise mirroring.
- The gate waits 1 s after a close before reopening. `mirror::on_event` ends any Asking or Showing screen window on `Mirroring { on: false }`, and core does not tag that event with a request. An old stream's end that lands after a near-instant reopen could therefore end the new request. The gate does not exercise this race; it is recorded here as a production observation, not changed.
- No build, test or run was performed by the worker. Only `rustfmt`, `qmllint` (syntax only, on an extracted copy) and `git diff --check` were run.
