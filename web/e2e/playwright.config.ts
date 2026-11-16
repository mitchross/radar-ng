import { defineConfig } from "@playwright/test";

// E2E_BASE_URL points the run at a deployed site; unset, it builds and previews this checkout.
const external = process.env.E2E_BASE_URL;
const baseURL = external ?? "http://127.0.0.1:4173";

export default defineConfig({
  testDir: ".",
  testMatch: "*.e2e.ts",
  outputDir: "results",
  timeout: 180_000,
  expect: { timeout: 30_000 },
  workers: 1,
  retries: 0,
  reporter: [["list"], ["html", { outputFolder: "report", open: "never" }], ["json", { outputFile: "results/results.json" }]],
  use: {
    baseURL,
    viewport: { width: 1280, height: 800 },
    trace: "on",
    screenshot: "on",
    // MapLibre needs WebGL; headless Chromium gets it from SwiftShader.
    launchOptions: { args: ["--use-angle=swiftshader", "--enable-unsafe-swiftshader", "--ignore-gpu-blocklist"] },
  },
  webServer: external
    ? undefined
    : { command: "bunx vite build && bunx vite preview --host 127.0.0.1 --port 4173 --strictPort", cwd: "..", url: baseURL, timeout: 180_000, reuseExistingServer: true },
});
