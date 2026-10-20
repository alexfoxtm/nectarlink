# LINK2 Recording — wave33 child qualification (source only)

Child: `alex.s/codex-mirror-recording-wave33` (isolated child of `1ca0f264`).
Parent `1ca0f264` upstream merge/history preserved; no rebase, no primary-mirror edits.

## Owned files (this dispatch only)

- `desktop/app/src/mirror.rs` — added one bounded synthetic production integration
  fixture `mirror::mac_receive_fixture::production_sink_recording_writes_video_only_mp4`.
  No production logic changed; recording paths reused verbatim.
- `LINK2-RECORDING.md` — this report (new).

Explicitly **not** touched (reviewed, no actual fix needed):

- `desktop/app/src/mac/mp4.rs` — bounded streaming video-only `MirrorRecorder`
  from wave32 left as-is. No muxer reimplementation, no new component tests.
- `desktop/app/qml/MirrorWindow.qml` — recording UI (`toggleRecording`, `recording`,
  `recordingStartedMs`, Ctrl+R, elapsed timer) already complete; no QML change.

No Cursor primary-mirror files edited. No caps changed. No FFmpeg shipped.

## What the fixture proves (production path, video-only)

- Begin while live: asserts `phase == Showing` and `!is_recording` before begin,
  then inserts a disposable `MirrorRecorder` with the live `LAST_CONFIG` dims and
  `LAST_SPS_PPS` seed — the same dims/seed `toggle_recording` uses — without
  touching real `Downloads` or toasts.
- Consume auth H.264 through existing callbacks: `sink.config` + `sink.packet`
  (real `mirror::sink()` the core selects) drive both the VideoToolbox decode
  thread (`ffi::video_frame` → `latest_bitmap`) and `rec.on_packet` via
  `Sink::packet`. No fake `MirrorSink`, no FFI spoof.
- Stop/close writes valid MP4: same remove-from-`RECORDERS` + `finish` that
  `stop_recording` wraps in `spawn_blocking`, asserted synchronously. File is
  `ftyp`/`moov`/`mdat` with `avc1`/`avcC`, carries the live fixture SPS/PPS,
  has no `soun` track, `>256 B` and `<8 MB`.
- Refuses audio honestly: `sink.audio_config` accepted (API stability),
  `sink.audio` PCM refused by the recorder without poisoning; next video packet
  still records and the file stays video-only.
- Preserves ownership/collisions/current frames: disposable temp path starts
  free (`create_new` inside `MirrorRecorder` never truncates); sibling
  `owner.txt` untouched; live red frame still publishes after recording packets;
  `sink.ended()` clears the live frame but the finished MP4 survives; closed
  request owns no sink.

Native Open/Reveal: finished file is a regular existing path; production toast
uses `transfers::TOAST_GROUP == "files"` with `ACTION_OPEN`/`ACTION_SHOW`,
handled by `notifications::on_toast` → `transfers::open`/`show_in_folder`
(`/usr/bin/open`, `-R` reveal on macOS). No new action code; existing
`transfers` synthetic command tests remain the Open/Reveal proof.

## Bounds (pass limits)

- Synthetic only: committed `desktop/app/src/mac/testdata/red-128x128.annexb`
  solid-red baseline IDR. No capture, mic, personal phone, permissions, app
  bundles, clones, or network pairing (unlike the `test-support` transport test).
- Bounded: 4 IDR packets total (1 live + 3 recorded), `0–100_000 µs` span
  (`MAX_DURATION_US` is 3 h), 4 frames (`MAX_FRAMES` 150 k), file `<8 MB`
  (`MAX_MDAT_BYTES` 1.8 GB, `MAX_PACKET_BYTES`/`MAX_SAMPLE_BYTES` 8 MiB).
- Timeouts: `await_bitmap` 10 s, frame-clear poll 10 s, no unbounded loops.
- Isolation: `FIXTURE_LOCK` serializes with the two existing fixtures;
  distinct `DeviceId([0x52; 32])` and generation `0x9E3779B9_7F4A7C16`;
  tempdir caller-owned output; global `RECORDERS`/`ACTIVE_RECORDERS` restored
  on the success path.
- No FFmpeg dependency, no `ffprobe` in tree; box checks are byte-level.

## Commands for Root (sole compiler; child did not build apps)

Run on macOS (VideoToolbox + `ffi::video_frame` required):

```sh
cargo test -p nectarlink-desktop --lib mac::mp4
cargo test -p nectarlink-desktop --lib mirror::mac_receive_fixture::production_sink_recording_writes_video_only_mp4
cargo test -p nectarlink-desktop --lib mirror::mac_receive_fixture::mac_view_only_receive_shows_decoded_frame_and_cancels
```

Expected: 3 + 1 + 1 pass. No `--features test-support` needed for the two
`mac_receive_fixture` default tests; the `transport_to_video_frame` test keeps
its `test-support` gate and is not part of this qualification.

Child verification (no shared CXX target build): `rustfmt --edition 2024 --check
desktop/app/src/mirror.rs` shows only pre-existing diffs outside the new
fixture; new fixture itself is fmt-clean. No `cargo build/test` run in child.

## Non-goals / handoff

- Do not enable production caps (`mirror.view`/`mirror.listen`, etc.) from this
  source change; Root review + native proof required before any qualification.
- Cursor owns primary mirror integration; Root integrates this child preserving
  `1ca0f264` and later Cursor changes.
- Wave32 muxer + 3 `mac::mp4` tests + ffprobe/FFmpeg 2-frame decode remain the
  component proof; this fixture adds only the production-callback integration
  and does not duplicate those cases.
