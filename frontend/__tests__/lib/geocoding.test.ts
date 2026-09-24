import { resetReverseGeocodeCache, reverseGeocode, searchCities } from "../../src/lib/geocoding";

const fetchMock = jest.fn();
const SERVER = "https://radar.example";

const grandRapids = {
  id: 4994358,
  name: "Grand Rapids",
  latitude: 42.963,
  longitude: -85.668,
  admin1: "Michigan",
  country: "United States",
  countryCode: "US",
};

const respond = (body: unknown, ok = true, status = 200) =>
  fetchMock.mockResolvedValue({ ok, status, json: async () => body });

beforeEach(() => {
  fetchMock.mockReset();
  global.fetch = fetchMock;
  resetReverseGeocodeCache();
});

const calledUrl = (n = 0) => new URL(fetchMock.mock.calls[n][0]);

describe("searchCities", () => {
  it("does not search for short queries", async () => {
    await expect(searchCities(SERVER, "g")).resolves.toEqual([]);
    expect(fetchMock).not.toHaveBeenCalled();
  });

  it("queries the self-hosted geocoder and keeps well-formed places", async () => {
    respond({ results: [grandRapids, { id: "bad", name: 3 }] });

    await expect(searchCities(SERVER, " Grand Rapids ")).resolves.toEqual([grandRapids]);

    const url = calledUrl();
    expect(url.origin + url.pathname).toBe("https://radar.example/api/geocode");
    expect(url.searchParams.get("q")).toBe("Grand Rapids");
    expect(url.searchParams.get("limit")).toBe("8");
  });

  it("explains a server without a geocoder", async () => {
    respond({ error: "geocoding_not_configured" }, false, 503);
    await expect(searchCities(SERVER, "Grand")).rejects.toThrow("not set up");
  });
});

describe("reverseGeocode", () => {
  it("asks the server for the rounded point and returns its place", async () => {
    respond({ place: grandRapids });

    await expect(reverseGeocode(SERVER, 42.9634567, -85.6681234)).resolves.toEqual(grandRapids);

    const url = calledUrl();
    expect(url.origin + url.pathname).toBe("https://radar.example/api/reverse-geocode");
    expect(url.searchParams.get("lat")).toBe("42.963");
    expect(url.searchParams.get("lon")).toBe("-85.668");
  });

  it("reuses one lookup for GPS jitter inside the same rounded point", async () => {
    respond({ place: grandRapids });
    const first = await reverseGeocode(SERVER, 42.9634, -85.6681);
    const second = await reverseGeocode(SERVER, 42.96341, -85.66811);
    expect(second).toBe(first);
    expect(fetchMock).toHaveBeenCalledTimes(1);
  });

  it("shares one request between concurrent callers", async () => {
    respond({ place: grandRapids });
    await Promise.all([
      reverseGeocode(SERVER, 42.9634, -85.6681),
      reverseGeocode(SERVER, 42.96341, -85.66811),
    ]);
    expect(fetchMock).toHaveBeenCalledTimes(1);
  });

  it("looks up again after moving to a new rounded point or server", async () => {
    respond({ place: grandRapids });
    await reverseGeocode(SERVER, 42.9634, -85.6681);
    await reverseGeocode(SERVER, 42.9649, -85.6681);
    await reverseGeocode("https://other.example", 42.9649, -85.6681);
    expect(fetchMock).toHaveBeenCalledTimes(3);
  });

  it("returns null when the server has no place for the point", async () => {
    respond({ place: null });
    await expect(reverseGeocode(SERVER, 0, 0)).resolves.toBeNull();
  });

  it("treats server failure as a missing label and retries on the next call", async () => {
    fetchMock.mockResolvedValueOnce({ ok: false, status: 503, json: async () => ({}) });
    await expect(reverseGeocode(SERVER, 42.9634, -85.6681)).resolves.toBeNull();
    respond({ place: grandRapids });
    await expect(reverseGeocode(SERVER, 42.9634, -85.6681)).resolves.toEqual(grandRapids);
    expect(fetchMock).toHaveBeenCalledTimes(2);
  });
});
