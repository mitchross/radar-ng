import { getWeatherInfo } from "../../src/lib/weatherCodes";

describe("getWeatherInfo", () => {
  it("returns correct label for clear sky", () => {
    expect(getWeatherInfo(0).label).toBe("Clear sky");
  });

  it("returns correct label for thunderstorm", () => {
    expect(getWeatherInfo(95).label).toBe("Thunderstorm");
  });

  it("returns Unknown for unrecognized code", () => {
    expect(getWeatherInfo(999).label).toBe("Unknown");
  });
});
