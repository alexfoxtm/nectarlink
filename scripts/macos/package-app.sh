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
# macdeployqt, an existing bundle at the output path, or missing license
# texts (GPL/MPL source-and-notice material must travel with the bundle).
set -euo pipefail

BINARY=""
OUTPUT_DIR=""
APP_NAME="Waypair Link"
BUNDLE_ID="com.waypair.link.nectar-qualification"
VERSION="0.1.4"
BUILD_VERSION=""
MACDEPLOYQT_ARG=""
QT_BIN_DIR="${QT_BIN_DIR:-}"
QML_DIR=""
UPDATER_HELPER=""
SPARKLE_FRAMEWORK_ARG=""
# Dedicated Nectar feed (Alex-authorized, Root-published): never the legacy
# Swift appcast, whose items must not offer themselves to this app; Sparkle
# also only installs versions newer than the host bundle's CFBundleVersion.
UPDATE_FEED="https://pub-a57457f4da43415ea6335550ee9e2983.r2.dev/waypair/nectar/macos/stable/appcast.xml"
UPDATE_ED_KEY="uMzztqVF0M7BMYkWsL48ZJDvSxrqoVNSvqBKxYdu8ss="

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"

usage() {
    cat <<'EOF'
Usage: scripts/macos/package-app.sh --binary <nectarlink-desktop> --output-dir <dir> [options]

Assemble a minimal UNSIGNED .app from an already-compiled binary. Nothing is
built, signed, installed, or launched.

Required:
  --binary <path>        Existing compiled nectarlink-desktop executable.
  --output-dir <dir>     Directory to create <App Name>.app in (created if absent).
                         Fails if the .app already exists there.

Options:
  --app-name <name>      App/display name. Default: "Waypair Link".
  --bundle-id <id>       CFBundleIdentifier. Default: com.waypair.link.nectar-qualification.
                         The default stays distinct from an installed legacy app.
                         Explicit --bundle-id com.waypair.mac is allowed for the
                         final replacement build. This script does not install,
                         migrate data, or rotate keys.
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
                         (read before this script assigns that name), else
                         /Applications/Waypair.app/Contents/Frameworks/Sparkle.framework
                         when present (verified Sparkle 2.10.0 source; never downloaded).
  --update-feed <url>    SUFeedURL written to Info.plist. Default: the separate Nectar
                         dedicated Nectar feed (NOT the legacy Swift appcast).
  --update-ed-key <key>  SUPublicEDKey written to Info.plist. Default: the existing
                         pinned Waypair Ed25519 public key.
  -h, --help             Show this help.

Discovery order for macdeployqt: --macdeployqt (explicit), then
--qt-bin-dir / QT_BIN_DIR, then MACDEPLOYQT env, then PATH, then
`qmake -query QT_INSTALL_BINS`.
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

# explicit (--sparkle-framework) wins, then the SPARKLE_FRAMEWORK value saved
# before this script assigns that name, then a local Waypair.app copy.
select_sparkle_framework() {
    local explicit="$1"
    local from_env="$2"
    if [[ -n "$explicit" ]]; then
        printf '%s' "$explicit"
    elif [[ -n "$from_env" ]]; then
        printf '%s' "$from_env"
    elif [[ -d "/Applications/Waypair.app/Contents/Frameworks/Sparkle.framework" ]]; then
        printf '%s' "/Applications/Waypair.app/Contents/Frameworks/Sparkle.framework"
    fi
}

# Qt framework stems (QtCore, QtGui, ...) linked by a Mach-O executable.
linked_qt_framework_names() {
    local exe="$1"
    command -v otool >/dev/null 2>&1 || fail "otool is required to verify Qt frameworks after macdeployqt"
    otool -L "$exe" | sed -n 's/.*\(Qt[[:alnum:]][[:alnum:]]*\)\.framework.*/\1/p' | sort -u
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
    local expected
    expected="$(helper_sparkle_rpath)"
    command -v otool >/dev/null 2>&1 || fail "otool is required to verify the updater helper rpath"
    otool -l "$exe" | grep -F -q "$expected" || fail "updater helper is not linked with rpath ${expected}. Root compiles that rpath; this script does not rewrite it or sign the helper."
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

APP_PATH="$OUTPUT_DIR/$APP_NAME.app"
# -e alone misses a dangling symlink at the output path; refuse on either.
if [[ -e "$APP_PATH" || -L "$APP_PATH" ]]; then
    fail "output collision: $APP_PATH already exists (file, directory, or symlink). Remove it yourself or choose another --output-dir/--app-name; refusing to overwrite."
fi

mkdir -p "$APP_PATH/Contents/MacOS" "$APP_PATH/Contents/Resources"

ICON_PATH="$REPO_ROOT/assets/brand/waypair/AppIcon.icns"
[[ -f "$ICON_PATH" ]] || fail "Waypair app icon is missing: $ICON_PATH"
cp "$ICON_PATH" "$APP_PATH/Contents/Resources/AppIcon.icns"

EXEC_NAME="nectarlink-desktop"
cp "$BINARY" "$APP_PATH/Contents/MacOS/$EXEC_NAME"
chmod +x "$APP_PATH/Contents/MacOS/$EXEC_NAME"

PLIST_UPDATE_FEED="$(xml_escape "$UPDATE_FEED")"
PLIST_BUNDLE_ID="$(xml_escape "$BUNDLE_ID")"
PLIST_APP_NAME="$(xml_escape "$APP_NAME")"
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
         i.e. _nectarlink._udp). No microphone/camera keys are declared: this
         packaging step claims no such capture. -->
    <key>NSLocalNetworkUsageDescription</key>
    <string>Waypair Link finds and connects to your paired phone on your local network.</string>
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
</dict>
</plist>
EOF

# License/notice material inside the bundle (GPL/MPL compliance, not a substitute
# for the corresponding-source offer documented in MACOS_PACKAGING.md).
cp "$REPO_ROOT/LICENSE" "$APP_PATH/Contents/Resources/LICENSE.txt" || fail "failed to copy LICENSE into the bundle"
mkdir -p "$APP_PATH/Contents/Resources/LICENSES"
# Copy ALL of LICENSES/ (every license text and the app-store exception), and
# let any copy error abort the packaging (set -e): notices must not be
# silently dropped.
cp -R "$REPO_ROOT/LICENSES/." "$APP_PATH/Contents/Resources/LICENSES/" || fail "failed to copy LICENSES/ into the bundle"

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
DEPLOY_ARGS=("$APP_PATH" -no-codesign)
[[ -d "$QML_DIR" ]] && DEPLOY_ARGS+=(-qmldir="$QML_DIR")
printf 'Running: %s %s\n' "$MACDEPLOYQT" "${DEPLOY_ARGS[*]}"
"$MACDEPLOYQT" "${DEPLOY_ARGS[@]}" || fail "macdeployqt failed; bundle left in place for inspection: $APP_PATH"

# The executable's own Qt linkage, not the mere presence of Contents/Frameworks.
# Copying Sparkle.framework creates that directory before macdeployqt runs.
qt_names="$(linked_qt_framework_names "$APP_PATH/Contents/MacOS/$EXEC_NAME")"
require_named_qt_frameworks "$APP_PATH/Contents/Frameworks" "$qt_names"

printf '\nUnsigned bundle assembled (NOT signed, NOT notarized, not installed):\n  %s\n' "$APP_PATH"
printf 'Identifier: %s  Version: %s (%s)\n' "$BUNDLE_ID" "$VERSION" "$BUILD_VERSION"
printf 'Executable SHA-256: %s\n' "$(shasum -a 256 "$APP_PATH/Contents/MacOS/$EXEC_NAME" | awk '{print $1}')"
printf 'Next gates (Root-owned): production signing/hardened runtime, Keychain protector, R2 updater, qualification. See docs/waypair/MACOS_PACKAGING.md\n'
