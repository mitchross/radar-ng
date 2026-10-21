# Glossary

Plain meanings for the words you'll see in radar-ng's docs and settings.

## Weather data

| Word | What it means |
|---|---|
| **MRMS** | *Multi-Radar Multi-Sensor.* NOAA's mosaic of every US weather radar, updated about every 2 minutes. It's the "what's happening now" radar. |
| **HRRR** | *High-Resolution Rapid Refresh.* NOAA's hourly forecast model for the US. radar-ng shows its simulated radar, wind and rain for the next hours. |
| **Nowcast** | radar-ng's own short forecast. It looks at how the last few radar scans moved and slides them forward up to 60 minutes. Afterwards it compares its guess with the real radar and keeps a score. |
| **NWS** | *National Weather Service.* The source of watches, warnings and advisories (api.weather.gov). It asks every app to send a contact email, which is why you set `NWS_USER_AGENT`. |
| **NHC** | *National Hurricane Center.* Tropical storm tracks and cones. |
| **NAQFC / AQM** | NOAA's air-quality forecast (PM2.5 smoke and dust, ozone). |
| **Open-Meteo** | A free, open forecast service. radar-ng runs its own copy for hourly and 7-day forecasts. |
| **Blitzortung** | A volunteer network of lightning detectors. radar-ng listens to its live feed. |
| **GRIB2** | The file format NOAA publishes weather data in. radar-ng reads it directly. |
| **CONUS** | *Continental US* (the lower 48 states). The default coverage area. |

## Maps

| Word | What it means |
|---|---|
| **Tile** | A small square map image. Maps are drawn from many tiles; zooming in loads more detailed ones. |
| **Basemap** | The background map (roads, borders, labels) that radar is drawn on top of. |
| **Protomaps / PMTiles** | The free basemap radar-ng uses, stored as one big `.pmtiles` file you download once. |
| **bbox** | *Bounding box:* the rectangle of the world you download map data for (`BASEMAP_BBOX`). |
| **Frame** | One moment in time on the radar loop. Playback steps through frames. |
| **Manifest** | A small list the server publishes (`/api/manifest.json`) of which layers and frames exist. The apps read it to know what to show. |
| **Palette** | The color scale for a layer (e.g. which color means heavy rain). |

## Running the server

| Word | What it means |
|---|---|
| **Stack** | Everything the server side needs, running together: the API, the workers, Temporal, Open-Meteo. The app's **Stack URL** is the address of yours. |
| **API / tile-server** | The part that answers the apps: tiles, the basemap and JSON, on port 8080 (`radar api`). |
| **Worker** | The part that does the work: downloads data, renders tiles, cleans up (`radar worker`). |
| **Temporal** | A job scheduler. It runs the workers' jobs on a timetable and retries them when they fail. |
| **Docker Compose** | A way to run several containers on one computer from one file (`deploy/docker-compose.yml`). |
| **Kubernetes / kind / k3s** | Software for running containers across a cluster of computers. kind and k3s are small versions for a laptop or home server. |
| **Kustomize** | Kubernetes' built-in way to combine a base set of files with optional add-ons (`kubectl apply -k`). |
| **PVC / RWO / RWX** | Disk storage in Kubernetes. *RWO* can attach to one machine at a time; *RWX* can be shared by several. |
| **GHCR** | GitHub's container registry, where the ready-made radar-ng images are published. |
