#!/bin/sh
# Tile cleanup: delete expired tile directories.
# Handles both the legacy layout (/data/tiles/{layer}/{timestamp}/...) and
# the multi-palette layout (/data/tiles/{layer}/{palette}/{timestamp}/...).
TILE_DIR="${TILE_DIR:-/data/tiles}"

echo "Tile cleanup running at $(date -u)"

# Entry points are timestamp dirs — whether they live 1 or 2 levels under /tiles/{layer}.
# We recognize a timestamp dir by name starting with a digit (ISO date).

cleanup_layer() {
    layer="$1"
    retention_minutes="$2"
    base="$TILE_DIR/$layer"
    [ -d "$base" ] || return

    # Legacy layout: /{layer}/{timestamp}/
    find "$base" -mindepth 1 -maxdepth 1 -type d -name "[0-9]*" -mmin +"$retention_minutes" \
        -exec rm -rf {} + 2>/dev/null

    # Multi-palette layout: /{layer}/{palette}/{timestamp}/
    find "$base" -mindepth 2 -maxdepth 2 -type d -name "[0-9]*" -mmin +"$retention_minutes" \
        -exec rm -rf {} + 2>/dev/null
}

# Radar (MRMS): keep 4 hours
cleanup_layer radar 240

# Nowcast (pysteps, Phase 2): keep 1 hour
cleanup_layer nowcast 60

# HRRR forecast layers: keep 12 hours (extended runs may go to +48h)
for layer in radar-hrrr temperature dewpoint humidity wind cape precip-type; do
    cleanup_layer "$layer" 720
done

echo "Cleanup complete"
