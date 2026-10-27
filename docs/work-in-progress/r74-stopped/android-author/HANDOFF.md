# Alex — Android selected-folder move source packet

Base: frozen Link `563bb1784d4c8eec81b8ae6c87b62eb110685f16`. Only two owned after-files are included under their repository-relative paths. `PATCH.diff` and `BEFORE-AFTER.json` bind the original and after bytes. No primary/immutable source, version, release notes, roadmap, shared core, native UI, or other worker files were changed.

## Result

The existing `storage.rename` Android backend can move a document without changing its name between existing directories inside one already-granted SAF tree. Same-parent SAF rename and all-files operations retain their previous code. Cross-tree move and simultaneous move+rename remain unsupported. The new path does not create a directory, copy/delete an item, request/persist a permission, change a capability, or start anything.

The same-file `SafMoveOperations` seam (PhoneStorage.kt:30) exists so the actual production move decision/dispatch (47) can be exercised by ordinary local JVM tests without an Android provider or a new dependency. Production `renameSaf` (591) supplies exact-tree DocumentsContract operations; this is not a test-only planner or a duplicate move implementation.

Before dispatch, the move path validates relative names, same tree and basename, both existing directory chains, provider source FLAG_SUPPORTS_MOVE, absence of an existing destination, and persisted read+write grant twice (before metadata traversal and immediately before dispatch). Directory moves reject textual descendants and target-ancestor document-ID aliases matching the source. IDs remain opaque. Non-directory/missing parents and missing flags fail closed. It performs one provider-native moveDocument call, uses its actual returned document ID (which may change), rejects a returned authority mismatch/malformed result, and confirms that the target's unchanged name resolves to that actual ID. SecurityException in the new path maps to Denied; explicit unsupported provider operations map to Unsupported.

## Limits that must remain visible

Checks before the call cannot make an external DocumentsProvider atomic. A provider may mutate before returning null, race with another writer, auto-rename on collision, return misleading data, or move successfully before target confirmation fails. A failed/unconfirmed response may therefore follow a real move. The implementation performs no rollback, automatic retry, copy, or delete fallback. Fake-provider failure coverage deliberately simulates a mutation followed by null/unconfirmed result and asserts a single dispatch. Do not describe this as atomic rollback or transaction safety.

Root confirmed the official API requires FLAG_SUPPORTS_MOVE and is available before this app's minimum SDK; Root retains official API evidence. Root also confirmed PhoneBrowserPage.qml remains explicitly read-only. This packet adds protocol backend support for existing authorized storage callers; it adds no Mac Move UI, changes no browser enable flag, and establishes no normal Mac/phone end-to-end behavior.

## Tests and qualification

Exactly two regression test methods were added to the existing `PhoneStorageTest.kt`:

- Main (36): same-tree cross-directory move, changed actual provider ID, exact source/target parent IDs, two current-grant checks, one mutation.
- Failure (47): refused/revoked grant, unsupported source, collision, cross-tree, move+rename, invalid path, non-directory parent, lexical and document-ID alias descendants; null and unconfirmed actual results do not retry. These use the same production decision/dispatch through a local fake provider.

Existing two tests are retained. No tests, compiler/JVM/SDK, builds, signing, Git, apps/OS probes, ADB, phone/emulator, network, keys, account/profile/vault, pairing, call, camera, capture, or playback actions were run. Source readback and byte hashes are the only verification in this author lane.

Root next: inspect `PATCH.diff`, apply after-files only if original hashes match, independently review, then use Root's existing disposable offline/generated-binding recipe to run `:app:testDebugUnitTest --tests app.nectarlink.android.storage.PhoneStorageTest`, related JVM tests, and lint/compile. No Android provider/permission or physical move acceptance can be inferred from the pure fake. Device/provider and normal client acceptance remain separate.
