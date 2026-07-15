import http from "k6/http";
import { check, sleep } from "k6";
import { Rate } from "k6/metrics";

const BASE_URL = (__ENV.BASE_URL || "https://radar-ng-api.vanillax.me").replace(/\/$/, "");
const serverErrors = new Rate("server_errors");

export const options = {
  stages: [
    { duration: "1m", target: 20 },
    { duration: "3m", target: 100 },
    { duration: "5m", target: 100 },
    { duration: "1m", target: 0 },
  ],
  thresholds: {
    http_req_failed: ["rate<0.01"],
    server_errors: ["rate<0.005"],
    "http_req_duration{name:manifest}": ["p(95)<500"],
    "http_req_duration{name:tile}": ["p(95)<800"],
    "http_req_duration{name:forecast}": ["p(95)<2000"],
  },
};

function record(response, accepted = [200]) {
  serverErrors.add(response.status >= 500);
  check(response, { "accepted response": (r) => accepted.includes(r.status) });
}

function encodePath(path) {
  return path.split("/").map(encodeURIComponent).join("/");
}

export default function () {
  const manifestResponse = http.get(`${BASE_URL}/api/manifest.json`, {
    tags: { name: "manifest" },
  });
  record(manifestResponse);

  let manifest;
  try {
    manifest = manifestResponse.json();
  } catch (_) {
    sleep(1);
    return;
  }

  const layer = manifest.layers?.radar;
  const frame = layer?.frames?.[layer.frames.length - 1];
  const path = frame?.path || layer?.timestamps?.[layer.timestamps.length - 1];
  if (path) {
    // A map viewport fans out several parallel raster requests. Transparent
    // tiles are intentionally absent, so 404 is an expected cacheable result;
    // 5xx and latency are the capacity signals.
    const z = 6;
    const tileRequests = [];
    for (let dx = -1; dx <= 1; dx += 1) {
      for (let dy = -1; dy <= 1; dy += 1) {
        tileRequests.push([
          "GET",
          `${BASE_URL}/tiles/radar/classic/${encodePath(path)}/${z}/${16 + dx}/${23 + dy}.png`,
          null,
          { tags: { name: "tile" } },
        ]);
      }
    }
    for (const response of http.batch(tileRequests)) record(response, [200, 404]);
  }

  if (__ITER % 10 === 0) {
    const forecast = http.get(`${BASE_URL}/api/forecast/42.9634/-85.6681`, {
      tags: { name: "forecast" },
    });
    record(forecast);
  }

  sleep(1 + Math.random() * 2);
}
