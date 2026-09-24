#!/bin/sh
# Download the glyph ranges the bundled styles use, at image build time, so
# clients load fonts from the tile server instead of protomaps.github.io.
# Usage: fetch-glyphs.sh <basemaps-assets commit> <output dir> "<font>"...
set -eu

commit="$1"
out="$2"
shift 2

for font in "$@"; do
  encoded=$(printf '%s' "$font" | sed 's/ /%20/g')
  mkdir -p "$out/$font"
  start=0
  while [ "$start" -le 65280 ]; do
    end=$((start + 255))
    curl -fsSL --retry 3 \
      "https://raw.githubusercontent.com/protomaps/basemaps-assets/$commit/fonts/$encoded/$start-$end.pbf" \
      -o "$out/$font/$start-$end.pbf"
    start=$((start + 256))
  done
done
