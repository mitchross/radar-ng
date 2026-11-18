#!/usr/bin/env bash
# Collect the "If it fails" evidence from #252 off a plugged-in Android device.
#
#   bash scripts/android-inmap-evidence.sh clear            # before the test: empty the log buffer
#   bash scripts/android-inmap-evidence.sh screenshot NAME  # during the test: save NAME.png
#   bash scripts/android-inmap-evidence.sh collect          # after: logcat, cache folder, device info
#
# Output goes to $EVIDENCE_DIR (default /tmp/radar-android-evidence/<timestamp>).
# Set ANDROID_SERIAL when more than one device is plugged in.
# Not run on a device when written. It reads and saves; it changes nothing on the phone
# except clearing the log buffer.
set -uo pipefail
cd "$(dirname "$0")/.." || exit 2

command -v adb >/dev/null || { echo "adb not found. Install Android platform-tools." >&2; exit 2; }

APP_ID="$(node -p "require('./identity').androidPackage" 2>/dev/null || echo com.vanillax.radarng)"
ADB=(adb)
[[ -n "${ANDROID_SERIAL:-}" ]] && ADB=(adb -s "$ANDROID_SERIAL")

if ! "${ADB[@]}" get-state >/dev/null 2>&1; then
  echo "No device found. Plug it in, allow USB debugging, check 'adb devices'." >&2
  exit 2
fi

STAMP_FILE="${EVIDENCE_STAMP_FILE:-/tmp/radar-android-evidence.latest}"
cmd="${1:-collect}"

# The folder the last `clear` made, else a new timestamped one.
evidence_dir() {
  if [[ -n "${EVIDENCE_DIR:-}" ]]; then
    echo "$EVIDENCE_DIR"
  elif [[ -s "$STAMP_FILE" ]]; then
    cat "$STAMP_FILE"
  else
    echo "/tmp/radar-android-evidence/$(date +%Y%m%d-%H%M%S)"
  fi
}

case "$cmd" in
  clear)
    OUT="${EVIDENCE_DIR:-/tmp/radar-android-evidence/$(date +%Y%m%d-%H%M%S)}"
    mkdir -p "$OUT"
    echo "$OUT" > "$STAMP_FILE"
    "${ADB[@]}" logcat -c
    echo "Log buffer cleared. Evidence folder: $OUT"
    ;;
  screenshot)
    name="${2:?usage: screenshot NAME}"
    OUT="$(evidence_dir)"
    mkdir -p "$OUT"
    "${ADB[@]}" exec-out screencap -p > "$OUT/$name.png"
    echo "Saved $OUT/$name.png"
    ;;
  collect)
    OUT="$(evidence_dir)"
    mkdir -p "$OUT"
    R="$OUT/results.txt"
    : > "$R"
    say() { echo "$*" | tee -a "$R"; }

    say "radar-ng #252 Android evidence, $(date -u +%FT%TZ)"
    say "app id: $APP_ID"
    say "model: $("${ADB[@]}" shell getprop ro.product.model | tr -d '\r')"
    say "android: $("${ADB[@]}" shell getprop ro.build.version.release | tr -d '\r') (sdk $("${ADB[@]}" shell getprop ro.build.version.sdk | tr -d '\r'))"
    say "app version: $("${ADB[@]}" shell dumpsys package "$APP_ID" | tr -d '\r' | grep -m1 versionName || echo 'not installed')"
    say ""

    # 1. Log lines for the app.
    "${ADB[@]}" logcat -d -v time > "$OUT/logcat-full.txt" 2>&1
    grep -iE "radar|maplibre|inmap" "$OUT/logcat-full.txt" > "$OUT/logcat-filtered.txt" || true
    grep -iE "FATAL EXCEPTION|ANR in" -A8 "$OUT/logcat-full.txt" > "$OUT/crashes.txt" || true
    say "logcat: $(wc -l < "$OUT/logcat-filtered.txt") matching lines of $(wc -l < "$OUT/logcat-full.txt") (logcat-filtered.txt)"
    say "crashes: $(grep -ciE "FATAL EXCEPTION|ANR in" "$OUT/logcat-full.txt" || true) FATAL EXCEPTION / ANR lines (crashes.txt)"

    # 2. The radar-inmap cache folder. run-as works only on debuggable builds, so a release
    #    build usually falls through to the "not readable" line. Say so rather than guess.
    mkdir -p "$OUT/radar-inmap"
    if "${ADB[@]}" shell run-as "$APP_ID" ls cache/radar-inmap >/dev/null 2>&1; then
      "${ADB[@]}" shell run-as "$APP_ID" find cache/radar-inmap -type f > "$OUT/radar-inmap/files.txt" 2>&1
      say "radar-inmap: $(wc -l < "$OUT/radar-inmap/files.txt") files listed (radar-inmap/files.txt)"
      if "${ADB[@]}" exec-out run-as "$APP_ID" tar -cf - cache/radar-inmap > "$OUT/radar-inmap/radar-inmap.tar" 2>/dev/null; then
        say "radar-inmap: tar saved (radar-inmap/radar-inmap.tar)"
      else
        say "radar-inmap: tar failed; file list only"
      fi
    else
      say "radar-inmap: not readable. Release builds block run-as. Use a debug build, or read the 'file ok/missing' and 'png ... ok/bad' parts of the status line."
    fi

    # 3. Screen now, for the status line text.
    if "${ADB[@]}" exec-out screencap -p > "$OUT/final-screen.png" 2>/dev/null; then
      say "screenshot: final-screen.png"
    fi

    # 4. What only a human can write down.
    say ""
    say "Copy in by hand (the script cannot read these):"
    say "  - the Fast radar status line text from Settings (error stage, fallback reason)"
    say "  - PostHog inmap_diag events for this device and time"
    say "  - your pass/fail for each of the 7 checks (docs/app/phone-acceptance.md, part A)"
    echo
    echo "Evidence folder: $OUT"
    ;;
  *)
    echo "usage: $0 clear | screenshot NAME | collect" >&2
    exit 64
    ;;
esac
