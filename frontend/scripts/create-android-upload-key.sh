#!/usr/bin/env bash
# One-time: create the Play upload key and the env file build-android-release.sh
# reads. Both live under ~/.radar-ng, outside the repo. Back them up (e.g. a
# password manager); with Play App Signing a lost upload key can be reset
# through Play Console, but it takes days.
set -euo pipefail

DIR="${RADAR_SECRETS_DIR:-$HOME/.radar-ng}"
KEYSTORE="$DIR/android-upload.jks"
ENV_FILE="$DIR/android-signing.env"
ALIAS="${KEY_ALIAS:-upload}"

if [[ -e "$KEYSTORE" || -e "$ENV_FILE" ]]; then
  echo "Refusing to overwrite: $KEYSTORE or $ENV_FILE already exists." >&2
  exit 1
fi
mkdir -p "$DIR" && chmod 700 "$DIR"

read -r -s -p "Upload keystore password (min 6 chars): " STORE_PASS; echo
read -r -s -p "Repeat password: " STORE_PASS2; echo
[[ "$STORE_PASS" == "$STORE_PASS2" && ${#STORE_PASS} -ge 6 ]] || { echo "Passwords differ or too short." >&2; exit 1; }

# PKCS12 keystores use one password for the store and the key.
keytool -genkeypair -v -storetype PKCS12 -keystore "$KEYSTORE" -alias "$ALIAS" \
  -keyalg RSA -keysize 4096 -validity 10000 \
  -storepass "$STORE_PASS" -keypass "$STORE_PASS" \
  -dname "CN=${KEY_CN:-Radar NG}, O=${KEY_ORG:-Radar NG}"
chmod 600 "$KEYSTORE"

umask 077
cat >"$ENV_FILE" <<ENV
ORG_GRADLE_PROJECT_RADAR_UPLOAD_STORE_FILE=$KEYSTORE
ORG_GRADLE_PROJECT_RADAR_UPLOAD_STORE_PASSWORD=$STORE_PASS
ORG_GRADLE_PROJECT_RADAR_UPLOAD_KEY_ALIAS=$ALIAS
ORG_GRADLE_PROJECT_RADAR_UPLOAD_KEY_PASSWORD=$STORE_PASS
ENV

echo "Created $KEYSTORE and $ENV_FILE."
echo "Upload certificate SHA-256 (Play Console asks for this when registering the upload key):"
keytool -list -v -keystore "$KEYSTORE" -alias "$ALIAS" -storepass "$STORE_PASS" | grep "SHA256:"
