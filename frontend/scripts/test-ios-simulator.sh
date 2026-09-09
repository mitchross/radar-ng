#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."

: "${SIMULATOR_UDID:?Set SIMULATOR_UDID to an available iPhone simulator UUID}"
QA_OUTPUT="${QA_OUTPUT:-/tmp/radar-ios-qa}"
mkdir -p "$QA_OUTPUT"
export MAESTRO_CLI_NO_ANALYTICS=1
export MAESTRO_CLI_ANALYSIS_NOTIFICATION_DISABLED=true

if [[ "${SKIP_BUILD:-0}" != "1" ]]; then
  bun install --frozen-lockfile
  bunx expo prebuild --platform ios --no-install
  (
    cd ios
    RUBYOPT="-r$(pwd)/../scripts/cocoapods_xcodeproj_object_version_70" pod install
  )
  # Let each target choose its SDK; forcing iphonesimulator breaks the Watch target.
  xcodebuild -workspace ios/radarng.xcworkspace -scheme radarng \
    -configuration Release \
    -destination "platform=iOS Simulator,id=$SIMULATOR_UDID" \
    -derivedDataPath "$QA_OUTPUT/DerivedData" \
    ONLY_ACTIVE_ARCH=YES CODE_SIGNING_ALLOWED=NO \
    > "$QA_OUTPUT/build.log" 2>&1
fi

xcrun simctl bootstatus "$SIMULATOR_UDID" -b
xcrun simctl install "$SIMULATOR_UDID" \
  "$QA_OUTPUT/DerivedData/Build/Products/Release-iphonesimulator/radarng.app"
maestro --device "$SIMULATOR_UDID" test \
  --format JUNIT --output "$QA_OUTPUT/results.xml" \
  --test-output-dir "$QA_OUTPUT/maestro" .maestro/
