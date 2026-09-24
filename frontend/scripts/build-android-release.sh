#!/usr/bin/env bash
# Local Android release build (no cloud builds). Produces a signed AAB for
# Play (FORMAT=aab, default) or a sideloadable APK (FORMAT=apk) under
# ../artifacts/android. Signing secrets come from ~/.radar-ng; see
# create-android-upload-key.sh.
set -euo pipefail
cd "$(dirname "$0")/.."

FORMAT="${FORMAT:-aab}"
SIGNING_ENV="${SIGNING_ENV:-$HOME/.radar-ng/android-signing.env}"
OUT_DIR="${OUT_DIR:-../artifacts/android}"

if [[ -f "$SIGNING_ENV" ]]; then
  set -a
  # shellcheck disable=SC1090
  source "$SIGNING_ENV"
  set +a
  echo "==> Signing with upload key $ORG_GRADLE_PROJECT_RADAR_UPLOAD_STORE_FILE"
elif [[ "${ALLOW_DEBUG_SIGNING:-0}" == "1" ]]; then
  echo "==> No $SIGNING_ENV; signing with the debug key (QA only, Play rejects it)"
else
  echo "No signing config at $SIGNING_ENV." >&2
  echo "Run scripts/create-android-upload-key.sh, or set ALLOW_DEBUG_SIGNING=1 for a QA build." >&2
  exit 1
fi

VERSION_NAME="$(node -p "require('./app.json').expo.version")"
VERSION_CODE="$(node -p "require('./app.json').expo.android.versionCode")"
echo "==> radarng ${VERSION_NAME} (${VERSION_CODE}), ${FORMAT}"

if [[ "${SKIP_PREBUILD:-0}" != "1" ]]; then
  # SDK 57+ prebuild recreates android/ from app.json and plugins; it holds only generated files.
  bunx expo prebuild --platform android --no-install
fi

case "$FORMAT" in
  aab) TASK=bundleRelease; SRC=android/app/build/outputs/bundle/release/app-release.aab ;;
  apk) TASK=assembleRelease; SRC=android/app/build/outputs/apk/release/app-release.apk ;;
  *) echo "FORMAT must be aab or apk" >&2; exit 1 ;;
esac

# The generated 2 GB heap has failed dex merging under memory pressure.
(cd android && ./gradlew "$TASK" --console=plain -Dorg.gradle.jvmargs="-Xmx4g -XX:MaxMetaspaceSize=1g")

mkdir -p "$OUT_DIR"
DEST="$OUT_DIR/radarng-${VERSION_NAME}-${VERSION_CODE}.${FORMAT}"
cp "$SRC" "$DEST"

echo "==> Signer:"
if [[ "$FORMAT" == "apk" ]]; then
  BT="$(ls -d "${ANDROID_HOME:-$HOME/Library/Android/sdk}"/build-tools/* | sort -V | tail -1)"
  "$BT/apksigner" verify --print-certs "$DEST" | grep -E "SHA-256|DN" || true
else
  keytool -printcert -jarfile "$DEST" | grep -E "Owner|SHA256" || true
fi
echo "==> $DEST"
echo "Bump expo.android.versionCode in app.json before the next Play upload."
