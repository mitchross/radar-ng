import {
  FIX_LIVE_MAX_AGE_MS,
  FIX_REFRESH_AFTER_MS,
  isFallbackLocation,
  locationNotice,
  parseDeviceFix,
  shouldRefreshFix,
  statusForFix,
} from "../../src/lib/locationStatus";

describe("parseDeviceFix", () => {
  it("accepts a well-formed persisted fix", () => {
    expect(parseDeviceFix('{"latitude":42.96,"longitude":-85.67,"at":1000}')).toEqual({
      latitude: 42.96,
      longitude: -85.67,
      at: 1000,
    });
  });

  it.each(["", "not json", '{"latitude":95,"longitude":0,"at":1}', '{"latitude":1,"longitude":2}'])(
    "rejects %p",
    (value) => {
      expect(parseDeviceFix(value)).toBeNull();
    },
  );
});

describe("fix freshness", () => {
  const now = 10_000_000;

  it("labels a fix live only while it is recent", () => {
    expect(statusForFix(now - FIX_LIVE_MAX_AGE_MS, now)).toBe("live");
    expect(statusForFix(now - FIX_LIVE_MAX_AGE_MS - 1, now)).toBe("last-known");
  });

  it("refreshes on foreground only once a fix is old or missing", () => {
    expect(shouldRefreshFix(null, now)).toBe(true);
    expect(shouldRefreshFix(now - FIX_REFRESH_AFTER_MS, now)).toBe(false);
    expect(shouldRefreshFix(now - FIX_REFRESH_AFTER_MS - 1, now)).toBe(true);
  });
});

describe("honest location labels", () => {
  it("never presents the fallback city as the user's position", () => {
    expect(isFallbackLocation("device", "denied")).toBe(true);
    expect(isFallbackLocation("device", "unavailable")).toBe(true);
    expect(isFallbackLocation("device", "live")).toBe(false);
    expect(isFallbackLocation("city", "denied")).toBe(false);
    expect(locationNotice("device", "denied", "Grand Rapids")).toBe("Location is off · showing Grand Rapids");
    expect(locationNotice("device", "unavailable", "Grand Rapids")).toBe(
      "Location unavailable · showing Grand Rapids",
    );
  });

  it("says nothing extra for a live fix or a chosen city", () => {
    expect(locationNotice("device", "live", "Grand Rapids")).toBeNull();
    expect(locationNotice("city", "denied", "Grand Rapids")).toBeNull();
  });
});
