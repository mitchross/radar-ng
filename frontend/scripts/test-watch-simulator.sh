#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
: "${WATCH_SIMULATOR_UDID:?Set WATCH_SIMULATOR_UDID to an available Watch simulator UUID}"
WATCH_QA_OUTPUT="${WATCH_QA_OUTPUT:-$(mktemp -d /tmp/radar-watch-qa.XXXXXX)}"
mkdir -p "$WATCH_QA_OUTPUT"
PAIRED_IPHONE_UDID="$(xcrun simctl list pairs --json | python3 -c '
import json, os, sys
pairs = json.load(sys.stdin)["pairs"].values()
print(next((p["phone"]["udid"] for p in pairs if p["watch"]["udid"] == os.environ["WATCH_SIMULATOR_UDID"]), ""))
')"
if [[ -n "$PAIRED_IPHONE_UDID" ]]; then
  xcrun simctl bootstatus "$PAIRED_IPHONE_UDID" -b
  if [[ -n "${IOS_SIMULATOR_APP_PATH:-}" ]]; then
    xcrun simctl install "$PAIRED_IPHONE_UDID" "$IOS_SIMULATOR_APP_PATH"
  fi
  if ! xcrun simctl get_app_container "$PAIRED_IPHONE_UDID" com.vanillax.radar-ng app >/dev/null 2>&1; then
    echo "The paired iPhone needs the companion app. Set IOS_SIMULATOR_APP_PATH to a simulator radarng.app build." >&2
    exit 1
  fi
fi

# Use CocoaPods' own Ruby/gems when installed through Homebrew on Apple Silicon.
if [[ -x /opt/homebrew/opt/ruby/bin/ruby && -d /opt/homebrew/opt/cocoapods/libexec ]]; then
  GEM_HOME=/opt/homebrew/opt/cocoapods/libexec /opt/homebrew/opt/ruby/bin/ruby scripts/prepare-watch-tests.rb
else
  ruby scripts/prepare-watch-tests.rb
fi
swiftc targets/watch/WatchAPI.swift targets/watch/WatchForecastPresentation.swift \
  tests/watch/PresentationChecks.swift -o "$WATCH_QA_OUTPUT/presentation-checks"
"$WATCH_QA_OUTPUT/presentation-checks"
xcrun simctl bootstatus "$WATCH_SIMULATOR_UDID" -b
# Start the permission-denial scenario from a known authorization state.
if xcrun simctl get_app_container "$WATCH_SIMULATOR_UDID" com.vanillax.radar-ng.watch app >/dev/null 2>&1; then
  xcrun simctl privacy "$WATCH_SIMULATOR_UDID" reset all com.vanillax.radar-ng.watch
fi
xcodebuild -project ios/radarng.xcodeproj -scheme WatchQA -configuration Release \
  -destination "platform=watchOS Simulator,id=$WATCH_SIMULATOR_UDID" \
  -derivedDataPath "$WATCH_QA_OUTPUT/DerivedData" \
  -resultBundlePath "$WATCH_QA_OUTPUT/results.xcresult" \
  CODE_SIGNING_ALLOWED=NO test > "$WATCH_QA_OUTPUT/test.log" 2>&1
printf 'Watch results: %s\n' "$WATCH_QA_OUTPUT"
