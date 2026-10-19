# macOS packaging — personal NectarLink local build

Packaging for the macOS port. These builds are for Alex's personal use only.
They keep the NectarLink name and mark and say they are a local build, not an
official NectarLink release (`FORK_POLICY.md`). They are not endorsed by
upstream and are not distributed as a product.

Current personal release, packaged from source `9b93a5c5` (native code
`0194b69d`):

| | |
| --- | --- |
| Bundle | `NectarLink.app`, `CFBundleIdentifier` `com.waypair.mac` |
| Version | `CFBundleShortVersionString` 0.1.4, `CFBundleVersion` 30 (release `0.1.4-local.2`) |
| Minimum macOS | 14.4 |
| Signature | existing Apple Development identity, hardened runtime, secure timestamp, no extra entitlements; `codesign --verify --deep --strict` passes |
| DMG | `NectarLink-0.1.4-local.2.dmg`, SHA-256 `bdac21b16c2c2c60b3f51c928f8f006dfd18db6c27c5225bf6507b6a19b7670e` |
| Source archive SHA-256 | `8f759caac0d1ef12673ae13c201af0c17b2b684cdd6cc5766757f6fa85edac65` |

The signing identity is the same one as build 29. The bundle passed the
packager's closure check, and the DMG's Ed25519 signature verified against
the pinned public key.

## Packager: `scripts/macos/package-app.sh`

The script assembles an **unsigned** `.app` from:

- an **already-compiled** `nectarlink-desktop` binary (the script never
  compiles anything), and
- a **discovered** `macdeployqt` (`--macdeployqt`, `--qt-bin-dir`/`QT_BIN_DIR`,
  `MACDEPLOYQT`, `PATH`, or `qmake -query QT_INSTALL_BINS`). Repo CI pins
  Qt 6.12.0 (`.github/workflows/ci.yml`). Here `macdeployqt` is
  `/opt/homebrew/bin/macdeployqt` (Homebrew `qtbase`, `qtdeclarative`,
  `qtsvg` 6.12.0_1).

It creates `Contents/MacOS/nectarlink-desktop` and an `Info.plist` with
`CFBundleIdentifier`, `CFBundleName`/`DisplayName` (default `NectarLink`),
`CFBundleExecutable`, `CFBundleShortVersionString`/`CFBundleVersion`, `APPL`,
`NSPrincipalClass`, `CFBundleIconFile` (`assets/brand/AppIcon.icns`),
`LSMinimumSystemVersion` 14.4, and the needed `NSLocalNetworkUsageDescription`
and `NSBonjourServices` (`_nectarlink._udp`, the mDNS service the core
announces). No microphone or camera usage keys are declared. The Sparkle keys
are described under Updates below.

It then runs `macdeployqt -no-codesign`, with `-libpath` from the sibling
qmake's `QT_INSTALL_LIBS` (`/opt/homebrew/lib`, since Homebrew splits Qt into
separate prefixes) and `-qmldir=desktop/app/qml`. The desktop build declares
the Svg module, so its image plugin dependencies are deployed, and embeds the
`@executable_path/../Frameworks` rpath (`desktop/app/build.rs`). `LICENSE`
and all of `LICENSES/` are copied into `Contents/Resources/`.

The script fails with a specific message on:

- a missing or non-executable binary, an undiscoverable `macdeployqt`, or an
  existing file, directory, or dangling symlink at the output path;
- invalid app names (must be one filename component), bundle ids, or
  versions, or missing license texts;
- a `macdeployqt` `ERROR:` line, even when the tool exits 0;
- an executable whose linked `Qt*.framework`s are not all under
  `Contents/Frameworks` (a directory holding only `Sparkle.framework` does
  not pass);
- an open closure: every Mach-O load must be a system library (`/System`,
  `/usr/lib`, `/usr/libexec`) or resolve to a file inside the bundle.
  `@executable_path`, `@loader_path`, and `@rpath` (the image's `LC_RPATH`
  plus the app executable's) are resolved, and an `LC_RPATH` of
  `/opt/homebrew/lib` does not count. A dylib's own install id is not a load
  edge. The check does not model every inherited dyld load chain.

Plist text is XML-escaped with `sed`, so macOS `/bin/bash` 3.2 and newer bash
produce the same entities. The bundle id defaults to
`com.waypair.link.nectar-qualification`; release builds pass
`--bundle-id com.waypair.mac`. The script itself never signs, installs,
touches a keychain or credential, or uses the network. `--help` documents
every option.

## Identity and data preserved

- Bundle id `com.waypair.mac` is the installed app's id. The release keeps
  it; there is no second compatibility stack and no bundle-id change.
- Data stays in `~/Library/Application Support/com.waypair.link.nectar`.
  Nothing is moved or migrated.
- **Native Keychain protection.** `MacosKeyProtector`
  (`core/nectarlink-core/src/identity/macos.rs`) seals the device identity
  key with AES-256-GCM under a random wrapping key kept in this app's own
  login-Keychain generic-password item (service
  `com.waypair.link.nectar.key-wrapping.v1`). `identity.key` is written as
  protector method 2. The wrapping key is created only when absent and is
  never overwritten or regenerated; any other Keychain error fails closed.
  The item's ACL follows the signed code identity, so releases keep the same
  signing identity.

## Updates: dedicated R2 route (fork-only)

A native Sparkle 2.10 helper (`scripts/macos/WaypairUpdater.swift`) is
spawned by Rust's macOS updater path on a user-initiated check; it does not
poll. With `--updater-helper`, the packager places it at
`Contents/Helpers/WaypairUpdater.app` (`CFBundleIdentifier`
`<host id>.updater`, `LSUIElement` true, `SUEnableAutomaticChecks` false),
checks its `@executable_path/../../../../Frameworks` rpath with `otool`, and
copies `Sparkle.framework` (from `--sparkle-framework`, `SPARKLE_FRAMEWORK`,
or a local `Waypair.app`) into the host's `Contents/Frameworks`.

The host plist carries:

- `SUFeedURL`: the dedicated Nectar appcast
  `waypair/nectar/macos/stable/appcast.xml` on the R2 public origin, never
  the legacy Swift appcast;
- `SUPublicEDKey`: the existing pinned Ed25519 public key, unchanged;
- `SUEnableAutomaticChecks` false.

Sparkle verifies the appcast's Ed25519 signature and installs only a
`sparkle:version` above the host's `CFBundleVersion`. Upstream's
GitHub-Releases installer path and SHA-only trust are not used on macOS. The
helper, feed, and publisher stay in this fork and are excluded from upstream
contributions (`FORK_POLICY.md`).

The DMG, its source archive, and `SHA256SUMS.txt` are under
`waypair/nectar/macos/releases/v0.1.4-local.2/`. The appcast offers build 30
with `minimumSystemVersion` 14.4. All objects were read back in full after
publication (`R2_RELEASE.md`), and the legacy Waypair and Cloud feeds were
left unchanged.

**Build 29 → 30 OTA passed on this Mac:** in-app check, Sparkle download from
R2, Install and Relaunch. After the update the executable was exactly the
released one, the deep strict signature held, the app relaunched by itself
to Home with the walkthrough preference kept, a fresh check reported 0.1.4 up
to date, the helper exited after the dialog closed, and the encrypted
identity was unchanged.

## Release sequence (Root)

Root owns the compilers, the signing identity, and the Ed25519 private key.

```sh
# 1. Desktop binary, Homebrew Qt 6.12 (sets the macOS 14.4 minimum).
cargo build --locked --release -p nectarlink-desktop

# 2. Updater helper, warnings as errors, for the same minimum.
swiftc -parse-as-library -O -warnings-as-errors -target arm64-apple-macos14.4 \
  -framework Sparkle -F <dir containing Sparkle.framework 2.10.0> \
  -Xlinker -rpath -Xlinker @executable_path/../../../../Frameworks \
  scripts/macos/WaypairUpdater.swift -o <scratch>/WaypairUpdater

# 3. Unsigned bundle.
scripts/macos/package-app.sh \
  --binary target/release/nectarlink-desktop \
  --output-dir <scratch>/nectar-release30 \
  --bundle-id com.waypair.mac --version 0.1.4 --build-version 30 \
  --updater-helper <scratch>/WaypairUpdater \
  --sparkle-framework <dir containing Sparkle.framework 2.10.0>/Sparkle.framework
```

4. **Sign inside out** with the existing Apple Development identity: every
   Mach-O file deepest first, then every `.app`, `.xpc`, `.framework`, and
   `.bundle` deepest first, then the app, each with
   `codesign --force --options runtime --timestamp --sign <identity>` and no
   `--entitlements`. Then
   `codesign --verify --deep --strict --verbose=2 NectarLink.app`. This
   step is a Root script outside the repository.
5. **DMG and appcast:** `hdiutil create` a DMG holding the signed app, sign
   it with Sparkle's Ed25519 tooling (the private key stays in the login
   Keychain, not synchronizable), and write the appcast item for the DMG URL,
   size, `sparkle:version`, and signature.
6. **Publish** with `scripts/releases/publish-r2.py` (`R2_RELEASE.md`), then
   read every object back.

Credentials are supplied at execution time and never written into the repo
or these docs.

## Licensing in the bundle

Desktop apps are GPL-3.0-or-later (with `LICENSES/APP-STORE-EXCEPTION.md`),
core libraries MPL-2.0. The bundle carries `LICENSE` and `LICENSES/*` under
`Contents/Resources/`, and the source archive of the exact built commit is
published beside each DMG. Retain all upstream copyright notices; upstream
contributions use DCO sign-off.

## Remaining gates

1. **No Developer ID, no notarization.** The bundle is signed with an Apple
   Development identity only. It is meant for the owner's Mac; first-launch
   Gatekeeper behaviour on other Macs is not covered.
2. **Device feature proof.** A fresh Nectar pairing with the Galaxy (after
   its 95 update) and acceptance of the paired features are still ahead.
3. **Cellular full-duplex audio** is unproven; that probe is deferred.
