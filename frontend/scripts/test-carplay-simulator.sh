#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
: "${IOS_SIMULATOR_UDID:?Set IOS_SIMULATOR_UDID to a booted iPhone simulator}"
CARPLAY_TEST_OUTPUT="${CARPLAY_TEST_OUTPUT:-$(mktemp -d /tmp/radar-carplay-qa.XXXXXX)}"
CARPLAY_TEST_APP="$CARPLAY_TEST_OUTPUT/CarPlayTestHost.app"
mkdir -p "$CARPLAY_TEST_APP"
cat > "$CARPLAY_TEST_APP/Info.plist" <<'PLIST'
<?xml version="1.0" encoding="UTF-8"?><plist version="1.0"><dict>
<key>CFBundleIdentifier</key><string>com.vanillax.radar-carplay-tests</string>
<key>CFBundleExecutable</key><string>CarPlayTestHost</string>
<key>CFBundleName</key><string>Radar Driving Preview</string>
<key>CFBundlePackageType</key><string>APPL</string>
<key>CFBundleVersion</key><string>1</string>
<key>CFBundleShortVersionString</key><string>1.0</string>
<key>MinimumOSVersion</key><string>26.0</string>
<key>UILaunchScreen</key><dict/>
<key>UIApplicationSceneManifest</key><dict><key>UIApplicationSupportsMultipleScenes</key><false/></dict>
</dict></plist>
PLIST
xcrun --sdk iphonesimulator swiftc -parse-as-library \
  -sdk "$(xcrun --sdk iphonesimulator --show-sdk-path)" -target arm64-apple-ios26.0-simulator \
  targets/carplay/RadarAPI.swift targets/carplay/RadarRouteProgress.swift \
  targets/carplay/RadarLocationManager.swift targets/carplay/RadarDrivingSession.swift \
  targets/carplay/RadarTileOverlay.swift targets/carplay/RadarManeuverFactory.swift targets/carplay/RadarMapController.swift \
  tests/carplay/CarPlayTestHost.swift -o "$CARPLAY_TEST_APP/CarPlayTestHost"
codesign --force --sign - "$CARPLAY_TEST_APP"
xcrun simctl install "$IOS_SIMULATOR_UDID" "$CARPLAY_TEST_APP"
CARPLAY_TEST_DATA="$(xcrun simctl get_app_container "$IOS_SIMULATOR_UDID" com.vanillax.radar-carplay-tests data)"
if [[ -f "$CARPLAY_TEST_DATA/Documents/result.txt" ]]; then
  mv "$CARPLAY_TEST_DATA/Documents/result.txt" "$CARPLAY_TEST_OUTPUT/previous-result.txt"
fi
xcrun simctl launch --terminate-running-process "$IOS_SIMULATOR_UDID" com.vanillax.radar-carplay-tests
for attempt in {1..90}; do
  if [[ -f "$CARPLAY_TEST_DATA/Documents/result.txt" ]]; then break; fi
  sleep 1
done
cp "$CARPLAY_TEST_DATA/Documents/result.txt" "$CARPLAY_TEST_OUTPUT/result.txt"
cp "$CARPLAY_TEST_DATA"/Documents/*.jpg "$CARPLAY_TEST_OUTPUT/"
cat "$CARPLAY_TEST_OUTPUT/result.txt"
printf '\nArtifacts: %s\n' "$CARPLAY_TEST_OUTPUT"
rg -q '^PASS' "$CARPLAY_TEST_OUTPUT/result.txt"
