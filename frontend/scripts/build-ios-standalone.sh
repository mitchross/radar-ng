#!/usr/bin/env bash

set -euo pipefail

cd "$(dirname "$0")/.."

# Archive with the release toolchain. Xcode 27.0 release is /Applications/Xcode.app;
# xcode-select on this Mac points at the 27.0 beta, and the old default here
# (/Applications/Xcode-beta.app) does not exist. A beta build is rejected for
# submission, so opting into one requires both DEVELOPER_DIR and ALLOW_BETA=1.
export DEVELOPER_DIR="${DEVELOPER_DIR:-/Applications/Xcode.app/Contents/Developer}"

SCHEME="${SCHEME:-radarng}"
WORKSPACE="${WORKSPACE:-ios/${SCHEME}.xcworkspace}"
CONFIGURATION="${CONFIGURATION:-Release}"
EXPORT_METHOD="${EXPORT_METHOD:-debugging}"
# Outside frontend/ios, which prebuild regenerates; see /artifacts in .gitignore.
BUILD_ROOT="${BUILD_ROOT:-../artifacts/ios}"
ARCHIVE_PATH="${ARCHIVE_PATH:-${BUILD_ROOT}/${SCHEME}.xcarchive}"
EXPORT_PATH="${EXPORT_PATH:-${BUILD_ROOT}/export-${EXPORT_METHOD}}"
DERIVED_DATA_PATH="${DERIVED_DATA_PATH:-${BUILD_ROOT}/DerivedData}"
TEAM_ID="${TEAM_ID:-$(node -e "process.stdout.write(require('./identity').appleTeamId || '')")}"
QUIET="${QUIET:-1}"

# CI numbers its builds 10.<run>.<attempt> (.github/workflows/build-ios.yml). Without IOS_BUILD_RUN
# a local build is plain "10", which sorts below every CI build, so App Store Connect would reject it.
# Default to 9000.<minutes since epoch>: CFBundleVersion takes up to three period-separated
# non-negative integers compared left to right
# (https://developer.apple.com/documentation/bundleresources/information-property-list/cfbundleversion),
# so 10.9000.<minutes> sorts above any CI run number below 9000. Prebuild is what writes the build
# number into the app, widget and Watch targets, so regenerate the native project the way CI does.
# Set IOS_BUILD_RUN yourself to skip this and use the project you prebuilt.
if [[ -z "${IOS_BUILD_RUN:-}" ]]; then
  export IOS_BUILD_RUN="9000.$(($(date +%s) / 60))"
  echo "==> Prebuild (local build number: app.json buildNumber + .${IOS_BUILD_RUN})"
  bunx expo prebuild --platform ios --clean --no-install
  (cd ios && RUBYOPT="-r$PWD/../scripts/cocoapods_xcodeproj_object_version_70${RUBYOPT:+ $RUBYOPT}" pod install)
fi

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

echo "==> Preflight"
bash ./scripts/preflight-ios-release.sh

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

