#!/bin/sh
# Init container (alpine): make sure /data/basemap.pmtiles holds the BBOX extract of the newest
# Protomaps daily build. Skips the work when the marker says it is already done.
# Marker format must stay "bbox=<BBOX> refresh=<REFRESH>" (plus " url=<PMTILES_URL>" when pinned) or every
# restart re-runs the extract. Used by the Kubernetes init container and by Compose's basemap-bootstrap.
set -eu

BASE_URL="https://build.protomaps.com"
BBOX="${BBOX:?BBOX env is required (e.g. -125,24,-66,50)}"
REFRESH="${REFRESH:-0}"
MAX_LOOKBACK="${MAX_LOOKBACK:-14}"
OUT="${OUT:-/data/basemap.pmtiles}"
TMP="${OUT}.tmp"
MARKER="${OUT}.marker"

# go-pmtiles release used for the extract; keep in step with the basemap container image.
PMTILES_VERSION="1.31.2"
SHA256_x86_64="3ed7dbf4ec2e6dfe5e25b6f70d1ffc932729f93c86db353bf514dd71010a312f"
SHA256_arm64="f8bd47e7ea866863489cad588fbaf2f31f42e5821f7a03f009b3769f05801cb1"

KEY="bbox=${BBOX} refresh=${REFRESH}${PMTILES_URL:+ url=${PMTILES_URL}}"
if [ -s "$OUT" ] && [ -f "$MARKER" ] && [ "$(head -n1 "$MARKER")" = "$KEY" ]; then
  echo "marker matches (${KEY}); extract already done"
  exit 0
fi

# The go-pmtiles image is distroless (no shell), so fetch the same release as a static binary.
case "$(uname -m)" in
  x86_64 | amd64) arch=x86_64; sum="$SHA256_x86_64" ;;
  aarch64 | arm64) arch=arm64; sum="$SHA256_arm64" ;;
  *) echo "FATAL: unsupported architecture $(uname -m)" >&2; exit 1 ;;
esac
tarball="go-pmtiles_${PMTILES_VERSION}_Linux_${arch}.tar.gz"
cd /tmp
wget -q -O "$tarball" "https://github.com/protomaps/go-pmtiles/releases/download/v${PMTILES_VERSION}/${tarball}"
echo "${sum}  ${tarball}" | sha256sum -c -
tar -xzf "$tarball" pmtiles
PMTILES=/tmp/pmtiles

# PMTILES_URL pins a specific build (optional). Otherwise: no `latest` alias and ~6-day retention,
# so walk back from today to the newest build that exists.
now=$(date -u +%s)
resolved="${PMTILES_URL:-}"
build="${resolved:+pinned}"
i=0
while [ -z "$resolved" ] && [ "$i" -le "$MAX_LOOKBACK" ]; do
  d=$(date -u -d "@$((now - i * 86400))" +%Y%m%d)
  if wget -q --spider -T 30 "${BASE_URL}/${d}.pmtiles"; then
    resolved="${BASE_URL}/${d}.pmtiles"
    build="$d"
    break
  fi
  echo "no build for ${d} (probe ${i}/${MAX_LOOKBACK})"
  i=$((i + 1))
done

if [ -z "$resolved" ]; then
  echo "FATAL: no Protomaps build found in the last ${MAX_LOOKBACK} days" >&2
  exit 1
fi

# Extract to a temp path and rename, so a crash never leaves a partial file that passes the -s check.
echo "extracting bbox=${BBOX} from ${resolved}"
"$PMTILES" extract "$resolved" "$TMP" --bbox="$BBOX"
mv -f "$TMP" "$OUT"
printf '%s\nbuild=%s extracted=%s\n' "$KEY" "$build" "$(date -u +%Y-%m-%dT%H:%M:%SZ)" >"$MARKER"
echo "done: $(ls -lh "$OUT")"
