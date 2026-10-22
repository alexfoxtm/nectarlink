# Mac50 build recipe addendum (source closure, Root-bound inputs)

This addendum and the shaping scripts live inside the Mac50 source tree under `scripts/macos/release/mac50/`. The deterministic client source archive is generated from the final committed source tree after those files and the feature integration are committed. Root supplies the final source commit, full tree, parent commit/tree, and source-offer archive hash from an external seal. Those values are never embedded in the commit they identify.

The source archive uses `git archive --format=tar --prefix=waypair-link-source/ <externally-pinned-commit>` followed by gzip with `mtime=0`. It therefore contains the exact shaping recipe and this addendum. The builder writes `SOURCE-OFFER.txt` during notices preparation, binding the same final commit, archive URL, and SHA-256. Root compares the actual archive member set, modes, bytes, and tree with the pinned Git commit before publication.

## Shaping files included in the source closure

- `build-mac50-candidate.py` builds the normal app, prepares third-party notices, packages, compiles the offered catalog, and signs in the reviewed leaf-first/inside-out order.
- `mac50_guard.py` consumes Root's external source/tree seal and private path map. It retains per-file checks for protected source, helper/tool bytes, frozen Mac49 evidence, signer/team, bundle identity, ADB seal, notices, and minimum macOS 27.0. The three frozen Homebrew command symlinks are accepted only when their resolved target bytes match the exact retained hashes; selected command/helper paths are checked against those same pins.
- `package-app.sh` resolves the repository root relative to its source location (or `WAYPAIR_REPO_ROOT`) and carries GPL/MPL license files into the app.
- `record-minimum-os.py` derives the declared minimum from all bundled Mach-O slices before signing.
- `macho_probe.py` supplies the checked inspection calls.
- `dmg-mac50.py` creates the UDZO image and binds the existing update-signature tool's result to the exact DMG bytes.
- `INVENTORY.json` records logical inputs, file roles, and public hashes. Root-only absolute paths remain in an external input map and do not enter the repository/archive.

No personal absolute path, credential, private key, or Root-only input map is included in the public closure. Tool identity and the exact source tree are checked by immutable input hashes plus the final commit/tree binding. The Root runner verifies selected tools/helpers and staged updater/ADB/provenance copies before creating the external seal or any build output; the public guard repeats selected executable/helper checks before each stage. Bundle identity remains `com.waypair.mac`, team `5H4D7574Z4`, the existing Apple Development leaf and update public key; no signer/key creation or rotation is part of this recipe. The minimum macOS remains 27.0.

## Root execution order

1. Create an external source seal for the fully integrated commit/tree and its parent, then create Root's private input map from the existing V3 pins plus the new final-source file hashes. Keep both outside the repository.
2. Run `build-mac50-candidate.py` with the external binding variables. It archives the exact commit, computes the source hash, and writes the matching source offer before package signing.
3. Run the Root-only static/source-archive reviewer. Require every frozen per-file, identity, signer, notice, minOS, app-manifest, source-offer, and archive-tree check to pass.
4. Run `dmg-mac50.py`, then the Root-only `root-mac50-dmg-check.py` readback. Preserve each attempt and stop on any failure.
5. Only after a separate Root decision and successful gates, publish through the existing `scripts/releases/publish-r2.py` using its supported DMG, source archive, optional `CHANGELOG.txt`, and signed appcast fields. The addendum/helpers are already inside the source archive; no publisher extension or extra asset field is needed.
6. Installation and Sparkle OTA remain separate attended Root gates. Build, static/source proof, DMG verification, or publication do not claim install/OTA/release acceptance.

The Root-only config is an external file parameter and is not uploaded. No artifact hash is known until an actual build creates the artifact. This source-only recipe makes no legal GO, public distribution, install, OTA, or release claim.
