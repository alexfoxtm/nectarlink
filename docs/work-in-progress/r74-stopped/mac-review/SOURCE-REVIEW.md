# R74 Mac transfer display dedup — independent source review

Alex, **GO — source-only** for the exact author packet identified below. No actionable correctness, safety, or compatibility defect was found in the reviewed change. This verdict permits Root to proceed to integration/qualification; it is not compiler, test, native-rendering, signed-artifact, installation, or device acceptance.

## Pins and file-only verification

- Immutable base: `563bb1784d4c8eec81b8ae6c87b62eb110685f16`, from `../link-source/SOURCE-HEAD.txt` and `../link-snapshot.json`.
- `../link-source/SOURCE-MANIFEST.json` has **1,523 source entries**, independently SHA-256 verified with zero mismatches. The directory has 1,525 regular files including its two snapshot metadata files (`SOURCE-HEAD.txt`, `SOURCE-MANIFEST.json`); this is not a source-count discrepancy.
- Both packet `before/` files are byte-equal to the corresponding immutable source files.
- Exactly the two declared Rust files exist under `after/`; the patch is byte-equal to the reconstructed unified before/after diff.

| Packet file | Before SHA-256 | After SHA-256 |
| --- | --- | --- |
| `desktop/app/src/transfers.rs` | `4af6eee98f275e3bd4718b598d67255b03fa9d03f27a58983e344be6062aa8ce` | `e9fbc140f182bfc6066cccae6a0d6730eb1741ad5474c410d0db625dc8af2173` |
| `desktop/app/src/bridge/transfers.rs` | `f48900dae993e2b1ccd2d64e72564d3edc5477c513f1a6ec44e427992fc6aa1b` | `1fe596147b20732ffcaab14c948caa3e8e46dbdbb2878323c24d553d452b518d` |

Patch `mac-transfer-dedup.patch` SHA-256: `6fef5070da75375cb4c853ec8753f5afba8dee0b4512f51fa0bb3f7a8a46fa76`.

## Findings

No blocking or advisory code-change finding. Positive source evidence follows; paths prefixed `after/` refer to `../mac-author/after/`, and `base/` refers to `../link-source/`.

| Concern | Result and exact evidence |
| --- | --- |
| Exact identity and file kind | PASS. `after/desktop/app/src/transfers.rs:146-153` requires outgoing direction, exact transfer ID, exact peer, and `QueueKind::is_file_based()`. Neither names, sizes, paths, nor display names are identity keys. `base/core/nectarlink-core/src/store.rs:126-130` defines file based as Files, Folder, HandoffDocument, Recording. Different ID, peer, incoming direction, and non-file kind return Independent. |
| Live ownership | PASS. `after/desktop/app/src/transfers.rs:154-159` chooses Live only for queue Sending plus live Running/Paused. Waiting, Failed, Expired, and Sending with other live states choose Queue. |
| Both projections agree | PASS for a common underlying state. `after/desktop/app/src/transfers.rs:192-195` suppresses a queue display row only when the shared helper returns Live. `after/desktop/app/src/bridge/transfers.rs:274-279` suppresses a live display row only when the same helper returns Queue. Both search the complete underlying collections. They do not search a prefiltered JSON/model list or compare string representations. An absent matching queue item leaves the live row intact. |
| Records and counters | PASS. All new operations are immutable reads and projection filtering. `after/desktop/app/src/transfers.rs:173-191` updates active queue counts/failure flags before any display-row skip; `:238-240` still counts the entire underlying queue. Existing queue JSON fields, including attempts and created/expiry timestamps, remain unchanged at `:217-235`. No state/store/transport mutation or record deletion was added. Live `count`/`active` remain counts of visible live model rows (`after/desktop/app/src/bridge/transfers.rs:322-331`); queue counters remain underlying queue counts. |
| Retry, cancel, pause/resume, TTL | PASS. Failed/expired/retry-Waiting queue rows survive, retaining the existing queue retry/cancel path. Existing action implementations remain unchanged at `after/desktop/app/src/transfers.rs:84-131,244-255` and `after/desktop/app/src/bridge/transfers.rs:381-390,420-440`. Running/Paused live ownership keeps the row used by existing pause/resume/cancel QML (`base/desktop/app/qml/components/TransferItem.qml:204-220`). Core TTL/store logic is untouched. |
| Progress and ETA | PASS. Surviving live rows clone the original `TransferView` (`after/desktop/app/src/bridge/transfers.rs:280-292`); existing total/done/progress/rate roles at `:213-225` are unchanged. The existing rate smoothing stays in `base/desktop/app/src/state.rs:606-668`, and aggregate percent/ETA stay in `base/desktop/app/qml/components/TransferItem.qml:22-48,60-68`. No per-file estimate or optimistic progress was introduced. |
| Refresh and Qt row changes | PASS by source inspection. Subscription already includes QUEUE and TRANSFERS (`after/desktop/app/src/bridge/transfers.rs:260-264`); queue replacement and transfer progress return the relevant changes (`base/desktop/app/src/state.rs:587-590,668`). Filtered rows still pass through the unchanged keyed diff and balanced begin/end remove/insert paths (`after/desktop/app/src/bridge/transfers.rs:294-321`; `base/desktop/app/src/bridge/mod.rs:102-140`). No model reset, stale reference retention, or unbalanced Qt row operation was added. |
| Rust enum/borrowing and platform compatibility | PASS by source inspection. QueueItem/QueueStatus are publicly exported (`base/core/nectarlink-core/src/lib.rs:167-172`), and match variants/fixture fields exist (`base/core/nectarlink-core/src/store.rs:343-380`; `base/core/nectarlink-core/src/transfer.rs:54-124`). `QueueKind` is Copy; discriminant-only `matches!` patterns do not move String/Vec payloads from borrowed objects. String equality borrows the stored IDs. The nested iterators share immutable state borrows and return owned rows/JSON. No target-specific API or cfg was added; existing macOS/Windows cfg branches remain unchanged, and the helper has no Linux-specific dependency. Compilation remains unperformed. |

## Tests and review dimensions

Exactly **two** new `#[test]` functions were added, both in the existing `transfers.rs` test module:

1. `queued_file_display_preserves_live_controls_and_queue_retry_ownership`, `after/desktop/app/src/transfers.rs:458-478`: main Running/Paused sequence, queue failure, failed live state, retry Waiting, resumed Sending, Done and Expired ownership.
2. `transfer_display_never_merges_other_ids_peers_incoming_or_non_file_items`, `after/desktop/app/src/transfers.rs:480-494`: isolation by ID, peer, incoming direction and non-file kind.

The fixture uses the actual core structs and enums. Existing test names and assertion lines are preserved; the patch does not remove or weaken an existing assertion. These tests cover the pure decision, not Qt signals/rendering, projection integration, or runtime timing. Root must execute the requested tests and its existing related checks.

| Dimension | Source assessment |
| --- | --- |
| Correctness | PASS within stated steady-state projection scope. |
| Security | PASS; no new input, authority, transport, persistence, secret, or account path. |
| Performance | Acceptable for this bounded change: two nested immutable scans over existing queue/transfer collections. No I/O or new work per file; no speculative indexing abstraction needed. |
| Maintainability | PASS; one shared pure ownership decision prevents policy drift between callers. |

## Qualification boundary

The two projections read the hub separately (`after/desktop/app/src/bridge/transfers.rs:271-293,324-325`) and update Qt properties/list signals sequentially. The packet accurately describes **steady-state** deduplication; source review does not claim an atomic render transition. Transient UI behavior under concurrent updates remains for Root's runtime qualification.

Performed: read HANDOFF, MANIFEST, both full after files, full patch and relevant immutable core/state/diff/QML source; file-only hashes, manifest verification, diff reconstruction, test-count and existing-assertion preservation checks.

Not performed: compiler/formatter/build, Rust/Qt/QML tests, SDK/JVM use, app launch, OS/native interaction, devices, network, Git, primary-checkout access, keys/profiles/accounts, or vault access. Read only `~/.agents/OBSIDIAN.md` as instructed. No source files were edited. Reports are confined to `mac-review/`.

Machine-readable evidence: `EVIDENCE.json`.
