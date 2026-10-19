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
numeric `CFBundleShortVersionString`/`CFBundleVersion`, `APPL`,
`NSPrincipalClass`, plus the actually-needed
`NSLocalNetworkUsageDescription` and `NSBonjourServices` with
`_nectarlink._udp` — the mDNS service the core announces
(`MdnsAddressLookup::builder().service_name("nectarlink")`); no unused
microphone/camera usage keys are declared), runs
`macdeployqt -no-codesign` (options confirmed against the installed
`macdeployqt --help`) with `-qmldir` when QML sources exist to bundle Qt
frameworks/plugins. Homebrew splits Qt into separate prefixes (`qtbase`,
`qtdeclarative`, `qtsvg`). The Mac build explicitly declares the Svg module
so macdeployqt includes its runtime image plugin dependencies, and embeds
`@executable_path/../Frameworks` for bundled transitive libraries. The
packager also supplies qmake's unified library directory as an extra search
hint; that hint alone does not repair macdeployqt's plugin-pass rpaths.
The script copies `LICENSE` + **all** of `LICENSES/` into
`Contents/Resources/`, failing on any copy error. App names are validated as
a single filename component (no `/`, `.`/`..`, or control characters). Plist
text is XML-escaped with `sed`, not bash `${var//pat/repl}`, so the same
entities are produced by macOS `/bin/bash` 3.2 and by newer bash (where `&`
in a replacement means the matched text). After `macdeployqt`, `otool -L` on
the bundled executable must name at least one `Qt*.framework`, and each of
those frameworks must exist under `Contents/Frameworks`. A directory that
contains only `Sparkle.framework` does not pass. `macdeployqt` output that
contains `ERROR:` fails the script even when the tool exits 0. A closure
conservative closure walk then checks every Mach-O load command for a system
library (`/System`, `/usr/lib`, `/usr/libexec`) or resolves to a file inside
the `.app`. The source build supplies the main framework rpath explicitly;
this check does not model every possible inherited dyld load chain. `@executable_path` and `@loader_path` are resolved from the
nearest `.app`'s `Contents/MacOS` and from the loading image. `@rpath` is
resolved with that image's `LC_RPATH` plus the nearest app executable's
`LC_RPATH`, and the substituted file must lie inside the bundle — an
`LC_RPATH` of `/opt/homebrew/lib` does not count. A dylib's own install id
is not a load edge: Homebrew frameworks often keep an `/opt/homebrew/...`
id after they have been copied, while dependents already use a bundle path.
`SPARKLE_FRAMEWORK` is read
into a separate variable before the script assigns that name. An existing
file, directory, or dangling symlink at the output path is a collision.

Properties by construction:

- **No signing, no entitlements, no notarization, no install.** Bundle
  identity is configurable (`--bundle-id`). The default remains
  `com.waypair.link.nectar-qualification`, so a qualification bundle does
  not use the installed legacy app's id. Root has authorized
  `--bundle-id com.waypair.mac` for the final replacement build. Passing
  that id does not install the bundle, replace `/Applications/Waypair.app`,
  migrate legacy data, add a compatibility layer, or rotate the Sparkle
  public key. Installation stays a later Root step.
- **No key access.** The script touches no keychain, signing identity,
  credential, or network resource, and creates no JIT/hardened-runtime
  entitlements (that decision is measured, not guessed).
- **Fail-clear.** Missing/non-executable binary, undiscoverable
  `macdeployqt`, an existing bundle at the output path, invalid
  identifier/version strings, missing license texts, a `macdeployqt` `ERROR:`
  line, a post-`macdeployqt` executable whose linked Qt frameworks are not
  in the bundle, or a Mach-O load that is still an absolute Homebrew path or
  an `@rpath`/`@loader_path`/`@executable_path` that does not resolve inside
  the bundle all abort with a specific message. `--help` documents everything.

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
3. **R2 updater (implemented in source; Root compiles/tests):** a native
   Sparkle 2.10 helper (`scripts/macos/WaypairUpdater.swift`) is spawned by
   Rust's macOS updater path on a user-initiated check. Packaging places
   that already-compiled binary inside its own app,
   `Contents/Helpers/WaypairUpdater.app`, with `CFBundleIdentifier`
   `<host id>.updater`, `LSUIElement` true, and `SUEnableAutomaticChecks`
   false. The helper does not poll. `Sparkle.framework` stays in the host's
   `Contents/Frameworks` (verified 2.10.0 source:
   `/Applications/Waypair.app/Contents/Frameworks/Sparkle.framework`). Root
   links the helper with
   `@executable_path/../../../../Frameworks` and signs inside-out; this
   script checks that rpath with `otool` and does not rewrite or sign it.
   The host plist carries `SUFeedURL` for the dedicated Nectar feed
   (`waypair/nectar/macos/stable/appcast.xml`, never the legacy Swift
   appcast), the existing pinned `SUPublicEDKey`, and
   `SUEnableAutomaticChecks` false. Upstream's GitHub-Releases installer
   path and any SHA-only trust are never used on macOS; an unbundled helper
   is reported truthfully as unavailable. Remaining: Root's Swift
   compile/sign, the (Alex-authorized) R2 publication of that dedicated
   appcast, and rollback proof.
4. **Notarization / distribution channel:** the free Apple Development owner
   channel ceiling stands until a separate Developer ID decision; first
   launch Gatekeeper behaviour must be documented, not hidden.
5. **Bundle identity at install time:** the packaging default stays
   `com.waypair.link.nectar-qualification`. Explicit `--bundle-id com.waypair.mac`
   is the authorized id for the replacement build and is not a second
   compatibility stack or a key change. This script still only writes the
   unsigned bundle under `--output-dir`. It does not install that bundle or
   touch the legacy app's data. TCC grants follow whatever code identity
   Root signs later.

No bundle produced by this script has been launched, signed, or installed;
qualification on a physical Mac/Galaxy pair remains ahead.

Owner release builds made with Homebrew Qt 6.12 require macOS 14.4 or newer (the linked QtCore LC_BUILD_VERSION). The bundle carries the existing Waypair AppIcon.icns. The verified Sparkle 2.10 framework can also be supplied explicitly from a retained qualified archive; an installed legacy app is not required.
