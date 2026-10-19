# R2 release — Waypair Link (Nectar base)

`scripts/releases/publish-r2.py` publishes one already-built, already-verified
release to Cloudflare R2. It builds, signs, installs, and verifies signatures
of nothing. Root supplies the artifacts, the signed appcast, the
qualification receipt, and the credentials at execution time.

## Key layout

Public origin (fixed): `https://pub-a57457f4da43415ea6335550ee9e2983.r2.dev`.
Every key is under `waypair/nectar/android/` or `waypair/nectar/macos/`;
anything else is refused. No legacy Link or Cloud key is ever written.

| Key | Kind |
|---|---|
| `waypair/nectar/android/releases/v<ver>/<name>-android.apk` | immutable |
| `waypair/nectar/android/releases/v<ver>/<source archive>` | immutable |
| `waypair/nectar/android/releases/v<ver>/SHA256SUMS.txt` | immutable, generated |
| `waypair/nectar/android/stable/update.json` | mutable, generated (`AppUpdater.MANIFEST_URL`) |
| `waypair/nectar/macos/releases/v<ver>/<name>.dmg` | immutable |
| `waypair/nectar/macos/releases/v<ver>/<source archive>` | immutable |
| `waypair/nectar/macos/releases/v<ver>/SHA256SUMS.txt` | immutable, generated |
| `waypair/nectar/macos/stable/appcast.xml` | mutable, Root's signed bytes as-is (`SUFeedURL`) |

`update.json` uses the upstream GitHub-release shape that `AppUpdater`
parses: `tag_name` (`v<ver>`), `html_url`, `draft: false`,
`prerelease: false`, and `assets` holding exactly the `*-android.apk` and
`SHA256SUMS.txt` with `name`, `size`, `browser_download_url`. `SHA256SUMS.txt`
lists the binary and the source archive as `<hex>  <name>` lines.

The appcast is never generated or edited. Root signs it for the DMG URL
above. Preflight only checks that it contains that exact DMG URL, a matching
`length="<size>"`, and a `sparkle:edSignature` attribute. That is a
consistency check, not signature verification.

## Plan

Root writes the plan. Relative paths resolve against the plan's directory.
Either platform section may be left out, but not both.

```json
{
  "qualification_receipt": {"path": "/abs/QUALIFICATION.md", "sha256": "<hex>", "size": 1234},
  "android": {
    "version": "0.47.0",
    "apk": {"path": "/abs/Waypair-Link-0.47.0-android.apk", "sha256": "<hex>", "size": 0},
    "source": {"path": "/abs/waypair-link-0.47.0-source.tar.gz", "sha256": "<hex>", "size": 0},
    "html_url": "https://… (optional; defaults to the release's SHA256SUMS.txt URL)"
  },
  "macos": {
    "version": "0.47.0",
    "dmg": {"path": "/abs/Waypair-Link-0.47.0.dmg", "sha256": "<hex>", "size": 0},
    "source": {"path": "/abs/waypair-link-0.47.0-source.tar.gz", "sha256": "<hex>", "size": 0},
    "appcast": {"path": "/abs/appcast.xml", "sha256": "<hex>", "size": 0}
  }
}
```

The receipt is only checked to exist with the stated hash. It is never
uploaded.

## Running

```sh
# Dry-run (default): standard library only, no network.
python3 scripts/releases/publish-r2.py --plan /abs/plan.json

# Publish, from Root's pinned publisher venv (boto3) with Root's AWS env:
#   AWS_ACCESS_KEY_ID, AWS_SECRET_ACCESS_KEY,
#   AWS_ENDPOINT_URL_S3=https://<account>.r2.cloudflarestorage.com
<venv>/bin/python scripts/releases/publish-r2.py --plan /abs/plan.json \
  --publish --bucket <bucket>
```

Each run writes `<log-dir>/<UTC stamp>-<mode>/run.log` (default log dir:
`<plan dir>/r2-publish-logs`). It also writes the generated `update.json` and
`SHA256SUMS.txt` under `generated/`, and saves each mutable manifest it
replaces under `prior/` for rollback. Credentials are only checked for
presence, never printed or logged.

## Phases and exit codes

1. **PREFLIGHT** (always, local): the plan parses, the receipt and every file
   exist with the exact size and SHA-256 from the plan, names are safe, and
   the appcast is consistent with the DMG. Failure: `PREFLIGHT FAILED`, exit 1.
2. **DRY-RUN** (default): prints every key, size, hash, and content type in
   upload order, then `DRY-RUN COMPLETE: nothing uploaded`, exit 0.
3. **REMOTE-PREFLIGHT** (`--publish`, read-only): each immutable key that
   already exists is compared by size and full SHA-256. Identical keys are
   skipped. Different bytes mean `REFUSED … nothing uploaded`, exit 2. A
   remote read error also exits 2 before any write.
4. **UPLOAD**: missing immutable artifacts, source archives, and
   `SHA256SUMS.txt` go first, then the mutable `update.json` and
   `appcast.xml`. Local bytes are re-hashed just before each upload, and the
   stored size is checked afterwards. On failure the script prints
   `UPLOAD FAILED after N upload(s)` with the uploaded keys, writes the
   traceback to `run.log`, and exits 3. Manifests are never written after an
   artifact failure. Success ends with `PUBLISH COMPLETE`, exit 0.

A rerun after a partial failure is safe: identical immutable keys are
skipped.

## Root gates not covered by this script

- Qualification of the exact APK and DMG on the physical Galaxy/Mac pair,
  recorded in the receipt the plan references.
- APK signer equality with legacy `com.waypair` (`apksigner verify
  --print-certs`), and the Ed25519 signing of the appcast with the pinned
  `SUPublicEDKey`, both before the plan is written.
- Public readback after publishing: fetch each public URL, compare bytes with
  the plan, and confirm `update.json` and `appcast.xml` are served fresh.
- An end-to-end update on device (Android `AppUpdater`, Mac Sparkle `--check`)
  and the rollback proof using the `prior/` manifests.
