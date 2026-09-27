import { defineConfig } from "vite";

// Same-origin in the browser: the tile-server API sends no CORS headers, so
// dev proxies /api, /tiles and /basemap to it and production serves this site
// from the same host (Caddy). RADAR_API overrides the upstream.
const upstream = process.env.RADAR_API ?? "https://radar-ng-api.vanillax.me";
// Cloudflare hotlink protection rejects image requests whose Referer isn't a
// vanillax.me page, so the dev proxy drops it (production is same-host).
const proxy = Object.fromEntries(
  ["/api", "/tiles", "/basemap"].map((p) => [
    p,
    {
      target: upstream,
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
  server: { port: 5173, proxy },
  preview: { port: 4173, proxy },
});
