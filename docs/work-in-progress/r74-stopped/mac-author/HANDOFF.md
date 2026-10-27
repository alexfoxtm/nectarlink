# R74 Mac transfer display dedup handoff

Alex, source-only author packet is ready against immutable source `563bb1784d4c8eec81b8ae6c87b62eb110685f16`.

## Contents

- `before/` and `after/`: only `desktop/app/src/transfers.rs` and `desktop/app/src/bridge/transfers.rs`.
- `mac-transfer-dedup.patch`: complete unified patch.
- `MANIFEST.json`: before/after SHA-256, patch SHA-256 and performed file-only checks.

No primary source, Git, SDK/compiler, Rust/JVM/Qt tests, app/OS/device, network, account, profile/key, version/changelog/roadmap, QML or vault work performed. Root owns all integration and checks. Immutable snapshot files remain byte-equal to before/.

## Behavior

A single pure `transfer_display_owner` decision is used by both existing projections. The records remain intact.

| Exact matching outgoing file transfer | Display owner |
| --- | --- |
| Queue Sending + live Running or Paused | Live (existing progress/ETA and pause/resume) |
| Queue Waiting, Failed or Expired | Queue (existing retry/cancel/TTL) |
| Queue Sending + live Waiting or terminal | Queue |
| Matching canonical queue absent | Live remains in the existing model |
| ID differs, peer differs, incoming, or non-file queue kind | Independent records |

No names, sizes or paths are identity keys. All queue counts and failed flags still reflect underlying records, even when a live row owns the display. This is steady-state projection dedup; Qt properties/list signals remain asynchronous and no atomic render transition is claimed. No per-member progress or capability enablement is introduced.

Two new pure tests are in existing transfers.rs tests: one transition sequence (Running/Paused, failed, retry Waiting, Sending, Done, Expired) and one identity isolation case (other IDs, other peers, incoming, non-file queue item). No source changes outside the two target files.

## Root verification commands (not run by author)

Run in Root's authorized disposable qualification tree after applying both files:

```sh
cargo test --locked --offline -p nectarlink-desktop transfers::tests::queued_file_display_preserves_live_controls_and_queue_retry_ownership -- --exact
cargo test --locked --offline -p nectarlink-desktop transfers::tests::transfer_display_never_merges_other_ids_peers_incoming_or_non_file_items -- --exact
cargo test --locked --offline -p nectarlink-desktop state::tests::transfer_events_update_matching_send_queue_item_in_place -- --exact
cargo test --locked --offline -p nectarlink-desktop bridge::transfers::tests::roles_describe_the_transfer -- --exact
cargo test --locked --offline -p nectarlink-desktop transfers::tests::
```

Use Root's established formatting/strict compilation and actual Qt qualification commands as appropriate. Existing `tst_transferitem.qml` remains relevant to retained aggregate percent/ETA. Native actions/rendering, signed artifact/install and physical peer acceptance remain unperformed and separate.
