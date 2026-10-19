# macOS packaging — Waypair Link (Nectar base)

Minimal, honest packaging for the macOS port. Decision context:
`Wave21-nectar-port/MIGRATION-DECISION.md` (scratch) — complete switch to
NectarLink Android+Mac as the Link base; legacy apps preserved untouched
until qualification.

## What exists

`scripts/macos/package-app.sh` assembles a **minimal unsigned `.app`** from:

- an **already-compiled** `nectarlink-desktop` binary (input; the script
  never compiles anything), and
- a **discovered** `macdeployqt` (`--macdeployqt`, `--qt-bin-dir`/`QT_BIN_DIR`,
  `MACDEPLOYQT`, `PATH`, or `qmake -query QT_INSTALL_BINS`). Repo CI pins
  **Qt 6.12.0** (`.github/workflows/ci.yml`); on this machine a matching
  `macdeployqt` is discovered at `/opt/homebrew/bin/macdeployqt`
  (Homebrew `qtbase 6.12.0_1`) — the script still takes explicit paths and
  reports clearly when no tool is discoverable, instead of assuming one.

It creates `Contents/MacOS/nectarlink-desktop`, a correct `Info.plist`
(`CFBundleIdentifier`, `CFBundleName`/`DisplayName`, `CFBundleExecutable`,
`CFBundleShortVersionString`, `CFBundleVersion`, `APPL`, `NSPrincipalClass`),
runs `macdeployqt -no-codesign` (plus `-qmldir` when QML sources exist) to
bundle Qt frameworks/plugins, and copies `LICENSE` + `LICENSES/` into
`Contents/Resources/`.

Properties by construction:

- **No signing, no entitlements, no notarization, no install.** Bundle
  identity is configurable (`--bundle-id`); the default
  `com.waypair.link.nectar-qualification` deliberately differs from the
  installed legacy `com.waypair.mac`, and passing `com.waypair.mac` is
  refused during qualification. Final identifier selection is a Root
  release-engineering decision for the qualified build.
- **No key access.** The script touches no keychain, signing identity,
  credential, or network resource, and creates no JIT/hardened-runtime
  entitlements (that decision is measured, not guessed).
- **Fail-clear.** Missing/non-executable binary, undiscoverable
  `macdeployqt`, an existing bundle at the output path, invalid
  identifier/version strings, or missing license texts all abort before or
  during assembly with a specific message. `--help` documents everything.

Expected Root build sequence (Root owns compilers; illustrative, not run here):

```sh
# 1. Build the desktop binary with the pinned toolchain + Qt 6.12.0, e.g.
#    cargo build --locked --release -p nectarlink-desktop
# 2. Assemble the unsigned bundle:
scripts/macos/package-app.sh \
  --binary target/release/nectarlink-desktop \
  --output-dir ~/orca/tmp/waypair-link-build \
  --qt-bin-dir <qt-6.12.0>/macos/bin
```

## Name and logo (upstream TRADEMARKS.md)

Per upstream `TRADEMARKS.md`, modified builds must not be distributed under
the NectarLink name or logo: this port ships as **Waypair Link** with
Waypair's own logo (default `--app-name "Waypair Link"`; no upstream icon is
bundled by the script). Attribution is retained — the GPL/MPL license texts
and upstream copyright notices travel in the bundle as described below, and
"based on NectarLink" wording is acceptable where accurate.

## Licensing in the bundle (compliance, already authorized)

Desktop/apps are GPL-3.0-or-later (with `LICENSES/APP-STORE-EXCEPTION.md`),
core libraries MPL-2.0. The bundle therefore carries `LICENSE` and
`LICENSES/*.txt` under `Contents/Resources/`, and the corresponding source
must remain publicly available for every distributed build (upstream repo /
Waypair fork at the exact built commit). Retain all upstream copyright
notices; upstream contributions use DCO sign-off. No approval gate remains —
this section is a shipping checklist.

## Unresolved distribution gates (Root follow-ups — not fabricated here)

1. **Production signing & hardened runtime:** pinned Apple Development
   identity, inside-out signing of Qt frameworks/plugins, library
   validation, secure timestamp; measured QML JIT vs interpreter
   entitlement decision. The unsigned output of `package-app.sh` is a
   qualification artifact only.
2. **Keychain integration:** macOS `KeyProtector` for the device identity
   (today's non-Windows default stores `identity.key` unprotected) with
   release code identity, before any real pairing.
3. **R2 updater:** single updater on the existing R2 appcast with Sparkle
   Ed25519 verification and rollback; upstream's GitHub-Releases updater is
   not the shipping channel.
4. **Notarization / distribution channel:** the free Apple Development owner
   channel ceiling stands until a separate Developer ID decision; first
   launch Gatekeeper behaviour must be documented, not hidden.
5. **Final bundle identity & data locations:** chosen at qualification so
   legacy `com.waypair.mac` data stays intact; TCC grants for the new code
   identity are fresh by design.

No bundle produced by this script has been launched, signed, or installed;
qualification on a physical Mac/Galaxy pair remains ahead.
