import { afterEach, beforeEach, expect, it, vi } from "vitest";

const values = new Map<string, string>();
const scripts: HTMLScriptElement[] = [];
let umamiTrack: ReturnType<typeof vi.fn>;
let reload: ReturnType<typeof vi.fn>;

// The module touches window/document at import, so every test gets a stub browser.
function stubBrowser() {
  umamiTrack = vi.fn(async () => {});
  reload = vi.fn();
  vi.stubGlobal("window", {
    umami: { track: umamiTrack },
    addEventListener: vi.fn(),
    dispatchEvent: vi.fn(),
  });
  vi.stubGlobal("location", { origin: "https://radar.example", reload });
  vi.stubGlobal("document", {
    createElement: (tag: string) => ({ tagName: tag.toUpperCase(), dataset: {}, remove: vi.fn() }),
    head: {
      append: (el: HTMLScriptElement) => {
        scripts.push(el);
        setTimeout(() => (el.onload as null | (() => void))?.(), 0);
      },
    },
  });
  vi.stubGlobal("navigator", { doNotTrack: undefined });
  vi.stubGlobal("localStorage", {
    getItem: (key: string) => values.get(key) ?? null,
    setItem: (key: string, value: string) => values.set(key, value),
  });
}

beforeEach(() => {
  vi.resetModules();
  vi.stubEnv("VITE_UMAMI_HOST", "https://ingest-umami.vanillax.me");
  vi.stubEnv("VITE_UMAMI_WEBSITE_ID", "0b6a1d26-3f0e-4dad-8b8a-6c3a9c1f2d10");
  scripts.length = 0;
  values.clear();
  stubBrowser();
});

afterEach(() => {
  vi.unstubAllEnvs();
  vi.unstubAllGlobals();
});

const load = () => import("./analytics");
const names = () => umamiTrack.mock.calls.map(c => c[0].name ?? "$pageview");

it("queues events until Umami loads, then sends a pageview and the queue", async () => {
  const a = await load();
  a.track("palette_changed", { palette: "radar" });
  expect(umamiTrack).not.toHaveBeenCalled();
  await a.initAnalytics();
  expect(scripts).toHaveLength(1);
  expect(scripts[0].src).toBe("https://ingest-umami.vanillax.me/script.js");
  expect(names()).toEqual(["$pageview", "palette_changed"]);
  expect(umamiTrack).toHaveBeenCalledWith(expect.objectContaining({
    website: "0b6a1d26-3f0e-4dad-8b8a-6c3a9c1f2d10", name: "palette_changed", data: { palette: "radar" },
  }));
});

it("never loads Umami when the stored preference is off", async () => {
  values.set("rng.analytics", "0");
  const a = await load();
  expect(a.analyticsEnabled()).toBe(false);
  a.track("playback_started");
  await a.initAnalytics();
  expect(scripts).toHaveLength(0);
  expect(umamiTrack).not.toHaveBeenCalled();
});

it("opting out persists, drops events and reloads to unload the tracker", async () => {
  const a = await load();
  await a.initAnalytics();
  await a.setAnalyticsEnabled(false);
  expect(values.get("rng.analytics")).toBe("0");
  expect(reload).toHaveBeenCalledOnce();
  umamiTrack.mockClear();
  a.track("layer_changed", { layer: "wind" });
  expect(umamiTrack).not.toHaveBeenCalled();
});

it("re-enabling after a page load starts Umami", async () => {
  values.set("rng.analytics", "0");
  const a = await load();
  await a.setAnalyticsEnabled(true);
  expect(values.get("rng.analytics")).toBe("1");
  expect(scripts).toHaveLength(1);
  expect(names()).toEqual(["$pageview"]);
});

it("stays inert when the Umami build config is missing or malformed", async () => {
  vi.stubEnv("VITE_UMAMI_WEBSITE_ID", "not-a-uuid");
  const a = await load();
  expect(a.ANALYTICS_AVAILABLE).toBe(false);
  vi.resetModules();
  vi.unstubAllEnvs();
  const b = await load();
  expect(b.ANALYTICS_AVAILABLE).toBe(false);
  await b.initAnalytics();
  expect(scripts).toHaveLength(0);
});

it("drops unknown properties and unknown events", async () => {
  const a = await load();
  await a.initAnalytics();
  umamiTrack.mockClear();
  a.track("layer_changed", { layer: "wind", token: "secret" });
  expect(umamiTrack.mock.calls[0][0].data).toEqual({ layer: "wind" });
  a.track("not_a_product_event");
  expect(umamiTrack).toHaveBeenCalledOnce();
});

it("respects Do Not Track", async () => {
  vi.stubGlobal("navigator", { doNotTrack: "1" });
  const a = await load();
  expect(a.analyticsEnabled()).toBe(false);
  await a.initAnalytics();
  expect(scripts).toHaveLength(0);
});

it("loads the recorder only after startReplay and the tracker are both ready", async () => {
  const a = await load();
  await a.initAnalytics();
  expect(scripts.map(s => s.src)).toEqual(["https://ingest-umami.vanillax.me/script.js"]);
  a.startReplay();
  a.startReplay();
  expect(scripts.map(s => s.src)).toEqual([
    "https://ingest-umami.vanillax.me/script.js",
    "https://ingest-umami.vanillax.me/recorder.js",
  ]);
  expect(scripts[1].dataset).toMatchObject({ websiteId: "0b6a1d26-3f0e-4dad-8b8a-6c3a9c1f2d10" });
});

it("never loads the recorder when analytics are off", async () => {
  values.set("rng.analytics", "0");
  const a = await load();
  a.startReplay();
  await a.initAnalytics();
  expect(scripts).toHaveLength(0);
});
