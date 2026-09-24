#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
: "${REVIEW_DEVICE_UDID:?Set REVIEW_DEVICE_UDID to the chosen device}"
export CARPLAY_REVIEW_OUTPUT="${CARPLAY_REVIEW_OUTPUT:-$(mktemp -d /tmp/radar-carplay-review.XXXXXX)}"
REVIEW_PLATFORM="${REVIEW_PLATFORM:-device}"
if [[ "$REVIEW_PLATFORM" != device && "$REVIEW_PLATFORM" != simulator ]]; then
  echo 'REVIEW_PLATFORM must be device or simulator' >&2; exit 1
fi
if [[ -x /opt/homebrew/opt/ruby/bin/ruby && -d /opt/homebrew/opt/cocoapods/libexec ]]; then
  GEM_HOME=/opt/homebrew/opt/cocoapods/libexec /opt/homebrew/opt/ruby/bin/ruby scripts/prepare-carplay-review.rb
else
  ruby scripts/prepare-carplay-review.rb
fi
if [[ "$REVIEW_PLATFORM" == device ]]; then
  REVIEW_DESTINATION="platform=iOS,id=$REVIEW_DEVICE_UDID"
  REVIEW_PRODUCT=Release-iphoneos
  REVIEW_SIGNING=(-allowProvisioningUpdates -allowProvisioningDeviceRegistration)
else
  REVIEW_DESTINATION="platform=iOS Simulator,id=$REVIEW_DEVICE_UDID"
  REVIEW_PRODUCT=Release-iphonesimulator
  REVIEW_SIGNING=(CODE_SIGNING_ALLOWED=NO)
fi
xcodebuild -project "$CARPLAY_REVIEW_OUTPUT/CarPlayReview.xcodeproj" -scheme CarPlayReview \
  -configuration Release -destination "$REVIEW_DESTINATION" \
  -derivedDataPath "$CARPLAY_REVIEW_OUTPUT/DerivedData" "${REVIEW_SIGNING[@]}" build \
  > "$CARPLAY_REVIEW_OUTPUT/build.log" 2>&1
REVIEW_APP="$CARPLAY_REVIEW_OUTPUT/DerivedData/Build/Products/$REVIEW_PRODUCT/CarPlayReview.app"
REVIEW_BUNDLE=com.vanillax.radar-ng.carplay-review
REVIEW_RUN_ID="$(uuidgen)"
export RADAR_REVIEW_RUN_ID="$REVIEW_RUN_ID"
# Forward optional location/scale settings identically on device and simulator.
REVIEW_ENVIRONMENT="$(python3 - <<'PY'
import json, os
print(json.dumps({k: v for k, v in os.environ.items() if k.startswith('RADAR_REVIEW_')}))
PY
)"
mkdir -p "$CARPLAY_REVIEW_OUTPUT/evidence"
if [[ "$REVIEW_PLATFORM" == device ]]; then
  xcrun devicectl device install app --device "$REVIEW_DEVICE_UDID" "$REVIEW_APP"
  xcrun devicectl device process launch --terminate-existing --device "$REVIEW_DEVICE_UDID" \
    --environment-variables "$REVIEW_ENVIRONMENT" "$REVIEW_BUNDLE"
else
  xcrun simctl install "$REVIEW_DEVICE_UDID" "$REVIEW_APP"
  python3 - "$REVIEW_DEVICE_UDID" "$REVIEW_BUNDLE" <<'PY'
import os, subprocess, sys
environment = dict(os.environ)
environment.update({'SIMCTL_CHILD_' + k: v for k, v in os.environ.items() if k.startswith('RADAR_REVIEW_')})
subprocess.run(['xcrun', 'simctl', 'launch', '--terminate-running-process', sys.argv[1], sys.argv[2]], env=environment, check=True)
PY
  REVIEW_DATA="$(xcrun simctl get_app_container "$REVIEW_DEVICE_UDID" "$REVIEW_BUNDLE" data)"
fi
REVIEW_LAST_CAPTURE=""
for attempt in {1..120}; do
  sleep 1
  REVIEW_READY=""
  if [[ "$REVIEW_PLATFORM" == device ]]; then
    if xcrun devicectl device copy from --device "$REVIEW_DEVICE_UDID" \
      --domain-type appDataContainer --domain-identifier "$REVIEW_BUNDLE" \
      --source "Documents/$REVIEW_RUN_ID/capture-ready.txt" --destination "$CARPLAY_REVIEW_OUTPUT/capture-ready.txt" \
      --timeout 5 >/dev/null 2>&1; then REVIEW_READY="$(cat "$CARPLAY_REVIEW_OUTPUT/capture-ready.txt")"; fi
  elif [[ -f "$REVIEW_DATA/Documents/$REVIEW_RUN_ID/capture-ready.txt" ]]; then
    REVIEW_READY="$(cat "$REVIEW_DATA/Documents/$REVIEW_RUN_ID/capture-ready.txt")"
  fi
  if [[ -n "$REVIEW_READY" && "$REVIEW_READY" != "$REVIEW_LAST_CAPTURE" ]]; then
    case "$REVIEW_READY" in
      01-approaching-turn|02-near-turn|03-next-maneuver) ;;
      *) echo 'Unexpected capture name' >&2; exit 1 ;;
    esac
    if [[ "$REVIEW_PLATFORM" == device ]]; then
      xcrun devicectl device capture screenshot --device "$REVIEW_DEVICE_UDID" \
        --destination "$CARPLAY_REVIEW_OUTPUT/evidence/$REVIEW_READY.png"
      touch "$CARPLAY_REVIEW_OUTPUT/captured-$REVIEW_READY"
      xcrun devicectl device copy to --device "$REVIEW_DEVICE_UDID" \
        --domain-type appDataContainer --domain-identifier "$REVIEW_BUNDLE" \
        --source "$CARPLAY_REVIEW_OUTPUT/captured-$REVIEW_READY" \
        --destination "Documents/$REVIEW_RUN_ID/captured-$REVIEW_READY"
    else
      xcrun simctl io "$REVIEW_DEVICE_UDID" screenshot --type=png "$CARPLAY_REVIEW_OUTPUT/evidence/$REVIEW_READY.png"
      touch "$REVIEW_DATA/Documents/$REVIEW_RUN_ID/captured-$REVIEW_READY"
    fi
    REVIEW_LAST_CAPTURE="$REVIEW_READY"
  fi
  if [[ "$REVIEW_PLATFORM" == device ]]; then
    if xcrun devicectl device copy from --device "$REVIEW_DEVICE_UDID" \
      --domain-type appDataContainer --domain-identifier "$REVIEW_BUNDLE" \
      --source "Documents/$REVIEW_RUN_ID/review-result.txt" --destination "$CARPLAY_REVIEW_OUTPUT/evidence/review-result.txt" \
      --timeout 5 >/dev/null 2>&1; then break; fi
  elif [[ -f "$REVIEW_DATA/Documents/$REVIEW_RUN_ID/review-result.txt" ]]; then break; fi
 done
if [[ "$REVIEW_PLATFORM" == device ]]; then
  for file in review-result.txt maneuver-evidence.json; do
    xcrun devicectl device copy from --device "$REVIEW_DEVICE_UDID" \
      --domain-type appDataContainer --domain-identifier "$REVIEW_BUNDLE" \
      --source "Documents/$REVIEW_RUN_ID/$file" --destination "$CARPLAY_REVIEW_OUTPUT/evidence/$file"
  done
else
  cp "$REVIEW_DATA/Documents/$REVIEW_RUN_ID/review-result.txt" "$REVIEW_DATA/Documents/$REVIEW_RUN_ID/maneuver-evidence.json" "$CARPLAY_REVIEW_OUTPUT/evidence/"
fi
cat "$CARPLAY_REVIEW_OUTPUT/evidence/review-result.txt"
printf '\nEvidence: %s/evidence\n' "$CARPLAY_REVIEW_OUTPUT"
rg -q '^PASS' "$CARPLAY_REVIEW_OUTPUT/evidence/review-result.txt"
