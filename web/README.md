# Radar NG web

The radar, forecast and alerts from the self-hosted stack in a browser: MapLibre GL JS over the same manifest, tiles, basemap and API the app uses. Nothing is fetched from a third party.

```bash
bun install
bun run dev        # http://localhost:5173, proxied to radar-ng-api.vanillax.me
RADAR_API=http://localhost:8080 bun run dev   # or any tile-server
bun run test
bun run build      # static files in dist/
```

**Same-origin only.** The API sends no CORS headers, and Cloudflare's hotlink protection rejects tile requests whose Referer isn't a vanillax.me page. The dev proxy therefore forwards `/api`, `/tiles` and `/basemap` and drops the Referer; in production the site must be served from a vanillax.me host that also routes those paths to the tile-server.

What it shows:
- **Map:** radar (observed, then nowcast to +60 min, then HRRR) and the other layers the manifest has frames for, over the light, dark or satellite basemap.
- **Playback:** a 1h or 48h loop that preloads the next frames, the same sequence as the app. The keyboard works too: space plays and pauses, the arrow keys step.
- **Click a point** to read the value there (nowcast and HRRR frames included).
- **Forecast panel:** city search, current conditions, NWS alerts, hourly, rain for the next 24h, 7-day, and conditions including air quality.
