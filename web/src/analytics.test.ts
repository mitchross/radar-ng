import { afterEach, beforeEach, expect, it, vi } from "vitest";

const posthog = {
  init: vi.fn(),
  capture: vi.fn(),
  opt_in_capturing: vi.fn(),
  opt_out_capturing: vi.fn(),
  startSessionRecording: vi.fn(),
  stopSessionRecording: vi.fn(),
};
vi.mock("posthog-js", () => ({ default: posthog }));

const values = new Map<string, string>();
const scripts: HTMLScriptElement[] = [];
let umamiTrack: ReturnType<typeof vi.fn>;

// The module touches window/document at import, so every test gets a stub browser.
function stubBrowser() {
  umamiTrack = vi.fn(async () => {});
  vi.stubGlobal("window", {
    umami: { track: umamiTrack },
    addEventListener: vi.fn(),
    dispatchEvent: vi.fn(),
    location: { reload: vi.fn() },
  });
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
  vi.stubEnv("VITE_POSTHOG_KEY", "phc_test");
  scripts.length = 0;
  values.clear();
  for (const fn of Object.values(posthog)) fn.mockClear();
  stubBrowser();
});

afterEach(() => {
  vi.unstubAllEnvs();
  vi.unstubAllGlobals();
});

const load = () => import("./analytics");

it("queues events until PostHog loads, then flushes them", async () => {
  const a = await load();
  a.track("layer_changed", { layer: "wind" });
  expect(posthog.capture).not.toHaveBeenCalled();
  await a.initAnalytics();
  expect(posthog.init).toHaveBeenCalledOnce();
  expect(posthog.capture).toHaveBeenCalledWith("layer_changed", expect.objectContaining({ layer: "wind" }));
});

it("never loads PostHog when the stored preference is off", async () => {
  values.set("rng.analytics", "0");
  const a = await load();
  expect(a.analyticsEnabled()).toBe(false);
  a.track("playback_started");
  await a.initAnalytics();
  expect(posthog.init).not.toHaveBeenCalled();
  expect(posthog.capture).not.toHaveBeenCalled();
});

it("opting out persists, drops queued events and stops the recording", async () => {
  const a = await load();
  await a.initAnalytics();
  await a.setAnalyticsEnabled(false);
  expect(values.get("rng.analytics")).toBe("0");
  expect(posthog.opt_out_capturing).toHaveBeenCalled();
  expect(posthog.stopSessionRecording).toHaveBeenCalledOnce();
  posthog.capture.mockClear();
  a.track("layer_changed");
  expect(posthog.capture).not.toHaveBeenCalled();
  await a.setAnalyticsEnabled(true);
  expect(posthog.opt_in_capturing).toHaveBeenCalled();
  a.track("layer_changed");
  expect(posthog.capture).toHaveBeenCalledOnce();
});

it("re-enabling after a page load initialises PostHog", async () => {
  values.set("rng.analytics", "0");
  const a = await load();
  await a.setAnalyticsEnabled(true);
  expect(posthog.init).toHaveBeenCalledOnce();
  expect(values.get("rng.analytics")).toBe("1");
});

it("leaves PostHog recording off when Umami owns the recorder", async () => {
  vi.stubEnv("VITE_ANALYTICS_RECORDER_OWNER", "umami");
  const a = await load();
  await a.initAnalytics();
  expect(posthog.startSessionRecording).not.toHaveBeenCalled();
  expect(posthog.init).toHaveBeenCalledWith("phc_test", expect.objectContaining({ disable_session_recording: true }));
});

it("stays inert when the Umami build config is missing or malformed", async () => {
  vi.stubEnv("VITE_UMAMI_HOST", "https://ingest-umami.vanillax.me");
  const a = await load();
  expect(a.ANALYTICS_AVAILABLE).toBe(true); // PostHog key is still configured
  vi.resetModules();
  vi.unstubAllEnvs();
  const b = await load();
  expect(b.ANALYTICS_AVAILABLE).toBe(false);
  await b.initAnalytics();
  expect(scripts).toHaveLength(0);
});

it("sends one event to each configured provider", async () => {
  vi.stubEnv("VITE_UMAMI_HOST", "https://ingest-umami.vanillax.me");
  vi.stubEnv("VITE_UMAMI_WEBSITE_ID", "0b6a1d26-3f0e-4dad-8b8a-6c3a9c1f2d10");
  const a = await load();
  a.track("palette_changed", { palette: "radar" });
  await a.initAnalytics();
  // One initial pageview plus the queued event, per provider.
  expect(posthog.capture.mock.calls.map(c => c[0])).toEqual(["$pageview", "palette_changed"]);
  expect(umamiTrack.mock.calls.map(c => c[0].name ?? "$pageview")).toEqual(["$pageview", "palette_changed"]);
  expect(umamiTrack).toHaveBeenCalledWith(expect.objectContaining({ name: "palette_changed", data: { palette: "radar" } }));
});

it("drops unknown properties and unknown events", async () => {
  const a = await load();
  await a.initAnalytics();
  posthog.capture.mockClear();
  a.track("layer_changed", { layer: "wind", token: "secret" });
  expect(posthog.capture.mock.calls[0][1]).not.toHaveProperty("token");
  a.track("not_a_product_event");
  expect(posthog.capture).toHaveBeenCalledOnce();
});

it("respects Do Not Track", async () => {
  vi.stubGlobal("navigator", { doNotTrack: "1" });
  const a = await load();
  expect(a.analyticsEnabled()).toBe(false);
  await a.initAnalytics();
  expect(posthog.init).not.toHaveBeenCalled();
});
