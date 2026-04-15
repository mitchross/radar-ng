#!/bin/bash
# Tile cleanup: delete expired tile directories
TILE_DIR="${TILE_DIR:-/data/tiles}"

echo "Tile cleanup running at $(date -u)"

# Radar (MRMS): keep 4 hours
find "$TILE_DIR/radar" -mindepth 1 -maxdepth 1 -type d -mmin +240 -exec rm -rf {} + 2>/dev/null

# HRRR layers: keep 8 hours
for layer in radar-hrrr temperature wind cape precip-type; do
    find "$TILE_DIR/$layer" -mindepth 1 -maxdepth 1 -type d -mmin +480 -exec rm -rf {} + 2>/dev/null
done

echo "Cleanup complete"
