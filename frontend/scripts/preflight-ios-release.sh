#!/usr/bin/env bash
#
# Pre-archive guard for iOS releases.
#
# Fails when the generated native tree would ship something the release is not
# supposed to contain: CarPlay background modes or the navigation entitlement in
# a build that is not a CarPlay build, permission prompts the app never uses,
# version drift between app.json and the generated targets, or a beta toolchain.
#
# Run it after a clean prebuild and before archiving. build-ios-standalone.sh
# calls it automatically.
#
#   RADAR_CARPLAY=1 scripts/preflight-ios-release.sh    # for a CarPlay build
#   ALLOW_BETA=1 scripts/preflight-ios-release.sh       # beta toolchain opt-in
#
set -euo pipefail

cd "$(dirname "$0")/.."

ALLOW_BETA="${ALLOW_BETA:-0}"
IOS_DIR="${IOS_DIR:-ios}"
APP_NAME="${APP_NAME:-radarng}"
INFO_PLIST="${IOS_DIR}/${APP_NAME}/Info.plist"
ENTITLEMENTS="${IOS_DIR}/${APP_NAME}/${APP_NAME}.entitlements"
PBXPROJ="${IOS_DIR}/${APP_NAME}.xcodeproj/project.pbxproj"

if [[ ! -f "$INFO_PLIST" ]]; then
  echo "preflight: no ${INFO_PLIST} — run a clean prebuild first:" >&2
  echo "  bunx expo prebuild --platform ios --clean" >&2
  exit 1
fi

# Print the toolchain into the release log, and refuse a beta unless asked.
# A beta Xcode reports the same "Xcode 27.0" string as the release build, so the
# developer directory path is the only reliable signal.
echo "==> Toolchain"
xcodebuild -version 2>/dev/null | sed 's/^/    /'
echo "    iOS SDK: $(xcodebuild -version -sdk iphoneos Path 2>/dev/null || echo unknown)"
ACTIVE_DEVELOPER_DIR="${DEVELOPER_DIR:-$(xcode-select -p)}"
echo "    active Xcode: ${ACTIVE_DEVELOPER_DIR}"

if [[ "$ALLOW_BETA" != "1" ]] && printf '%s' "$ACTIVE_DEVELOPER_DIR" | tr '[:upper:]' '[:lower:]' | grep -q beta; then
  echo "preflight FAIL: active Xcode is a beta (${ACTIVE_DEVELOPER_DIR})." >&2
  echo "  Use the release toolchain: DEVELOPER_DIR=/Applications/Xcode.app/Contents/Developer" >&2
  echo "  Or set ALLOW_BETA=1 if this build is deliberately not for submission." >&2
  exit 1
fi

APP_VERSION="$(node -e "process.stdout.write(require('./app.json').expo.version)")"
BUILD_NUMBER="$(node -e "process.stdout.write(String(require('./app.json').expo.ios.buildNumber))")"
BUNDLE_ID="$(node -e "process.stdout.write(require('./app.json').expo.ios.bundleIdentifier)")"

python3 - "$INFO_PLIST" "$ENTITLEMENTS" "$PBXPROJ" \
  "$APP_VERSION" "$BUILD_NUMBER" "$BUNDLE_ID" "${RADAR_CARPLAY:-0}" <<'PY'
import os, plistlib, re, sys

info_path, ent_path, pbxproj_path, version, build_number, bundle_id, carplay = sys.argv[1:8]
failures = []


def fail(msg):
    failures.append(msg)


with open(info_path, "rb") as f:
    info = plistlib.load(f)

# Permission prompts the app must not ship. The app requests When-In-Use location
# only; "Always" location, Face ID and motion were default-plugin leftovers.
for key in sorted(info):
    if key.startswith(("NSLocationAlways", "NSFaceID", "NSMotion")):
        fail(f"unused permission key in Info.plist: {key}")

entitlements = {}
if os.path.exists(ent_path):
    with open(ent_path, "rb") as f:
        entitlements = plistlib.load(f)

# CarPlay-only settings belong to RADAR_CARPLAY=1 builds. Shipping them in an
# ordinary release means App Review sees background modes for features it cannot
# see (guideline 2.5.4).
if carplay != "1":
    modes = info.get("UIBackgroundModes") or []
    if modes:
        fail(
            "UIBackgroundModes present ("
            + ", ".join(modes)
            + ") but RADAR_CARPLAY != 1"
        )
    if "com.apple.developer.carplay-maps" in entitlements:
        fail("carplay-maps entitlement present but RADAR_CARPLAY != 1")

if info.get("CFBundleShortVersionString") != version:
    fail(
        "CFBundleShortVersionString is "
        f"{info.get('CFBundleShortVersionString')!r}, app.json says {version!r}"
    )
if info.get("CFBundleVersion") != build_number:
    fail(
        f"CFBundleVersion is {info.get('CFBundleVersion')!r}, "
        f"app.json ios.buildNumber says {build_number!r}"
    )

# The Watch and widget targets take their versions from Xcode build settings, so
# compare them against app.json rather than their (versionless) Info.plist.
try:
    with open(pbxproj_path, encoding="utf-8", errors="replace") as f:
        pbxproj = f.read()
except OSError:
    fail(f"cannot read {pbxproj_path}")
    pbxproj = ""

suffixes = {}
for match in re.finditer(r'PRODUCT_BUNDLE_IDENTIFIER = "([^"]+)";', pbxproj):
    target_id = match.group(1)
    if not target_id.startswith(bundle_id) or target_id == bundle_id:
        continue
    block = pbxproj[: match.start()]
    marketing = re.findall(r"MARKETING_VERSION = ([^;]+);", block)
    current = re.findall(r"CURRENT_PROJECT_VERSION = ([^;]+);", block)
    if marketing:
        suffixes[target_id] = suffixes.get(target_id, {})
        suffixes[target_id]["marketing"] = marketing[-1].strip()
    if current:
        suffixes[target_id] = suffixes.get(target_id, {})
        suffixes[target_id]["current"] = current[-1].strip()

if not suffixes:
    fail(
        "no Watch or widget target found in the Xcode project — "
        "did @bacons/apple-targets run?"
    )
for target_id, values in sorted(suffixes.items()):
    if values.get("marketing") != version:
        fail(f"{target_id} MARKETING_VERSION is {values.get('marketing')!r}, app.json says {version!r}")
    if values.get("current") != build_number:
        fail(f"{target_id} CURRENT_PROJECT_VERSION is {values.get('current')!r}, app.json says {build_number!r}")

if failures:
    print("preflight FAIL:", file=sys.stderr)
    for msg in failures:
        print(f"  - {msg}", file=sys.stderr)
    sys.exit(1)

print("preflight OK")
print(f"    version {version} ({build_number})")
print(f"    CarPlay build: {'yes' if carplay == '1' else 'no'}")
print(f"    targets checked: {', '.join(sorted(suffixes)) or 'none'}")
PY
