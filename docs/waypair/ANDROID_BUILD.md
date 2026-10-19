# Android build — Waypair Link (Nectar base)

Fork build configuration for the Nectar-based Android app. Decision context:
`Wave21-nectar-port/MIGRATION-DECISION.md` (scratch): complete switch to
NectarLink Android + Mac, no legacy wire bridge, fresh explicit Nectar
pairing, legacy installs and data untouched until qualification.

Everything lives in `android/app/build.gradle.kts`. Without any of the inputs
below the build is upstream's (`app.nectarlink`, version from `Cargo.toml`).

## Identity and version

| Gradle property | Default (upstream) | Waypair release |
| --- | --- | --- |
| `waypair.applicationId` | `app.nectarlink` | `com.waypair` |
| `waypair.versionName` | workspace version in `Cargo.toml` | chosen by Root |
| `waypair.versionCode` | derived from `Cargo.toml` (`1.2.3` → `1002003`) | chosen by Root |

Each is independent and validated (package-name syntax, non-blank name,
integer code 1..2100000000). The Kotlin namespace stays `app.nectarlink.android`.

Intended release identity:

- **`com.waypair`, same signer as the installed legacy app.** Root preserves
  and supplies that existing key; this build never generates, rotates, or
  prints keys. Android only accepts the upgrade with the same signer and a
  `versionCode` above the installed one, so Root picks the code explicitly.
- **Fresh explicit Nectar pairing.** The Nectar app keeps its own identity
  and trust store (`noBackupFilesDir`); it reads, converts, or imports no
  Waypair protocol-2 pairing, key, or data.
- **Legacy data untouched until qualified.** Installing a signed
  `com.waypair` release is an in-place upgrade of the legacy app, so it
  happens only after the Nectar-based Link qualifies and Alex decides to
  switch. Qualification runs on the debug package below (or another
  explicitly chosen non-legacy `waypair.applicationId`), beside the legacy app.

## Debug builds never replace an installed app

The `debug` build type adds `applicationIdSuffix = ".nectar.debug"`:
`app.nectarlink.nectar.debug` upstream, `com.waypair.nectar.debug` with the
Waypair id. Neither collides with `app.nectarlink`, `com.waypair`, or legacy
Waypair's `com.waypair.debug`. Debug builds are signed with the local debug
key and read no signing input.

## Release signing fails closed

There is no debug-key fallback for release anymore. Inputs, in order:

1. `WAYPAIR_SIGNING`: path to a properties file with `storeFile`,
   `storePassword`, `keyAlias`, `keyPassword`, the legacy Waypair build's
   established input. Unlike the legacy build there is no implicit
   `~/.config/...` default: the path must be given explicitly.
2. Otherwise upstream CI's `NECTARLINK_KEYSTORE` (`.jks`, alias `nectarlink`)
   and `NECTARLINK_KEYSTORE_PASSWORD`, so `.github/workflows/release.yml`
   keeps working.

`android.injected.signing.*` is refused, as in the legacy build. If the inputs
are absent or incomplete, or a named file does not exist, the release
`signingConfig` is not created and `checkReleaseSigning` fails
`packageRelease`, `packageReleaseBundle`, and `signReleaseBundle` (and so
`assembleRelease`, `bundleRelease`, `assemble`, `build`). The error names the
missing inputs, never their values. `testDebugUnitTest`, `lintDebug`,
`assembleDebug` (upstream CI's set), and the release unit tests and lint need
no credentials.

## Prerequisites (Root owns the sole compiler)

Observed on this Mac on 2026-10-19 (directory listings only, nothing
installed): SDK platform `android-37.0` present; build-tools 35.0.0 and
36.0.0 only (`37.0.0`, used for `apksigner` in `release.yml`, missing); NDK
`29.0.14206865` only (`30.0.16248370` required, missing); `cargo-ndk`
missing; Rust targets installed are Apple only (Android targets missing);
Homebrew OpenJDK 17 present; `ANDROID_SDK_ROOT` set, `ANDROID_HOME` unset;
no `android/local.properties`.

Setup (not run here):

```sh
~/Library/Android/sdk/cmdline-tools/latest/bin/sdkmanager \
  "platforms;android-37.0" "ndk;30.0.16248370" "build-tools;37.0.0"
rustup target add aarch64-linux-android x86_64-linux-android armv7-linux-androideabi
cargo install --locked cargo-ndk
export ANDROID_HOME="$HOME/Library/Android/sdk"
export JAVA_HOME="$(/usr/libexec/java_home -v 17)"
```

Build (not run here), from `android/`:

```sh
# Credential-free checks, same as upstream CI.
./gradlew --no-daemon testDebugUnitTest lintDebug assembleDebug
# Qualification debug build under the Waypair id: com.waypair.nectar.debug.
./gradlew --no-daemon assembleDebug -Pwaypair.applicationId=com.waypair
# Waypair release, Root only, with the preserved signer:
WAYPAIR_SIGNING=<path to signing.properties> ./gradlew --no-daemon assembleRelease \
  -Pwaypair.applicationId=com.waypair \
  -Pwaypair.versionName=<name> -Pwaypair.versionCode=<code above installed> \
  -Pnectarlink.abis=arm64-v8a,armeabi-v7a
"$ANDROID_HOME/build-tools/37.0.0/apksigner" verify --print-certs \
  app/build/outputs/apk/release/app-release.apk
```

Root compares the printed certificate with the legacy `com.waypair` signer
before any install.

## Updater: separate gate, no OTA claim

No updater work is part of this change, and public OTA updates for the
Waypair build do **not** work. Every non-debug build still carries upstream's
`AppUpdater`, which polls GitHub Releases of `thtbee/nectarlink` and verifies
`SHA256SUMS.txt`. Under `com.waypair` it would advertise upstream versions,
and its install session (pinned to the app's own package, with a different
signer) could not install them. Before any Waypair release is distributed,
a separate port (task M4-T2) must replace it with the existing R2 feed:
authenticated feed and artifact, version checks against `versionCode`, and
tested rollback, without touching the legacy `com.waypair` feed or signers.

## Other open gates

- Branding: the app is still labelled "NectarLink" (`res/values/strings.xml`).
  Per `TRADEMARKS.md`, a modified build ships under Waypair's name and logo.
  That's the package/branding worker's change.
- Licensing: the Android app is GPL-3.0-or-later with
  `LICENSES/APP-STORE-EXCEPTION.md`, the core MPL-2.0. Retain notices and
  publish the source of the exact built commit.
- No build, install, signing, or device run has been performed for this
  configuration; qualification on a physical Galaxy is still ahead.
