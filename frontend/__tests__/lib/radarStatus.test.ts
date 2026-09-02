import {
  radarButtonAccessibilityLabel,
  radarQueryIsOffline,
  radarStatus,
} from "../../src/lib/radarStatus";

const NOW = Date.parse("2026-09-02T21:00:00Z");
const secondsAgo = (seconds: number) => Math.floor(NOW / 1000) - seconds;

describe("radarStatus", () => {
  it("treats a NetInfo-paused cached query as offline", () => {
    expect(radarQueryIsOffline(false, true)).toBe(true);
    expect(radarQueryIsOffline(true, false)).toBe(true);
    expect(radarQueryIsOffline(false, false)).toBe(false);
  });

  it("calls a recent displayed observation live", () => {
    expect(radarStatus({
      frameTimeSeconds: secondsAgo(10 * 60),
      refreshFailed: false,
      loading: false,
      nowMilliseconds: NOW,
    })).toEqual({
      label: "LIVE",
      accessibilityLabel: "Live radar available",
      tone: "live",
    });
  });

  it("reports the age of a stale displayed observation", () => {
    expect(radarStatus({
      frameTimeSeconds: secondsAgo(125 * 60),
      refreshFailed: false,
      loading: false,
      nowMilliseconds: NOW,
    })).toEqual({
      label: "UPDATED 2H AGO",
      accessibilityLabel: "Radar updated 2 hours ago",
      tone: "stale",
    });
  });

  it("never labels cached data live after a refresh failure", () => {
    expect(radarStatus({
      frameTimeSeconds: secondsAgo(60),
      refreshFailed: true,
      loading: false,
      nowMilliseconds: NOW,
    })).toEqual({
      label: "OFFLINE",
      accessibilityLabel: "Radar offline. Showing the last saved observation",
      tone: "offline",
    });
  });

  it("distinguishes first load from unavailable data", () => {
    expect(radarStatus({
      frameTimeSeconds: null,
      refreshFailed: false,
      loading: true,
      nowMilliseconds: NOW,
    }).label).toBe("LOADING");
    expect(radarStatus({
      frameTimeSeconds: null,
      refreshFailed: true,
      loading: false,
      nowMilliseconds: NOW,
    }).label).toBe("UNAVAILABLE");
  });

  it("clamps a future observation to live instead of showing a negative age", () => {
    expect(radarStatus({
      frameTimeSeconds: secondsAgo(-30),
      refreshFailed: false,
      loading: false,
      nowMilliseconds: NOW,
    }).label).toBe("LIVE");
  });

  it("keeps status in the button label when a weather headline is present", () => {
    const status = radarStatus({
      frameTimeSeconds: secondsAgo(30 * 60),
      refreshFailed: false,
      loading: false,
      nowMilliseconds: NOW,
    });

    expect(radarButtonAccessibilityLabel(status, "Rain arriving soon")).toBe(
      "Open full radar. Radar updated 30 minutes ago. Rain arriving soon",
    );
  });
});
