import {
  fetchForecast,
  fetchAlerts,
  fetchSelfHostedManifest,
  fetchStormPrefetchPlan,
  checkServerHealth,
} from "../../src/lib/api";

const mockFetch = jest.fn();
global.fetch = mockFetch;

afterEach(() => {
  mockFetch.mockReset();
});

describe("fetchForecast", () => {
  it("hits the tile-server proxy with lat/lon path params", async () => {
    const forecast = { current: { temperature_2m: 72 } };
    mockFetch.mockResolvedValueOnce({
      ok: true,
      json: () => Promise.resolve(forecast),
    });

    const result = await fetchForecast("https://radar-ng-api.vanillax.me", 38.9, -77.0);
    expect(result).toEqual(forecast);
    expect(mockFetch).toHaveBeenCalledWith(
      "https://radar-ng-api.vanillax.me/api/forecast/38.9/-77",
    );
  });

  it("throws on non-ok response", async () => {
    mockFetch.mockResolvedValueOnce({ ok: false, status: 500 });
    await expect(fetchForecast("http://x", 0, 0)).rejects.toThrow("Forecast error: 500");
  });
});

describe("fetchAlerts", () => {
  it("sends correct User-Agent header", async () => {
    mockFetch.mockResolvedValueOnce({
      ok: true,
      json: () => Promise.resolve({ features: [] }),
    });

    await fetchAlerts(38.9, -77.0);
    const calledUrl = mockFetch.mock.calls[0][0] as string;
    const calledOptions = mockFetch.mock.calls[0][1] as RequestInit;
    expect(calledUrl).toContain("point=38.9,-77");
    expect(calledOptions.headers).toEqual(
      expect.objectContaining({ "User-Agent": expect.stringContaining("radar-ng") }),
    );
  });
});

describe("fetchSelfHostedManifest", () => {
  it("fetches manifest from server URL", async () => {
    const manifest = {
      layers: { radar: { timestamps: ["2026-04-14T18:00:00Z"] } },
      tile_url_template: "/tiles/{layer}/{palette}/{timestamp}/{z}/{x}/{y}.png",
      updated_at: "2026-04-14T18:04:00Z",
    };
    mockFetch.mockResolvedValueOnce({
      ok: true,
      json: () => Promise.resolve(manifest),
    });
    const result = await fetchSelfHostedManifest("http://localhost:8080");
    expect(result).toEqual(manifest);
    expect(mockFetch).toHaveBeenCalledWith("http://localhost:8080/api/manifest.json");
  });
});

describe("fetchStormPrefetchPlan", () => {
  it("passes location, zoom, and palette to the tile server", async () => {
    const plan = { plan_id: null, storm_cell_id: null, bboxes: [], tile_urls: [] };
    mockFetch.mockResolvedValueOnce({
      ok: true,
      status: 200,
      json: () => Promise.resolve(plan),
    });

    const result = await fetchStormPrefetchPlan(
      "https://radar.example",
      42.96,
      -85.67,
      "vivid",
      6,
    );

    expect(result).toEqual(plan);
    expect(mockFetch).toHaveBeenCalledWith(
      "https://radar.example/api/storm-prefetch?lat=42.96&lon=-85.67&zoom=6&palette=vivid",
    );
  });
});

describe("checkServerHealth", () => {
  it("returns true when server is healthy", async () => {
    mockFetch.mockResolvedValueOnce({ ok: true });
    const result = await checkServerHealth("http://localhost:8080");
    expect(result).toBe(true);
  });

  it("returns false when server is unreachable", async () => {
    mockFetch.mockRejectedValueOnce(new Error("Network error"));
    const result = await checkServerHealth("http://localhost:8080");
    expect(result).toBe(false);
  });
});
