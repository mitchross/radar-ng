import { defineConfig } from "vite";

// Same-origin in the browser: the tile-server API sends no CORS headers, so
// dev proxies /api, /tiles, /replays, /level2 and /basemap to it and production serves this site
// from the same host (radar api). RADAR_API overrides the upstream, RADAR_BASEMAP the /basemap one,
// RADAR_REPLAYS the replay routes (a local `radar api` over imported cases, live radar elsewhere).
const upstream = process.env.RADAR_API ?? "https://radar-ng-api.vanillax.me";
const basemapUpstream = process.env.RADAR_BASEMAP ?? upstream;
const replayUpstream = process.env.RADAR_REPLAYS ?? upstream;
const targets: [string, string][] = [
  // First match wins, so the replay API sits before /api.
  ["/api/replays", replayUpstream],
  ["/replays", replayUpstream],
  ["/api", upstream],
  ["/tiles", upstream],
  ["/level2", upstream],
  ["/basemap", basemapUpstream],
];
// Cloudflare hotlink protection rejects image requests whose Referer isn't a
// vanillax.me page, so the dev proxy drops it (production is same-host).
const proxy = Object.fromEntries(
  targets.map(([p, target]) => [
    p,
    {
      target,
      changeOrigin: true,
      configure: (server: { on: (event: "proxyReq", cb: (req: { removeHeader: (h: string) => void }) => void) => void }) => {
        server.on("proxyReq", (req) => {
          req.removeHeader("referer");
          req.removeHeader("origin");
        });
      },
    },
  ]),
);

export default defineConfig({
  // MapLibre 6 loads its worker via new URL("./maplibre-gl-worker.mjs", import.meta.url);
  // pre-bundling moves the entry into .vite/deps and breaks that relative URL.
  optimizeDeps: { exclude: ["maplibre-gl"] },
  worker: { format: "es" },
  server: { port: 5173, proxy },
  preview: { port: 4173, proxy },
});
