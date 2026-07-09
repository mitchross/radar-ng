#!/usr/bin/env bash

set -euo pipefail

cd "$(dirname "$0")/.."

export DEVELOPER_DIR="${DEVELOPER_DIR:-/Applications/Xcode-beta.app/Contents/Developer}"

SCHEME="${SCHEME:-radarng}"
WORKSPACE="${WORKSPACE:-ios/${SCHEME}.xcworkspace}"
CONFIGURATION="${CONFIGURATION:-Release}"
EXPORT_METHOD="${EXPORT_METHOD:-debugging}"
BUILD_ROOT="${BUILD_ROOT:-build/ios-standalone}"
ARCHIVE_PATH="${ARCHIVE_PATH:-${BUILD_ROOT}/${SCHEME}.xcarchive}"
EXPORT_PATH="${EXPORT_PATH:-${BUILD_ROOT}/export-${EXPORT_METHOD}}"
DERIVED_DATA_PATH="${DERIVED_DATA_PATH:-${BUILD_ROOT}/DerivedData}"
TEAM_ID="${TEAM_ID:-$(node -e "process.stdout.write(require('./app.json').expo.ios.appleTeamId || '')")}"
QUIET="${QUIET:-1}"

if [[ ! -d "$WORKSPACE" ]]; then
  echo "Workspace not found: $WORKSPACE" >&2
  echo "Run from frontend after ios has been generated." >&2
  exit 1
fi

case "$EXPORT_METHOD" in
  debugging|release-testing|app-store-connect|validation|enterprise)
    ;;
  *)
    echo "Unsupported EXPORT_METHOD: $EXPORT_METHOD" >&2
    echo "Use debugging, release-testing, app-store-connect, validation, or enterprise." >&2
    exit 1
    ;;
esac

mkdir -p "$BUILD_ROOT"
EXPORT_OPTIONS_PLIST="${BUILD_ROOT}/ExportOptions.${EXPORT_METHOD}.plist"

cat >"$EXPORT_OPTIONS_PLIST" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>destination</key>
  <string>export</string>
  <key>method</key>
  <string>${EXPORT_METHOD}</string>
  <key>signingStyle</key>
  <string>automatic</string>
  <key>stripSwiftSymbols</key>
  <true/>
PLIST

if [[ -n "$TEAM_ID" ]]; then
  cat >>"$EXPORT_OPTIONS_PLIST" <<PLIST
  <key>teamID</key>
  <string>${TEAM_ID}</string>
PLIST
fi

cat >>"$EXPORT_OPTIONS_PLIST" <<'PLIST'
</dict>
</plist>
PLIST

XCODEBUILD_FLAGS=(-allowProvisioningUpdates)
if [[ "${REGISTER_DEVICES:-0}" == "1" ]]; then
  XCODEBUILD_FLAGS+=(-allowProvisioningDeviceRegistration)
fi
if [[ "$QUIET" == "1" ]]; then
  XCODEBUILD_FLAGS+=(-quiet)
fi

echo "==> Archiving ${SCHEME} (${CONFIGURATION})"
echo "    export method: ${EXPORT_METHOD}"
echo "    archive: ${ARCHIVE_PATH}"
rm -rf "$ARCHIVE_PATH" "$EXPORT_PATH"

xcodebuild \
  -workspace "$WORKSPACE" \
  -scheme "$SCHEME" \
  -configuration "$CONFIGURATION" \
  -destination 'generic/platform=iOS' \
  -derivedDataPath "$DERIVED_DATA_PATH" \
  -archivePath "$ARCHIVE_PATH" \
  "${XCODEBUILD_FLAGS[@]}" \
  archive

echo "==> Exporting IPA"
echo "    export path: ${EXPORT_PATH}"
xcodebuild \
  -exportArchive \
  -archivePath "$ARCHIVE_PATH" \
  -exportOptionsPlist "$EXPORT_OPTIONS_PLIST" \
  -exportPath "$EXPORT_PATH" \
  "${XCODEBUILD_FLAGS[@]}"

APP_PATH="$(find "$ARCHIVE_PATH/Products/Applications" -maxdepth 1 -type d -name '*.app' -print -quit)"
WATCH_APP_PATH="$(find "$ARCHIVE_PATH/Products/Applications" -type d -path '*/Watch/*.app' -print -quit)"
IPA_PATH="$(find "$EXPORT_PATH" -maxdepth 1 -type f -name '*.ipa' -print -quit)"

if [[ -z "$APP_PATH" || -z "$IPA_PATH" ]]; then
  echo "Archive/export finished, but expected app or IPA was not found." >&2
  exit 1
fi

codesign --verify --deep --strict "$APP_PATH"
if [[ -n "$WATCH_APP_PATH" ]]; then
  codesign --verify --deep --strict "$WATCH_APP_PATH"
fi

echo "==> Done"
echo "    IPA: ${IPA_PATH}"
echo "    App: ${APP_PATH}"
if [[ -n "$WATCH_APP_PATH" ]]; then
  echo "    Watch app embedded: ${WATCH_APP_PATH}"
else
  echo "    Watch app embedded: no"
fi

