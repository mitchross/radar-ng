# radar-ng

> Hyper-local weather radar with a self-hosted tile pipeline. CARROT-Weather UI energy, NWS-grade data, runs on your homelab.

[![status](https://img.shields.io/badge/api-radar--ng--api.vanillax.me-blue)](https://radar-ng-api.vanillax.me/api/health) [![expo](https://img.shields.io/badge/expo-SDK%2055-000)](https://expo.dev) [![python](https://img.shields.io/badge/python-3.12-3776AB)](https://www.python.org/) [![talos](https://img.shields.io/badge/talos-v1.13-326CE5)](https://talos.dev) [![argo](https://img.shields.io/badge/argocd-3.x-EF7B4D)](https://argo-cd.readthedocs.io/) [![otel](https://img.shields.io/badge/otel-traces%20%2B%20logs%20%2B%20metrics-425CC7)](https://opentelemetry.io)

![hero](docs/screenshots/hero.png)

---

## Why this exists

Commercial radar apps rate-limit you, slap ads on the freezing-rain warning, and round your zip code to the nearest 5 miles. radar-ng is the opposite: pure NOAA data, sub-2-minute refresh, every layer the public APIs offer, **rendered on hardware you own**. The phone app is the windshield. The Kubernetes pipeline is the engine.

| | radar-ng | most weather apps |
|---|---|---|
| Radar latency | < 3 min from NOAA | 5–15 min |
| Data source | Direct NOAA MRMS / HRRR / NLDN | Third-party aggregator |
| Forecast | Self-hosted Open-Meteo | Vendor-locked |
| Privacy | Your server, your tiles | Sells your location |
| Layer count | 8+ (radar · composite · temp · wind · CAPE · precip type · rain · cloud · lightning · tropical · nowcast) | 1–3 |
| Cost at scale | $0 (you pay power) | Tier-gated |
| Trace one tap end-to-end | yes — Tempo + Loki | no |

---

## System architecture

```mermaid
flowchart TB
    subgraph Sources["☁️ public data — all free, no auth"]
        MRMS[("NOAA MRMS S3<br/>2-min radar · CONUS")]
        HRRR[("NOAA HRRR S3<br/>hourly · 18-h forecast")]
        GFS[("NOAA GFS<br/>global · 6 h")]
        NLDN[("Blitzortung WS<br/>lightning")]
        NHC[("NHC GIS<br/>tropical")]
        NWS[("NWS API<br/>active alerts")]
    end

    subgraph Ingest["⚙️ ingest layer · Python 3.12"]
        IM["ingest-mrms<br/><sub>MergedBaseReflectivityQC_00.50</sub>"]
        IRC["ingest-radar-composite<br/><sub>MergedReflectivityQComposite</sub>"]
        IH["ingest-hrrr<br/><sub>5 forecast variables</sub>"]
        IL["ingest-lightning"]
        IT["ingest-tropical"]
        NOW["nowcast<br/><sub>pysteps S-PROG</sub>"]
        OMS["open-meteo-sync<br/><sub>cron · GFS/HRRR</sub>"]
    end

    subgraph Storage["💾 longhorn PVCs"]
        TILES[("tiles · 50 Gi")]
        GRIDS[("grids · 20 Gi")]
        STATE[("state · 5 Gi")]
        OMD[("openmeteo · 30 Gi")]
    end

    subgraph Serving["🚀 serving"]
        TS["tile-server<br/><sub>Caddy + FastAPI · HPA 2-6</sub>"]
        BM["basemap<br/><sub>Protomaps · go-pmtiles</sub>"]
        OM["open-meteo<br/><sub>self-hosted forecast API</sub>"]
    end

    subgraph Edge["🌐 edge"]
        GW["Cloudflare → gateway-external<br/>radar-ng-api.vanillax.me"]
    end

    subgraph Client["📱 clients · Expo SDK 55"]
        APP["Phone app<br/>iOS · Android"]
        WATCH["Apple Watch"]
        CAR["CarPlay"]
    end

    MRMS --> IM & IRC
    HRRR --> IH
    GFS --> OMS
    NLDN --> IL
    NHC --> IT

    IM --> TILES & GRIDS
    IRC --> TILES
    IH --> TILES & GRIDS
    IL --> STATE
    IT --> STATE
    OMS --> OMD
    GRIDS --> NOW --> TILES
    OMD --> OM

    TILES --> TS
    GRIDS --> TS
    STATE --> TS
    OM --> TS
    BM --> TS

    TS --> GW
    GW --> APP
    NWS -.direct.-> APP
    APP --> WATCH & CAR
```

Every box runs on a Talos Linux cluster managed by ArgoCD. Manifests live in [`talos-argocd-proxmox/my-apps/development/radar-ng/`](https://github.com/mitchross/talos-argocd-proxmox/tree/main/my-apps/development/radar-ng).

---

## Per-frame data pipeline

What happens when NOAA drops a new MRMS scan on S3:

```mermaid
sequenceDiagram
    autonumber
    participant S3 as NOAA MRMS S3
    participant ING as ingest-mrms
    participant POOL as ThreadPool(3)
    participant FS as tiles PVC
    participant API as tile-server
    participant APP as Mobile app

    loop every 120 s
        ING->>S3: ListObjects ?prefix=…QC_00.50
        S3-->>ING: 12 keys (3 unprocessed)
        Note over ING: BACKLOG_PER_CYCLE=3<br/>newest-first
        ING->>S3: GET latest.grib2.gz (~8 MB)
        S3-->>ING: GRIB2 payload
        Note over ING: pygrib decode<br/>→ float32 lat×lon grid
        ING->>FS: write raw grid (.bin)<br/>storm-cell JSON

        par classic palette
            POOL->>FS: 4 200 PNGs · z4–z9
        and vivid palette
            POOL->>FS: 4 200 PNGs · z4–z9
        and muted palette
            POOL->>FS: 4 200 PNGs · z4–z9
        end

        Note over ING: state.add(key)<br/>commit per frame
    end

    APP->>API: GET /api/manifest.json
    API-->>APP: { layers, timestamps, palettes }
    APP->>API: GET /tiles/radar/classic/{ts}/{z}/{x}/{y}.png
    API-->>APP: 256×256 PNG (Caddy gzip · BILINEAR)
    Note over APP: MapLibre RasterSource<br/>sliding window of 7 frames<br/>rasterFadeDuration: 120 ms
```

Hot-path numbers (post Phase-2 perf work):

| stage | input | output | duration |
|---|---|---|---|
| S3 list | prefix scan | 12 keys | ~ 200 ms |
| GRIB2 download | latest object | 8 MB | ~ 1 s |
| pygrib decode | gzipped GRIB | float32 grid | ~ 800 ms |
| **palette render** ×3 | float32 → RGBA → PNG pyramid | 12 600 PNGs | **~ 2 min parallel** (was ~22 min serial) |
| storm-cell detect | grid → JSON | 1 file | ~ 300 ms |
| **total per frame** | — | — | **≈ 2 min** |

---

## Frontend architecture

```mermaid
flowchart LR
    subgraph Net["🌐 react-query"]
        Manifest["useManifest()"]
        Forecast["useForecast()"]
        Alerts["useAlerts()"]
        Wind["useWindField()"]
        Storms["useStormCells()"]
        Light["useLightning()"]
        Trop["useTropical()"]
    end

    subgraph Store["🧠 zustand + mmkv"]
        WS["useWeatherStore<br/><sub>frames · index · layer<br/>palette · opacity · serverUrl<br/>timelineMode · extrasVisible</sub>"]
        MMKV[("MMKV<br/>persisted prefs")]
    end

    subgraph Map["🗺 maplibre composition"]
        WM["WeatherMap<br/><sub>+ zoom buttons</sub>"]
        RO["RadarOverlay<br/><sub>sliding window of 7 frames</sub>"]
        WLO["WeatherLayerOverlay"]
        AP["AlertPolygon"]
        SC["StormCellsOverlay"]
        LO["LightningOverlay"]
        WP["WindParticlesOverlay<br/><sub>Skia worklet</sub>"]
    end

    subgraph Chrome["📐 chrome"]
        TL["TimelineBar<br/><sub>past · now · future merged</sub>"]
        FAB["RadarFABs<br/><sub>Layers · ⚡ · Pin · Style</sub>"]
        LL["LayerLegendCard<br/><sub>Now / Soon / Later</sub>"]
        MINI["RadarMiniMap<br/><sub>home tab</sub>"]
    end

    subgraph OTel["📊 telemetry"]
        T["telemetry.ts<br/><sub>OTLP/HTTP exporter</sub>"]
    end

    Manifest --> WS & T
    Forecast --> WS
    Alerts --> WS
    Wind --> WP
    Storms --> SC
    Light --> LO
    Trop --> TL
    WS <--> MMKV

    WS --> RO & WLO & TL & FAB & LL & MINI
    WM --- RO & WLO & AP & SC & LO

    T -.OTLP.-> Otel(("otel.vanillax.me"))
```

**Stack rules**

- All server data flows `react-query → zustand → component`
- MMKV persists user prefs (server URL, theme, palette, opacity, extras-visible)
- OTel client (`src/lib/telemetry.ts`) ships traces + logs to `otel.vanillax.me/v1/traces` and `/v1/logs` — see [Observability](#observability)
- Sliding window of 7 frames mounted at all times in `RadarOverlay` so timeline scrubbing doesn't unmount/refetch

---

## Observability — full pipeline

You can trace a single tap on the radar tab end-to-end through the mobile span → backend span → ingest log line, all keyed by a shared trace ID.

```mermaid
flowchart TB
    APP["📱 mobile app<br/>telemetry.ts"]
    POD["⚙️ backend pods<br/>shared/logger.py JSON"]
    KUBE["🛞 kubelet stdout"]

    AGENT["otel-agent (DaemonSet)<br/><sub>opentelemetry/otel-agent-collector</sub>"]
    GW["otel-gateway × 2<br/><sub>opentelemetry/otel-gateway-collector</sub>"]

    LOKI[("Loki<br/><sub>k8s_namespace_name<br/>k8s_pod_name<br/>service_name</sub>")]
    TEMPO[("Tempo<br/><sub>traces by trace_id</sub>")]
    PROM[("Prometheus<br/><sub>radar_ng_* metrics</sub>")]
    GRAF["Grafana<br/><sub>grafana.vanillax.me</sub>"]

    APP -.OTLP/HTTP.-> GW
    POD --> KUBE --> AGENT --> GW
    GW --> LOKI
    GW --> TEMPO
    POD -.ServiceMonitor scrape.-> PROM

    LOKI --> GRAF
    TEMPO --> GRAF
    PROM --> GRAF
```

### What's deployed

| component | namespace | role |
|---|---|---|
| `opentelemetry-operator` | `opentelemetry` | manages collectors |
| `otel-agent-collector` (DaemonSet, 7 pods) | `opentelemetry` | scrapes node + pod logs/metrics |
| `otel-gateway-collector` (2 replicas) | `opentelemetry` | OTLP receiver, fans out to Tempo/Loki/Prom |
| `loki-stack` (gateway, backend, read, write, chunks-cache) | `loki-stack` | log storage |
| `kube-prometheus-stack` (Prom + Grafana + Alertmanager) | `prometheus-stack` | metrics + dashboards |
| `tile-server` `/api/metrics` → ServiceMonitor | `radar-ng` | Prometheus scrape target |
| `radar-ng-dashboard` ConfigMap | `prometheus-stack` | auto-imports into Grafana |

### Mobile app instrumentation

`src/lib/telemetry.ts` wires:

```ts
// Wrap any async op in a span — records exceptions, sets error status
await trace("manifest.fetch", async (span) => {
  span.setAttribute("server", serverUrl);
  return await fetch(`${serverUrl}/api/manifest.json`).then(r => r.json());
});

// Or emit a structured log event
logEvent("warn", "tile fetch slow", { url, duration_ms: 4200 });
```

The OTLP exporter ships in batches (32 records / 5 s flush) to `otel.vanillax.me/v1/traces` and `/v1/logs`. Service name = `radar-ng-mobile`. Resource attributes include platform, OS version, app version. RN's global error handler is also piped in so red-screen crashes show up in Loki.

### Backend instrumentation

`services/shared/logger.py` writes one-line JSON per record:

```json
{"ts":"2026-04-28T05:18:12+0000","level":"INFO","service":"ingest-mrms","msg":"rendered","layer":"radar","palette":"classic","timestamp":"2026-04-28T05:18:12+00:00","tiles":4264}
```

Kubelet picks up stdout, otel-agent forwards to gateway, gateway labels with OTel semconv (`k8s_namespace_name`, `k8s_pod_name`, `service_name`) and writes to Loki.

`tile-server` additionally exports Prometheus metrics at `/api/metrics`:

```
radar_ng_forecast_requests_total          counter
radar_ng_forecast_cache_hits_total        counter
radar_ng_forecast_upstream_errors_total   counter
radar_ng_manifest_requests_total          counter
radar_ng_tile_timestamps{layer="radar"}   gauge
radar_ng_mrms_age_seconds                 gauge
```

### Grafana dashboard panels

`monitoring/prometheus-stack/radar-ng-dashboard.yaml` — auto-imported, currently shows:

| panel | query / source |
|---|---|
| MRMS radar data age (s) | `radar_ng_mrms_age_seconds` |
| tile-server requests/sec by endpoint | rate of `radar_ng_*_requests_total` |
| Forecast cache hit % | `radar_ng_forecast_cache_hits_total / requests_total` |
| Log volume by container | Loki count over time, by `k8s_container_name` |
| Recent error lines (radar-ng) | Loki `{namespace} \|~ "(?i)error"` |
| Tile timestamps per layer | `radar_ng_tile_timestamps{layer="…"}` |
| Pod CPU vs request | `kube_pod_container_resource_requests`-relative |
| Pod memory vs limit | same |
| HPA replicas (tile-server) | `kube_horizontalpodautoscaler_status_current_replicas` |
| OOM kills (1 h) | `increase(kube_pod_container_status_last_terminated_reason{reason="OOMKilled"}[1h])` |
| Container restarts (1 h) | `increase(kube_pod_container_status_restarts_total[1h])` |

### Loki query cookbook

Bookmark these in Grafana → Explore (Loki):

```logql
# Every error/warn line across radar-ng (last 1h)
{k8s_namespace_name="radar-ng"} |~ "(?i)error|warn|exception|traceback"

# Specifically ingest-mrms render cycle timings
{k8s_namespace_name="radar-ng", k8s_pod_name=~"ingest-mrms.*"}
  | json
  | msg = "frame_done"
  | line_format "{{.timestamp}} took {{.duration_s}}s"

# Tile-server slow requests (> 1s)
{k8s_namespace_name="radar-ng", k8s_pod_name=~"tile-server.*"}
  | json
  | duration > 1.0

# Mobile app errors (red-screen crashes captured by global handler)
{service_name="radar-ng-mobile"} |= "ERROR"

# Find any log entry with a given trace ID (paste in trace_id from Tempo)
{k8s_namespace_name="radar-ng"} |= "<trace_id>"
```

### Trace cookbook

In Grafana → Explore (Tempo), search by service name `radar-ng-mobile`. Spans of interest:

- `manifest.fetch` — root span when the radar tab loads
- `tile.fetch` — when MapLibre asks for a tile (currently unwrapped — could be added)
- `forecast.fetch` — home tab forecast load
- `map.fetchStyle` — basemap style JSON load (already instrumented in `WeatherMap.tsx`)

Click a span → "Logs for this span" → Loki shows backend lines correlated by trace ID.

---

## Data sources

```mermaid
flowchart LR
    subgraph free["free tier · no server"]
        IEM[IEM NEXRAD<br/><sub>5-min radar tiles</sub>]
        OMP[Open-Meteo public<br/><sub>10 k req/day</sub>]
        NWP[NWS public API<br/><sub>active alerts</sub>]
    end
    subgraph self["self-hosted tier · this repo"]
        SH[radar-ng-api<br/><sub>your tile server</sub>]
    end
    free --> APP["📱 phone"]
    self --> APP
    style free fill:#2a2a2a,stroke:#888
    style self fill:#1a4d3a,stroke:#52c47a
```

Switch in **Settings → Data Source**. Both can run side-by-side; the manifest fetch determines which layers are available.

### Self-hosted layer matrix

| layer | source | cadence | resolution | retention | tile path |
|---|---|---|---|---|---|
| **radar** | MRMS MergedBaseReflectivityQC_00.50 | 2 min | ~1 km | 4 h | `/tiles/radar/{palette}/{ts}/` |
| **radar-composite** | MRMS MergedReflectivityQComposite | 2 min | ~1 km | 4 h | `/tiles/radar-composite/{palette}/{ts}/` |
| **radar-hrrr** | HRRR composite reflectivity | 1 h × 18 hr | 3 km | 8 h | `/tiles/radar-hrrr/{palette}/{ts}/` |
| **temperature** | HRRR 2 m temp | 1 h × 24 hr | 3 km | 8 h | `/tiles/temperature/{ts}/` |
| **wind** | HRRR 10 m U + V | 1 h × 24 hr | 3 km | 8 h | `/tiles/wind/{ts}/` (+ vector grid) |
| **cape** | HRRR convective energy | 1 h × 24 hr | 3 km | 8 h | `/tiles/cape/{ts}/` |
| **precip-type** | HRRR categorical | 1 h × 24 hr | 3 km | 8 h | `/tiles/precip-type/{ts}/` |
| **precip-accum** | HRRR APCP | 1 h × 24 hr | 3 km | 8 h | `/tiles/precip-accum/{ts}/` |
| **cloud** | HRRR total cloud cover | 1 h × 24 hr | 3 km | 8 h | `/tiles/cloud/{ts}/` |
| **nowcast** | pysteps S-PROG of MRMS | 2 min × +60 min | 1 km | 1 h | `/tiles/nowcast/{palette}/{ts}/` |
| **lightning** | Blitzortung WS | 1 min | strikes | 1 h | `/api/lightning` |
| **tropical** | NHC GIS feeds | 6 h | track | 7 d | `/api/tropical` |

### Roadmap (more free MRMS products on the same S3 bucket)

| product | premium-app feature it replaces |
|---|---|
| `MESH_Max_30min` | hail-size circles (RadarOmega Pro) |
| `RotationTrackML_30min` | mid-level rotation tracks (RadarScope Pro) |
| `EchoTop_18` | convective storm height |
| `VIL` | vertically integrated liquid (storm severity) |
| `MultisensorQPE_24H_Pass2` | 24-h rainfall total, gauge-corrected |
| `WinterPrecipType` | rain/snow/sleet/freezing-rain |

Each new layer reuses the same `ingest-mrms` image with a different `MRMS_PREFIX` env var — see `deployment-ingest-radar-composite.yaml` for the pattern.

---

## API surface

| endpoint | method | description | cache |
|---|---|---|---|
| `/api/health` | GET | `ok` / `degraded` (mrms staleness) | none |
| `/api/manifest.json` | GET | layers, timestamps, palettes | 15 s |
| `/api/forecast/{lat}/{lon}` | GET | Open-Meteo proxy | 5 min |
| `/api/inspect/{layer}/{ts}/{lat}/{lon}` | GET | bilinear point-sample of raw grid | 60 s |
| `/api/wind-field/{ts}` | GET | int8-packed U/V vectors | 5 min |
| `/api/storms/{ts}` | GET | detected cell centroids + tracks | 60 s |
| `/api/lightning?since=…` | GET | Blitzortung strikes | 30 s |
| `/api/tropical` | GET | active NHC systems | 5 min |
| `/api/metrics` | GET | Prometheus counters + gauges | none |
| `/tiles/{layer}/{palette}/{ts}/{z}/{x}/{y}.png` | GET | static tile (Caddy) | 120 s |
| `/basemap/styles/*` | GET | Protomaps style JSON | 1 h |
| `/basemap/tiles/{z}/{x}/{y}.mvt` | GET | vector basemap | 1 d |

```bash
$ curl https://radar-ng-api.vanillax.me/api/health
{"status":"ok","mrms_age_s":118,"mrms_max_age_s":600,"reasons":[],"checked_at":"2026-04-28T05:18:12Z"}
```

If `mrms_age_s` exceeds `MRMS_MAX_AGE_S` (default 600 s) the status flips to `degraded` — useful for paging or for the app to show a "data delayed" banner.

---

## Operations — Talos + Argo

```mermaid
flowchart LR
    DEV["🧑 dev"]
    GH["github.com/mitchross/<br/>talos-argocd-proxmox"]
    GITEA["gitea.vanillax.me/<br/>vanillax/radar-ng"]
    CI["Gitea Actions runner<br/><sub>act_runner-nogpu</sub>"]
    REG["registry.vanillax.me"]
    ARGO["ArgoCD<br/><sub>argo.vanillax.me</sub>"]
    K8S["Talos cluster<br/><sub>radar-ng namespace</sub>"]

    DEV -- code --> GITEA
    DEV -- manifests --> GH
    GITEA -- push --> CI --> REG
    GH -- webhook --> ARGO
    ARGO -- sync --> K8S
    REG -- imagePull --> K8S
```

**Two repos:**
- `radar-ng` (this repo, Gitea) — Expo app + Python services + CI workflows
- `talos-argocd-proxmox` (GitHub) — gitops manifests, Argo source-of-truth

**Push flow:**
1. Push code changes to `radar-ng`. CI builds + pushes images to `registry.vanillax.me/radar-ng-*:vN.N.{N+1}`
2. Bump image tags in `talos-argocd-proxmox/my-apps/development/radar-ng/deployment-*.yaml`
3. Push gitops repo → Argo webhook → `Synced` → pods roll

**Disaster recovery for failed CI:** trigger `.gitea/workflows/retag-from-latest.yml` via Gitea Actions UI. Pulls `:latest`, computes next semver, re-tags, pushes. Used when basemap CI flakes and a child workflow needs a fresh `:vN.N.N` to exist.

**Cluster:** 3 control planes + 4 workers (incl. 1 GPU node), Talos v1.13.0-rc.0, k8s 1.35. Storage via Longhorn (3-replica).

---

## Self-hosting

### Hardware sizing

| profile | hosts | min | recommended |
|---|---|---|---|
| **lab** | docker-compose, single node | 4 cores · 4 GB · 20 GB SSD | 8 cores · 8 GB · 50 GB SSD |
| **prod** | full K8s + HRRR + nowcast | 16 cores · 16 GB · 100 GB SSD | 24 cores · 32 GB · 200 GB NVMe |

NOAA pull ≈ 50–100 MB/h. Steady-state disk ≈ 5 GB; spikes to ~10 GB during HRRR runs.

### Quick start (docker-compose)

```bash
git clone https://gitea.vanillax.me/vanillax/radar-ng.git
cd radar-ng/deploy
docker compose up -d

# wait ~2 min for first MRMS frame
curl http://localhost:8080/api/health
curl http://localhost:8080/api/manifest.json | jq '.layers | keys'
```

### Kubernetes (Talos + ArgoCD)

```bash
kubectl apply -k my-apps/development/radar-ng
```

Includes `ServiceMonitor`, `HorizontalPodAutoscaler` (tile-server 2→6), `PodDisruptionBudget`, and the Grafana dashboard in `monitoring/prometheus-stack/radar-ng-dashboard.yaml`.

### Resource shapes (production)

| service | requests | limits |
|---|---|---|
| ingest-mrms | 1 cpu · 1 Gi | **6 cpu · 6 Gi** (parallel palette render) |
| ingest-radar-composite | same as mrms | same |
| ingest-hrrr | 0.5 cpu · 1 Gi | 4 cpu · 4 Gi |
| nowcast | 0.5 cpu · 1 Gi | 4 cpu · 4 Gi |
| ingest-lightning / tropical | 50 m · 32 Mi | 200 m · 128 Mi |
| tile-server | 0.2 cpu · 256 Mi | 2 cpu · 1 Gi (HPA 2-6) |
| open-meteo | 0.2 cpu · 512 Mi | 2 cpu · 2 Gi |

---

## Building the app

### Prerequisites

| tool | version | notes |
|---|---|---|
| [Bun](https://bun.sh) | 1.1+ | replaces npm/yarn |
| Xcode | 26+ | iOS deployment target 26.0 |
| CocoaPods | 1.15+ | `gem install cocoapods` |
| Android Studio | API 35 | required for native modules |
| JDK | 17 | Gradle requirement |
| Watchman | latest | Metro file watching on macOS |

### Run

```bash
bun install
bun start            # Metro bundler
bun run ios          # iOS simulator (macOS only)
bun run android      # Android emulator/device
```

### iOS gotchas

- Boot a simulator first: `xcrun simctl list devices booted`
- Grant location: `xcrun simctl privacy booted grant location com.vanillax.radar-ng`
- Don't hand-edit `ios/` — regenerated by `expo prebuild`
- CarPlay config plugin (`plugins/withCarPlayScene.js`) declares both phone + CarPlay scenes — keep both

### Android gotchas

- Use Android Studio's emulator with API 35
- Reach the host backend via `10.0.2.2` (the emulator loopback magic)
- Predictive back gesture is disabled (bottom-sheet conflict)

### Connecting to your backend

In the app: **Settings → Data Source → Self-Hosted → `http://<lan-ip>:8080`**.

| platform | server URL |
|---|---|
| iOS simulator (macOS host) | `http://localhost:8080` |
| Android emulator | `http://10.0.2.2:8080` |
| Physical device on LAN | `http://<your-LAN-IP>:8080` |
| Public | `https://radar-ng-api.vanillax.me` |

---

## Project layout

```
radar-ng/
├─ src/                         Expo / React Native
│  ├─ app/                        file-based routing (expo-router)
│  ├─ components/                 map · timeline · home · inspector · alerts
│  │  ├─ home/RadarMiniMap.tsx       home-tab radar preview
│  │  ├─ map/WeatherMap.tsx          MapLibre wrapper + zoom buttons
│  │  ├─ map/RadarOverlay.tsx        sliding-window 7-frame raster
│  │  ├─ map/RadarFABs.tsx           layer picker · ⚡ extras · pin · style
│  │  ├─ map/LayerLegendCard.tsx     Now/Soon/Later tag + color scale
│  │  └─ timeline/TimelineBar.tsx    merged past·now·future scrubber
│  ├─ hooks/                      react-query data hooks
│  ├─ lib/
│  │  ├─ api.ts                      api functions
│  │  ├─ constants.ts                LAYERS · DEFAULTS
│  │  ├─ telemetry.ts                OTel SDK wiring
│  │  └─ tileUrl.ts                  tile URL builders
│  ├─ stores/useWeatherStore.ts   zustand · MMKV-persisted
│  └─ types/weather.ts            TS interfaces
├─ services/                    Python backend
│  ├─ shared/                     tiler · palettes · state · logger · storms
│  ├─ ingest-mrms/                MRMS radar (env-driven product)
│  ├─ ingest-hrrr/                HRRR forecast (5 vars)
│  ├─ ingest-lightning/           Blitzortung WS
│  ├─ ingest-tropical/            NHC
│  ├─ nowcast/                    pysteps extrapolation
│  ├─ tile-server/                Caddy + FastAPI
│  └─ tile-cleanup/               retention cron
├─ targets/                     extra Apple targets
│  ├─ watch/                      watchOS app
│  └─ carplay/                    CarPlay scene
├─ deploy/docker-compose.yml    lab single-node
├─ plugins/                     Expo config plugins (CarPlay)
├─ pmtiles-data/                Protomaps basemap source
└─ .gitea/workflows/            CI: 8 build-* + retag-from-latest
```

---

## Tech stack at a glance

```mermaid
flowchart LR
    subgraph FE["📱 frontend"]
        EX[Expo SDK 55]
        RN[React Native 0.83]
        R[React 19.2]
        ML[MapLibre Native]
        SK[Shopify Skia]
        Z[Zustand 5]
        TQ[TanStack Query 5]
        OT[OpenTelemetry SDK]
    end
    subgraph BE["⚙️ backend"]
        PY[Python 3.12]
        PG[pygrib]
        NP[numpy]
        PIL[Pillow]
        FA[FastAPI]
        CD[Caddy 2]
    end
    subgraph INF["☁️ infra"]
        TLOS[Talos Linux]
        K8S[Kubernetes 1.35]
        AC[ArgoCD]
        LH[Longhorn]
        GW[Gateway API]
        CF[Cloudflare]
    end
    subgraph OBS["📊 observability"]
        OTE[OTel Operator + Gateway]
        TM[Tempo]
        LK[Loki]
        PR[Prometheus]
        GR[Grafana]
    end

    style FE fill:#1e3a5f,stroke:#4a90e2,color:#fff
    style BE fill:#1a4d3a,stroke:#52c47a,color:#fff
    style INF fill:#3a1e4d,stroke:#a456d4,color:#fff
    style OBS fill:#4a3a1e,stroke:#d4a52e,color:#fff
```

---

## Performance notes

Phase-2 fixes that landed:

- **Palette render parallelized** with `ThreadPoolExecutor(3)` — PIL releases the GIL during PNG encode; three palettes finish in roughly the time of one.
- **`PNG optimize=False, compress_level=1`** — tiles are short-lived and Caddy gzips on the wire; the extra zlib pass halved throughput for ~5 % size win.
- **`Image.BILINEAR` resize** on tile render — crisper edges than NEAREST.
- **Backlog catch-up** — `BACKLOG_PER_CYCLE=3`, newest-first, state committed per frame.
- **Resource bumps** — ingest-mrms now 6 cpu / 6 Gi; OOMKilled count went from 7/day to 0.
- **Sliding-window radar overlay** — 7 frames mounted at once on the client (3 each side of current); scrubbing/playing the timeline no longer unmounts → re-fetches → renders.
- **`rasterFadeDuration: 120 ms` + `rasterResampling: linear`** — smooth frame transitions, smooth display-side scaling.
- **Per-frame state commits** — eliminated the bug where the backlog of unprocessed keys was being marked done without rendering.

End result: per-frame total ≈ 2 min (was ≈ 22 min). MRMS staleness budget (`MRMS_MAX_AGE_S=600`) holds. Mobile playback feels like a native weather app, not a tech demo.

---

## License

MIT
