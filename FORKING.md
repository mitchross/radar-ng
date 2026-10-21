# Forking radar-ng

Your own copy of radar-ng: your app name on your phone, pointing at your server, published under
your GitHub account. Nothing here is needed to **run** the backend; that's
[docs/self-host/](docs/self-host/compose.md). This page is for when you build and ship **the app**.

Every value below falls back to the upstream project's own value when you don't set it, so an
unchanged clone still builds exactly like upstream.

## Before you fork

Read [docs/reference/third-party-terms.md](docs/reference/third-party-terms.md) before you put a build
in a store. In short:

- **Satellite basemap (Esri World Imagery):** the public endpoint is commercially licensed imagery.
  Shipping it in an app without an Esri agreement is a risk. The doc lists safer options (drop
  satellite, or self-host public-domain imagery such as NAIP).
- **NWS:** api.weather.gov needs a real contact in the User-Agent. Use your own email (below).
- **Nominatim:** the app no longer calls it, so there's nothing to set.

## Checklist

### 1. Pick your app's identity

Set these in `frontend/.env` (a plain `KEY=value` file; create it if it doesn't exist) or in your shell:

| Variable | What it is | Upstream value (used when unset) |
|---|---|---|
| `RADAR_BUNDLE_ID` | iOS bundle ID. The widget and Watch derive theirs (`.radar-widget`, `.watch`) | `com.vanillax.radar-ng` |
| `RADAR_APP_GROUP` | App Group shared by the app, widget and CarPlay | `group.<RADAR_BUNDLE_ID>` |
| `RADAR_ANDROID_PACKAGE` | Android applicationId | `com.vanillax.radarng` |
| `RADAR_APPLE_TEAM_ID` | Your Apple Developer team ID (signing) | upstream's team |

All four are read by [`frontend/identity.js`](frontend/identity.js), the one place they live.

### 2. Point the app at your server

| Variable | What it is | Upstream value |
|---|---|---|
| `EXPO_PUBLIC_RADAR_DEFAULT_URL` | The stack a fresh install talks to. Users can still change it in **Settings → Advanced Mode → Stack URL** | `https://radar-ng-api.vanillax.me` |
| `RADAR_BASEMAP_RASTER_URL` | tileserver-gl raster endpoint for the Watch and widget map images | `https://maps.vanillax.me/raster` |

Release builds also read the basemap styles from [`frontend/.env.production`](frontend/.env.production).
Point those lines at your own map server, or delete them to use the bundled Protomaps style.

### 3. Copy the values into the native code

Swift can't read environment variables at runtime, so the widget, CarPlay and Watch keep their own
copy. After changing anything in steps 1–2:

```bash
cd frontend
node scripts/sync-native-identity.js        # rewrites the Swift constants
node scripts/sync-native-identity.js --check  # prints nothing when everything matches
```

Commit the result. The test suite runs `--check`, so a mismatch fails `bun run test`.

### 4. Set your NWS contact on the server

In your backend's settings (`deploy/.env` for Compose, `deploy/kubernetes/base/settings.env` for
Kubernetes): `NWS_USER_AGENT=(radar-ng, you@example.com)`, with your own email.

### 5. Images publish under your account automatically

The GitHub workflows publish to `ghcr.io/<your-github-user>/radar-ng-*` (lowercased). There's
nothing to edit. Point your deployment at your images instead of `ghcr.io/mitchross/…` when you're
ready.

### 6. Build the app

Identity and the native constants are baked into the native build, so **after steps 1–3 you need a
native rebuild** (a fresh `expo prebuild` / Xcode or Gradle build, or `bun run ios:ipa`). A Metro
reload isn't enough. See [docs/app/running-the-app.md](docs/app/running-the-app.md).

## The web radar

The web radar (`web/`) is served from the same host as the API, so there's no default URL to set.

- Local dev proxies to `RADAR_API` (defaults to the upstream API). Set it to your server:
  `RADAR_API=https://your-api.example.com bun run dev`.
- **Analytics are off** unless you build with `VITE_POSTHOG_KEY`. `VITE_POSTHOG_HOST` sets your
  PostHog host. `web/security-headers.conf` allows upstream's PostHog host in its CSP; change that
  line if you turn analytics on with your own host.
