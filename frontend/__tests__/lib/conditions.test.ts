import {
  daylightNote,
  dewPointFeel,
  feelsLikeNote,
  hourlyPrecipitation,
  hPaToInHg,
  pressureLabel,
  windToward,
} from "../../src/lib/conditions";
import type { OpenMeteoResponse } from "../../src/types/weather";

describe("pressure", () => {
  it("converts hPa to inHg and labels it", () => {
    expect(hPaToInHg(1013.25)).toBe(29.92);
    expect(pressureLabel(1000)).toBe("Low");
    expect(pressureLabel(1013)).toBe("Normal");
    expect(pressureLabel(1030)).toBe("High");
  });
});

describe("windToward", () => {
  it("points where the wind blows, not where it comes from", () => {
    expect(windToward(22.5)).toBe(202.5); // NNE wind blows toward SSW
    expect(windToward(270)).toBe(90);
    expect(windToward(0)).toBe(180);
  });
});

describe("notes", () => {
  it("describes dew point and feels-like", () => {
    expect(dewPointFeel(50)).toBe("Comfortable");
    expect(dewPointFeel(60)).toBe("Sticky");
    expect(dewPointFeel(70)).toBe("Oppressive");
    expect(feelsLikeNote(59, 54)).toBe("Wind makes it feel colder");
    expect(feelsLikeNote(85, 92)).toBe("Humidity makes it feel warmer");
    expect(feelsLikeNote(70, 71)).toBe("Similar to the actual temperature");
  });
});

describe("hourlyPrecipitation", () => {
  it("returns the next hours' amounts from the current hour, keeping gaps", () => {
    const forecast = {
      hourly: {
        time: ["2026-09-27T10:00", "2026-09-27T11:00", "2026-09-27T12:00", "2026-09-27T13:00"],
        precipitation: [9, 0.1, null, -1],
      },
    } as unknown as OpenMeteoResponse;
    const now = new Date("2026-09-27T11:10").getTime();
    expect(hourlyPrecipitation(forecast, now, 4)).toEqual([0.1, null, 0, null]);
  });
});

describe("daylightNote", () => {
  const sunrise = new Date("2026-09-27T06:37");
  const sunset = new Date("2026-09-27T18:32");
  it("counts down to sunrise, then to sunset", () => {
    expect(daylightNote(new Date("2026-09-27T04:34"), sunrise, sunset)).toBe("Sunrise in 2h 3m");
    expect(daylightNote(new Date("2026-09-27T13:04"), sunrise, sunset)).toBe("5h 28m of daylight left");
    expect(daylightNote(new Date("2026-09-27T20:00"), sunrise, sunset)).toBe("11h 55m of daylight today");
    expect(daylightNote(new Date(), null, sunset)).toBeNull();
  });
});
