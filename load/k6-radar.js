// Load test for the radar-ng tile-server: the traffic the app, widget and Watch
// generate. Run it from inside the cluster (load/k8s/k6-job.yaml) so Cloudflare
// and the home uplink don't skew results, or from anywhere with BASE_URL.
//
// The API rate-limits per client IP (20 rps, burst 60, /api/* only). Each k6
// pod is one client, so per-pod load stays under that and the Job runs several
// pods in parallel: total load = PODS x VUs per pod. Tiles are served by Caddy
// and are not rate-limited.
import http from "k6/http";
import { check, sleep } from "k6";
import { Rate, Trend } from "k6/metrics";

const BASE_URL = (__ENV.BASE_URL || "https://radar-ng-api.vanillax.me").replace(/\/$/, "");
const VUS = Number(__ENV.VUS || 10); // per pod
const serverErrors = new Rate("server_errors");
const rateLimited = new Rate("rate_limited");
const playbackFrame = new Trend("playback_frame_ms", true);

// 404 is a normal answer here (dry tiles are not written; no nowcast for a
// point), so it must not count toward http_req_failed. 5xx and 429 still do.
http.setResponseCallback(http.expectedStatuses(200, 404));

export const options = {
  stages: [
    { duration: "1m", target: VUS },          // ramp
    { duration: "5m", target: VUS },          // steady: 10 pods x VUS
    { duration: "1m", target: VUS * 2 },      // step up
    { duration: "3m", target: VUS * 2 },      // 2x load
    { duration: "1m", target: 0 },
  ],
  thresholds: {
    http_req_failed: ["rate<0.01"],
    server_errors: ["rate<0.005"],
    rate_limited: ["rate<0.01"],
    "http_req_duration{name:manifest}": ["p(95)<300"],
    "http_req_duration{name:tile}": ["p(95)<300"],
    "http_req_duration{name:forecast}": ["p(95)<1000"],
    "http_req_duration{name:alerts}": ["p(95)<1500"],
    "http_req_duration{name:nowcast}": ["p(95)<1000"],
    "http_req_duration{name:geocode}": ["p(95)<1000"],
    playback_frame_ms: ["p(95)<500"],
  },
  summaryTrendStats: ["avg", "p(50)", "p(95)", "p(99)", "max"],
};

// Users spread across the continental US, rounded like the app rounds (2 dp).
const PLACES = [
  [42.96, -85.67], [41.88, -87.63], [40.71, -74.01], [34.05, -118.24], [29.76, -95.37],
  [33.45, -112.07], [39.74, -104.99], [47.61, -122.33], [25.76, -80.19], [36.16, -86.78],
  [44.98, -93.27], [32.78, -96.8], [38.63, -90.2], [35.96, -83.92], [42.36, -71.06],
];
const CITIES = ["Chicago", "Denver", "Knoxville", "Seattle", "Miami", "Boston"];

function record(response, accepted = [200]) {
  serverErrors.add(response.status >= 500);
  rateLimited.add(response.status === 429);
  check(response, { "accepted response": (r) => accepted.includes(r.status) });
}

const encodePath = (path) => path.split("/").map(encodeURIComponent).join("/");

// Web-mercator tile for a point.
function tileFor(lat, lon, z) {
  const n = 2 ** z;
  const x = Math.floor(((lon + 180) / 360) * n);
  const r = (lat * Math.PI) / 180;
  const y = Math.floor(((1 - Math.log(Math.tan(r) + 1 / Math.cos(r)) / Math.PI) / 2) * n);
  return [x, y];
}

function tiles(path, lat, lon, z, span, tag) {
  const [cx, cy] = tileFor(lat, lon, z);
  const reqs = [];
  for (let dx = -span; dx <= span; dx += 1) {
    for (let dy = -span; dy <= span; dy += 1) {
      reqs.push(["GET", `${BASE_URL}/tiles/radar/classic/${encodePath(path)}/${z}/${cx + dx}/${cy + dy}.png`, null, { tags: { name: "tile", client: tag } }]);
    }
  }
  // Transparent (dry) tiles are not written, so 404 is an expected result.
  const started = Date.now();
  for (const r of http.batch(reqs)) record(r, [200, 404]);
  return Date.now() - started;
}

function manifest(tag) {
  const r = http.get(`${BASE_URL}/api/manifest.json`, { tags: { name: "manifest", client: tag } });
  record(r);
  try {
    return r.json();
  } catch (_) {
    return null;
  }
}

export default function () {
  const [lat, lon] = PLACES[(__VU + __ITER) % PLACES.length];
  const roll = Math.random();

  if (roll < 0.2) {
    // Widget / Watch refresh: manifest + a 2x2 area at z7.
    const m = manifest("widget");
    const frames = m?.layers?.radar?.frames || [];
    const f = frames[frames.length - 1];
    if (f) tiles(f.path, lat, lon, 7, 0, "widget");
    sleep(2 + Math.random() * 3);
    return;
  }

  // App session: Home, then radar playback.
  const m = manifest("app");
  record(http.get(`${BASE_URL}/api/forecast/${lat}/${lon}`, { tags: { name: "forecast", client: "app" } }));
  record(http.get(`${BASE_URL}/api/alerts?lat=${lat}&lon=${lon}`, { tags: { name: "alerts", client: "app" } }), [200, 502]);
  record(http.get(`${BASE_URL}/api/nowcast/${lat}/${lon}`, { tags: { name: "nowcast", client: "app" } }), [200, 404]);
  if (roll > 0.9) {
    const city = CITIES[__ITER % CITIES.length];
    record(http.get(`${BASE_URL}/api/geocode?q=${city}`, { tags: { name: "geocode", client: "app" } }), [200, 502, 503]);
  }

  // Playback: a 3x3 viewport at z6 for the last 6 frames, 500 ms apart (2 FPS).
  const frames = (m?.layers?.radar?.frames || []).slice(-6);
  for (const f of frames) {
    playbackFrame.add(tiles(f.path, lat, lon, 6, 1, "app"));
    sleep(0.5);
  }
  sleep(1 + Math.random() * 2);
}

export function handleSummary(data) {
  return { stdout: JSON.stringify({ pod: __ENV.POD_NAME || "local", metrics: data.metrics }) + "\n" };
}
