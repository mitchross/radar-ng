# Privacy — what Radar NG sends, to whom, and for how long

Written for task 1.4 of `docs/tasks/2026-09-23-mobile-audit-and-plan.md` (findings F-R6, F-P9).
Covers the iOS app, the watchOS app and the radar widget. The CarPlay target is included but is
built only with `RADAR_CARPLAY=1` and is not in the currently submitted build.

This document is the source for the App Store Connect privacy labels and the Play Console Data
safety form (§5, §6). Update it in the same change as any new network call.

## 1. Coordinate precision

Every position that leaves the app is rounded in one place, `frontend/src/lib/coordinates.ts`:

| `PRECISION` | Decimals | Ground distance | Applies to |
| --- | --- | --- | --- |
| `WEATHER` | 2 | ≈1.1 km | `/api/forecast`, `/api/nowcast`, `/api/storm-prefetch` |
| `POINT` | 3 | ≈110 m | `/api/alerts`, `/api/reverse-geocode`, `/api/inspect` |

Rounding is applied at the API boundary (`src/lib/api.ts`, `src/lib/inspector.ts`) rather than at
the call sites, so a new caller cannot forget it. `targets/watch/WatchAPI.swift` mirrors the same
two values. React-query keys carry `locationKey(...)` instead of the raw pair, so GPS jitter no
longer creates a new cache entry — and therefore a new request — every few metres (F-P9).

**Why 2 decimals costs no accuracy.** MRMS radar and the self-hosted nowcast grid are 1 km cells,
and the Open-Meteo models behind the forecast are 1–11 km. Both are already coarser than a 1.1 km
fix, so the forecast and nowcast answers are byte-identical.

**Why alerts keep 3 decimals.** NWS alert polygons can be much smaller than a forecast cell, so at
1.1 km a point near a warning boundary could be rounded to the wrong side of it. 110 m makes that
negligible.

**Store consequence to be aware of.** Apple's taxonomy puts street-level location in *Precise
Location* and city/postal level in *Coarse Location*. The 2-decimal weather paths qualify as
coarse; the 3-decimal alert path is still street-level and so counts as precise. Rounding alerts to
2 decimals as well would let the whole app declare coarse location only, at the cost of the
boundary behaviour above. This is a deliberate, reversible tradeoff — not an oversight.

The raw GPS fix is *not* rounded in the store: the map marker legitimately uses full precision on
device, where it never leaves the app except through the two paths above.

## 2. What leaves the device

| Destination | Request | Position sent | Purpose |
| --- | --- | --- | --- |
| Self-hosted tile server (default `https://radar-ng-api.vanillax.me`; user-configurable) | `GET /api/forecast/{lat}/{lon}` | 2 dp | forecast |
| " | `GET /api/nowcast/{lat}/{lon}` | 2 dp | radar motion nowcast |
| " | `GET /api/storm-prefetch?lat&lon&zoom&palette` | 2 dp | offline storm tiles |
| " | `GET /api/inspect/{layer}/{timestamp}/{lat}/{lon}` | 3 dp | layer value at the marker |
| " | `GET /api/manifest.json`, `/api/health`, `/api/livez` | none (IP + UA only) | frames, health |
| " | `GET /tiles/{layer}/{palette}/{timestamp}/{z}/{x}/{y}.png` | none — z/x/y reveals the viewed area | radar frames |
| " | `GET /basemap/tiles/{z}/{x}/{y}.mvt` | none — as above | vector basemap |
| " | `GET /api/alerts?lat&lon` | 3 dp | active NWS alerts; the **server** asks `api.weather.gov` with its own contact User-Agent |
| " | `GET /api/geocode?q=` | none — the typed city name only | city search; the server asks the self-hosted Photon |
| " | `GET /api/reverse-geocode?lat&lon` | 3 dp | place label for the device position; the server asks the self-hosted Photon |
| " | `GET /basemap/imagery/{z}/{x}/{y}.jpg`, or the self-hosted VersaTiles host's `/imagery/...` in release builds | none — z/x/y reveals the viewed area | satellite imagery; the **server** fetches public-domain USGS tiles |
| Apple, via `MKDirections` (**CarPlay build only**) | route request with `MKMapItem(location:)` | **full precision** | turn-by-turn routing |
| Configured OTLP endpoint (opt-in, off by default) | traces and logs | none — location-shaped attributes are stripped by `src/lib/telemetryPrivacy.ts` | diagnostics |

Notably **not** sent anywhere: the device's position is never included in telemetry, never written
to the widget's snapshot, and never part of the watch hand-off.

The app sends no `User-Agent` to NWS at all: the server makes that request, using the
`NWS_USER_AGENT` contact it already uses for storm-watch polling.

## 3. Removed: third-party geocoding and direct NWS calls

The self-hosted-only rule (plan Phase S, 2026-09-24) means the app never calls a third party at
runtime. Three calls moved behind the tile server:

- **Reverse geocoding.** It was public Nominatim on every GPS fix, and briefly the platform
  geocoder (`CLGeocoder` / `android.location.Geocoder`, which still sends the position to Apple or
  Google). It is now `GET /api/reverse-geocode`, which the server answers from a self-hosted Photon
  instance. Results are cached per rounded 110 m cell for five minutes on the device and for a day on
  the server, and concurrent callers share one lookup.
- **City search.** It was Open-Meteo's public geocoding API. It is now `GET /api/geocode`, backed by
  the same Photon.
- **Weather alerts.** The phone and Watch called `api.weather.gov` directly. Now the server proxies
  NWS's own point matching via `GET /api/alerts`, with a 60 s cache. An upstream failure is a 502,
  never an empty list.

Satellite imagery no longer comes from Esri: the server fetches public-domain USGS orthoimagery and
serves it (plan task S.1). The app contacts no third party at runtime.

## 4. Retention

### 4.1 Tile-server access logs — retained for ~24 hours

The forecast, nowcast, storm-prefetch and inspect paths contain the rounded position, and the tile
server logs them. Verified read-only against the cluster on 2026-09-24:

- `backend/api/Caddyfile` ends with `log { output stdout format console }`. Log records carry the
  full `uri`, e.g. `"uri": "/api/forecast/42.96/-85.67"`.
- The uvicorn process logs the same path a second time
  (`INFO: 10.244.5.241:0 - "GET /api/livez HTTP/1.1" 200 OK`), so a coordinate appears **twice per
  request**. Removing Caddy's `log` directive alone would not stop it.
- The `opentelemetry/OpenTelemetryCollector/otel-agent` DaemonSet tails `/var/log/pods/*/*/*.log`
  and excludes only `opentelemetry_*` and `loki-stack_*`, so `radar-ng/tile-server` is in scope. It
  exports through `otel-gateway` to Loki.
- `loki-stack/ConfigMap/loki` sets `retention_period: 24h`, `retention_delete_delay: 2h` and
  `retention_enabled: true` (compaction every 10 min). Storage is S3.
- On the node, kubelet keeps `containerLogMaxSize: 10Mi` × `containerLogMaxFiles: 5` per container —
  size-bounded, not time-bounded, and removed with the pod.

Net effect: a rounded position is queryable in the homelab's Loki for roughly 24–26 hours, and
duplicated in node-local container logs until ~50 MB rotates.

UNVERIFIED outside the cluster: the S3 backend (`192.168.10.133`) may have bucket lifecycle rules
or ZFS snapshots that retain deleted chunks past Loki's 24 h.

### 4.2 Everything else

The app has no analytics SDK, no crash reporter and no advertising SDK. Telemetry is opt-in and
inert unless `EXPO_PUBLIC_TELEMETRY_ENABLED=1` and `EXPO_PUBLIC_OTLP_BASE` are both set at build
time.

Upstream services the *server* calls (NWS, the self-hosted Photon) receive the server's address, not
the device's.

## 5. Draft App Store Connect answers

Privacy labels for the submitted build (default server = first-party):

- **Location → Coarse Location.** Collected. Purposes: App Functionality. Not used for tracking.
  Not linked to the user's identity. Sent only to the developer's own tile server.
- **Location → Precise Location.** Collected, but only on the alerts path (§1). Same purposes and
  linkage. *If the executor round alerts to 2 decimals, drop this row.*
- **Identifiers.** None collected by the app. The server sees source IPs in access logs (§4.1); do
  not declare an identifier unless the logs begin feeding an account or an ad system.
- **Diagnostics.** Only if a build ships with `EXPO_PUBLIC_TELEMETRY_ENABLED=1`. Otherwise none.
- **Tracking.** No. Nothing is used to track across apps or websites, and the app contacts no third
  party at runtime.
- **Privacy policy URL.** Required for the location rows and **not yet present** in `app.json`.

## 6. Draft Play Console Data safety answers

Android has never been built natively (see the plan's Phase 6), so these are the answers the form
will need once it exists:

- **Location → Approximate location.** Collected, not shared, encrypted in transit, required for app
  functionality, ephemeral processing. Not used for advertising.
- **Location → Precise location.** Collected, same handling — or drop this row if alerts move to 2
  decimals.
- **Data deletion.** No account and no server-side user record, so there is nothing for a user to
  request deletion of; the access-log retention in §4.1 is the one caveat and it is bounded at ~24 h.
- **Data collected / shared.** Nothing else. Confirm no third-party SDK is bundled that reports
  independently.
- `ACCESS_FINE_LOCATION` is requested alongside `ACCESS_COARSE_LOCATION`. The app asks for
  `Location.Accuracy.Balanced`, so check whether `ACCESS_FINE_LOCATION` is still needed before
  answering — an unnecessary permission is an easy rejection.

## 7. Open items

1. NWS `User-Agent` contact address (blocked on the user supplying one).
2. Privacy policy URL for both stores (blocked on the user; no such page exists yet).
3. ~~Esri satellite terms~~ — resolved: satellite is self-hosted USGS imagery (D7, task S.1).
4. `ACCESS_FINE_LOCATION` necessity on Android.
5. Retention at the S3 backend for Loki (§4.1, UNVERIFIED).
6. **The privacy manifest exists but declares no collected data.** `frontend/ios/radarng/PrivacyInfo.xcprivacy`
   is generated during `pod install` (aggregated from the CocoaPods manifests, because
   `expo-build-properties`' `apple.privacyManifestAggregationEnabled` defaults to `true`), and is
   referenced from `project.pbxproj`. It declares the three required-reason API types the native
   pods need — `UserDefaults`/`CA92.1`, `SystemBootTime`/`35F9.1`, `FileTimestamp`/`C617.1` — plus
   `NSPrivacyCollectedDataTypes: []` and `NSPrivacyTracking: false`.

   The empty collected-data list is the open question the audit (F-R6) flags: the app does send a
   rounded position to a server, so confirm that `NSPrivacyCollectedDataTypes` and the App Store
   Connect answers in §5 say the same thing. Apple treats App Store Connect as authoritative and the
   manifest list as optional, so an empty list is defensible — an inconsistent one is not.

   Note this file only appears **after** `pod install`. Checking `frontend/ios/` on a tree where the
   prebuild or pods have not been re-run will show no manifest, which is not evidence that the
   shipped app lacks one.
