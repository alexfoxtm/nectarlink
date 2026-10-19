# Android build — personal NectarLink local build

Fork build configuration for the Nectar-based Android app. These builds are
for Alex's personal use only. They keep the NectarLink name and mark and say
they are a local build, not an official NectarLink release
(`FORK_POLICY.md`). They are not endorsed by upstream and are not distributed
as a product.

Everything lives in `android/app/build.gradle.kts`. Without any of the inputs
below the build is upstream's (`app.nectarlink`, version from `Cargo.toml`).

## Identity and version

| Gradle property | Default (upstream) | Personal release |
| --- | --- | --- |
| `waypair.applicationId` | `app.nectarlink` | `com.waypair` |
| `waypair.versionName` | workspace version in `Cargo.toml` (`0.1.4`) | `0.1.4-local.<n>` |
| `waypair.versionCode` | derived from `Cargo.toml` (`1.2.3` → `1002003`) | chosen by Root, above the installed code |

Each is independent and validated (package-name syntax, non-blank name,
integer code 1..2100000000). The Kotlin namespace stays `app.nectarlink.android`
and the label is "NectarLink".

Package identity is preserved, not migrated:

- **`com.waypair`, same signer as the installed app (signer B).** Root keeps
  and supplies that existing key; this build never generates, rotates, or
  prints keys. Android accepts an upgrade only with the same signer and a
  higher `versionCode`, so Root picks the code explicitly.
- **Fresh explicit Nectar pairing.** The app keeps its own identity and trust
  store (`noBackupFilesDir`) and reads, converts, or imports no legacy
  Waypair protocol-2 pairing, key, or data.
- **Device identity in the Android Keystore.** `KeystoreKeyProtector` seals
  the identity key with a non-exportable AES-GCM Keystore key.

Current personal release, built from source `9b93a5c5`:

| | |
| --- | --- |
| Version | `0.1.4-local.2`, `versionCode` 95 |
| ABI | `arm64-v8a` only |
| Signer | signer B, preserved |
| Checks | lint 0 errors, 16 KiB zip alignment |
| APK SHA-256 | `1ec2da61609e39e583998d2020ec4cc84f35c592a058a241c0b6b4f0ddc8c732` |
| Source archive SHA-256 | `8f759caac0d1ef12673ae13c201af0c17b2b684cdd6cc5766757f6fa85edac65` |

The previous release was `0.1.4-local.1`, `versionCode` 94.

## Debug builds never replace an installed app

The `debug` build type adds `applicationIdSuffix = ".nectar.debug"`:
`app.nectarlink.nectar.debug` upstream, `com.waypair.nectar.debug` with the
fork id. Neither collides with `app.nectarlink`, `com.waypair`, or legacy
Waypair's `com.waypair.debug`. Debug builds are signed with the local debug
key, read no signing input, and do not update themselves.

## Release signing fails closed

There is no debug-key fallback for release. Inputs, in order:

1. `WAYPAIR_SIGNING`: path to a properties file with `storeFile`,
   `storePassword`, `keyAlias`, `keyPassword`. There is no implicit
   `~/.config/...` default: the path must be given explicitly.
2. Otherwise upstream CI's `NECTARLINK_KEYSTORE` (`.jks`, alias `nectarlink`)
   and `NECTARLINK_KEYSTORE_PASSWORD`, so `.github/workflows/release.yml`
   keeps working.

`android.injected.signing.*` is refused. If the inputs are absent or
incomplete, or a named file does not exist, the release `signingConfig` is
not created and `checkReleaseSigning` fails `packageRelease`,
`packageReleaseBundle`, and `signReleaseBundle` (and so `assembleRelease`,
`bundleRelease`, `assemble`, `build`). The error names the missing inputs,
never their values. `testDebugUnitTest`, `lintDebug`, `assembleDebug`
(upstream CI's set), and the release unit tests and lint need no
credentials.

## Toolchain

Pinned by the repo: Gradle 9.8.1 (wrapper), AGP 9.4.1, Kotlin 2.4.21,
`compileSdk`/`targetSdk` 37, `minSdk` 26, NDK `30.0.16248370`. Upstream
`release.yml` uses JDK 21 and build-tools 37.0.0 for `apksigner`.

Installed on this Mac: SDK platform `android-37.0`, NDK `30.0.16248370`,
build-tools `37.0.0`, `cargo-ndk` 4.1.2, the `aarch64-linux-android` Rust
target (stable toolchain), and Homebrew `openjdk@21`.

Setup for a fresh machine:

```sh
~/Library/Android/sdk/cmdline-tools/latest/bin/sdkmanager \
  "platforms;android-37.0" "ndk;30.0.16248370" "build-tools;37.0.0"
rustup target add aarch64-linux-android
cargo install --locked cargo-ndk
export ANDROID_HOME="$HOME/Library/Android/sdk"
export JAVA_HOME="/opt/homebrew/opt/openjdk@21"
```

Add `x86_64-linux-android` (and drop `-Pnectarlink.abis`) only for an
x86_64 emulator build.

## Build

From `android/`:

```sh
# Credential-free checks, same as upstream CI.
./gradlew --no-daemon testDebugUnitTest lintDebug assembleDebug
# Debug build under the fork id: com.waypair.nectar.debug.
./gradlew --no-daemon assembleDebug -Pwaypair.applicationId=com.waypair
# Personal release, Root only, with the preserved signer B:
WAYPAIR_SIGNING=<path to signer B's signing.properties> ./gradlew --no-daemon assembleRelease \
  -Pwaypair.applicationId=com.waypair \
  -Pwaypair.versionName=0.1.4-local.2 -Pwaypair.versionCode=95 \
  -Pnectarlink.abis=arm64-v8a
"$ANDROID_HOME/build-tools/37.0.0/apksigner" verify --print-certs \
  app/build/outputs/apk/release/app-release.apk
```

Root compares the printed certificate with the installed `com.waypair`
signer before any install or publication. The signing properties file and
keystore stay outside the repository.

## Updates: dedicated R2 route (fork-only)

Release builds check the dedicated Nectar manifest on R2, not GitHub
Releases: `waypair/nectar/android/stable/update.json`. Client rules are in
`ANDROID_R2.md`, publication in `R2_RELEASE.md`. The route is separate from
the legacy Waypair and Cloud feeds (`waypair/android/...`,
`waypair/cloud/...`), which this release left unchanged. The R2 client stays
in this fork and is excluded from upstream contributions (`FORK_POLICY.md`).

An update is offered when the manifest's tag is newer than the running
`versionName`. Android installs it only over the same package and signer,
with a higher `versionCode`.

The `0.1.4-local.2` APK, its source archive, and `SHA256SUMS.txt` are under
`waypair/nectar/android/releases/v0.1.4-local.2/`, and the stable manifest
points to them. All four objects were read back in full after publication.

## Qualification status

| Device | State |
| --- | --- |
| Owned disposable emulator | 94 → 95 normal in-app OTA passed: Check for updates, Update, Android's "allow from this source" prompt, in-app download and install, no `adb install` after the 94 fixture. The installed APK matched the published SHA-256, the UID and Cloud UID were unchanged, Home (no setup wizard) and Dark appearance were kept, and a fresh check reported 0.1.4-local.2 up to date. |
| Galaxy (physical) | `0.1.4-local.1` / 94 installed with signer B under `com.waypair` and launched. 95 is not installed yet; the phone is disconnected. New permissions stay under the owner's control. |

## Remaining gates

- Galaxy: install 95 (by OTA from 94), then a fresh Nectar pairing with the
  Mac and acceptance of the paired features on the device.
- Full-duplex audio over cellular is unproven; that probe is deferred.

## Licensing

The Android app is GPL-3.0-or-later with `LICENSES/APP-STORE-EXCEPTION.md`,
the core MPL-2.0. Retain notices and keep the source of the exact built
commit available; the source archive is published beside each APK.
