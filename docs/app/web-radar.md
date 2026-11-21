# The web radar

The radar, forecast and alerts in a browser. It uses the same server, map tiles and data as the
phone app, and fetches nothing from third parties. The code is in [`web/`](../../web/README.md).

## Try it locally

You need [Bun](https://bun.sh) and a running radar-ng server (see
[getting-started](../start/getting-started.md)).

```bash
cd web
bun install
RADAR_API=http://localhost:8080 bun run dev    # your server's address
```

Open <http://localhost:5173>.

To check it worked, look for radar on the map and a play button that steps through frames. If the
map is empty, check that `curl $RADAR_API/api/health` answers. Without `RADAR_API`, the dev server
uses the upstream project's public server.

## Host it for real

The tile-server serves it. The `radar-ng-tile-server` image carries the built site and answers
every path that is not `/api`, `/tiles`, `/basemap`, `/replays`, `/level2` or `/v1` with it. Open
your server's address in a browser (`http://localhost:8080` on the Compose stack).

- Route the whole hostname to the tile-server (port 8080). No second container or path rule is needed.
- A web change ships with the next backend image release.
- To serve your own build, run `bun run build` and set `WEB_ROOT` to the `dist/` folder.
- To turn the web radar off, set `WEB_ROOT` to an empty value. The server then answers 404 on `/`.

The site and the API share one address because the server sends no CORS headers.

## Analytics

Umami. Off unless you build the image with `VITE_UMAMI_HOST` and `VITE_UMAMI_WEBSITE_ID`. The
image build copies the host into `WEB_ANALYTICS_ORIGIN`, the one third-party origin the page's CSP
allows. Only the allowlisted product events in `web/src/analytics.ts` are sent, never coordinates
or search text. Session replay (Umami `recorder.js`) starts after any `alertBounds` link is cleared
from the URL; Umami's per-site settings mask text. Do Not Track and the Settings privacy switch turn both off.

## What it shows

- Map: radar (what's happened, then the nowcast to +60 min, then the HRRR forecast) and the
  other layers your server has, over a light, dark or satellite basemap.
- Playback: a 1-hour or 48-hour loop. Space plays and pauses, the arrow keys step.
- Click anywhere to read the value at that point.
- Forecast panel: city search, current conditions, alerts, hourly, rain for the next 24 hours,
  7-day, and air quality.

Developer details (packed radar tiles, `?raster=1`, tests) are in [web/README.md](../../web/README.md).
