# Third-party terms before store distribution

Research for task 1.6 (finding F-R7) of `docs/tasks/2026-09-23-mobile-audit-and-plan.md`.
**Decision D7 (2026-09-24): option C, self-host.** The user requires every runtime request to go to
their own servers, with no API keys. Satellite moves to self-hosted US/Canada/Mexico public-domain
imagery (plan task S.1); the keyed ArcGIS path (option B) is ruled out.

## 1. Esri World Imagery (the satellite basemap)

The bundled `satellite` style is served by the tile server at
`backend/basemap/styles/satellite.json` and points directly at Esri:

```
https://services.arcgisonline.com/ArcGIS/rest/services/World_Imagery/MapServer/tile/{z}/{y}/{x}
```

There is no API key and no ArcGIS account, and tile requests go straight from the user's device to
Esri. The attribution was `"Tiles © Esri and imagery providers"`; it is now the service's own
`copyrightText`, `Powered by Esri — Esri, Vantor, Earthstar Geographics, and the GIS User
Community`, because the generic text omitted the required provider credits and Esri's licensing
page requires both a "Powered by Esri" mark and the data-source credits.

### What the service says about itself

The ArcGIS item record is authoritative and machine-readable
(`https://www.arcgis.com/sharing/rest/content/items/10df2279f9684e4a9f6a7f08febac2a9?f=json`):

- `licenseInfo`: *"This work is licensed under the Esri Master License Agreement."* It adds that the
  layer **"is not intended to be used to export tiles for offline"**, and points offline use at a
  separate *World Imagery (for Export)* layer.
- `accessInformation`: `Esri, Vantor, Earthstar Geographics, and the GIS User Community`.

So the imagery is licensed under Esri's commercial Master License Agreement, not under an open
licence, and **no published exception** covers a third-party app on a store reading that endpoint
anonymously. Hosting the style JSON yourself changes nothing about the imagery's licence. The
endpoint staying up is not evidence of permission. This is a real, if historically tolerated,
licensing gap, and the risk it carries is a takedown or an App Store complaint rather than a
runtime failure.

On retirement: Esri's published sunsetting applies to **World Imagery Prime** and **Clarity**, which
are different services. No retirement date is announced for the standard `World_Imagery/MapServer`
endpoint — but that is not the same as support for new account-free applications, which is
**UNVERIFIED**. Do not apply the Prime/Clarity dates, or the older generic migration deadlines, to
this endpoint.

### The compliant path, if you keep Esri

ArcGIS Location Platform, per its own licensing page
(`https://location.arcgis.com/help/licensing-and-attribution/`):

- *"Use access tokens generated from your ArcGIS Location Platform subscription for all ArcGIS
  location service requests"* and *"Ensure all service calls are authenticated."*
- MapLibre GL JS is explicitly a supported client, so no ArcGIS SDK or runtime licence string is
  needed for the tiles alone.
- Attribute with **"Powered by Esri"** plus the data-source credits. Esri states you cannot remove
  required attribution while using their resources.
- Metering (`https://developers.arcgis.com/pricing/`, `https://location.arcgis.com/pricing/`):
  **2,000,000 basemap tiles/month free, then $0.15 per 1,000 tiles.** Basemap sessions are metered
  separately (1,000 free/month, then $4.00 per 1,000). Billing counts requests to
  `basemaps-api.arcgis.com` and its siblings.

Practical cost of the change: an API key must ship inside the app, where it can be extracted; a
hosted dependency appears in a project whose whole point is self-hosting; and each operator of a
fork would need their own key. The keyed imagery URL is
`https://ibasemaps-api.arcgis.com/arcgis/rest/services/World_Imagery/MapServer/tile/{z}/{y}/{x}?token=…`.
Metering is billed to the operator's subscription.

Not verified from a primary source: Esri's retention of tile-request logs, and whether the legacy
`services.arcgisonline.com` endpoint has a published retirement date (see above — the announced
retirements are for different services). Treat both as **UNVERIFIED**.

### Options

| | Change | Residual risk |
| --- | --- | --- |
| **A. Keep the public no-key endpoint** | none | Unlicensed use of commercially licensed imagery in a shipped app; could be throttled or withdrawn without notice. Nothing in the codebase today records that this is a known gamble. |
| **B. Move to the keyed ArcGIS service** | swap the style URL, add a key, add "Powered by Esri" | Compliant, but ships an extractable key, adds metered billing, and contradicts the self-hosted default. Fork operators need their own key. |
| **C. Drop satellite, or self-host imagery** | remove the style, or point it at imagery you host | Lowest legal risk. Public-domain US imagery (USGS/NAIP) can be self-hosted through the existing PMTiles path for the US; global options (e.g. Sentinel-2 cloudless) carry their own attribution terms and lower detail. Costs engineering time. |

A middle path worth noting: keep A as the *shipped default* but make the satellite style opt-in
through the existing `EXPO_PUBLIC_BASEMAP_SATELLITE_STYLE_URL` escape hatch (already documented in
`docs/configuration.md`), so a build that ships without it has no third-party imagery dependency at
all. That is a scoping decision, not a legal fix, and does not by itself make A compliant.

### Recommendation (second opinion, 2026-09-24)

**D7 → option C.** Operator-hosted NAIP imagery avoids a shared credential and per-operator Esri
billing, and fits a project whose deployment model is already self-hosted. Concretely, the
self-hosting path is shorter than the keyed path, because the delivery machinery exists:
`deploy/docker-compose.yml` already runs a go-pmtiles container for the vector basemap.

What C still needs before it can replace the Esri tiles:

1. A **raster** archive plus its own route and style document. Today's `/basemap/tiles/*` route
   fronts the *vector* PMTiles archive (`backend/api/Caddyfile`), but the delivery mechanism is
   already there: `deploy/docker-compose.yml` runs `protomaps/go-pmtiles`, which serves any PMTiles
   archive from a volume over HTTP range requests. A second archive and a raster style is an
   extension of that, not a new subsystem.
2. A decision about coverage and depth, because that is where the cost lives:

   | Source | Coverage | Licence | Notes |
   | --- | --- | --- | --- |
   | NAIP (USDA) | US only | Public domain, no attribution | ~0.6–1 m native. Best quality, US-only, and no attribution burden. |
   | Sentinel-2 cloudless / ESA WorldCover style mosaics | Global | Free, attribution required (ESA/Copernicus) | 10 m, so visibly softer at high zoom. The only realistic global option. |

3. A download-and-convert pipeline, and the disk to hold it.

The data volumes are the real reason this was never done. For scale:

- The self-hosted **vector** basemap is a `pmtiles extract` of a CONUS bounding box: **~1–2 GB**
  (`deploy/docker-compose.yml`, `docs/self-hosting.md`).
- NAIP as a program is on the order of **16 PB**, and a single CONUS vintage is on the order of
  **~180 TB** uncompressed at native resolution (USDA FPAC/NGAC presentation, 2024; BigData Earth
  analysis). That is the wrong number to plan against, though — a weather map does not need 0.6 m
  imagery, and downsampling is exactly what the conversion step does. The useful precedent is the
  ESA WorldCover toolchain, which turns **116 GB** of GeoTIFFs into a single PMTiles archive with
  `geotiff2pmtiles` (pure Go, no system dependencies). A CONUS raster basemap downsampled to the
  zooms a radar map actually uses lands in the hundreds of GB, not petabytes.

4. Until that exists, the transitional state is "no satellite in shipped builds", which is a
   user-visible feature removal and needs its own release note.

So the gap is a **volume and pipeline** problem, not a licensing one for the US, and not an
architectural one for the app.

`World Imagery Prime` and `Clarity` are not alternatives; they are the services Esri is retiring.

## 2. NWS User-Agent

`api.weather.gov` asks for a `User-Agent` that identifies the application **and a contact**.
`src/lib/api.ts` (`fetchAlerts`) and `targets/watch/WatchAPI.swift` currently send
`radar-ng/2.0 (self-hosted-weather-radar)` with no address. This is a policy request rather than a
licence, it is cheap to satisfy, and it is blocked only on the user supplying an address to use.

## 3. Nominatim

No longer applicable. Task 1.4 replaced reverse geocoding with the platform geocoder, so public
Nominatim is no longer called at all — see `docs/privacy.md` §3. The usage-policy problem is gone.

## 4. Related

- `docs/privacy.md` §2 lists every destination the app contacts, so this page and that one should
  be updated together.
- `docs/configuration.md` documents the `EXPO_PUBLIC_BASEMAP_*` overrides.
