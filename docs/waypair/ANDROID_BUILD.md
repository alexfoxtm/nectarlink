# Android build — Waypair Link fork build

This is the independently maintained **Waypair Link** Android fork, based on
NectarLink. The app label is **Waypair Link**; About identifies it as an
**independently maintained fork**. These builds are for Alex's personal use.
The dedicated R2 publication supports this fork's updater and does not make
the app an official upstream release or a supported third-party product.
Original attribution and license notices remain in Open-source Licenses and
the source (`FORK_POLICY.md`). The `-local` version marker identifies the
fork build; it does not mean the artifact is unpublished.

Everything lives in `android/app/build.gradle.kts`. Without any of the inputs
below the build is upstream's (`app.nectarlink`, version from `Cargo.toml`).

## Identity and version

| Gradle property | Default (upstream) | Waypair Link release |
| --- | --- | --- |
| `waypair.applicationId` | `app.nectarlink` | `com.waypair` |
| `waypair.versionName` | workspace version in `Cargo.toml` (`0.1.4`) | `0.1.4-local.<n>` |
| `waypair.versionCode` | derived from `Cargo.toml` (`1.2.3` → `1002003`) | chosen by Root, above the installed code |

`waypair.versionName` and `waypair.versionCode` are independent properties;
changing one does not derive or change the other. Each is validated
(package-name syntax, non-blank name, integer code 1..2100000000). The Kotlin
namespace stays `app.nectarlink.android`; the packaged label is **Waypair
Link**.

Package identity is preserved, not migrated:

- **`com.waypair`, same signer as the installed app (signer B).** Root keeps
  and supplies that existing key; this build never generates, rotates, or
  prints keys. Android accepts an upgrade only with the same signer and a
  higher `versionCode`, so Root picks the code explicitly.
- **Stable app and trust identity.** A rename is an in-place update under
  `com.waypair`, with the existing data locations, Keystore identity, and
  pairing records preserved. Do not import keys, reset trust, or migrate
  identity data.
- **Device identity in the Android Keystore.** `KeystoreKeyProtector` seals
  the identity key with a non-exportable AES-GCM Keystore key.

Current published personal-use release, built from source
`405256518f8601cd4ef802cb3a4193afc027bef0`:

| | |
| --- | --- |
| Version | `0.1.4-local.3`, `versionCode` 96 |
| App label / About | `Waypair Link` / `Independently maintained fork` |
| ABI | `arm64-v8a` only |
| Signer / package | signer B preserved / `com.waypair` |
| Checks | lint 0 errors, APK and native libraries 16 KiB aligned |
| APK SHA-256 | `756fa704570cf8f53a81fa4a69638e12a29d0106c62a77af0ba7b1f8afcc40f7` |

The previous release was `0.1.4-local.2`, `versionCode` 95. The version
name and code remain separate release properties.

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
# Waypair Link release, Root only, with the preserved signer B:
WAYPAIR_SIGNING=<path to signer B's signing.properties> ./gradlew --no-daemon assembleRelease \
  -Pwaypair.applicationId=com.waypair \
  -Pwaypair.versionName=0.1.4-local.3 -Pwaypair.versionCode=96 \
  -Pnectarlink.abis=arm64-v8a
"$ANDROID_HOME/build-tools/37.0.0/apksigner" verify --print-certs \
  app/build/outputs/apk/release/app-release.apk
```

Root compares the printed certificate with the installed `com.waypair`
signer before any install or publication. The signing properties file and
keystore stay outside the repository.

## Updates: dedicated fork-only R2 route

Release builds check this fork's dedicated R2 manifest, not the GitHub
Releases API: `waypair/nectar/android/stable/update.json`. The historical
`nectar` path is retained as a stable channel ID. Client rules are in
`ANDROID_R2.md`, publication in `R2_RELEASE.md`. The route is separate from
the existing Waypair Link and Cloud feeds (`waypair/android/...`,
`waypair/cloud/...`), which this fork leaves unchanged. All R2 updater code,
configuration, and publication stay in this fork and are excluded from
upstream contributions (`FORK_POLICY.md`).

An update is offered when the manifest's tag is newer than the running
`versionName`. Android installs it only over the same package and signer,
with a higher `versionCode`.

The current published release is `0.1.4-local.3` / code 96 under
`waypair/nectar/android/releases/v0.1.4-local.3/`; its stable manifest uses
the dedicated fork-only route above. The qualification receipt below records
the APK identity and local runtime evidence. The R2 publication/readback record
is maintained separately from that qualification.

## Qualification status

| Device | State |
| --- | --- |
| Owned disposable emulator — 95 history | `0.1.4-local.2` / code 95 OTA from 94 passed through the in-app update flow. This is historical 95 evidence, not qualification of the current build. |
| Owned disposable emulator — current 96 qualification | Receipt `/Users/AleksandrSlobodanyuk/orca/tmp/waypair-wave23-20261019/release/ANDROID96-BRAND-QUALIFICATION.json` records source `405256518f8601cd4ef802cb3a4193afc027bef0`, package `com.waypair`, version `0.1.4-local.3` / code 96, signer B preserved, label `Waypair Link`, About text `Independently maintained fork`, zero lint errors, and 16 KiB alignment. It records launch to Home without setup and no physical Galaxy use. APK SHA-256: `756fa704570cf8f53a81fa4a69638e12a29d0106c62a77af0ba7b1f8afcc40f7`. |
| Physical Galaxy | No install or launch status is claimed here; the owner was disconnected during this qualification. Alex owns any Galaxy install and paired-device acceptance. |

## Remaining gates

- Galaxy: owner-led install and paired-feature acceptance remain unverified.
- Full-duplex audio over cellular is unproven; that probe is deferred.

## Licensing

The Android app is GPL-3.0-or-later with `LICENSES/APP-STORE-EXCEPTION.md`,
the core MPL-2.0. Retain notices and keep the source of the exact built
commit available; the source archive is published beside each APK.
