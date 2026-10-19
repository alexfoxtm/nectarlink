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

Discovery order for macdeployqt: --macdeployqt, --qt-bin-dir / QT_BIN_DIR,
MACDEPLOYQT env, PATH, then `qmake -query QT_INSTALL_BINS`.
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

# Keep Info.plist values free of characters that would break the plist or the shell.
[[ "$BUNDLE_ID" =~ ^[A-Za-z0-9.-]+$ ]] || fail "invalid --bundle-id: $BUNDLE_ID"
[[ "$VERSION" =~ ^[A-Za-z0-9.-]+$ ]]   || fail "invalid --version: $VERSION"
[[ "$APP_NAME" != *"<"* && "$APP_NAME" != *">"* && "$APP_NAME" != *"&"* ]] || fail "invalid --app-name: $APP_NAME"
[[ -n "$BUILD_VERSION" ]] || BUILD_VERSION="$VERSION"
[[ "$BUILD_VERSION" =~ ^[A-Za-z0-9.-]+$ ]] || fail "invalid --build-version: $BUILD_VERSION"
if [[ "$BUNDLE_ID" == "com.waypair.mac" ]]; then
    fail "--bundle-id com.waypair.mac is refused during qualification: it would collide with the installed legacy app. Pick a distinct id; final id selection is a Root release decision."
fi

# --- Discover macdeployqt (explicit first; never assumed, never downloaded) ---
MACDEPLOYQT=""
if [[ -n "$MACDEPLOYQT_ARG" ]]; then
    MACDEPLOYQT="$MACDEPLOYQT_ARG"
elif [[ -n "${MACDEPLOYQT:-}" ]]; then
    MACDEPLOYQT="$MACDEPLOYQT"
elif [[ -n "$QT_BIN_DIR" && -x "$QT_BIN_DIR/macdeployqt" ]]; then
    MACDEPLOYQT="$QT_BIN_DIR/macdeployqt"
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
[[ ! -e "$APP_PATH" ]] || fail "output collision: $APP_PATH already exists. Remove it yourself or choose another --output-dir/--app-name; refusing to overwrite."

mkdir -p "$APP_PATH/Contents/MacOS" "$APP_PATH/Contents/Resources"

EXEC_NAME="nectarlink-desktop"
cp "$BINARY" "$APP_PATH/Contents/MacOS/$EXEC_NAME"
chmod +x "$APP_PATH/Contents/MacOS/$EXEC_NAME"

cat > "$APP_PATH/Contents/Info.plist" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleIdentifier</key>
    <string>$BUNDLE_ID</string>
    <key>CFBundleName</key>
    <string>$APP_NAME</string>
    <key>CFBundleDisplayName</key>
    <string>$APP_NAME</string>
    <key>CFBundleExecutable</key>
    <string>$EXEC_NAME</string>
    <key>CFBundleShortVersionString</key>
    <string>$VERSION</string>
    <key>CFBundleVersion</key>
    <string>$BUILD_VERSION</string>
    <key>CFBundlePackageType</key>
    <string>APPL</string>
    <key>CFBundleInfoDictionaryVersion</key>
    <string>6.0</string>
    <key>NSPrincipalClass</key>
    <string>NSApplication</string>
    <key>NSHighResolutionCapable</key>
    <true/>
</dict>
</plist>
EOF

# License/notice material inside the bundle (GPL/MPL compliance, not a substitute
# for the corresponding-source offer documented in MACOS_PACKAGING.md).
cp "$REPO_ROOT/LICENSE" "$APP_PATH/Contents/Resources/LICENSE.txt"
mkdir -p "$APP_PATH/Contents/Resources/LICENSES"
cp "$REPO_ROOT"/LICENSES/*.txt "$APP_PATH/Contents/Resources/LICENSES/" 2>/dev/null || true
[[ -f "$REPO_ROOT/LICENSES/APP-STORE-EXCEPTION.md" ]] && cp "$REPO_ROOT/LICENSES/APP-STORE-EXCEPTION.md" "$APP_PATH/Contents/Resources/LICENSES/"

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
