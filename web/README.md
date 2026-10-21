# Radar NG web

> Running or hosting it? Start with [docs/app/web-radar.md](../docs/app/web-radar.md). This page is for developers.

The radar, forecast and alerts from the self-hosted stack in a browser: MapLibre GL JS over the same manifest, tiles, basemap and API the app uses. Nothing is fetched from a third party.

```bash
bun install
bun run dev        # http://localhost:5173, proxied to the upstream API by default
RADAR_API=http://localhost:8080 bun run dev   # or any tile-server
bun run test
bun run build      # static files in dist/
```

**Same-origin only.** The API sends no CORS headers, and the upstream deployment's Cloudflare hotlink protection rejects tile requests whose Referer isn't one of its own pages. The dev proxy therefore forwards `/api`, `/tiles`, `/packed` and `/basemap` and drops the Referer. In production, serve the site from the same host that routes those paths to the tile-server (see [web-radar.md](../docs/app/web-radar.md#host-it-for-real)).

What it shows:
- **Map:** radar (observed, then nowcast to +60 min, then HRRR) and the other layers the manifest has frames for, over the light, dark or satellite basemap.
- **Packed radar:** frames with a `packed` slot in the manifest draw from packed data tiles (docs/reference/packed-tiles.md) in a WebGL layer; the rest use PNG rasters. Add `?raster=1` to the URL to force PNG rasters.
- **Playback:** a 1h or 48h loop that preloads the next frames, the same sequence as the app. The keyboard works too: space plays and pauses, the arrow keys step.
- **Click a point** to read the value there (nowcast and HRRR frames included).
- **Forecast panel:** city search, current conditions, NWS alerts, hourly, rain for the next 24h, 7-day, and conditions including air quality.
