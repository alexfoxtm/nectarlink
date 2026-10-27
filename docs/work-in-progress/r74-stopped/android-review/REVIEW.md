# Alex — R74 Android selected-folder move independent source review

Verdict: **GO for the bounded source packet.** No blocking or actionable source defect found within the requested same-name, cross-parent move inside one already-granted SAF tree. This is backend protocol support; compilation, JVM tests, Android DocumentsProvider acceptance, device behavior, and a usable Mac Move feature remain unqualified.

Reviewed frozen source pin: `563bb1784d4c8eec81b8ae6c87b62eb110685f16`, recorded in `../link-snapshot.json`. Read `../android-author/HANDOFF.md`, `PATCH.diff`, `BEFORE-AFTER.json`, and both complete original and after versions of `PhoneStorage.kt` and `PhoneStorageTest.kt`. Original and after SHA-256 bytes independently match all four manifest values:

| File | Before | After |
| --- | --- | --- |
| `android/app/src/main/kotlin/app/nectarlink/android/storage/PhoneStorage.kt` | `fe84d380e8edb43627d58942cb0f69fdb9587f454c7927e5e600bbf4a778b2e5` | `c1de9e10934c5d28efab5bb070df11beb83930d23cd4c541938883a4e44b6ee4` |
| `android/app/src/test/kotlin/app/nectarlink/android/storage/PhoneStorageTest.kt` | `4b1ece6045090a240b40ab06885ae5f9b5e98d1980ed1079bfda5225ed4f8300` | `3b7eb65379cc9d8096cd8e420c936f06b72b396ba5524bb00f5154e45c0e2be0` |

## Findings and positive source evidence

No actionable findings. Dimensions: security PASS within scope; correctness PASS at source review; performance acceptable for the existing child-list lookup strategy; maintainability PASS for one production decision/dispatch function with a small provider-operations seam.

The following line references are in `../android-author/android/app/src/main/kotlin/app/nectarlink/android/storage/PhoneStorage.kt`:

- **Path and tree boundary:** public `rename` retains complete relative-path validation at 235–239; `splitSafPath` resolves the exact registered folder name without normalization at 376–381. `renameSaf` rejects different tree URIs and tree-root moves at 591–596. The helper rejects malformed segments, changed basenames, and same textual parent at 47–58. It neither decodes a display path into a document ID nor canonicalizes opaque provider IDs.
- **Grants and existing parents:** the exact tree must have current persisted read and write permission at 605–608, queried before metadata traversal and again immediately before dispatch at 58 and 84. Directory traversal at 60–73 and 78 requires existing directory entries. It never calls the existing `ensureSafDir` creation helper. Provider `SecurityException` in this path maps to `Denied` through 598 and 405–411.
- **Provider capability and refusal:** source `FLAG_SUPPORTS_MOVE` is projected/read with absent or null flags defaulting to zero at 425–446, mapped at 610–615, and required at 74. Existing destination names fail before mutation at 80–82. Textual descendants, target ancestors with the source directory's document ID (including aliases), and aliases of the source parent are refused at 60–79. Cross-tree and simultaneous move+rename have no fallback.
- **One provider-native mutation:** the helper dispatches once at 85; the adapter makes one `DocumentsContract.moveDocument` call at 618–625, with source, source-parent, and target-parent URIs all built using the original selected tree. Explicit unsupported provider operations map to `Unsupported` at 626–627. There is no copy, delete, automatic retry, rollback, permission acquisition, or grant persistence in this branch.
- **Actual result and scope:** the adapter rejects a mismatching returned authority and an unparseable document result at 629–635. It extracts only the returned document ID; it does not query/open the returned URI or follow it into another tree. The helper confirms that the unchanged target name under the already-resolved target parent returns the actual result ID at 85–90, allowing providers to change IDs. Subsequent lookup uses the original selected tree through 610–611. This confirmation cannot expand provider scope.
- **Preserved behavior:** source diff leaves all-files rename/move at 235–268 and the same-parent SAF rename body at 643–646 unchanged. Packet edits are limited to the Android storage file and its existing test file; Core request authorization, peer/session ownership, Stop behavior, and selected USB-path admission receive no edit.

## Test seam assessment

Exactly one added main test method (`PhoneStorageTest.kt`:36) and one added failure test method (:47); both original methods are retained. Both added methods call the actual `moveSafDocument` used by production `renameSaf`, rather than a separately copied planner. The fake only supplies grant state, child metadata, and one provider mutation. It exercises changed returned IDs, exact parent dispatch, two grant checks, initial/revoked grant refusal, unsupported source, collision, cross-tree, move+rename, invalid path, non-directory parent, textual/document-ID alias descendants, and null/unconfirmed results without retry. This is suitable focused production-body test coverage at source review, not evidence that the tests passed or Android URI/provider integration works.

## Acceptance limits

The provider owns mutation semantics and old-document grant revocation. Preflight metadata and permission checks cannot make the external provider transaction atomic: revocation, collisions, names, parent relationships, and IDs may race or the provider may mutate before null, exception, or failed confirmation. In particular, `Failed` or `Denied` after dispatch does **not** establish that the original document remained in place. The code and handoff accurately preserve that uncertainty and perform no recovery mutation.

Directory alias checks cover the selected path's resolved ancestors, not a global scan of an arbitrary multi-parent provider graph. Provider conformance and rejection of invalid directory relationships remain provider acceptance gates. No broader guarantee is inferred from the pure fake.

The immutable `desktop/app/qml/pages/PhoneBrowserPage.qml`:8–12 explicitly describes a read-only browser with no rename/delete/upload, and :26–40 still uses the enable/consent/off path. The backend addition does not implement or enable a Mac Move control. The unchanged Core request path (`core/nectarlink-core/src/storage.rs`:453–485) checks `STORAGE_WRITE` before the platform call and has no retry loop in this handler; that inspection does not qualify the full client/session flow.

No SDK, JVM, compiler, test runner, Git, network, apps, OS/provider interaction, device, ADB, key/account/profile/signer, or vault operation was performed. Only local source reads, diffs, hashes, and this owned report were used. Official API facts were supplied as Root-verified context, not independently browsed in this lane. Root retains integration, related existing tests, compile/lint, provider/device, and normal-client acceptance responsibility.
