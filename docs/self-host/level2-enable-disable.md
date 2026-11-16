# Level II close-up: enable and disable

Operator steps for turning Level II close-up reflectivity on for a region, off for a region, and
off everywhere, and for checking that MRMS radar is still served afterwards. Background:
[design](../develop/design/level2-close-up.md), [pool runbook](../develop/level2-pool.md),
[launch-gate report](../reference/level2-launch-gate.md).

**What this release is:** the lowest reflectivity sweep from one station at a time, on top of
MRMS, at zoom 8 and closer, within 150 km of the station.

**What this release is not:** no velocity, no higher tilts, no multi-station mosaics and no
stronger or destructive QC. Do not enable a region to get any of these; they are not built.

Upstream deployment: every change below is a talos-argocd-proxmox PR that Mitch merges and Argo CD
syncs. Never `kubectl apply` or edit live objects. Enabling a region needs Mitch's explicit
approval after every launch gate passes.

## How Level II is switched

Three settings decide what a visitor sees. Clients have no flag of their own: they follow
`sites[].enabled` from `/api/level2/sites`.

| Layer | Setting | Effect when off |
|---|---|---|
| Route | Public HTTPRoute rules sending `/api/level2/` and `/level2/tiles/` on the public hosts to the `level2-api` Service | Those paths reach the tile-server, where the gate is off: `404`. Clients show MRMS only. |
| Gate | `LEVEL2_VIEWPORT_ENABLED=true` on `level2-api` (with its `LEVEL2_PG_*` and `LEVEL2_S3_*` secrets) | Every Level II route returns `404`. |
| Stations | `LEVEL2_ENABLED_STATIONS` on `level2-api`, comma-separated station IDs | A station not listed shows `enabled: false`; views near it fall back with `out_of_range`, or go to another enabled station within 150 km. |

Rules for `LEVEL2_ENABLED_STATIONS`:

- Always set it. If it is unset, the server enables `KGRR`.
- List only feed-verified stations (`FEED_VERIFIED` in `rust/crates/radar-level2/src/catalogue.rs`;
  `KGRR`, `KDTX`, `KLOT`, `KAPX` at the time of writing). One unknown or unverified ID makes the
  Level II service refuse to start: the API logs `Level II viewport disabled` and all Level II
  routes return `404`. It fails closed, for every station, not only the bad one.
- <!-- TODO(C3): state whether the pool, publisher and recorder also read this list, from the C3
  deploy examples. --> Keep the same list on every Level II workload that reads it.

What clients do after a change:

- Turned off (404): the client drops Level II at once and shows MRMS, then asks again every
  5 minutes.
- Unreachable or 5xx: the client keeps its last station list for 30 s, then shows MRMS.
- Turned on: the site list is cached 15 s, and clients that saw a 404 ask again within 5 minutes.
- A station removed from the list: its ready views expire within their 60 s lease, and the next
  15 s refresh falls back.

## Before you change anything

- [ ] The [launch-gate report](../reference/level2-launch-gate.md) is filled in and every gate for
  the region passes.
- [ ] Mitch has approved this region in writing (link it in the PR).
- [ ] MRMS is healthy now ([check](#confirm-mrms-is-still-served)). Do not enable Level II while
  MRMS is stale; the fallback would have nothing behind it.

## Enable a region

1. Open a talos PR (`radar-ng/level2-enable-<region>`) that:
   - sets `LEVEL2_ENABLED_STATIONS` on `level2-api` to the approved stations;
   - adds the Level II path rules to the public HTTPRoutes (API host and web host), pointing at the
     `level2-api` Service. `POST /api/level2/views` must be allowed at the gateway;
   - changes nothing on the tile-server.
2. Mitch merges. Wait for Argo CD to sync and the `level2-api` pods to be Ready.
3. Check, with `API` and `WEB` set to the public hosts:

   ```sh
   curl -s "$API/api/level2/sites" | jq '{enabled_regions, sites: [.sites[] | select(.enabled) | {id, state: .health.state}]}'
   curl -s -o /dev/null -w '%{http_code}\n' "$WEB/api/level2/sites"   # 200 on the web host too
   ```

   `enabled_regions` lists exactly the approved stations, each with a health state.
4. Submit one real view near each new station and follow it to a tile:

   ```sh
   body='{"centre":{"lat":42.96,"lon":-85.67},"zoom":9,"tiles":[[134,188],[134,189],[135,188],[135,189]],"palette":"classic"}'
   curl -s -H 'Content-Type: application/json' -d "$body" "$API/api/level2/views" | jq .
   # poll the returned /api/level2/views/<id> until state is ready, then fetch manifest_url
   # and one tile from it: expect 200 image/png
   ```

   Compute the tile set for the station you check; the example is near KGRR. The tiles must
   contain the centre and stay within 8 x 8.
5. Open the web map and the app at the station, zoom past 9, and check the provenance line
   (station, scan time, tilt, beam height).
6. [Confirm MRMS is still served.](#confirm-mrms-is-still-served)
7. Comment on #293 with the PR, the station list and the check output.

## Disable a region

1. Open a talos PR that removes the stations from `LEVEL2_ENABLED_STATIONS`. Leave at least one
   station, or use the [kill switch](#kill-switch-level-ii-off-everywhere) instead: an empty list
   is not a supported way to turn Level II off.
2. Mitch merges; wait for the rollout.
3. Check: `/api/level2/sites` shows those stations `enabled: false`, and a view submitted at one of
   them returns `state: fallback` (`out_of_range`, or another enabled station within 150 km).
4. [Confirm MRMS is still served.](#confirm-mrms-is-still-served)

## Kill switch: Level II off everywhere

Use when Level II is wrong for visitors and you do not want to debug first. MRMS is not affected.

1. Open a talos PR that removes the Level II path rules from the public HTTPRoutes. The public hosts
   then send `/api/level2/` and `/level2/tiles/` to the tile-server, which answers `404`. The shadow
   host and the in-cluster Service keep working, so measurement continues.
2. If Level II itself is the problem (bad data reaching anyone), also set
   `LEVEL2_VIEWPORT_ENABLED=false` on `level2-api` in the same PR.
3. Mitch merges. Check that both public hosts return `404`:

   ```sh
   for h in "$API" "$WEB"; do curl -s -o /dev/null -w "$h %{http_code}\n" "$h/api/level2/sites"; done
   ```

4. [Confirm MRMS is still served.](#confirm-mrms-is-still-served)

If the PR path is too slow during an incident, Mitch (only) can scale `level2-api` to zero by
hand. The public Level II paths then return 5xx and clients show MRMS within about 30 s. Argo CD
will undo the scale on its next self-heal unless sync is paused for the app, so follow it with
the kill-switch PR at once.

## Confirm MRMS is still served

Run after every change above. All four must hold.

```sh
# 1. Manifest is fresh: newest MRMS frame under 10 minutes old.
latest=$(curl -s "$API/api/manifest.json" | jq -r '.layers.radar.latest')
echo "$latest  age $(( $(date -u +%s) - $(date -u -d "$latest" +%s) )) s"

# 2. A tile answers: 200 image/png, or 204/404 for a dry tile. text/html means a routing mistake.
path=$(curl -s "$API/api/manifest.json" | jq -r '.layers.radar.frames[-1].path')
curl -s -o /dev/null -w '%{http_code} %{content_type}\n' "$API/tiles/radar/classic/$path/6/16/23.png"

# 3. The web host serves the same manifest.
curl -s "$WEB/api/manifest.json" | jq -r '.layers.radar.latest'
```

4. In Prometheus, `radar_ng_mrms_age_seconds` is below 600 and `RadarNgObservedRadarStale` is not
   firing. `load/drills/observe.py --base "$API"` prints the same numbers every 15 s if you want a
   timeline.

If MRMS is stale, Level II is not the cause unless the change touched the tile-server or MRMS
workers; check those first ([troubleshooting](troubleshooting.md)).
