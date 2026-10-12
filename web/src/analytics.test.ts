import { afterEach, beforeEach, expect, it, vi } from "vitest";

const posthog = {
  init: vi.fn(),
  capture: vi.fn(),
  has_opted_out_capturing: vi.fn(() => false),
  opt_in_capturing: vi.fn(),
  opt_out_capturing: vi.fn(),
  startSessionRecording: vi.fn(),
  stopSessionRecording: vi.fn(),
};
vi.mock("posthog-js", () => ({ default: posthog }));

const values = new Map<string, string>();

beforeEach(() => {
  vi.resetModules();
  vi.stubEnv("VITE_POSTHOG_KEY", "phc_test");
  values.clear();
  for (const fn of Object.values(posthog)) fn.mockClear();
  vi.stubGlobal("localStorage", {
    getItem: (key: string) => values.get(key) ?? null,
    setItem: (key: string, value: string) => values.set(key, value),
  });
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
  expect(posthog.capture).toHaveBeenCalledWith("layer_changed", { layer: "wind" });
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
  expect(posthog.opt_out_capturing).toHaveBeenCalledOnce();
  expect(posthog.stopSessionRecording).toHaveBeenCalledOnce();
  a.track("layer_changed");
  expect(posthog.capture).not.toHaveBeenCalled();
  await a.setAnalyticsEnabled(true);
  expect(posthog.opt_in_capturing).toHaveBeenCalledOnce();
  expect(posthog.startSessionRecording).toHaveBeenCalledOnce();
  a.track("layer_changed");
  expect(posthog.capture).toHaveBeenCalledOnce();
});

it("re-enabling after a page load initialises PostHog and clears its persisted opt-out", async () => {
  values.set("rng.analytics", "0");
  posthog.has_opted_out_capturing.mockReturnValueOnce(true);
  const a = await load();
  await a.setAnalyticsEnabled(true);
  expect(posthog.init).toHaveBeenCalledOnce();
  expect(posthog.opt_in_capturing).toHaveBeenCalledOnce();
  expect(values.get("rng.analytics")).toBe("1");
});
