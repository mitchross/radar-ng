import { sharedStateJson, type SharedStateInput } from "../../src/lib/sharedState";

const base: SharedStateInput = {
  serverUrl: "https://radar-ng-api.vanillax.me",
  activePalette: "classic",
  temperatureUnit: "fahrenheit",
  locationMode: "device",
  latitude: 42.963412,
  longitude: -85.668129,
  selectedPlace: null,
  devicePlace: { id: 1, name: "Grand Rapids", admin1: "Michigan", latitude: 42.96, longitude: -85.67 },
  lastFixAt: 1_790_000_000_000,
};

describe("shared state for the widget, CarPlay and Watch", () => {
  it("carries a rounded device fix with its label and age", () => {
    const s = JSON.parse(sharedStateJson(base));
    expect(s).toMatchObject({ v: 1, serverUrl: base.serverUrl, palette: "classic", temperatureUnit: "fahrenheit" });
    expect(s.location).toEqual({ mode: "device", lat: 42.96, lon: -85.67, label: "Grand Rapids, Michigan", at: base.lastFixAt });
  });

  it("marks a chosen city so extensions show it instead of their own GPS", () => {
    const chicago = { id: 2, name: "Chicago", admin1: "Illinois", latitude: 41.88, longitude: -87.63 };
    const s = JSON.parse(
      sharedStateJson({ ...base, locationMode: "city", selectedPlace: chicago, latitude: 41.8781, longitude: -87.6298 }),
    );
    expect(s.location).toEqual({ mode: "city", lat: 41.88, lon: -87.63, label: "Chicago, Illinois", at: null });
  });

  it("sends no location before the first fix", () => {
    expect(JSON.parse(sharedStateJson({ ...base, latitude: null, longitude: null })).location).toBeNull();
  });

  it("is stable for an unchanged state, so nothing is re-published", () => {
    expect(sharedStateJson(base)).toBe(sharedStateJson({ ...base }));
  });
});
