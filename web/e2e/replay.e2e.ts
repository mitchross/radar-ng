import { expect, test, type Page, type Request, type TestInfo } from "@playwright/test";
import { appendFileSync, mkdirSync } from "node:fs";
import { join } from "node:path";
import type { ReplayCase } from "../src/shared/replayApi";
import { replayFrames, replayTileUrl, replayTimeLabel, type ReplayFrame, type ReplayManifest } from "../src/shared/replaySource";

// E2E_CASES limits the run, e.g. "greenfield-2024"; default is both first-release cases.
const CASES = (process.env.E2E_CASES ?? "greenfield-2024,rolling-fork-2023").split(",").map((s) => s.trim()).filter(Boolean);
const SHOTS = join(import.meta.dirname, "results", "shots");
const LOG = join(import.meta.dirname, "results", "acceptance.jsonl");
mkdirSync(SHOTS, { recursive: true });

const path = (r: Request | string) => new URL(typeof r === "string" ? r : r.url()).pathname;
const isLiveTile = (r: Request) => path(r).startsWith("/tiles/");
const isCaseTile = (id: string) => (r: Request) => path(r).startsWith(`/replays/${id}/`);

interface Loaded { c: ReplayCase; manifest: ReplayManifest }

async function caseData(page: Page, id: string): Promise<Loaded> {
  const cat = await (await page.request.get("/api/replays")).json();
  const c = (cat.cases as ReplayCase[]).find((x) => x.id === id);
  expect(c, `${id} is in /api/replays`).toBeTruthy();
  const manifest = (await (await page.request.get(c!.manifest_url)).json()) as ReplayManifest;
  return { c: c!, manifest };
}

/** The tile path prefix one frame's tiles use, without z/x/y. */
const framePrefix = (m: ReplayManifest, f: ReplayFrame) => path(replayTileUrl(`http://x${m.tile_url_template}`, f.source, "classic", f.path).split("/{z}")[0]) + "/";

async function shot(page: Page, info: TestInfo, id: string, step: string) {
  const body = await page.screenshot();
  await info.attach(`${id}-${step}`, { body, contentType: "image/png" });
  const file = join(SHOTS, `${id}-${step}.png`);
  await page.screenshot({ path: file });
}

function record(id: string, check: string, data: Record<string, unknown>) {
  appendFileSync(LOG, JSON.stringify({ at: new Date().toISOString(), base: process.env.E2E_BASE_URL ?? "local-preview", case: id, check, ...data }) + "\n");
}

/** Open the site, let the live radar paint first, then open the picker. */
async function openPicker(page: Page, id: string) {
  const liveTile = page.waitForResponse((r) => isLiveTile(r.request()) && r.status() === 200, { timeout: 60_000 });
  await page.goto("/");
  await liveTile;
  await page.waitForTimeout(3_000);
  await page.locator("#storms").click();
  const button = page.locator(`button.replay-case[data-case="${id}"]`);
  await expect(button).toBeVisible();
  return button;
}

/** Enter the case and wait for its manifest and one painted tile; returns ms to that tile. */
async function enter(page: Page, id: string): Promise<number> {
  const button = await openPicker(page, id);
  const t0 = Date.now();
  const tile = page.waitForResponse((r) => isCaseTile(id)(r.request()) && r.url().endsWith(".png") && r.status() === 200, { timeout: 120_000 });
  clickedAt.set(page, Date.now());
  await button.click();
  await expect(page.locator("body")).toHaveClass(/\breplay\b/);
  await expect(page.locator("#replay-banner")).toContainText("Historical replay");
  const res = await tile;
  expect(res.headers()["content-type"]).toContain("image/png");
  const ms = Date.now() - t0;
  await expect(page.locator("#frame-title")).toContainText("Historical replay");
  return ms;
}

async function scrubTo(page: Page, pos: number) {
  await page.locator("#slider").evaluate((el, v) => {
    const s = el as HTMLInputElement;
    s.value = String(v);
    s.dispatchEvent(new Event("input", { bubbles: true }));
  }, pos);
}

const sliderValue = (page: Page) => page.locator("#slider").evaluate((el) => Number((el as HTMLInputElement).value));

const clickedAt = new WeakMap<Page, number>();

/** Live tile requests issued after the case was clicked. */
function watchLive(page: Page): string[] {
  const live: string[] = [];
  page.on("request", (r) => { const t = clickedAt.get(page); if (t !== undefined && isLiveTile(r)) live.push(path(r)); });
  return live;
}

for (const id of CASES) {
  test.describe(id, () => {
    test("load and scrub", async ({ page }, info) => {
      const { c, manifest } = await caseData(page, id);
      const frames = replayFrames(manifest, "radar");
      const ms = await enter(page, id);
      const live = watchLive(page);
      await expect(page.locator("#slider")).toHaveAttribute("max", String(frames.length - 1));
      await expect(page.locator("#frame-time")).toContainText(replayTimeLabel(frames[0].time, c.timezone));
      await shot(page, info, id, "01-load");

      const mid = Math.floor(frames.length / 2);
      const tile = page.waitForResponse((r) => path(r.url()).startsWith(framePrefix(manifest, frames[mid])) && r.status() === 200);
      await scrubTo(page, mid);
      await expect(page.locator("#frame-time")).toContainText(replayTimeLabel(frames[mid].time, c.timezone));
      await tile;
      await scrubTo(page, frames.length - 1);
      await expect(page.locator("#frame-time")).toContainText(replayTimeLabel(frames[frames.length - 1].time, c.timezone));
      await shot(page, info, id, "02-scrub-end");
      expect(live, "no live tiles during replay").toHaveLength(0);
      record(id, "load-scrub", { frames: frames.length, first_tile_ms: ms, live_tile_requests: live.length });
    });

    test("change layer", async ({ page }, info) => {
      const { manifest } = await caseData(page, id);
      await enter(page, id);
      const live = watchLive(page);
      const seen: Record<string, number> = {};
      page.on("response", (r) => {
        const m = path(r.url()).match(/\/tiles\/([^/]+)\/classic\//);
        if (m && isCaseTile(id)(r.request()) && r.status() === 200) seen[m[1]] = (seen[m[1]] ?? 0) + 1;
      });
      const pos = Math.floor(replayFrames(manifest, "radar").length / 2);
      await scrubTo(page, pos);

      await page.locator("#sheet-toggle").click();
      const hail = page.waitForResponse((r) => path(r.url()).includes("/tiles/hail-size/classic/") && r.status() === 200);
      await page.locator('[data-group="severe"] button[data-value="hail-size"]').click();
      await expect(page.locator("#frame-title")).toContainText("Historical replay");
      await hail;
      await shot(page, info, id, "03-hail");

      await page.locator("#sheet-close").click();
      await page.locator("#layers button[data-layer='radar']").click();
      await page.locator("#sheet-toggle").click();
      const comp = page.waitForResponse((r) => path(r.url()).includes("/tiles/radar-composite/classic/") && r.status() === 200);
      await page.locator('[data-group="product"] button[data-value="radar-composite"]').click();
      await expect(page.locator("#frame-title")).toContainText("composite");
      await comp;
      await shot(page, info, id, "04-composite");

      await page.locator("#sheet-close").click();
      await page.locator("#layers button[data-layer='precip-24h']").click();
      await expect(page.locator("#frame-title")).toContainText("Past 24 hr precip");
      await expect(page.locator("#frame-time")).toContainText(/Rolling 24-hour total|No data for this layer/);
      await shot(page, info, id, "05-precip24h");
      expect(live).toHaveLength(0);
      record(id, "change-layer", { tiles_by_layer: seen, live_tile_requests: live.length });
    });

    test("loop", async ({ page }, info) => {
      const { manifest } = await caseData(page, id);
      const n = replayFrames(manifest, "radar").length;
      await enter(page, id);
      while ((await page.locator("#speed").textContent())?.trim() !== "4×") await page.locator("#speed").click();
      const start = n - 3;
      await scrubTo(page, start);
      await page.locator("#play").click();
      await expect(page.locator("#play")).toHaveAttribute("aria-label", "Pause radar animation");
      const t0 = Date.now();
      await expect.poll(() => sliderValue(page), { timeout: 60_000, intervals: [100] }).toBeLessThan(start);
      const wrapMs = Date.now() - t0;
      await expect.poll(() => sliderValue(page), { timeout: 60_000, intervals: [100] }).toBeGreaterThan(1);
      await shot(page, info, id, "06-loop");
      await page.locator("#play").click();
      await expect(page.locator("#play")).toHaveAttribute("aria-label", "Play radar animation");
      record(id, "loop", { frames: n, start, wrap_ms: wrapMs });
    });

    test("slow network", async ({ page }, info) => {
      const { c, manifest } = await caseData(page, id);
      const frames = replayFrames(manifest, "radar");
      const cdp = await page.context().newCDPSession(page);
      await cdp.send("Network.enable");
      // About "Slow 4G": 400 ms RTT, 1.6 Mbit/s down, 750 kbit/s up.
      const conditions = { offline: false, latency: 400, downloadThroughput: 200_000, uploadThroughput: 93_750 };
      await cdp.send("Network.emulateNetworkConditions", conditions);
      const live = watchLive(page);
      const ms = await enter(page, id);
      await expect(page.locator("#replay-banner [role=alert]")).toHaveCount(0);
      const target = Math.min(10, frames.length - 1);
      const s0 = Date.now();
      await scrubTo(page, target);
      await expect(page.locator("#frame-time")).toContainText(replayTimeLabel(frames[target].time, c.timezone));
      const labelMs = Date.now() - s0;
      await page.waitForResponse((r) => path(r.url()).startsWith(framePrefix(manifest, frames[target])) && r.status() === 200, { timeout: 120_000 });
      const tileMs = Date.now() - s0;
      await shot(page, info, id, "07-slow-network");
      expect(live).toHaveLength(0);
      record(id, "slow-network", { conditions, first_tile_ms: ms, scrub_label_ms: labelMs, scrub_tile_ms: tileMs });
    });

    test("missing data: recorded gaps", async ({ page }, info) => {
      const { c, manifest } = await caseData(page, id);
      const master = replayFrames(manifest, "radar");
      const steps = master.slice(1).map((f, i) => ({ i, dt: f.time - master[i].time }));
      const masterGaps = steps.filter((s) => s.dt > 210);
      const layers: Record<string, { gaps: number; stale: number }> = {};
      for (const l of Object.keys(manifest.layers)) {
        const fs = replayFrames(manifest, l);
        layers[l] = { gaps: fs.filter((f) => f.gap).length, stale: fs.filter((f) => !f.gap && Math.floor(Date.parse(f.timestamp) / 1000) !== f.time).length };
      }
      await enter(page, id);
      const live = watchLive(page);
      // A missing base scan is skipped, not invented: the slider steps straight across it.
      for (const g of masterGaps.slice(0, 2)) {
        await scrubTo(page, g.i + 1);
        await expect(page.locator("#frame-time")).toContainText(replayTimeLabel(master[g.i + 1].time, c.timezone));
      }
      if (masterGaps.length) await shot(page, info, id, "08-base-gap");
      const gapLayer = Object.entries(layers).find(([l, v]) => l !== "radar" && v.gaps > 0)?.[0];
      if (gapLayer) {
        const pos = replayFrames(manifest, gapLayer).findIndex((f) => f.gap);
        if (gapLayer === "precip-24h") await page.locator("#layers button[data-layer='precip-24h']").click();
        else if (gapLayer === "radar-composite") {
          await page.locator("#sheet-toggle").click();
          await page.locator('[data-group="product"] button[data-value="radar-composite"]').click();
        } else {
          await page.locator("#sheet-toggle").click();
          await page.locator(`[data-group="severe"] button[data-value="${gapLayer}"]`).click();
        }
        await scrubTo(page, pos);
        await expect(page.locator("#frame-time")).toContainText("No data for this layer at this time");
        await shot(page, info, id, "09-layer-gap");
      }
      expect(live).toHaveLength(0);
      record(id, "missing-data", { base_frames: master.length, base_gaps_over_3_5_min: masterGaps.map((g) => ({ after: master[g.i].timestamp, seconds: g.dt })), layers, gap_layer_checked: gapLayer ?? null });
    });

    test("missing data: forced tile 404", async ({ page }, info) => {
      const { c, manifest } = await caseData(page, id);
      const frames = replayFrames(manifest, "radar");
      const k = Math.min(5, frames.length - 2);
      const back = Math.floor(frames.length / 2);
      const prefix = framePrefix(manifest, frames[k]);
      let forced = 0;
      await page.route((u) => u.pathname.startsWith(prefix), (route) => { forced++; return route.fulfill({ status: 404, body: "" }); });
      await enter(page, id);
      const live = watchLive(page);
      await scrubTo(page, k);
      await expect(page.locator("#frame-time")).toContainText(replayTimeLabel(frames[k].time, c.timezone));
      await expect.poll(() => forced, { timeout: 30_000 }).toBeGreaterThan(0);
      await expect(page.locator("#replay-banner")).toContainText("Historical replay");
      await shot(page, info, id, "10-forced-404");
      // A later, not yet prefetched frame still loads after the failure.
      const next = page.waitForResponse((r) => path(r.url()).startsWith(framePrefix(manifest, frames[back])) && r.status() === 200);
      await scrubTo(page, back);
      await next;
      expect(live, "a failed case tile never falls back to live").toHaveLength(0);
      record(id, "forced-404", { frame: frames[k].timestamp, forced_404s: forced, live_tile_requests: live.length });
    });

    test("missing data: manifest failure and retry", async ({ page }, info) => {
      const { c } = await caseData(page, id);
      let fail = true;
      await page.route((u) => u.pathname === c.manifest_url, (route) => (fail ? route.fulfill({ status: 503, body: "" }) : route.continue()));
      const button = await openPicker(page, id);
      const live = watchLive(page);
      clickedAt.set(page, Date.now());
      await button.click();
      await expect(page.locator("#replay-banner [role=alert]")).toContainText("Couldn't load this replay");
      await expect(page.locator("#frame-title")).toHaveText("Replay unavailable");
      await shot(page, info, id, "11-manifest-error");
      fail = false;
      await page.locator("#replay-retry").click();
      await expect(page.locator("#frame-title")).toContainText("Historical replay");
      expect(live).toHaveLength(0);
      record(id, "manifest-failure", { live_tile_requests: live.length });
    });

    test("return to live", async ({ page }, info) => {
      await enter(page, id);
      const replayAfter: string[] = [];
      const liveTile = page.waitForResponse((r) => isLiveTile(r.request()) && r.status() === 200, { timeout: 60_000 });
      await page.locator("#return-live").click();
      page.on("request", (r) => { if (isCaseTile(id)(r) || path(r).startsWith("/api/replays/")) replayAfter.push(path(r)); });
      await expect(page.locator("body")).not.toHaveClass(/\breplay\b/);
      await expect(page.locator("#replay-banner")).toBeHidden();
      await expect(page.locator("#frame-title")).not.toContainText("Historical replay");
      await expect(page.locator("#live")).toBeVisible();
      await liveTile;
      await shot(page, info, id, "12-live");
      expect(replayAfter, "no case requests after returning to live").toHaveLength(0);
      record(id, "return-to-live", { live_chip: await page.locator("#live").textContent() });
    });
  });
}
