import {
  fetchForecast,
  fetchRadarNowcast,
  fetchAlerts,
  fetchSelfHostedManifest,
  fetchStormPrefetchPlan,
  fetchServerStatus,
  fetchWithTimeout,
  healthLevelOf,
} from "../../src/lib/api";

const mockFetch = jest.fn();
global.fetch = mockFetch;

afterEach(() => {
  mockFetch.mockReset();
  jest.useRealTimers();
});

const calledUrl = () => mockFetch.mock.calls[0][0] as string;
const calledInit = () => mockFetch.mock.calls[0][1] as RequestInit;

describe("fetchWithTimeout", () => {
  it("always hands fetch an AbortSignal", async () => {
    mockFetch.mockResolvedValueOnce({ ok: true });
    await fetchWithTimeout("http://x");
    expect(calledInit().signal).toBeInstanceOf(AbortSignal);
    expect(calledInit().signal?.aborted).toBe(false);
  });

  it("aborts the request when the deadline passes", async () => {
    jest.useFakeTimers();
    mockFetch.mockImplementationOnce(
      (_url: string, init: RequestInit) =>
        new Promise((_, reject) => {
          init.signal!.addEventListener("abort", () => reject(new Error("aborted")));
        }),
    );
    const pending = expect(fetchWithTimeout("http://x", {}, undefined, 1000)).rejects.toThrow("aborted");
    jest.advanceTimersByTime(1001);
    await pending;
  });

  it("chains the caller's signal (react-query cancellation)", async () => {
    const caller = new AbortController();
    mockFetch.mockImplementationOnce(
      (_url: string, init: RequestInit) =>
        new Promise((_, reject) => {
          init.signal!.addEventListener("abort", () => reject(new Error("aborted")));
        }),
    );
    const pending = expect(fetchWithTimeout("http://x", {}, caller.signal)).rejects.toThrow("aborted");
    caller.abort();
    await pending;
  });
});

describe("fetchRadarNowcast", () => {
  it("fetches the self-hosted point motion nowcast", async () => {
    const nowcast = { status: "ok", points: [] };
    mockFetch.mockResolvedValueOnce({
      ok: true,
      status: 200,
      json: () => Promise.resolve(nowcast),
    });

    await expect(fetchRadarNowcast("https://radar.example", 42.96, -85.67)).resolves.toEqual(nowcast);
    expect(calledUrl()).toBe("https://radar.example/api/nowcast/42.96/-85.67");
  });
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
    expect(calledUrl()).toBe("https://radar-ng-api.vanillax.me/api/forecast/38.9/-77");
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
    expect(calledUrl()).toContain("point=38.9,-77");
    expect(calledInit().headers).toEqual(
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
    expect(calledUrl()).toBe("http://localhost:8080/api/manifest.json");
  });

  it("rejects malformed network manifests", async () => {
    mockFetch.mockResolvedValueOnce({
      ok: true,
      status: 200,
      json: () => Promise.resolve({ layers: { radar: { timestamps: "not-an-array" } } }),
    });

    await expect(fetchSelfHostedManifest("http://localhost:8080")).rejects.toThrow(
      "Invalid radar manifest",
    );
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

    const result = await fetchStormPrefetchPlan("https://radar.example", 42.96, -85.67, "vivid", 6);

    expect(result).toEqual(plan);
    expect(calledUrl()).toBe(
      "https://radar.example/api/storm-prefetch?lat=42.96&lon=-85.67&zoom=6&palette=vivid",
    );
  });
});

describe("fetchServerStatus / healthLevelOf", () => {
  it("parses a 503 degraded body instead of treating it as down", async () => {
    const body = { status: "degraded", mrms_age_s: 1800, reasons: ["mrms_stale"] };
    mockFetch.mockResolvedValueOnce({ ok: false, status: 503, json: () => Promise.resolve(body) });
    const status = await fetchServerStatus("http://localhost:8080");
    expect(status).toEqual(body);
    expect(healthLevelOf(status)).toBe("degraded");
  });

  it("reports ok from a 200 body", async () => {
    mockFetch.mockResolvedValueOnce({
      ok: true,
      status: 200,
      json: () => Promise.resolve({ status: "ok", mrms_age_s: 90 }),
    });
    expect(healthLevelOf(await fetchServerStatus("http://x"))).toBe("ok");
  });

  it("returns null (error) when unreachable or the body is not a health payload", async () => {
    mockFetch.mockRejectedValueOnce(new Error("Network error"));
    expect(await fetchServerStatus("http://x")).toBeNull();
    mockFetch.mockResolvedValueOnce({ ok: false, status: 502, json: () => Promise.reject(new Error("html")) });
    expect(await fetchServerStatus("http://x")).toBeNull();
    mockFetch.mockResolvedValueOnce({ ok: true, status: 200, json: () => Promise.resolve({ detail: "nope" }) });
    expect(await fetchServerStatus("http://x")).toBeNull();
    expect(healthLevelOf(null)).toBe("error");
    expect(healthLevelOf(undefined)).toBe("error");
  });
});
