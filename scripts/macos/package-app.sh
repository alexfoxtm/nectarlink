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
                         Configurable during qualification; deliberately NOT
                         com.waypair.mac, so the legacy app is never collided with.
  --version <x.y.z>      CFBundleShortVersionString. Default: 0.1.4 (workspace version).
  --build-version <n>    CFBundleVersion. Default: same as --version.
  --macdeployqt <path>   Explicit macdeployqt. Wins over all discovery.
  --qt-bin-dir <dir>     Directory containing macdeployqt (or set QT_BIN_DIR).
  --qml-dir <dir>        QML sources for macdeployqt -qmldir. Default: desktop/app/qml in this repo.
  -h, --help             Show this help.

Discovery order for macdeployqt: --macdeployqt (explicit), then
--qt-bin-dir / QT_BIN_DIR, then MACDEPLOYQT env, then PATH, then
`qmake -query QT_INSTALL_BINS`.
No signing identity, keychain, entitlements file, or network is ever touched.
EOF
}

fail() { printf 'error: %s\n' "$*" >&2; exit 1; }

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
[[ "$BUNDLE_ID" =~ ^[A-Za-z0-9.-]+$ ]] || fail "invalid --bundle-id: $BUNDLE_ID"
# CFBundleShortVersionString: numeric major.minor[.patch]. CFBundleVersion:
# non-negative integers separated by dots (Apple's numeric build format).
[[ "$VERSION" =~ ^[0-9]+\.[0-9]+(\.[0-9]+)?$ ]] || fail "invalid --version (expected numeric x.y or x.y.z): $VERSION"
[[ -n "$BUILD_VERSION" ]] || BUILD_VERSION="$VERSION"
[[ "$BUILD_VERSION" =~ ^[0-9]+(\.[0-9]+){0,2}$ ]] || fail "invalid --build-version (expected dot-separated integers): $BUILD_VERSION"

# XML-escape plist text values so a legal-looking name cannot break the plist.
xml_escape() {
    local value="$1"
    value="${value//&/&amp;}"
    value="${value//</&lt;}"
    value="${value//>/&gt;}"
    value="${value//\"/&quot;}"
    value="${value//\'/&apos;}"
    printf '%s' "$value"
}
if [[ "$BUNDLE_ID" == "com.waypair.mac" ]]; then
    fail "--bundle-id com.waypair.mac is refused during qualification: it would collide with the installed legacy app. Pick a distinct id; final id selection is a Root release decision."
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

EXEC_NAME="nectarlink-desktop"
cp "$BINARY" "$APP_PATH/Contents/MacOS/$EXEC_NAME"
chmod +x "$APP_PATH/Contents/MacOS/$EXEC_NAME"

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

# Bundle Qt frameworks/plugins. -no-codesign: this script never signs; any
# production signature is applied later by Root's release process.
DEPLOY_ARGS=("$APP_PATH" -no-codesign)
[[ -d "$QML_DIR" ]] && DEPLOY_ARGS+=(-qmldir="$QML_DIR")
printf 'Running: %s %s\n' "$MACDEPLOYQT" "${DEPLOY_ARGS[*]}"
"$MACDEPLOYQT" "${DEPLOY_ARGS[@]}" || fail "macdeployqt failed; bundle left in place for inspection: $APP_PATH"

[[ -d "$APP_PATH/Contents/Frameworks" ]] || fail "macdeployqt produced no Contents/Frameworks; bundle may be incomplete: $APP_PATH"

printf '\nUnsigned bundle assembled (NOT signed, NOT notarized, not installed):\n  %s\n' "$APP_PATH"
printf 'Identifier: %s  Version: %s (%s)\n' "$BUNDLE_ID" "$VERSION" "$BUILD_VERSION"
printf 'Executable SHA-256: %s\n' "$(shasum -a 256 "$APP_PATH/Contents/MacOS/$EXEC_NAME" | awk '{print $1}')"
printf 'Next gates (Root-owned): production signing/hardened runtime, Keychain protector, R2 updater, qualification. See docs/waypair/MACOS_PACKAGING.md\n'
