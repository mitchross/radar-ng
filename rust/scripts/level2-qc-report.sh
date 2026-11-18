#!/usr/bin/env bash
# Fetch + verify hazard-cases.json volumes, then run qc_review: level2-qc-report.sh [OUT_DIR]; see docs/develop/level2-qc-review.md.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
CASES="$ROOT/crates/radar-level2/testdata/hazard-cases.json"
ARCHIVE="${LEVEL2_ARCHIVE_DIR:-${TMPDIR:-/tmp}/level2-archive}"
OUT="${1:-${TMPDIR:-/tmp}/level2-qc-report}"
mkdir -p "$ARCHIVE" "$OUT"

sha() { if command -v sha256sum >/dev/null; then sha256sum "$1"; else shasum -a 256 "$1"; fi | cut -d' ' -f1; }

jq -r '.[] | select(.file != null) | [.file, .url, .bytes, .sha256] | @tsv' "$CASES" |
while IFS=$'\t' read -r file url bytes want; do
  path="$ARCHIVE/$file"
  if [ ! -f "$path" ]; then
    curl -fsSL --retry 3 -o "$path.part" "$url"
    mv "$path.part" "$path"
  fi
  got_bytes=$(wc -c < "$path" | tr -d ' ')
  got=$(sha "$path")
  if [ "$got_bytes" != "$bytes" ] || [ "$got" != "$want" ]; then
    echo "FAIL  $file: $got_bytes bytes, sha256 $got (want $bytes, $want)" >&2
    exit 1
  fi
  case "$file" in *.gz) [ -f "${path%.gz}" ] || gunzip -kf "$path" ;; esac
  echo "OK    $file $bytes $got"
done

jq -r '.[] | select(.file == null) | "MISSING  \(.category | join(",")): \(.reason)"' "$CASES"

cd "$ROOT"
cargo run --release --locked -p radar-level2 --example qc_review -- \
  --cases "$CASES" --archive "$ARCHIVE" --out "$OUT"
echo "metrics and panels: $OUT"
