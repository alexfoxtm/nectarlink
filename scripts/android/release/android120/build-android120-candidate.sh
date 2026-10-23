#!/bin/bash
# Public source build recipe; this packet's author did not execute it.
set -euo pipefail
: "${WAYPAIR_REPO_ROOT:?select the exact reviewed source checkout}"
: "${ANDROID_HOME:?select an already installed SDK37}"
: "${JAVA_HOME:?select the retained JDK17}"
: "${CARGO_TARGET_DIR:?select a canonical existing build cache outside source}"
release_version=$(sed -n 's/^version=//p' "$WAYPAIR_REPO_ROOT/release-version.properties")
release_build=$(sed -n 's/^build=//p' "$WAYPAIR_REPO_ROOT/release-version.properties")
[[ "$release_version" == "0.2.1" && "$release_build" == "120" ]] || { echo "Canonical release identity differs" >&2; exit 2; }
cd "$WAYPAIR_REPO_ROOT/android"
export CARGO_NET_OFFLINE=true
export CARGO_BUILD_JOBS=4
# Signing properties are supplied only by the owner through the existing Gradle
# WAYPAIR_SIGNING mechanism. No key, certificate or provisioning profile created.
./gradlew --offline --no-daemon :app:assembleRelease :app:lintRelease :app:testDebugUnitTest \
  -Pwaypair.applicationId=com.waypair -Pwaypair.versionName="$release_version" \
  -Pwaypair.versionCode="$release_build" \
  -Pwaypair.sourceUrl=https://pub-a57457f4da43415ea6335550ee9e2983.r2.dev/waypair/nectar/android/releases/v0.2.1/Waypair-Link-0.2.1-android-source.tar.gz \
  -Pnectarlink.abis=arm64-v8a --console=plain
