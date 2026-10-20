# macOS packaging — personal Waypair Link build

Packaging for the macOS port. These builds are for Alex's personal use only,
under the Waypair Link name and original monochrome mark. About identifies an
independently maintained fork; original notices remain (`FORK_POLICY.md`).
Keep one installation at `/Applications/Waypair Link.app`, with its existing
identity, settings and pairing. Develop and test it in place; do not create
separate QA/test apps or alternate bundle IDs. Code tests and synthetic data
fixtures remain appropriate. The bootstrap release snapshot below is historical.

Build and test from one coordinator checkout. Do not share a Cargo target directory between worktrees: CXX-Qt build-script outputs can retain a different checkout's bridge list. Use separate target directories for independent builds, or consolidate the source and refresh only generated workspace packages before switching. Keep qualified archives, signing helpers and registry dependencies. Source workers do not compile apps.

Current personal release, packaged from source `9b93a5c5` (native code
`0194b69d`):

| | |
| --- | --- |
| Bundle | `NectarLink.app`, `CFBundleIdentifier` `com.waypair.mac` |
| Version | `CFBundleShortVersionString` 0.1.4, `CFBundleVersion` 30 (release `0.1.4-local.2`) |
| Minimum macOS | Derived from all bundled Mach-O architectures; current local Qt closure requires 27.0 |
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
  Qt 6.12.0 (`.github/workflows/ci.yml`), but the shipped Mac builds deploy
  Homebrew bottles: `macdeployqt` is `/opt/homebrew/bin/macdeployqt`
  (`qtbase`, `qtdeclarative`, `qtsvg` 6.12.0_1). The exact Qt and library
  sources of a build are the ones its notices record (below).

It creates `Contents/MacOS/nectarlink-desktop` and an `Info.plist` with
`CFBundleIdentifier`, `CFBundleName`/`DisplayName` (default `NectarLink`),
`CFBundleExecutable`, `CFBundleShortVersionString`/`CFBundleVersion`, `APPL`,
`NSPrincipalClass`, `CFBundleIconFile` (`assets/brand/AppIcon.icns`),
`LSMinimumSystemVersion` initially 14.4, raised by `record-minimum-os.py` to the actual highest bundled deployment target before signing, and the needed `NSLocalNetworkUsageDescription`
and `NSBonjourServices` (`_nectarlink._udp`, the mDNS service the core
announces). No microphone or camera usage keys are declared. The Sparkle keys
are described under Updates below.

It then runs `macdeployqt -no-codesign`, with `-libpath` from the sibling
qmake's `QT_INSTALL_LIBS` (`/opt/homebrew/lib`, since Homebrew splits Qt into
separate prefixes) and `-qmldir=desktop/app/qml`. The desktop build declares
the Svg module, so its image plugin dependencies are deployed, and embeds the
`@executable_path/../Frameworks` rpath (`desktop/app/build.rs`). `LICENSE`
and all of `LICENSES/` are copied into `Contents/Resources/`, with the three
`assets/fonts/OFL-*.txt` texts and the prepared third-party notices
(`--notices-dir`, see Third-party notices below) in `Resources/LICENSES`.

The script fails with a specific message on:

- a missing or non-executable binary, an undiscoverable `macdeployqt`, or an
  existing file, directory, or dangling symlink at the output path;
- invalid app names (must be one filename component), bundle ids, or
  versions, or missing license texts, font licenses, or `--notices-dir`;
- notices that differ from their manifest, or would overwrite a repository
  license file;
- any bundled Mach-O (thin or fat, 32- or 64-bit), other than the app
  executable and the updater helper's own executable, whose framework,
  library, or helper no notice covers (a new Homebrew dependency fails here
  until its keg is added to the notices);
- a bundled `Sparkle.framework` that is not the one the notices were
  prepared from (version, build, or binary slices without signatures);
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
`com.waypair.mac`; the staging and installed app use the same normal identity.
The script itself never signs, installs,
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
with a reported `minimumSystemVersion` of 14.4 (later found stale in Wave24; it did not prove older-OS compatibility). All objects were read back in full after
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
# 1. Desktop binary. The current Homebrew Qt closure requires macOS 27.0;
# record-minimum-os.py determines the host app minimum after deployment.
cargo build --locked --release -p nectarlink-desktop

# 2. Updater helper, warnings as errors, for the same minimum.
swiftc -parse-as-library -O -warnings-as-errors -target arm64-apple-macos14.4 \
  -framework Sparkle -F <dir containing Sparkle.framework 2.10.0> \
  -Xlinker -rpath -Xlinker @executable_path/../../../../Frameworks \
  scripts/macos/WaypairUpdater.swift -o <scratch>/WaypairUpdater

# 3. Third-party notices, offline (see Third-party notices below).
cargo xtask notices <scratch>/rust-notices.txt --target aarch64-apple-darwin \
  --license-texts <spdx-texts>
python3 scripts/macos/prepare-notices.py prepare \
  --rust-notices <scratch>/rust-notices.txt \
  --keg /opt/homebrew/opt/qtbase --keg /opt/homebrew/opt/qtdeclarative --keg /opt/homebrew/opt/qtsvg \
  --keg /opt/homebrew/opt/<each bundled library's formula> ... \
  --license-texts <spdx-texts> \
  --sparkle-framework <dir containing Sparkle.framework 2.10.0>/Sparkle.framework \
  --sparkle-license <Sparkle checkout at tag 2.10.0>/LICENSE \
  --adb-pin scripts/macos/adb-platform-tools.json \
  --source-commit <full commit> --source-archive-url <published source archive URL> \
  --source-archive-sha256 <its SHA-256> --output-dir <scratch>/notices

# 4. Unsigned bundle.
scripts/macos/package-app.sh \
  --binary target/release/nectarlink-desktop \
  --output-dir <scratch>/nectar-release30 \
  --bundle-id com.waypair.mac --version 0.1.4 --build-version 30 \
  --notices-dir <scratch>/notices \
  --updater-helper <scratch>/WaypairUpdater \
  --sparkle-framework <dir containing Sparkle.framework 2.10.0>/Sparkle.framework \
  --adb-helper <signed-adb> --adb-seal <seal.json> --adb-notice <ADB-NOTICE.txt>
```

5. **Sign inside out** with the existing Apple Development identity: first
   verify and preserve the pre-signed `Contents/Helpers/adb` against its seal
   when present. Sign every other Mach-O file deepest first, then every `.app`, `.xpc`, `.framework`, and
   `.bundle` deepest first, then the app, each with
   `codesign --force --options runtime --timestamp --sign <identity>` and no
   `--entitlements`. Then
   `codesign --verify --deep --strict --verbose=2 NectarLink.app`. This
   step is a Root script outside the repository.
6. **DMG and appcast:** `hdiutil create` a DMG holding the signed app, sign
   it with Sparkle's Ed25519 tooling (the private key stays in the login
   Keychain, not synchronizable), and write the appcast item for the DMG URL,
   size, `sparkle:version`, and signature.
7. **Publish** with `scripts/releases/publish-r2.py` (`R2_RELEASE.md`), then
   read every object back.

Credentials are supplied at execution time and never written into the repo
or these docs.

## Licensing in the bundle

Desktop apps are GPL-3.0-or-later (with `LICENSES/APP-STORE-EXCEPTION.md`),
core libraries MPL-2.0. The bundle carries `LICENSE` and `LICENSES/*` under
`Contents/Resources/`, and the source archive of the exact built commit is
published beside each DMG. Retain all upstream copyright notices; upstream
contributions use DCO sign-off.

### Third-party notices

Settings › Open-source licenses opens `Contents/Resources/LICENSES`. Besides
the repository texts it holds:

| File | Made by | Contents |
| --- | --- | --- |
| `THIRD-PARTY-NOTICES.txt` | `cargo xtask notices --target aarch64-apple-darwin` | Every crate in the desktop app's normal dependency graph for that target, with its license files; Qt's LGPL-3.0 statement; the fonts' OFL texts. Crates that ship no license file get the standard texts of the licenses they name. |
| `OFL-*.txt` | `package-app.sh`, from `assets/fonts` | The licenses of the three fonts compiled into the app. |
| `QT-NOTICES.txt` | `prepare-notices.py` | The exact Qt version and Homebrew packages, the upstream source archives, Homebrew patches, formulas and bottles with SHA-256, and the third-party code Qt's SBOMs declare, with every license text that code needs. |
| `THIRD-PARTY-SOURCES.txt`, `third-party/` | `prepare-notices.py` | Each bundled Homebrew library's own license files, formula and SBOM, and the license texts its declared license names (`third-party/licenses/`, or the repository texts beside them); Qt's SBOMs; Sparkle's LICENSE; provenance records (`PROVENANCE.json`) for Homebrew, Sparkle, ADB, the pinned license texts and the notice coverage. |
| `SOURCE-OFFER.txt` | `prepare-notices.py` | The exact commit, its published source archive URL and SHA-256. |
| `BUNDLED-COMPONENTS.txt` | `package-app.sh` (after deployment) | Every executable file in the bundle and the notice that covers it. |
| `ADB-NOTICE.txt` | `--adb-notice` | Google's platform-tools notice; it must match the pin's `noticeSha256`. |

Everything is prepared offline from explicit inputs; nothing is downloaded,
built, or read from a keychain. `cargo xtask notices` runs `cargo metadata
--locked --offline --filter-platform aarch64-apple-darwin`, so the crate set
is the target's resolved graph rather than the whole `Cargo.lock`. Feature
unification across the workspace can only add crates, never drop one.

`prepare-notices.py prepare` refuses to write anything when an input is
missing or does not match:

- `--rust-notices` must be the macOS output, not the Windows one.
- Each `--keg` (a `/opt/homebrew/opt/<formula>` link or a Cellar directory)
  must be from homebrew/core, record its source archive and SHA-256 in its
  `sbom.spdx.json`, keep its formula in `.brew/`, and ship a license file
  (Qt kegs are covered by their SBOMs instead). The license each keg's SBOM
  declares must resolve to texts: the first `OR` choice whose texts all
  exist and every `AND` part, from `--license-texts` or the repository's
  `LICENSES/`. `NOASSERTION` or an unmet expression refuses. The keg's own
  files still ship, for their copyright lines. Pass every formula whose
  libraries macdeployqt bundles: for build 32 these were qtbase,
  qtdeclarative, qtsvg, glib, gettext, graphite2, dbus, freetype, icu4c@78,
  openssl@4, harfbuzz, pcre2, libpng, jpeg-turbo, zstd, brotli, libb2,
  double-conversion and md4c. The bundle check names any that is missing.
- `--license-texts` is a directory of `<SPDX id>.txt` files from one
  commit of [SPDX license-list-data](https://github.com/spdx/license-list-data)
  (`text/`). Its `PROVENANCE.json` must name that commit and list each
  file's `id` and `sha256`. A text whose hash differs refuses, and an unlisted
  text is never used. The commit and the hash of every text used are recorded
  in `third-party/PROVENANCE.json`. For the current graph, the build-32 kegs
  and Qt 6.12.0, these texts are needed: AFL-2.1, Apache-2.0, BSD-2-Clause,
  BSD-3-Clause, BSD-4-Clause, BSL-1.0, CC0-1.0, FTL,
  GFDL-1.3-no-invariants-only, GPL-2.0-only, HPND, HPND-sell-variant, ICU,
  IJG, Imlib2, LGPL-2.1-or-later, libpng-2.0, Linux-syscall-note, MIT,
  Qt-GPL-exception-1.0, Unicode-3.0, X11 and Zlib. Both tools say what is
  missing. Qt's `LicenseRef-*` texts come from its SBOMs;
  `LicenseRef-Qt-Commercial` is never chosen.
- `--sparkle-framework` must be Sparkle 2.10.0 (2064) whose binary, with
  signatures removed, equals the official release artifact's, and
  `--sparkle-license` must be the LICENSE at tag 2.10.0 (commit
  `eef1a539a373c1f1a320624b1130fc5de7b2e100`).
- `--source-commit` is the full commit; `--source-archive-url` and
  `--source-archive-sha256` name the source archive published beside the
  DMG, so it is made before packaging.

`verify` also requires the manifest's coverage map to equal the one in the
hashed `third-party/PROVENANCE.json`. `check-bundle` (run by
`package-app.sh`) checks every bundled `Sparkle.framework` against
`PROVENANCE.json`: version, build, and the SHA-256 of each architecture
slice with its signature removed (from temporary copies; the bundle is not
touched). A notices folder prepared before this record existed must be
prepared again.

ADB's notice is Google's aggregated file. The pin records no tag or commit
for r37.0.1, and AOSP has no `platform-tools-37.0.1` tag for adb, so the
notices say that the exact adb source revision is not recorded.

## Remaining gates

1. **No Developer ID, no notarization.** The bundle is signed with an Apple
   Development identity only. It is meant for the owner's Mac; first-launch
   Gatekeeper behaviour on other Macs is not covered.
2. **Device feature proof.** A fresh Nectar pairing with the Galaxy (after
   its 95 update) and acceptance of the paired features are still ahead.
3. **Cellular full-duplex audio** is unproven; that probe is deferred.

## Bundled ADB for a Mac without developer tools

The desktop USB watcher is a client of localhost port 5037. The Mac fork now
makes one background startup attempt for that server when the port is closed.
`NECTARLINK_NO_ADB=1` disables the attempt and the USB watcher. An existing
listener is left alone; the helper is never run as an ADB client that could
replace a server after a version mismatch. Wi-Fi discovery is independent.

Release preparation is offline: obtain the exact versioned Google archive in
`scripts/macos/adb-platform-tools.json`, then run `prepare-adb.py --archive
<archive> --output-dir <new-directory>`. The script checks the archive and the
single `platform-tools/adb` member's size and SHA-256, copies only adb and its
vendor notice, and never executes a helper. It does not use a floating latest
URL or search PATH, Homebrew or Android SDK installations.

The release owner signs that helper with the existing application identity
before writing a schema1 seal of its **signed** SHA-256 and size. The seal's
`relativePath` must be `Helpers/adb`. Supply all three bundle inputs together:
`--adb-helper <signed-adb> --adb-seal <seal.json> --adb-notice <ADB-NOTICE.txt>`.
Packaging verifies the seal before creating the bundle, then copies the helper
after Qt deployment. It installs the seal under `Resources/PhoneSetup` and the
notice under `Resources/LICENSES`. Signing the rest of the app must preserve
this helper's exact already-signed bytes, verifying the owner certificate and
seal before signing the outer app last.

At runtime the fixed foreground-server arguments are `-L
tcp:127.0.0.1:5037 nodaemon server`. A bind race fails without killing an existing
listener. Symlinks, escaped paths, invalid manifests and mismatched helper bytes
are refused. No runtime download, server restart, phone application install or
Android permission change is part of this bootstrap. Fresh-host USB hardware
acceptance is separate from the pure decision tests and existing-server check.

The bundled helper deliberately has the usual shared ADB-server lifetime: when
Waypair quits, a server it started may remain available for other tools and the
next launch. Waypair does not signal or kill it. If it exits, this one-shot
bootstrap does not respawn it until the next app launch; the USB watcher keeps
retrying localhost. This avoids a periodic process restart loop.

Exception to the general inside-out signing order above: **do not re-sign
`Contents/Helpers/adb`** after writing its seal. Verify its existing owner
certificate and signed bytes, sign every other Mach-O and nested bundle, and
sign the outer application last. Finally compare its SHA-256 and byte size to
`Resources/PhoneSetup/adb-seal.json` again and run the strict deep bundle check.

### Deployment-metadata correction (Wave24)

A complete scan found the locally installed Qt and transitive libraries require
macOS 27.0, including the same libraries in the previous build31. Earlier
Info.plist/feed values of 14.4 were incorrect compatibility metadata, not proof
of support on 14.4. The owner Mac runs27.0.1. The offline bundler now scans every
bundled Mach-O architecture, refuses missing deployment metadata and records
the real maximum in Info.plist and `Resources/build-minimum-os.json` before
signing. The publisher must use that value in the appcast. Do not edit load
commands to lower this requirement. Older-macOS support needs a separately built
and qualified dependency toolchain. The updater helper's own14.4 target does
not determine the Qt host application's minimum.

## QA bundle cleanup

Create temporary test app bundles inside a `qa.noindex` directory under the round’s `~/orca/tmp/` folder. Keep a distinct test window title and an isolated `--data-dir`; never reset the installed app’s data or keys. Register only the exact test bundle when native Services checks require it, then unregister that same path and quit the fixture after the bounded check.

After qualification, publication and installation no longer depend on a staged `.app`, remove its LaunchServices registration and move the obsolete bundle to Trash. Retain the signed DMG, source archive, receipts and required signing/build dependencies. Do not reset LaunchServices or the Dock globally, and do not remove Application Support, Keychain items or user-owned sessions as app cleanup.
