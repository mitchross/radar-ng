#!/usr/bin/env bash
#
# carplay-resign.sh — personal-use sideload hack for radar-ng's CarPlay scene.
#
# Apple does not grant `com.apple.developer.carplay-maps` to non-navigation apps.
# Xcode's signing flow will silently strip the entitlement during a normal
# Release build. This script archives the app, then re-signs the .app bundle
# with a patched entitlements plist that re-adds the CarPlay entitlement,
# then installs to a connected device. iOS only checks the entitlements
# embedded in the signed binary at runtime, not the dev-portal profile.
#
# Requirements:
#   - Apple Developer account configured in Xcode (free tier won't work — needs
#     paid account so the cert chains validate)
#   - `ios-deploy` (brew install ios-deploy) OR an iPhone plugged in with
#     Xcode's Devices window open
#   - jq (brew install jq) for app.json parsing
#
# Usage:
#   TEAM_ID=ABC1234567 SIGNING_IDENTITY="Apple Development: you@example.com (XYZ)" \
#     ./scripts/carplay-resign.sh

set -euo pipefail

cd "$(dirname "$0")/.."

if [[ -z "${TEAM_ID:-}" ]]; then
  echo "TEAM_ID not set. Find it at https://developer.apple.com/account → Membership."
  exit 1
fi

if [[ -z "${SIGNING_IDENTITY:-}" ]]; then
  echo "SIGNING_IDENTITY not set. Find it with:"
  echo "  security find-identity -v -p codesigning | grep 'Apple Development'"
  exit 1
fi

BUNDLE_ID="$(jq -r '.expo.ios.bundleIdentifier' app.json)"
SCHEME="$(jq -r '.expo.slug' app.json)"
WORKSPACE="ios/${SCHEME}.xcworkspace"
ARCHIVE="build/${SCHEME}.xcarchive"
APP_PATH="${ARCHIVE}/Products/Applications/${SCHEME}.app"

if [[ ! -d "$WORKSPACE" ]]; then
  echo "Workspace $WORKSPACE not found. Run: npx expo prebuild --platform ios"
  exit 1
fi

echo "==> Archiving (Release) for generic iOS device"
rm -rf "$ARCHIVE"
xcodebuild \
  -workspace "$WORKSPACE" \
  -scheme "$SCHEME" \
  -configuration Release \
  -destination 'generic/platform=iOS' \
  -archivePath "$ARCHIVE" \
  -allowProvisioningUpdates \
  archive

if [[ ! -d "$APP_PATH" ]]; then
  echo "Archive succeeded but $APP_PATH not found. Aborting."
  exit 1
fi

echo "==> Writing patched entitlements"
PATCHED="$(mktemp -t entitlements-XXXXXX).plist"
cat > "$PATCHED" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>application-identifier</key><string>${TEAM_ID}.${BUNDLE_ID}</string>
  <key>com.apple.developer.team-identifier</key><string>${TEAM_ID}</string>
  <key>com.apple.developer.carplay-maps</key><true/>
  <key>get-task-allow</key><true/>
</dict>
</plist>
EOF
echo "    $PATCHED"

echo "==> Re-signing $APP_PATH"
codesign --force --sign "$SIGNING_IDENTITY" \
  --entitlements "$PATCHED" \
  --preserve-metadata=identifier,requirements,flags,runtime \
  "$APP_PATH"

echo "==> Verifying embedded entitlements"
codesign -d --entitlements - "$APP_PATH" 2>&1 | grep -F "com.apple.developer.carplay-maps" \
  || { echo "carplay-maps entitlement missing after re-sign — codesign rejected it."; exit 1; }

echo "==> Installing to device"
if command -v ios-deploy >/dev/null; then
  ios-deploy --bundle "$APP_PATH"
else
  echo "ios-deploy not installed. Install it with: brew install ios-deploy"
  echo "Or drag $APP_PATH into Xcode → Window → Devices and Simulators → Installed Apps → +"
  exit 1
fi

echo "==> Done. Plug into car's CarPlay or open the CarPlay simulator from Xcode."
