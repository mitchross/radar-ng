import { fetchRadarManifest, fetchForecast, fetchAlerts } from "../../src/lib/api";

const mockFetch = jest.fn();
global.fetch = mockFetch;

afterEach(() => {
  mockFetch.mockReset();
});

describe("fetchRadarManifest", () => {
  it("returns parsed manifest on success", async () => {
    const manifest = {
      version: "2.0",
      generated: 1776220529,
      host: "https://tilecache.rainviewer.com",
      radar: { past: [{ time: 1776213000, path: "/v2/radar/abc123" }], nowcast: [] },
      satellite: { infrared: [] },
    };
    mockFetch.mockResolvedValueOnce({
      ok: true,
      json: () => Promise.resolve(manifest),
    });

    const result = await fetchRadarManifest();
    expect(result).toEqual(manifest);
    expect(mockFetch).toHaveBeenCalledWith(
      "https://api.rainviewer.com/public/weather-maps.json"
    );
  });

  it("throws on non-ok response", async () => {
    mockFetch.mockResolvedValueOnce({ ok: false, status: 500 });
    await expect(fetchRadarManifest()).rejects.toThrow("RainViewer API error: 500");
  });
});

describe("fetchForecast", () => {
  it("passes correct parameters and returns forecast", async () => {
    const forecast = { current: { temperature_2m: 72 } };
    mockFetch.mockResolvedValueOnce({
      ok: true,
      json: () => Promise.resolve(forecast),
    });

    const result = await fetchForecast(38.9, -77.0);
    expect(result).toEqual(forecast);

    const calledUrl = mockFetch.mock.calls[0][0] as string;
    expect(calledUrl).toContain("latitude=38.9");
    expect(calledUrl).toContain("longitude=-77");
    expect(calledUrl).toContain("temperature_unit=fahrenheit");
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
      expect.objectContaining({ "User-Agent": expect.stringContaining("StormScope") })
    );
  });
});

describe("fetchSelfHostedManifest", () => {
  it("fetches manifest from server URL", async () => {
    const { fetchSelfHostedManifest } = require("../../src/lib/api");
    const manifest = {
      layers: { radar: { timestamps: ["2026-04-14T18:00:00Z"] } },
      tile_url_template: "/tiles/{layer}/{timestamp}/{z}/{x}/{y}.png",
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

describe("checkServerHealth", () => {
  it("returns true when server is healthy", async () => {
    const { checkServerHealth } = require("../../src/lib/api");
    mockFetch.mockResolvedValueOnce({ ok: true });
    const result = await checkServerHealth("http://localhost:8080");
    expect(result).toBe(true);
  });

  it("returns false when server is unreachable", async () => {
    const { checkServerHealth } = require("../../src/lib/api");
    mockFetch.mockRejectedValueOnce(new Error("Network error"));
    const result = await checkServerHealth("http://localhost:8080");
    expect(result).toBe(false);
  });
});
