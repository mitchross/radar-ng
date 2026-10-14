#!/usr/bin/env bash
# Decode and render the newest live NOAA files with the release binaries.
#
#   rust/scripts/live-noaa-check.sh [WORK_DIR]      (default: rust/target/live-noaa)
#
# Downloads the newest MRMS MergedBaseReflectivityQC frame and the latest HRRR wrfsfcf01 (+ .idx and
# the ranged 13-variable subset) with the production radar-grib code, then checks:
#   - MRMS decodes to a CONUS lat/lon grid, south-up, with real echoes;
#   - every HRRR variable radar-ng reads has an .idx record and a matching message that decodes;
#   - `radar render-once` renders the MRMS frame into tiles for every palette.
# Needs: cargo (release build), jq, network access to noaa-mrms-pds / noaa-hrrr-bdp-pds.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
WORK="${1:-$ROOT/target/live-noaa}"
CACHE="$WORK/cache"
OUT="$WORK/dump"
TILES="$WORK/tiles"
mkdir -p "$CACHE"
rm -rf "$OUT" "$TILES"

fail=0
ok() { printf 'PASS  %s\n' "$*"; }
bad() { printf 'FAIL  %s\n' "$*"; [ -z "${GITHUB_ACTIONS:-}" ] || echo "::error title=live-noaa::$*"; fail=1; }
trap '[ -z "${GITHUB_ACTIONS:-}" ] || echo "::error title=live-noaa::line $LINENO: $BASH_COMMAND exited $?"' ERR

(cd "$ROOT" && cargo build --release --locked -p radar --bin radar -p radar-bench --bin noaa_dump)
BIN="$ROOT/target/release"

"$BIN/noaa_dump" fetch --cache "$CACHE" --json "$WORK/fetch.json"
mrms=$(jq -r '.mrms.file // empty' "$WORK/fetch.json")
hrrr=$(jq -r '.hrrr.file // empty' "$WORK/fetch.json")
idx=$(jq -r '.hrrr.idx_file // empty' "$WORK/fetch.json")
subset=$(jq -r '.hrrr.subset_file // empty' "$WORK/fetch.json")
[ -n "$mrms" ] && [ -n "$hrrr" ] && [ -n "$idx" ] || { echo "fetch report lacks files:"; cat "$WORK/fetch.json"; exit 1; }
echo "MRMS $(basename "$mrms"); HRRR $(basename "$(dirname "$hrrr")")/$(basename "$hrrr")"

dump=("$BIN/noaa_dump" dump --mrms "$mrms" --hrrr "$hrrr" --hrrr-idx "$idx" --out "$OUT")
[ -n "$subset" ] && dump+=(--hrrr-subset "$subset")
"${dump[@]}"
R="$OUT/rust.json"

# MRMS: 3500 x 7000 CONUS grid at 0.01 deg, flipped south-up, some echoes.
read -r h w lat0 lat1 lon0 lon1 nan < <(jq -r '.mrms | "\(.shape[0]) \(.shape[1]) \(.lat_first) \(.lat_last) \(.lon_first) \(.lon_last) \(.nan_cells)"' "$R")
if [ "$h" = 3500 ] && [ "$w" = 7000 ] \
   && awk -v a="$lat0" -v b="$lat1" -v c="$lon0" -v d="$lon1" 'BEGIN{exit !(a>19 && a<21 && b>54 && b<56 && c>-131 && c<-129 && d>-61 && d<-59)}'; then
  ok "mrms grid ${h}x${w}, lat ${lat0}..${lat1}, lon ${lon0}..${lon1}"
else
  bad "mrms grid ${h}x${w}, lat ${lat0}..${lat1}, lon ${lon0}..${lon1} (want 3500x7000 CONUS, south-up)"
fi
cells=$((h * w))
[ "$nan" -lt "$cells" ] && ok "mrms values present ($nan of $cells cells missing)" || bad "mrms: every cell missing"

# HRRR: each variable has .idx records and a selected, decodable message.
while IFS=$'\t' read -r var selected err records; do
  if [ "$selected" != null ] && [ -z "$err" ] && [ "$records" -gt 0 ]; then
    ok "hrrr $var: message #$selected, $records idx record(s)"
  else
    bad "hrrr $var: selected=$selected idx_records=$records ${err}"
  fi
done < <(jq -r '.hrrr as $h | $h.vars | to_entries[] |
  [.key, (.value.selected // "null" | tostring), (.value.error // ""),
   (($h.idx.picks[.key].records // []) | length | tostring)] | @tsv' "$R")
if [ -n "$subset" ]; then
  # The ranged download (what production fetches) must decode to the same values as the whole file.
  diff_vars=$(jq -r '.hrrr as $h | [$h.subset.vars | to_entries[] | . as $e
    | ([$h.decode[] | select(.index == $h.vars[$e.key].selected) | .values_sha256][0]) as $want
    | select($e.value.error != null or ($e.value.missing // false) or $e.value.values_sha256 != $want)
    | $e.key] | join(",")' "$R")
  [ -z "$diff_vars" ] && ok "hrrr ranged subset matches the whole file" || bad "hrrr ranged subset differs: $diff_vars"
fi

# Render the MRMS frame.
if LOG_LEVEL=warn "$BIN/radar" render-once "$mrms" "$TILES" >/dev/null; then
  for p in classic muted vivid; do
    n=$(find "$TILES/$p" -name '*.png' 2>/dev/null | wc -l)
    [ "$n" -gt 0 ] && ok "render $p: $n tiles" || bad "render $p: no tiles"
  done
else
  bad "radar render-once failed"
fi

exit "$fail"
