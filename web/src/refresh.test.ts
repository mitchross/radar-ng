import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { startPeriodic } from "./refresh";

const PERIOD = 15 * 60_000;
const listeners = new Set<() => void>();
const doc = { visibilityState: "visible" as DocumentVisibilityState };
const fire = () => listeners.forEach((l) => l());

beforeEach(() => {
  vi.useFakeTimers();
  listeners.clear();
  doc.visibilityState = "visible";
  vi.stubGlobal("window", { setInterval: (f: () => void, ms: number) => setInterval(f, ms), clearInterval: (id: number) => clearInterval(id) });
  vi.stubGlobal("document", {
    get visibilityState() { return doc.visibilityState; },
    addEventListener: (_: string, l: () => void) => listeners.add(l),
    removeEventListener: (_: string, l: () => void) => listeners.delete(l),
  });
});

afterEach(() => {
  vi.useRealTimers();
  vi.unstubAllGlobals();
});

it("runs once per period while the page is visible", () => {
  const fn = vi.fn();
  const stop = startPeriodic(fn, PERIOD);
  vi.advanceTimersByTime(PERIOD * 2);
  expect(fn).toHaveBeenCalledTimes(2);
  stop();
  vi.advanceTimersByTime(PERIOD);
  expect(fn).toHaveBeenCalledTimes(2);
  expect(listeners.size).toBe(0);
});

it("skips hidden ticks and catches up as soon as the tab is visible again", () => {
  const fn = vi.fn();
  startPeriodic(fn, PERIOD);
  doc.visibilityState = "hidden";
  fire();
  vi.advanceTimersByTime(PERIOD * 3);
  expect(fn).not.toHaveBeenCalled();
  doc.visibilityState = "visible";
  fire();
  expect(fn).toHaveBeenCalledOnce();
});

it("does not re-run when the tab was hidden for less than a period", () => {
  const fn = vi.fn();
  startPeriodic(fn, PERIOD);
  doc.visibilityState = "hidden";
  fire();
  vi.advanceTimersByTime(PERIOD / 2);
  doc.visibilityState = "visible";
  fire();
  expect(fn).not.toHaveBeenCalled();
});
