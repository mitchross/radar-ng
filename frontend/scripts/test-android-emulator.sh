#!/usr/bin/env bash
# Android counterpart of test-ios-simulator.sh: build a release APK, install
# it on a running emulator or device, and run the shared Maestro flows.
set -euo pipefail
cd "$(dirname "$0")/.."

: "${ANDROID_SERIAL:?Set ANDROID_SERIAL to a running emulator or device (adb devices)}"
QA_OUTPUT="${QA_OUTPUT:-/tmp/radar-android-qa}"
APP_ID="com.vanillax.radarng"
mkdir -p "$QA_OUTPUT"
export MAESTRO_CLI_NO_ANALYTICS=1
export MAESTRO_CLI_ANALYSIS_NOTIFICATION_DISABLED=true

if [[ "${SKIP_BUILD:-0}" != "1" ]]; then
  bun install --frozen-lockfile
  # SDK 57 prebuild recreates android/ from scratch; it holds only generated files.
  bunx expo prebuild --platform android --no-install
  # Release build signed with the template debug keystore; for QA only.
  (cd android && ./gradlew assembleRelease) > "$QA_OUTPUT/build.log" 2>&1
fi

adb -s "$ANDROID_SERIAL" wait-for-device
adb -s "$ANDROID_SERIAL" install -r android/app/build/outputs/apk/release/app-release.apk
maestro --device "$ANDROID_SERIAL" test -e APP_ID="$APP_ID" \
  --format JUNIT --output "$QA_OUTPUT/results.xml" \
  --test-output-dir "$QA_OUTPUT/maestro" .maestro/
