import {
  privacySafeAttributes,
  telemetryErrorType,
  telemetryQueryFamily,
} from "../../src/lib/telemetryPrivacy";

describe("telemetry privacy", () => {
  it("drops precise location attributes", () => {
    const attributes = privacySafeAttributes({
      "geo.lat": 42.9634,
      "geo.lon": -85.6681,
      latitude: 42.9634,
      coordinates: "42.9634,-85.6681",
      "http.status_code": 500,
    });

    expect(attributes).toEqual({ "http.status_code": 500 });
    expect(JSON.stringify(attributes)).not.toContain("42.9634");
    expect(JSON.stringify(attributes)).not.toContain("-85.6681");
  });

  it("reduces coordinate-bearing query keys to a stable family", () => {
    const attributes = {
      "query.family": telemetryQueryFamily([
        "forecast",
        42.9634,
        -85.6681,
        "https://radar.example",
      ]),
    };

    expect(attributes).toEqual({ "query.family": "forecast" });
    expect(JSON.stringify(attributes)).not.toContain("42.9634");
    expect(JSON.stringify(attributes)).not.toContain("-85.6681");
    expect(telemetryQueryFamily([42.9634, -85.6681])).toBe("unknown");
  });

  it("exports bounded error types instead of free-form messages", () => {
    const error = new Error("request failed at 42.9634,-85.6681");
    expect(telemetryErrorType(error)).toBe("Error");
    error.name = "42.9634,-85.6681";
    expect(telemetryErrorType(error)).toBe("Error");
    error.name = "Error.42.9634.-85.6681";
    expect(telemetryErrorType(error)).toBe("Error");
    expect(telemetryErrorType(new TypeError("safe type"))).toBe("TypeError");
    expect(telemetryErrorType("42.9634,-85.6681")).toBe("NonError");
  });
});
