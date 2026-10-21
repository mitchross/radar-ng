# The web radar

The radar, forecast and alerts in a browser. It uses the same server, map tiles and data as the
phone app, and fetches nothing from third parties. The code is in [`web/`](../../web/README.md).

## Try it locally

**You need:** [Bun](https://bun.sh) and a running radar-ng server (see
[getting-started](../start/getting-started.md)).

```bash
cd web
bun install
RADAR_API=http://localhost:8080 bun run dev    # your server's address
```

Open <http://localhost:5173>.

**How to know it worked:** the map shows radar, and the play button steps through frames. If the
map is empty, check that `curl $RADAR_API/api/health` answers. Without `RADAR_API`, the dev server
uses the upstream project's public server.

## Host it for real

The web radar is a folder of static files (`bun run build` writes them to `web/dist/`), or the
`ghcr.io/<owner>/radar-ng-web` image (nginx on port 8080, health check `/healthz`).

**It must live on the same address as your server.** The server doesn't send CORS headers, so the
browser only loads data from the site's own host. Put a reverse proxy or ingress in front that
sends:

| Path | Goes to |
|---|---|
| `/api`, `/tiles`, `/packed`, `/basemap` | the tile-server (port 8080) |
| everything else (`/`) | the web radar |

The upstream cluster does this with one Gateway API `HTTPRoute` (two rules on one hostname). Any
proxy that routes by path works: Caddy, Traefik, nginx, an Ingress.

## Analytics

Off unless you build with `VITE_POSTHOG_KEY` (and optionally `VITE_POSTHOG_HOST`). If you turn
them on with your own PostHog host, add that host to the CSP in `web/security-headers.conf`.

## What it shows

- **Map:** radar (what's happened, then the nowcast to +60 min, then the HRRR forecast) and the
  other layers your server has, over a light, dark or satellite basemap.
- **Playback:** a 1-hour or 48-hour loop. Space plays and pauses, the arrow keys step.
- **Click anywhere** to read the value at that point.
- **Forecast panel:** city search, current conditions, alerts, hourly, rain for the next 24 hours,
  7-day, and air quality.

Developer details (packed radar tiles, `?raster=1`, tests) are in [web/README.md](../../web/README.md).
