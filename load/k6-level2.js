// Load test for Level II close-up (issue #293). Each virtual user does what one web or app
// client does with close-up open (web/src/shared/level2.ts, Level2Session): read the sites,
// submit the exact visible tile set, poll while it warms, read the viewport manifest, play the
// six sweeps, and re-submit the same view every 15 s to keep it alive. A fallback or a pending
// timeout sends the user back to MRMS, so the script then reads the MRMS manifest and one tile.
//
// Point BASE_URL at the shadow host or the in-cluster level2-api Service, never at a public host
// (Level II is off there and /api/level2/sites returns 404; setup() stops the run).
//
// Rate limits, per client IP: POST /api/level2/views 0.5 rps, burst 10; /api/* 20 rps, burst 60.
// One user re-submits every 15 s, so one IP carries at most 7 users. Keep VUS at 5 per pod and
// run 20 pods for 100 users (load/k8s/k6-level2-job.yaml). More VUs per pod measures the limiter.
//
// PROFILE: steady (default) ramps to VUS, holds, then steps to 2 x VUS; overflow spreads users
// over more than 8 stations; smoke is a 1-user, 30 s check of the flow.
import http from "k6/http";
import { check, fail, sleep } from "k6";
import { Counter, Rate, Trend } from "k6/metrics";

const BASE_URL = (__ENV.BASE_URL || "http://level2-api.radar-ng.svc.cluster.local:8080").replace(/\/$/, "");
// MRMS is served by the tile-server, not the stateless level2-api, and the shadow route
// (components/level2-shadow-route) carries only the Level II paths, so MRMS_URL stays separate.
const MRMS_URL = (__ENV.MRMS_URL || "http://tile-server.radar-ng.svc.cluster.local:8080").replace(/\/$/, "");
const VUS = Number(__ENV.VUS || 5); // per pod
const PROFILE = __ENV.PROFILE || "steady";
const SESSION_TICKS = Number(__ENV.SESSION_TICKS || 8); // 15 s refreshes before the user pans away
const OVERFLOW_STATIONS = Number(__ENV.OVERFLOW_STATIONS || 12); // > the pool's 8-station cap

// Mirrors of the server and client constants (radar-level2 viewport.rs, web/src/shared/level2.ts).
const REFRESH_S = Number(__ENV.REFRESH_S || 15); // the client refresh; lower only for dry runs
const PENDING_LIMIT_S = 30;
const MAX_AGE_S = 600;
const FRAME_STEP_S = 0.5; // 2 FPS playback, like load/k6-radar.js
const PALETTES = ["classic", "classic", "classic", "muted", "vivid"]; // most people keep classic
const TILE_TEMPLATE = "/level2/tiles/{station}/{path}/{palette}/{z}/{x}/{y}.png";

const serverErrors = new Rate("server_errors");
const rateLimited = new Rate("rate_limited");
const missingTile = new Rate("level2_missing_advertised_tile"); // 404/410 on a tile a ready manifest promised
const pendingTimeout = new Rate("level2_pending_timeout");
const fellBack = new Rate("level2_fallback");
const fallbackReasons = new Counter("level2_fallback_reasons");
const coldReady = new Trend("level2_cold_ready_ms", true); // submit -> ready, reported, not a pass rule
const observedAge = new Trend("level2_observed_age_s"); // newest frame scan_end age at each manifest read
const mrmsAge = new Trend("mrms_observed_age_s"); // MRMS latest age whenever a user is on MRMS
const playbackFrame = new Trend("level2_playback_frame_ms", true);
const stationsAsked = new Counter("level2_view_submits_by_station");

// 204 and 404 are dry MRMS tiles (nothing to draw); they are answers, not failures.
http.setResponseCallback(http.expectedStatuses(200, 202, 204, 304, 404));

const steady = [
  { duration: "1m", target: VUS },
  { duration: "10m", target: VUS },
  { duration: "1m", target: VUS * 2 },
  { duration: "5m", target: VUS * 2 },
  { duration: "1m", target: 0 },
];
const overflow = [
  { duration: "1m", target: VUS },
  { duration: "10m", target: VUS },
  { duration: "1m", target: 0 },
];
const smoke = [{ duration: "30s", target: 1 }];

export const options = {
  stages: PROFILE === "overflow" ? overflow : PROFILE === "smoke" ? smoke : steady,
  setupTimeout: "30s",
  thresholds: {
    http_req_failed: ["rate<0.01"],
    server_errors: ["rate<0.005"],
    rate_limited: ["rate<0.01"],
    // #293 launch gate: p95 manifest < 500 ms, p95 cached tile < 800 ms.
    "http_req_duration{name:l2_manifest}": ["p(95)<500"],
    "http_req_duration{name:l2_tile}": ["p(95)<800"],
    "http_req_duration{name:l2_submit}": ["p(95)<500"],
    "http_req_duration{name:l2_status}": ["p(95)<300"],
    "http_req_duration{name:l2_sites}": ["p(95)<300"],
    // #293: observed age never over 10 minutes, Level II or MRMS.
    level2_observed_age_s: [`max<=${MAX_AGE_S}`],
    mrms_observed_age_s: [`max<=${MAX_AGE_S}`],
    // A ready manifest promises every tile it lists; a miss is a publication bug.
    level2_missing_advertised_tile: ["rate<=0"],
  },
  summaryTrendStats: ["avg", "p(50)", "p(95)", "p(99)", "max", "count"],
};

function record(r, accepted) {
  serverErrors.add(r.status >= 500);
  rateLimited.add(r.status === 429);
  check(r, { "accepted response": (x) => accepted.includes(x.status) });
}

function json(r) {
  try {
    return r.json();
  } catch (_) {
    return null;
  }
}

const ageS = (iso) => (Date.now() - Date.parse(iso)) / 1000;

export function setup() {
  const r = http.get(`${BASE_URL}/api/level2/sites`, { tags: { name: "l2_sites" } });
  if (r.status === 404) fail(`Level II is off at ${BASE_URL} (sites 404). Use the shadow host or level2-api Service.`);
  const body = json(r);
  if (r.status !== 200 || !body || !Array.isArray(body.sites)) fail(`sites returned ${r.status}`);
  const enabled = body.sites.filter((s) => s.enabled);
  if (enabled.length === 0) fail("no enabled Level II stations; set LEVEL2_ENABLED_STATIONS on the shadow API");
  // Overflow centres: the enabled stations first, then the nearest other catalogue sites, so the
  // run asks for more distinct stations than the pool's 8-station cap.
  const others = body.sites.filter((s) => !s.enabled);
  const near = (s) => Math.min(...enabled.map((e) => Math.hypot(e.lat - s.lat, e.lon - s.lon)));
  others.sort((a, b) => near(a) - near(b) || (a.id < b.id ? -1 : 1));
  const spread = enabled.concat(others).slice(0, Math.max(OVERFLOW_STATIONS, enabled.length));
  return {
    enabled: enabled.map((s) => ({ id: s.id, lat: s.lat, lon: s.lon })),
    spread: spread.map((s) => ({ id: s.id, lat: s.lat, lon: s.lon })),
  };
}

// Web-mercator tile coordinates (same formulas as viewport.rs tile_x / tile_y).
const tileX = (lon, z) => ((lon + 180) / 360) * 2 ** z;
const tileY = (lat, z) => {
  const r = (lat * Math.PI) / 180;
  return ((1 - Math.log(Math.tan(r) + 1 / Math.cos(r)) / Math.PI) / 2) * 2 ** z;
};

// A phone- or laptop-sized view near a station: centre within ~60 km, z9-11, 4x3 or 5x4 tiles
// (plus the client's one-tile margin, still inside the 64-tile, 8-span server limits).
function viewNear(site) {
  const lat = site.lat + (Math.random() - 0.5) * 1.0;
  const lon = site.lon + (Math.random() - 0.5) * 1.4;
  const zoom = 9 + Math.floor(Math.random() * 3);
  const [w, h] = Math.random() < 0.5 ? [4, 3] : [5, 4];
  const cx = tileX(lon, zoom);
  const cy = tileY(lat, zoom);
  const x0 = Math.floor(cx - w / 2) - 1;
  const y0 = Math.floor(cy - h / 2) - 1;
  const tiles = [];
  for (let x = x0; x < x0 + w + 2; x += 1) for (let y = y0; y < y0 + h + 2; y += 1) tiles.push([x, y]);
  return {
    centre: { lat: Math.round(lat * 1e4) / 1e4, lon: Math.round(lon * 1e4) / 1e4 },
    zoom,
    tiles,
    palette: PALETTES[Math.floor(Math.random() * PALETTES.length)],
  };
}

function sites(tag) {
  const r = http.get(`${BASE_URL}/api/level2/sites`, { tags: { name: "l2_sites", profile: tag } });
  record(r, [200, 304]);
}

function submit(body, station, tag) {
  stationsAsked.add(1, { station });
  const r = http.post(`${BASE_URL}/api/level2/views`, body, {
    headers: { "Content-Type": "application/json" },
    tags: { name: "l2_submit", profile: tag },
  });
  record(r, [200, 202]);
  return { status: r.status, body: json(r) };
}

function status(viewId, tag) {
  const r = http.get(`${BASE_URL}/api/level2/views/${encodeURIComponent(viewId)}`, {
    tags: { name: "l2_status", profile: tag },
  });
  record(r, [200, 304]);
  return json(r);
}

// Pending -> ready / fallback within the client's 30 s limit, polling at poll_after_s.
function settle(first, tag) {
  const started = Date.now();
  let s = first;
  while (s && s.state === "pending") {
    if ((Date.now() - started) / 1000 > PENDING_LIMIT_S) {
      pendingTimeout.add(true);
      return { state: "fallback", reason: "pending_timeout" };
    }
    sleep(Math.min(15, Math.max(2, Number(s.poll_after_s) || 5)));
    s = status(s.view_id, tag);
  }
  pendingTimeout.add(false);
  if (s && s.state === "ready") coldReady.add(Date.now() - started, { profile: tag });
  return s || { state: "fallback", reason: "unreachable" };
}

function manifest(url, tag) {
  const r = http.get(`${BASE_URL}${url}`, { tags: { name: "l2_manifest", profile: tag } });
  record(r, [200, 304]);
  const env = json(r);
  if (!env || env.schema_version !== 1) return null;
  const layer = env.manifest && env.manifest.layers && env.manifest.layers[`level2-ref-${env.station.id}`];
  const frames = (layer && layer.frames) || [];
  check(env, {
    "own tile template": (e) => e.tile_url_template === TILE_TEMPLATE,
    "1-6 frames": () => frames.length >= 1 && frames.length <= 6,
  });
  if (frames.length) observedAge.add(ageS(frames[frames.length - 1].scan_end), { profile: tag });
  return { env, frames };
}

function tileUrl(env, frame, t) {
  return `${BASE_URL}${TILE_TEMPLATE.replace("{station}", encodeURIComponent(env.station.id))
    .replace("{path}", frame.path.split("/").map(encodeURIComponent).join("/"))
    .replace("{palette}", encodeURIComponent(env.palettes[0]))
    .replace("{z}", t.z)
    .replace("{x}", t.x)
    .replace("{y}", t.y)}`;
}

// Plays the loop once. Frames already fetched by this user are skipped, like a browser cache.
function playback(m, seen, tag) {
  for (const f of m.frames) {
    const key = `${m.env.station.id}/${f.path}/${m.env.palettes[0]}`;
    if (seen.has(key)) continue;
    seen.add(key);
    const started = Date.now();
    const reqs = m.env.required_tiles.map((t) => [
      "GET",
      tileUrl(m.env, f, t),
      null,
      { tags: { name: "l2_tile", profile: tag } },
    ]);
    for (const r of http.batch(reqs)) {
      serverErrors.add(r.status >= 500);
      rateLimited.add(r.status === 429);
      missingTile.add(r.status === 404 || r.status === 410);
      check(r, { "tile png": (x) => x.status === 200 && String(x.headers["Content-Type"]).startsWith("image/png") });
    }
    playbackFrame.add(Date.now() - started, { profile: tag });
    sleep(FRAME_STEP_S);
  }
}

// On fallback the map keeps MRMS: prove MRMS is still served and fresh.
// The tile is MRMS z6 under the view centre; 204 or 404 is a dry tile.
function mrms(centre, tag) {
  const r = http.get(`${MRMS_URL}/api/manifest.json`, { tags: { name: "mrms_manifest", profile: tag } });
  record(r, [200, 304]);
  const m = json(r);
  const radar = m && m.layers && m.layers.radar;
  if (!radar || !radar.latest) return;
  mrmsAge.add(ageS(radar.latest), { profile: tag });
  const frame = (radar.frames || [])[radar.frames.length - 1];
  if (!frame) return;
  const path = frame.path.split("/").map(encodeURIComponent).join("/");
  const x = Math.floor(tileX(centre.lon, 6));
  const y = Math.floor(tileY(centre.lat, 6));
  const t = http.get(`${MRMS_URL}/tiles/radar/classic/${path}/6/${x}/${y}.png`, { tags: { name: "mrms_tile", profile: tag } });
  record(t, [200, 204, 404]);
}

export default function (data) {
  const tag = PROFILE;
  const pool = PROFILE === "overflow" ? data.spread : data.enabled;
  const site = pool[(__VU + __ITER) % pool.length];
  const view = viewNear(site);
  const body = JSON.stringify(view);
  const seen = new Set();
  let held = null; // { url, newest } of the manifest this user is showing

  sites(tag);
  let s = settle(submit(body, site.id, tag).body, tag);
  for (let tick = 0; tick < SESSION_TICKS; tick += 1) {
    const ready = s && s.state === "ready" && s.manifest_url;
    fellBack.add(!ready, { profile: tag });
    if (ready && held && held.url === s.manifest_url) {
      // Same revision: the client keeps its copy. Its age still counts.
      observedAge.add(ageS(held.newest), { profile: tag });
    } else if (ready) {
      const m = manifest(s.manifest_url, tag);
      held = m && m.frames.length ? { url: s.manifest_url, newest: m.frames[m.frames.length - 1].scan_end } : null;
      if (m) playback(m, seen, tag);
    } else {
      held = null;
      fallbackReasons.add(1, { reason: String((s && s.reason) || "unknown"), profile: tag });
      mrms(view.centre, tag);
    }
    sleep(REFRESH_S);
    // The client's 15 s refresh: sites, then the same view again (renews demand and the lease).
    sites(tag);
    const again = submit(body, site.id, tag);
    s = again.body && again.body.state === "pending" ? settle(again.body, tag) : again.body;
  }
}

export function handleSummary(data) {
  return { stdout: JSON.stringify({ pod: __ENV.POD_NAME || "local", profile: PROFILE, metrics: data.metrics }) + "\n" };
}
