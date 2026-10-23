#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-or-later
#
# package-app.sh — assemble a minimal, UNSIGNED macOS .app bundle from an
# already-compiled nectarlink-desktop binary and a discovered macdeployqt.
#
# This script deliberately does NOT:
#   * build anything (no cargo, no Qt build, no compiler invocation),
#   * sign, notarize, or add entitlements (production signing is a Root-owned
#     follow-up; see docs/waypair/MACOS_PACKAGING.md),
#   * guess QML/JIT entitlements,
#   * install the bundle anywhere or write outside --output-dir,
#   * access any keychain, credential, or network resource.
#
# It fails clearly on: missing/non-executable input binary, undiscoverable
# macdeployqt, an existing bundle at the output path, missing license
# texts (GPL/MPL source-and-notice material must travel with the bundle),
# missing or altered prepared third-party notices (prepare-notices.py),
# macdeployqt ERROR lines (that tool exits 0 when it cannot resolve an
# @rpath), a post-deploy bundle that is not closed, a bundled framework,
# library or helper that no notice covers, or a bundled Sparkle.framework
# that is not the one the notices were prepared from.
set -euo pipefail

BINARY=""
OUTPUT_DIR=""
APP_NAME="Waypair Link"
BUNDLE_ID="com.waypair.mac"
VERSION="0.2.1"
BUILD_VERSION=""
MACDEPLOYQT_ARG=""
QT_BIN_DIR="${QT_BIN_DIR:-}"
QML_DIR=""
UPDATER_HELPER=""
ADB_HELPER=""
ADB_SEAL=""
ADB_NOTICE=""
NOTICES_DIR=""
SPARKLE_FRAMEWORK_ARG=""
# Dedicated Nectar feed (Alex-authorized, Root-published): never the legacy
# Swift appcast, whose items must not offer themselves to this app; Sparkle
# also only installs versions newer than the host bundle's CFBundleVersion.
UPDATE_FEED="https://pub-a57457f4da43415ea6335550ee9e2983.r2.dev/waypair/nectar/macos/stable/appcast.xml"
UPDATE_ED_KEY="uMzztqVF0M7BMYkWsL48ZJDvSxrqoVNSvqBKxYdu8ss="

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="${WAYPAIR_REPO_ROOT:-$(cd "$SCRIPT_DIR/../../../.." && pwd)}"
MACHO_PROBE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/macho_probe.py"

usage() {
    cat <<'EOF'
Usage: scripts/macos/package-app.sh --binary <nectarlink-desktop> --output-dir <dir> [options]

Assemble a minimal UNSIGNED .app from an already-compiled binary. Nothing is
built, signed, installed, or launched.

Required:
  --binary <path>        Existing compiled nectarlink-desktop executable.
  --output-dir <dir>     Directory to create <App Name>.app in (created if absent).
                         Fails if the .app already exists there.
  --notices-dir <dir>    Third-party notices made beforehand by
                         `scripts/macos/prepare-notices.py prepare` (Rust crates,
                         Qt, Homebrew libraries, Sparkle, ADB, source offer).
                         Verified against its manifest, copied into
                         Resources/LICENSES with the font licenses, then every
                         bundled Mach-O must be covered by it, and the bundled
                         Sparkle.framework must be the one it was prepared from
                         (pass the same --sparkle-framework to both).

Options:
  --app-name <name>      App/display name. Default: "Waypair Link".
  --bundle-id <id>       CFBundleIdentifier. Default: com.waypair.mac.
                         This script does not install, migrate data, or rotate keys.
  --version <x.y.z>      CFBundleShortVersionString. Default: 0.1.4 (workspace version).
  --build-version <n>    CFBundleVersion. Default: same as --version.
  --macdeployqt <path>   Explicit macdeployqt. Wins over all discovery.
  --qt-bin-dir <dir>     Directory containing macdeployqt (or set QT_BIN_DIR).
  --qml-dir <dir>        QML sources for macdeployqt -qmldir. Default: desktop/app/qml in this repo.
  --updater-helper <p>   Compiled WaypairUpdater (scripts/macos/WaypairUpdater.swift,
                         built/signed by Root). Bundled as its own app at
                         Contents/Helpers/WaypairUpdater.app, not beside the
                         host executable. Root links it with rpath
                         @executable_path/../../../../Frameworks.
  --sparkle-framework <p> Sparkle.framework to bundle. Default: SPARKLE_FRAMEWORK env
                         (read before this script assigns that name). Root supplies
                         the pinned framework path; no host-app search or download.
  --adb-helper <path>    Pre-signed pinned adb; requires --adb-seal and --adb-notice.
  --adb-seal <path>      Post-sign schema1 checksum manifest. Packaging verifies
                         bytes and copies it to Resources/PhoneSetup/adb-seal.json.
  --adb-notice <path>    Vendor notice copied into Open-source Licenses.
  --update-feed <url>    SUFeedURL written to Info.plist. Default: the separate Nectar
                         dedicated Nectar feed (NOT the legacy Swift appcast).
  --update-ed-key <key>  SUPublicEDKey written to Info.plist. Default: the existing
                         pinned Waypair Ed25519 public key.
  -h, --help             Show this help.

Discovery order for macdeployqt: --macdeployqt (explicit), then
--qt-bin-dir / QT_BIN_DIR, then MACDEPLOYQT env, then PATH, then
`qmake -query QT_INSTALL_BINS`.
Homebrew splits Qt modules (QtSvg lives in qtsvg, not qtbase). macdeployqt
only searches the libraries it infers from the binary unless given
-libpath. This script passes -libpath from the sibling qmake's
QT_INSTALL_LIBS from the selected pinned qmake toolchain.
No signing identity, keychain, entitlements file, or network is ever touched.
EOF
}

fail() { printf 'error: %s\n' "$*" >&2; exit 1; }

# Plist text escape that does not use bash ${var//pat/repl}. On bash 5.3 (and
# other bash builds that treat & in the replacement as the matched text) that
# form turns "&lt;" into "<lt;". macOS /bin/bash 3.2 does not. sed '\&' is a
# literal ampersand on both the system BSD sed and GNU sed.
xml_escape() {
    printf '%s' "$1" | sed \
        -e 's/&/\&amp;/g' \
        -e 's/</\&lt;/g' \
        -e 's/>/\&gt;/g' \
        -e 's/"/\&quot;/g' \
        -e "s/'/\\&apos;/g" \
        | tr -d '\n'
}

# Reverse-DNS bundle ids, including an explicit com.waypair.mac. The default
# assigned above stays the qualification id; this only checks shape.
bundle_id_ok() {
    [[ "$1" =~ ^[A-Za-z0-9.-]+$ ]]
}

# Explicit or externally configured pinned framework only; no host-app discovery.
select_sparkle_framework() {
    local explicit="$1"
    local from_env="$2"
    if [[ -n "$explicit" ]]; then
        printf '%s' "$explicit"
    elif [[ -n "$from_env" ]]; then
        printf '%s' "$from_env"
    fi
}

# Qt framework stems (QtCore, QtGui, ...) linked by a Mach-O executable.
linked_qt_framework_names() {
    local exe="$1"
    local inspection
    inspection="$(python3 "$MACHO_PROBE" otool -L "$exe")" || fail "Qt linkage inspection failed: $exe"
    printf '%s\n' "$inspection" | sed -n 's/.*\(Qt[[:alnum:]][[:alnum:]]*\)\.framework.*/\1/p' | sort -u
}

# names is a newline-separated list of those stems. Sparkle.framework is not
# one of them and must not satisfy the check by merely existing.
require_named_qt_frameworks() {
    local frameworks_dir="$1"
    local names="$2"
    local name
    if [[ -z "$names" ]]; then
        fail "no Qt framework is linked after macdeployqt. A Sparkle-only Contents/Frameworks directory does not count as a Qt deploy."
    fi
    while IFS= read -r name; do
        [[ -n "$name" ]] || continue
        if [[ ! -d "$frameworks_dir/${name}.framework" ]]; then
            fail "linked ${name}.framework is missing from ${frameworks_dir} after macdeployqt. Sparkle.framework does not satisfy Qt deployment."
        fi
    done <<< "$names"
}

# Add a search hint for Homebrew's split Qt prefixes. This does not fix the
# plugin pass on its own: build.rs declares Svg and the bundle framework rpath.
qt_install_libs_from_deployqt() {
    local deployqt="$1"
    local bindir qmake libs candidate
    bindir="$(cd "$(dirname "$deployqt")" && pwd)"
    qmake=""
    for candidate in "$bindir/qmake6" "$bindir/qmake"; do
        if [[ -x "$candidate" ]]; then
            qmake="$candidate"
            break
        fi
    done
    if [[ -z "$qmake" ]]; then
        if command -v qmake6 >/dev/null 2>&1; then
            qmake="$(command -v qmake6)"
        elif command -v qmake >/dev/null 2>&1; then
            qmake="$(command -v qmake)"
        fi
    fi
    [[ -n "$qmake" ]] || return 0
    libs="$("$qmake" -query QT_INSTALL_LIBS 2>/dev/null || true)"
    libs="$(printf '%s' "$libs" | tr -d '\r\n')"
    [[ -n "$libs" && -d "$libs" ]] || return 0
    printf '%s' "$libs"
}

is_macho_file() {
    local magic
    magic="$(od -An -t x1 -N 4 -- "$1" 2>/dev/null | tr -d ' \n' || true)"
    case "$magic" in
        cffaedfe|cefaedfe|feedfacf|feedface|cafebabe|bebafeca) return 0 ;;
        *) return 1 ;;
    esac
}

# Absolute library paths other than the OS. Toolchain-specific prefixes
# (including Cellar and opt kegs) must not remain after deployment.
absolute_non_system_ref() {
    local ref="$1"
    [[ "$ref" == /* ]] || return 1
    case "$ref" in
        /System/*|/usr/lib/*|/usr/libexec/*) return 1 ;;
        *) return 0 ;;
    esac
}

enclosing_macos_dir() {
    local path="$1"
    local fallback="${2:-}"
    local d
    d="$(cd "$(dirname "$path")" && pwd -P)"
    while [[ "$d" != "/" ]]; do
        if [[ "$(basename "$d")" == *.app ]]; then
            printf '%s/Contents/MacOS' "$d"
            return 0
        fi
        d="$(dirname "$d")"
    done
    if [[ -n "$fallback" ]]; then
        printf '%s/Contents/MacOS' "$fallback"
        return 0
    fi
    return 1
}

existing_file_path() {
    local path="$1"
    local dir base
    dir="$(dirname "$path")"
    base="$(basename "$path")"
    [[ -f "$dir/$base" ]] || return 1
    dir="$(cd "$dir" && pwd -P)" || return 1
    printf '%s/%s' "$dir" "$base"
}

path_inside_dir() {
    local root="$1"
    local path="$2"
    [[ "$path" == "$root" || "$path" == "$root"/* ]]
}

# LC_RPATH of this image, then of the main executable in the nearest .app.
# dyld walks that chain for @rpath. An rpath is only useful here when the
# substituted file lands inside the bundle; the external toolchain prefix does not.
closure_rpaths() {
    local binary="$1"
    local bundle="$2"
    local macos exe inspection
    inspection="$(python3 "$MACHO_PROBE" otool -l "$binary")" || fail "rpath inspection failed: $binary"
    printf '%s\n' "$inspection" | awk '$1 == "cmd" && $2 == "LC_RPATH" { want = 1; next } want && $1 == "path" { print $2; want = 0 }'
    macos="$(enclosing_macos_dir "$binary" "$bundle")"
    [[ -n "$macos" && -d "$macos" ]] || return 0
    for exe in "$macos"/*; do
        [[ -f "$exe" && ! -L "$exe" ]] || continue
        [[ "$exe" -ef "$binary" ]] && continue
        is_macho_file "$exe" || continue
        inspection="$(python3 "$MACHO_PROBE" otool -l "$exe")" || fail "rpath inspection failed: $exe"
        printf '%s\n' "$inspection" | awk '$1 == "cmd" && $2 == "LC_RPATH" { want = 1; next } want && $1 == "path" { print $2; want = 0 }'
    done
}

expand_rpath_prefix() {
    local binary="$1"
    local rpath="$2"
    local bundle="$3"
    local loader exe
    loader="$(dirname "$binary")"
    exe="$(enclosing_macos_dir "$binary" "$bundle")"
    case "$rpath" in
        @loader_path) printf '%s' "$loader" ;;
        @loader_path/*) printf '%s/%s' "$loader" "${rpath#@loader_path/}" ;;
        @executable_path) printf '%s' "$exe" ;;
        @executable_path/*) printf '%s/%s' "$exe" "${rpath#@executable_path/}" ;;
        /*) printf '%s' "$rpath" ;;
        *) printf '%s/%s' "$loader" "$rpath" ;;
    esac
}

# Success when ref names a file inside the bundle. System absolute paths are
# the caller's responsibility.
ref_resolves_inside_bundle() {
    local binary="$1"
    local ref="$2"
    local bundle="$3"
    local tail rpath prefix candidate inspected_rpaths
    case "$ref" in
        @executable_path/*)
            prefix="$(enclosing_macos_dir "$binary" "$bundle")"
            candidate="$(existing_file_path "$prefix/${ref#@executable_path/}")" || return 1
            path_inside_dir "$bundle" "$candidate"
            ;;
        @loader_path/*)
            candidate="$(existing_file_path "$(dirname "$binary")/${ref#@loader_path/}")" || return 1
            path_inside_dir "$bundle" "$candidate"
            ;;
        @rpath/*)
            tail="${ref#@rpath/}"
            inspected_rpaths="$(closure_rpaths "$binary" "$bundle")" || fail "rpath-chain inspection failed: $binary"
            while IFS= read -r rpath; do
                [[ -n "$rpath" ]] || continue
                prefix="$(expand_rpath_prefix "$binary" "$rpath" "$bundle")"
                candidate="$(existing_file_path "$prefix/$tail")" || continue
                if path_inside_dir "$bundle" "$candidate"; then
                    return 0
                fi
            done <<< "$inspected_rpaths"
            return 1
            ;;
        *)
            return 1
            ;;
    esac
}

# Every Mach-O under the bundle must load only system libraries or files
# inside the bundle. A plugin @rpath/QtSvg that macdeployqt could not rewrite
# fails here, as does a load command that still names the host toolchain prefix. The
# library's own install id is not a load edge (see the caller). @rpath counts
# only when LC_RPATH substitution finds the file inside the bundle.
require_bundle_closure() {
    local bundle="$1"
    local root f rel ref ids inspection inspected_dependencies
    root="$(cd "$bundle" && pwd -P)"
    while IFS= read -r -d '' f; do
        is_macho_file "$f" || continue
        rel="${f#"$root"/}"
        # otool -L repeats LC_ID_DYLIB. That id is not a load edge: a copied
        # A framework may keep its build-prefix path as its own id while
        # dependents use @executable_path or a bundle-relative @rpath.
        inspection="$(python3 "$MACHO_PROBE" otool -D "$f")" || fail "install-id inspection failed: $f"
        ids="$(printf '%s\n' "$inspection" | awk 'NR > 1 && $0 !~ /:$/ { print }')"
        inspection="$(python3 "$MACHO_PROBE" otool -L "$f")" || fail "dependency inspection failed: $f"
        inspected_dependencies="$(printf '%s\n' "$inspection" | sed -n $'s/^\t//p' | sed 's/ (compatibility version.*$//; s/ (architecture.*$//')"
        while IFS= read -r ref; do
            [[ -n "$ref" ]] || continue
            if [[ -n "$ids" ]] && printf '%s\n' "$ids" | grep -F -x -q -- "$ref"; then
                continue
            fi
            if absolute_non_system_ref "$ref"; then
                fail "bundle is not closed after macdeployqt: ${rel} loads ${ref}. A Homebrew absolute library path is not a bundled framework."
            fi
            case "$ref" in
                @executable_path/*|@loader_path/*|@rpath/*)
                    if ! ref_resolves_inside_bundle "$f" "$ref" "$root"; then
                        fail "bundle is not closed after macdeployqt: ${rel} depends on ${ref}, which does not resolve to a file inside the bundle. Missing plugin or transitive Qt frameworks fail the deploy."
                    fi
                    ;;
            esac
        done <<< "$inspected_dependencies"
    done < <(find "$root" -type f -print0)
}

# From Contents/Helpers/WaypairUpdater.app/Contents/MacOS, four levels up is
# the host's Contents/. Root sets this rpath at compile time. This script
# does not rewrite load commands.
helper_sparkle_rpath() {
    printf '%s' '@executable_path/../../../../Frameworks'
}

# Distinct from the host so Sparkle's installer does not match the helper
# when it looks up the host by bundle id and path.
helper_bundle_id() {
    printf '%s' "$1.updater"
}

write_helper_info_plist() {
    local dest="$1"
    local host_id="$2"
    local helper_id
    helper_id="$(xml_escape "$(helper_bundle_id "$host_id")")"
    cat > "$dest" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleIdentifier</key>
    <string>${helper_id}</string>
    <key>CFBundleName</key>
    <string>WaypairUpdater</string>
    <key>CFBundleExecutable</key>
    <string>WaypairUpdater</string>
    <key>CFBundlePackageType</key>
    <string>APPL</string>
    <key>CFBundleInfoDictionaryVersion</key>
    <string>6.0</string>
    <key>LSUIElement</key>
    <true/>
    <!-- One user-started check. This helper does not poll. -->
    <key>SUEnableAutomaticChecks</key>
    <false/>
</dict>
</plist>
EOF
}

require_helper_sparkle_rpath() {
    local exe="$1"
    local expected inspection
    expected="$(helper_sparkle_rpath)"
    inspection="$(python3 "$MACHO_PROBE" otool -l "$exe")" || fail "updater rpath inspection failed: $exe"
    printf '%s\n' "$inspection" | grep -F -q "$expected" || fail "updater helper is not linked with rpath ${expected}. Root compiles that rpath; this script does not rewrite it or sign the helper."
}

install_updater_helper() {
    local app_path="$1"
    local helper_bin="$2"
    local host_id="$3"
    local helper_app="$app_path/Contents/Helpers/WaypairUpdater.app"
    local helper_exe="$helper_app/Contents/MacOS/WaypairUpdater"
    mkdir -p "$helper_app/Contents/MacOS" || fail "failed to create the nested updater app"
    cp "$helper_bin" "$helper_exe" || fail "failed to copy the updater helper into the nested app"
    chmod +x "$helper_exe"
    write_helper_info_plist "$helper_app/Contents/Info.plist" "$host_id"
    require_helper_sparkle_rpath "$helper_exe"
}

# --- macOS Services (docs/waypair/MAC_CAPTURE_SERVICES.md) ---
# The two public NSServices declared in the host Info.plist. Both share
# the provider method the app registers (`waypairCapture`), matching the
# `waypairCapture:userData:error:` selector implemented in
# desktop/app/src/mac/services.rs; `NSUserData` selects photo vs scan.
# The displayed names lead with the app name, so a qualification bundle
# (--app-name "Waypair Link (Qualification)") lists distinct Services
# from a release install. --app-name is validated to contain no '/', so
# no displayed name can contain one either (AppKit would drop the text
# before a slash). Only the public Services mechanism is used: no
# private Continuity Camera registration, no extra entitlement.

service_photo_menu() { printf '%s: Take Photo with Phone' "$1"; }
service_scan_menu() { printf '%s: Scan Document with Phone' "$1"; }

# Escape for a quoted .strings value: backslash first, then quotes.
strings_escape() {
    printf '%s' "$1" | sed -e 's/\\/\\\\/g' -e 's/"/\\"/g' | tr -d '\n'
}

# ServicesMenu.strings for en and zh_CN, keyed by the (English) menu
# item defaults, as AppKit looks them up. Values lead with the app name
# in both languages for the same qualification-distinctness reason.
write_services_menu_strings() {
    local resources="$1"
    local app_name="$2"
    local photo_en scan_en photo_zh scan_zh
    photo_en="$(strings_escape "$(service_photo_menu "$app_name")")"
    scan_en="$(strings_escape "$(service_scan_menu "$app_name")")"
    photo_zh="$(strings_escape "${app_name}：用手机拍照")"
    scan_zh="$(strings_escape "${app_name}：用手机扫描文档")"
    mkdir -p "$resources/en.lproj" "$resources/zh_CN.lproj" || fail "failed to create Services localization directories"
    cat > "$resources/en.lproj/ServicesMenu.strings" <<EOF
/* Services menu items (NSServices NSMenuItem defaults). */
"$photo_en" = "$photo_en";
"$scan_en" = "$scan_en";
EOF
    cat > "$resources/zh_CN.lproj/ServicesMenu.strings" <<EOF
/* Services menu items (NSServices NSMenuItem defaults). */
"$photo_en" = "$photo_zh";
"$scan_en" = "$scan_zh";
EOF
}

# Sourced for the scratch checks. Executing the file still packages.
if [[ "${BASH_SOURCE[0]}" != "$0" ]]; then
    return 0
fi

while [[ $# -gt 0 ]]; do
    case "$1" in
        --binary)       [[ $# -ge 2 ]] || fail "--binary needs a value"; BINARY="$2"; shift 2 ;;
        --output-dir)   [[ $# -ge 2 ]] || fail "--output-dir needs a value"; OUTPUT_DIR="$2"; shift 2 ;;
        --app-name)     [[ $# -ge 2 ]] || fail "--app-name needs a value"; APP_NAME="$2"; shift 2 ;;
        --bundle-id)    [[ $# -ge 2 ]] || fail "--bundle-id needs a value"; BUNDLE_ID="$2"; shift 2 ;;
        --version)      [[ $# -ge 2 ]] || fail "--version needs a value"; VERSION="$2"; shift 2 ;;
        --build-version) [[ $# -ge 2 ]] || fail "--build-version needs a value"; BUILD_VERSION="$2"; shift 2 ;;
        --macdeployqt)  [[ $# -ge 2 ]] || fail "--macdeployqt needs a value"; MACDEPLOYQT_ARG="$2"; shift 2 ;;
        --qt-bin-dir)   [[ $# -ge 2 ]] || fail "--qt-bin-dir needs a value"; QT_BIN_DIR="$2"; shift 2 ;;
        --qml-dir)      [[ $# -ge 2 ]] || fail "--qml-dir needs a value"; QML_DIR="$2"; shift 2 ;;
        --updater-helper) [[ $# -ge 2 ]] || fail "--updater-helper needs a value"; UPDATER_HELPER="$2"; shift 2 ;;
        --sparkle-framework) [[ $# -ge 2 ]] || fail "--sparkle-framework needs a value"; SPARKLE_FRAMEWORK_ARG="$2"; shift 2 ;;
        --adb-helper) [[ $# -ge 2 ]] || fail "--adb-helper needs a value"; ADB_HELPER="$2"; shift 2 ;;
        --adb-seal) [[ $# -ge 2 ]] || fail "--adb-seal needs a value"; ADB_SEAL="$2"; shift 2 ;;
        --adb-notice) [[ $# -ge 2 ]] || fail "--adb-notice needs a value"; ADB_NOTICE="$2"; shift 2 ;;
        --notices-dir) [[ $# -ge 2 ]] || fail "--notices-dir needs a value"; NOTICES_DIR="$2"; shift 2 ;;
        --update-feed)  [[ $# -ge 2 ]] || fail "--update-feed needs a value"; UPDATE_FEED="$2"; shift 2 ;;
        --update-ed-key) [[ $# -ge 2 ]] || fail "--update-ed-key needs a value"; UPDATE_ED_KEY="$2"; shift 2 ;;
        -h|--help)      usage; exit 0 ;;
        *)              fail "unknown argument: $1 (try --help)" ;;
    esac
done

[[ -n "$BINARY" ]]     || { usage >&2; fail "--binary is required"; }
[[ -n "$OUTPUT_DIR" ]] || { usage >&2; fail "--output-dir is required"; }
[[ -f "$BINARY" ]]     || fail "input binary not found: $BINARY (build it first; this script never compiles)"
[[ -x "$BINARY" ]]     || fail "input binary is not executable: $BINARY"

# --- Validate every value that becomes a path component or plist text ---
# App name becomes a single directory name: never a path, never empty/dot, never
# carrying control characters (including newlines).
[[ -n "$APP_NAME" ]] || fail "--app-name must not be empty"
[[ "$APP_NAME" != "." && "$APP_NAME" != ".." ]] || fail "invalid --app-name: $APP_NAME"
[[ "$APP_NAME" != */* ]] || fail "invalid --app-name (must be a single filename component, no '/'): $APP_NAME"
[[ "$APP_NAME" != *[[:cntrl:]]* ]] || fail "invalid --app-name (control characters are not allowed)"
bundle_id_ok "$BUNDLE_ID" || fail "invalid --bundle-id: $BUNDLE_ID"
# CFBundleShortVersionString: numeric major.minor[.patch]. CFBundleVersion:
# non-negative integers separated by dots (Apple's numeric build format).
[[ "$VERSION" =~ ^[0-9]+\.[0-9]+(\.[0-9]+)?$ ]] || fail "invalid --version (expected numeric x.y or x.y.z): $VERSION"
[[ -n "$BUILD_VERSION" ]] || BUILD_VERSION="$VERSION"
[[ "$BUILD_VERSION" =~ ^[0-9]+(\.[0-9]+){0,2}$ ]] || fail "invalid --build-version (expected dot-separated integers): $BUILD_VERSION"

[[ "$UPDATE_FEED" == https://* && "$UPDATE_FEED" != *[[:cntrl:]\<\>\"\'\ ]* ]] || fail "invalid --update-feed (expected a clean https URL): $UPDATE_FEED"
[[ "$UPDATE_ED_KEY" =~ ^[A-Za-z0-9+/]{43}=$ ]] || fail "invalid --update-ed-key (expected the 44-character base64 Ed25519 public key)"
if [[ -n "$UPDATER_HELPER" ]]; then
    [[ -f "$UPDATER_HELPER" && -x "$UPDATER_HELPER" ]] || fail "updater helper not found or not executable: $UPDATER_HELPER (Root compiles scripts/macos/WaypairUpdater.swift; this script never compiles)"
fi
# Verify the post-sign helper bytes before creating an output bundle. The
# runtime validates this same manifest before spawning; packaging never signs
# or runs adb and must not mutate its already-sealed bytes.
if [[ -n "$ADB_HELPER$ADB_SEAL$ADB_NOTICE" ]]; then
    [[ -n "$ADB_HELPER" && -n "$ADB_SEAL" && -n "$ADB_NOTICE" ]] || fail "ADB inputs must be supplied together"
    python3 - "$ADB_HELPER" "$ADB_SEAL" "$ADB_NOTICE" <<'PYADB'
import hashlib, json, os, re, sys
from pathlib import Path
helper, seal, notice = map(Path, sys.argv[1:])
for path in (helper, seal, notice):
    if path.is_symlink() or not path.is_file():
        raise SystemExit("ADB input must be a regular non-symlink file")
if not os.access(helper, os.X_OK) or seal.stat().st_size > 4096 or not 0 < notice.stat().st_size <= 4 * 1024 * 1024:
    raise SystemExit("invalid ADB input size or executable mode")
manifest = json.loads(seal.read_text())
if (set(manifest) != {"schema", "relativePath", "sha256", "size"}
    or type(manifest.get("schema")) is not int or manifest["schema"] != 1
    or manifest.get("relativePath") != "Helpers/adb"
    or type(manifest.get("size")) is not int or not 0 < manifest["size"] <= 64 * 1024 * 1024
    or manifest["size"] != helper.stat().st_size
    or not re.fullmatch(r"[0-9a-f]{64}", manifest.get("sha256", ""))
    or hashlib.sha256(helper.read_bytes()).hexdigest() != manifest["sha256"]):
    raise SystemExit("ADB seal mismatch")
PYADB
fi
# Save the incoming environment before any assignment to SPARKLE_FRAMEWORK.
# Clearing that name first made the env step unreachable.
if [[ -n "$UPDATER_HELPER" ]]; then
    SPARKLE_FRAMEWORK_ENV="${SPARKLE_FRAMEWORK:-}"
    SPARKLE_FRAMEWORK="$(select_sparkle_framework "$SPARKLE_FRAMEWORK_ARG" "$SPARKLE_FRAMEWORK_ENV")"
    [[ -n "$SPARKLE_FRAMEWORK" && -d "$SPARKLE_FRAMEWORK" ]] || fail "Sparkle.framework not found. Pass --sparkle-framework or set SPARKLE_FRAMEWORK; nothing is downloaded. Updater cannot be bundled without it."
fi

# --- Discover macdeployqt (never assumed, never downloaded) ---
# Documented order: explicit --macdeployqt > --qt-bin-dir/QT_BIN_DIR > MACDEPLOYQT
# env > PATH > qmake query. Preserve the incoming env value separately: resetting
# MACDEPLOYQT first (as an earlier revision did) silently destroyed this step.
MACDEPLOYQT_ENV="${MACDEPLOYQT:-}"
MACDEPLOYQT=""
if [[ -n "$MACDEPLOYQT_ARG" ]]; then
    MACDEPLOYQT="$MACDEPLOYQT_ARG"
elif [[ -n "$QT_BIN_DIR" && -x "$QT_BIN_DIR/macdeployqt" ]]; then
    MACDEPLOYQT="$QT_BIN_DIR/macdeployqt"
elif [[ -n "$MACDEPLOYQT_ENV" ]]; then
    MACDEPLOYQT="$MACDEPLOYQT_ENV"
elif command -v macdeployqt >/dev/null 2>&1; then
    MACDEPLOYQT="$(command -v macdeployqt)"
elif command -v qmake >/dev/null 2>&1; then
    _bins="$(qmake -query QT_INSTALL_BINS 2>/dev/null || true)"
    [[ -n "$_bins" && -x "$_bins/macdeployqt" ]] && MACDEPLOYQT="$_bins/macdeployqt"
fi
[[ -n "$MACDEPLOYQT" ]] || fail "macdeployqt not found. Install Qt (repo CI pins Qt 6.12.0) or pass --macdeployqt/--qt-bin-dir. Nothing was created."
[[ -x "$MACDEPLOYQT" ]] || fail "macdeployqt is not executable: $MACDEPLOYQT"

[[ -n "$QML_DIR" ]] || QML_DIR="$REPO_ROOT/desktop/app/qml"

# GPL-3.0 (apps) / MPL-2.0 (core) notice + source requirement material.
[[ -f "$REPO_ROOT/LICENSE" ]]   || fail "LICENSE not found at repo root: $REPO_ROOT (license texts must travel with the bundle)"
[[ -d "$REPO_ROOT/LICENSES" ]] || fail "LICENSES/ not found at repo root: $REPO_ROOT (license texts must travel with the bundle)"
# The fonts are compiled into the binary; their licenses ship as files.
FONT_LICENSES=(OFL-Figtree.txt OFL-InstrumentSerif.txt OFL-SpaceMono.txt)
for ofl in "${FONT_LICENSES[@]}"; do
    ofl_path="$REPO_ROOT/assets/fonts/$ofl"
    [[ -f "$ofl_path" && ! -L "$ofl_path" && -s "$ofl_path" ]] || fail "font license missing: assets/fonts/$ofl"
    grep -q 'SIL OPEN FONT LICENSE' "$ofl_path" || fail "assets/fonts/$ofl is not an SIL Open Font License text"
done
[[ -n "$NOTICES_DIR" ]] || fail "--notices-dir is required: third-party notices must travel with the bundle (make them with scripts/macos/prepare-notices.py prepare; nothing is generated or downloaded here)"
[[ -d "$NOTICES_DIR" && ! -L "$NOTICES_DIR" ]] || fail "notices directory not found: $NOTICES_DIR"
python3 "$REPO_ROOT/scripts/macos/prepare-notices.py" verify "$NOTICES_DIR" >/dev/null || fail "prepared notices failed verification: $NOTICES_DIR"

APP_PATH="$OUTPUT_DIR/$APP_NAME.app"
# -e alone misses a dangling symlink at the output path; refuse on either.
if [[ -e "$APP_PATH" || -L "$APP_PATH" ]]; then
    fail "output collision: $APP_PATH already exists (file, directory, or symlink). Remove it yourself or choose another --output-dir/--app-name; refusing to overwrite."
fi

mkdir -p "$APP_PATH/Contents/MacOS" "$APP_PATH/Contents/Resources"

ICON_PATH="$REPO_ROOT/assets/brand/waypair/AppIcon-monochrome.icns"
[[ -f "$ICON_PATH" ]] || fail "Waypair Link monochrome app icon is missing: $ICON_PATH"
cp "$ICON_PATH" "$APP_PATH/Contents/Resources/AppIcon.icns"

EXEC_NAME="nectarlink-desktop"
cp "$BINARY" "$APP_PATH/Contents/MacOS/$EXEC_NAME"
chmod +x "$APP_PATH/Contents/MacOS/$EXEC_NAME"

PLIST_UPDATE_FEED="$(xml_escape "$UPDATE_FEED")"
PLIST_BUNDLE_ID="$(xml_escape "$BUNDLE_ID")"
PLIST_APP_NAME="$(xml_escape "$APP_NAME")"
PLIST_PHOTO_MENU="$(xml_escape "$(service_photo_menu "$APP_NAME")")"
PLIST_SCAN_MENU="$(xml_escape "$(service_scan_menu "$APP_NAME")")"
PLIST_VERSION="$(xml_escape "$VERSION")"
PLIST_BUILD_VERSION="$(xml_escape "$BUILD_VERSION")"
cat > "$APP_PATH/Contents/Info.plist" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleIdentifier</key>
    <string>$PLIST_BUNDLE_ID</string>
    <key>CFBundleName</key>
    <string>$PLIST_APP_NAME</string>
    <key>CFBundleDisplayName</key>
    <string>$PLIST_APP_NAME</string>
    <key>CFBundleExecutable</key>
    <string>$EXEC_NAME</string>
    <key>CFBundleShortVersionString</key>
    <string>$PLIST_VERSION</string>
    <key>CFBundleVersion</key>
    <string>$PLIST_BUILD_VERSION</string>
    <key>CFBundlePackageType</key>
    <string>APPL</string>
    <key>CFBundleInfoDictionaryVersion</key>
    <string>6.0</string>
    <key>NSPrincipalClass</key>
    <string>NSApplication</string>
    <key>CFBundleIconFile</key>
    <string>AppIcon</string>
    <key>LSMinimumSystemVersion</key>
    <string>14.4</string>
    <key>NSHighResolutionCapable</key>
    <true/>
    <!-- Actually needed: core config lan_discovery announces/discovers via
         mDNS service "nectarlink" (MdnsAddressLookup::builder().service_name("nectarlink"),
         i.e. _nectarlink._udp). No camera key is declared. -->
    <key>NSLocalNetworkUsageDescription</key>
    <string>$PLIST_APP_NAME finds and connects to your paired phone on your local network.</string>
    <!-- PC call audio (desktop/app/src/mac/bt_hfp.rs): only after the user
         starts it during a call, the Mac serves an already-paired phone as
         a Bluetooth hands-free unit. Without these keys the app never
         touches Bluetooth and reports call audio unavailable. -->
    <key>NSBluetoothAlwaysUsageDescription</key>
    <string>$PLIST_APP_NAME uses Bluetooth to bring a phone call's audio to this Mac when you ask it to.</string>
    <key>NSMicrophoneUsageDescription</key>
    <string>$PLIST_APP_NAME uses the microphone for a phone call you move to this Mac.</string>
    <key>NSBonjourServices</key>
    <array>
        <string>_nectarlink._udp</string>
    </array>
    <!-- Sparkle reads these from the host bundle. The helper is the nested
         Contents/Helpers/WaypairUpdater.app, not this executable. Dedicated
         Nectar feed; the pinned key is the existing Waypair Ed25519 public
         key, unchanged. Automatic checks stay off: the helper is one
         user-started check and does not poll. -->
    <key>SUFeedURL</key>
    <string>$PLIST_UPDATE_FEED</string>
    <key>SUPublicEDKey</key>
    <string>$UPDATE_ED_KEY</string>
    <key>SUEnableAutomaticChecks</key>
    <false/>
    <!-- Public macOS Services (desktop/app/src/mac/services.rs registers
         the provider at launch). NSPortName must equal CFBundleName.
         Image-returning only (PNG/TIFF, no NSSendTypes), so AppKit
         enables these only in hosts that accept returned images;
         plain-text and non-adopting hosts keep them disabled. The
         provider waits at most 90 s of the 95 s NSTimeout and never
         falls back to the general clipboard. -->
    <key>NSServices</key>
    <array>
        <dict>
            <key>NSMenuItem</key>
            <dict>
                <key>default</key>
                <string>$PLIST_PHOTO_MENU</string>
            </dict>
            <key>NSMessage</key>
            <string>waypairCapture</string>
            <key>NSPortName</key>
            <string>$PLIST_APP_NAME</string>
            <key>NSRequiredContext</key>
            <dict/>
            <key>NSReturnTypes</key>
            <array>
                <string>public.png</string>
                <string>public.tiff</string>
            </array>
            <key>NSServiceDescription</key>
            <string>Take a photo with your paired phone</string>
            <key>NSTimeout</key>
            <string>95000</string>
            <key>NSUserData</key>
            <string>photo</string>
        </dict>
        <dict>
            <key>NSMenuItem</key>
            <dict>
                <key>default</key>
                <string>$PLIST_SCAN_MENU</string>
            </dict>
            <key>NSMessage</key>
            <string>waypairCapture</string>
            <key>NSPortName</key>
            <string>$PLIST_APP_NAME</string>
            <key>NSRequiredContext</key>
            <dict/>
            <key>NSReturnTypes</key>
            <array>
                <string>public.png</string>
                <string>public.tiff</string>
            </array>
            <key>NSServiceDescription</key>
            <string>Scan a document with your paired phone</string>
            <key>NSTimeout</key>
            <string>95000</string>
            <key>NSUserData</key>
            <string>scan</string>
        </dict>
    </array>
</dict>
</plist>
EOF

# Localized Services menu names (keys are the NSMenuItem defaults above).
write_services_menu_strings "$APP_PATH/Contents/Resources" "$APP_NAME"

# License/notice material inside the bundle (GPL/MPL compliance, not a substitute
# for the corresponding-source offer documented in MACOS_PACKAGING.md).
cp "$REPO_ROOT/LICENSE" "$APP_PATH/Contents/Resources/LICENSE.txt" || fail "failed to copy LICENSE into the bundle"
mkdir -p "$APP_PATH/Contents/Resources/LICENSES"
# Copy ALL of LICENSES/ (every license text and the app-store exception), and
# let any copy error abort the packaging (set -e): notices must not be
# silently dropped.
cp -R "$REPO_ROOT/LICENSES/." "$APP_PATH/Contents/Resources/LICENSES/" || fail "failed to copy LICENSES/ into the bundle"
for ofl in "${FONT_LICENSES[@]}"; do
    cp "$REPO_ROOT/assets/fonts/$ofl" "$APP_PATH/Contents/Resources/LICENSES/$ofl" || fail "failed to copy $ofl into the bundle"
done
for entry in "$NOTICES_DIR"/*; do
    [[ ! -e "$APP_PATH/Contents/Resources/LICENSES/$(basename "$entry")" ]] || fail "prepared notice $(basename "$entry") would overwrite a repository license file"
done
cp -R "$NOTICES_DIR/." "$APP_PATH/Contents/Resources/LICENSES/" || fail "failed to copy the prepared notices into the bundle"

# Nested helper app, so LaunchServices and Sparkle see a different bundle id
# and path from the host. Sparkle.framework stays in the host's
# Contents/Frameworks. The helper's rpath reaches that directory; this script
# does not rewrite it.
if [[ -n "$UPDATER_HELPER" ]]; then
    install_updater_helper "$APP_PATH" "$UPDATER_HELPER" "$BUNDLE_ID"
    mkdir -p "$APP_PATH/Contents/Frameworks"
    cp -R "$SPARKLE_FRAMEWORK" "$APP_PATH/Contents/Frameworks/Sparkle.framework" || fail "failed to copy Sparkle.framework into the bundle"
    printf 'Bundled nested Sparkle helper: %s (framework stays at Contents/Frameworks: %s)\n' "$UPDATER_HELPER" "$SPARKLE_FRAMEWORK"
fi

# Bundle Qt frameworks/plugins. -no-codesign: this script never signs; any
# production signature is applied later by Root's release process.
# The extra library search hint does not replace the executable's declared
# Qt dependencies or bundle-relative framework rpath (both set in build.rs).
DEPLOY_ARGS=("$APP_PATH" -no-codesign)
qt_libs="$(qt_install_libs_from_deployqt "$MACDEPLOYQT")"
if [[ -n "$qt_libs" ]]; then
    DEPLOY_ARGS+=(-libpath="$qt_libs")
else
    printf 'warning: QT_INSTALL_LIBS not found beside %s; macdeployqt will run without -libpath\n' "$MACDEPLOYQT" >&2
fi
[[ -d "$QML_DIR" ]] && DEPLOY_ARGS+=(-qmldir="$QML_DIR")
printf 'Running: %s %s\n' "$MACDEPLOYQT" "${DEPLOY_ARGS[*]}"
deploy_log="$(mktemp "${TMPDIR:-/tmp}/waypair-macdeployqt.XXXXXX")"
set +e
"$MACDEPLOYQT" "${DEPLOY_ARGS[@]}" >"$deploy_log" 2>&1
deploy_status=$?
set -e
cat "$deploy_log"
if [[ "$deploy_status" -ne 0 ]]; then
    rm -f "$deploy_log"
    fail "macdeployqt failed; bundle left in place for inspection: $APP_PATH"
fi
if grep -E -q '^ERROR:' "$deploy_log"; then
    rm -f "$deploy_log"
    fail "macdeployqt reported ERROR but exited 0. Refusing to claim the bundle is deployed. Left in place for inspection: $APP_PATH"
fi
rm -f "$deploy_log"

# Add the pre-signed helper after macdeployqt: deployment must never rewrite
# its signed bytes. Root's inside-out signer verifies and preserves this file.
if [[ -n "$ADB_HELPER" ]]; then
    mkdir -p "$APP_PATH/Contents/Helpers" "$APP_PATH/Contents/Resources/PhoneSetup"
    cp -p "$ADB_HELPER" "$APP_PATH/Contents/Helpers/adb"
    cp "$ADB_SEAL" "$APP_PATH/Contents/Resources/PhoneSetup/adb-seal.json"
    cp "$ADB_NOTICE" "$APP_PATH/Contents/Resources/LICENSES/ADB-NOTICE.txt"
fi

# The executable's own Qt linkage, not the mere presence of Contents/Frameworks.
# Copying Sparkle.framework creates that directory before macdeployqt runs.
qt_names="$(linked_qt_framework_names "$APP_PATH/Contents/MacOS/$EXEC_NAME")" || fail "Qt linkage inspection failed"
require_named_qt_frameworks "$APP_PATH/Contents/Frameworks" "$qt_names"
require_bundle_closure "$APP_PATH"
# Every Mach-O except the app's own executables needs a notice. This also
# writes Resources/LICENSES/BUNDLED-COMPONENTS.txt, before signing.
python3 "$REPO_ROOT/scripts/macos/prepare-notices.py" check-bundle "$NOTICES_DIR" "$APP_PATH" >/dev/null || fail "bundle notices are incomplete; bundle left in place for inspection: $APP_PATH"
# Derive the supported OS from every actual bundled architecture, never from
# the compiler host or a stale fixed deployment promise. Run before signing.
python3 "$(dirname "${BASH_SOURCE[0]}")/record-minimum-os.py" "$APP_PATH" || fail "minimum-OS inspection failed"

printf '\nUnsigned bundle assembled (NOT signed, NOT notarized, not installed):\n  %s\n' "$APP_PATH"
printf 'Identifier: %s  Version: %s (%s)\n' "$BUNDLE_ID" "$VERSION" "$BUILD_VERSION"
printf 'Executable SHA-256: %s\n' "$(shasum -a 256 "$APP_PATH/Contents/MacOS/$EXEC_NAME" | awk '{print $1}')"
printf 'Next gates (Root-owned): production signing/hardened runtime, Keychain protector, R2 updater, qualification. See docs/waypair/MACOS_PACKAGING.md\n'
