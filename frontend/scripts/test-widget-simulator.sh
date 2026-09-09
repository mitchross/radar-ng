#!/bin/bash
set -euo pipefail
cd "$(dirname "$0")/.."
: "${IOS_SIMULATOR_UDID:?Set IOS_SIMULATOR_UDID to a booted iPhone simulator}"
WIDGET_TEST_OUTPUT="${WIDGET_TEST_OUTPUT:-/tmp/radar-widget-qa}"
WIDGET_TEST_APP="$WIDGET_TEST_OUTPUT/WidgetTestHost.app"
mkdir -p "$WIDGET_TEST_APP"
cat > "$WIDGET_TEST_APP/Info.plist" <<'PLIST'
<?xml version="1.0" encoding="UTF-8"?><plist version="1.0"><dict>
<key>CFBundleIdentifier</key><string>com.vanillax.radar-widget-tests</string>
<key>CFBundleExecutable</key><string>WidgetTestHost</string>
<key>CFBundleName</key><string>WidgetTestHost</string>
<key>CFBundlePackageType</key><string>APPL</string>
<key>CFBundleVersion</key><string>1</string>
<key>CFBundleShortVersionString</key><string>1.0</string>
<key>MinimumOSVersion</key><string>26.0</string>
<key>UILaunchScreen</key><dict/>
<key>UIApplicationSceneManifest</key><dict><key>UIApplicationSupportsMultipleScenes</key><false/></dict>
</dict></plist>
PLIST
xcrun --sdk iphonesimulator swiftc -parse-as-library -D WIDGET_TEST_HOST \
  -sdk "$(xcrun --sdk iphonesimulator --show-sdk-path)" \
  -target arm64-apple-ios26.0-simulator \
  targets/radar-widget/RadarWidget.swift targets/radar-widget/RadarSnapshot.swift \
  tests/widget/WidgetTestHost.swift -o "$WIDGET_TEST_APP/WidgetTestHost"
codesign --force --sign - "$WIDGET_TEST_APP"
xcrun simctl install "$IOS_SIMULATOR_UDID" "$WIDGET_TEST_APP"
WIDGET_TEST_DATA="$(xcrun simctl get_app_container "$IOS_SIMULATOR_UDID" com.vanillax.radar-widget-tests data)"
rm -f "$WIDGET_TEST_DATA/Documents/result.txt"
xcrun simctl launch --terminate-running-process "$IOS_SIMULATOR_UDID" com.vanillax.radar-widget-tests
for attempt in {1..30}; do
  if [[ -f "$WIDGET_TEST_DATA/Documents/result.txt" ]]; then break; fi
  sleep 1
done
cp "$WIDGET_TEST_DATA/Documents/result.txt" "$WIDGET_TEST_OUTPUT/result.txt"
cat "$WIDGET_TEST_OUTPUT/result.txt"
rg -q '^PASS' "$WIDGET_TEST_OUTPUT/result.txt"
cp "$WIDGET_TEST_DATA/Documents/snapshot.png" "$WIDGET_TEST_OUTPUT/snapshot.png"
xcrun simctl io "$IOS_SIMULATOR_UDID" screenshot "$WIDGET_TEST_OUTPUT/widget-states.png"
