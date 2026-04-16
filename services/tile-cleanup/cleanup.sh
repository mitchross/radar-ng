#!/bin/sh
# Tile cleanup: delete expired tile directories.
TILE_DIR="${TILE_DIR:-/data/tiles}"

echo "Tile cleanup running at $(date -u)"

# Radar (MRMS): keep 4 hours
find "$TILE_DIR/radar" -mindepth 1 -maxdepth 1 -type d -mmin +240 -exec rm -rf {} + 2>/dev/null

# Nowcast (pysteps, Phase 2): keep 1 hour
find "$TILE_DIR/nowcast" -mindepth 1 -maxdepth 1 -type d -mmin +60 -exec rm -rf {} + 2>/dev/null

# HRRR forecast layers: keep 12 hours (extended runs may go to +48h)
for layer in radar-hrrr temperature dewpoint humidity wind cape precip-type; do
    find "$TILE_DIR/$layer" -mindepth 1 -maxdepth 1 -type d -mmin +720 -exec rm -rf {} + 2>/dev/null
done

echo "Cleanup complete"
