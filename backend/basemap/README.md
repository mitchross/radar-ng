# Protomaps self-hosted basemap

Serves vector XYZ tiles from a local PMTiles archive (North America ~35 GB).
The app uses these tiles when `dataSource === "selfhosted"`.

## One-time PMTiles download

Protomaps builds are published at <https://build.protomaps.com/>. Pick a recent
`.pmtiles` build (global ~120 GB, North America regional ~35 GB) and drop it at
the path referenced by the compose volume.

```sh
# Example — regional North America
mkdir -p ./backend/pmtiles-data
curl -L -o ./backend/pmtiles-data/basemap.pmtiles \
  "https://build.protomaps.com/<BUILD_DATE>.pmtiles"
```

The `basemap` compose service mounts `./backend/pmtiles-data` at `/data` and serves the
file. Replace the file in-place to update; `go-pmtiles` picks up the new bytes
without a restart.

## Tile URL

The app loads tiles through Caddy (so CORS + caching are consistent):

- Style JSON:  `/basemap/styles/positron.json`, `/basemap/styles/dark-matter.json`
- XYZ tiles:   `/basemap/tiles/{z}/{x}/{y}.mvt`

Both are proxied to the `basemap` service (port 8081) by the tile-server
Caddyfile.

## Styles

`styles/positron.json` and `styles/dark-matter.json` are minimal MapLibre style
docs adapted from Protomaps' basemaps-assets repo. They reference the
`/basemap/tiles/{z}/{x}/{y}.mvt` endpoint and resolve `{serverUrl}` relative to
whatever the app requested them from.
