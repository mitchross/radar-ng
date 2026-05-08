import { searchCities } from "../../src/lib/geocoding";

const fetchMock = jest.fn();

beforeEach(() => {
  fetchMock.mockReset();
  global.fetch = fetchMock;
});

describe("searchCities", () => {
  it("does not search for short queries", async () => {
    await expect(searchCities("g")).resolves.toEqual([]);
    expect(fetchMock).not.toHaveBeenCalled();
  });

  it("normalizes Open-Meteo geocoding results", async () => {
    fetchMock.mockResolvedValue({
      ok: true,
      json: async () => ({
        results: [
          {
            id: 4994358,
            name: "Grand Rapids",
            latitude: 42.9634,
            longitude: -85.6681,
            admin1: "Michigan",
            country: "United States",
            country_code: "US",
          },
        ],
      }),
    });

    await expect(searchCities("Grand Rapids")).resolves.toEqual([
      {
        id: 4994358,
        name: "Grand Rapids",
        latitude: 42.9634,
        longitude: -85.6681,
        admin1: "Michigan",
        country: "United States",
        countryCode: "US",
      },
    ]);

    const url = new URL(fetchMock.mock.calls[0][0]);
    expect(url.origin + url.pathname).toBe("https://geocoding-api.open-meteo.com/v1/search");
    expect(url.searchParams.get("name")).toBe("Grand Rapids");
    expect(url.searchParams.get("count")).toBe("8");
  });
});
