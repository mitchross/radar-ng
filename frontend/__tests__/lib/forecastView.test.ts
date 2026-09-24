import {
  dailyView,
  hourlyView,
  isNightAt,
  nextHourBanner,
  nowcastHeadline,
  precipitationNext24h,
  weekRange,
} from "../../src/lib/forecastView";
import { getCumulusCondition, getIconKind } from "../../src/lib/cumulusTheme";
import { displayTemperature, formatDegrees } from "../../src/lib/temperature";
import type { OpenMeteoResponse } from "../../src/types/weather";

const pad = (n: number) => String(n).padStart(2, "0");
const localIso = (d: Date) =>
  `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}T${pad(d.getHours())}:00`;
const dateKey = (d: Date) => `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}`;

// Two local days: today 07:00–19:00 daylight, tomorrow 08:00–18:00.
const today = new Date(2026, 8, 24, 21, 0);
const tomorrow = new Date(2026, 8, 25);
const daily: OpenMeteoResponse["daily"] = {
  time: [dateKey(today), dateKey(tomorrow)],
  temperature_2m_max: [70, null],
  temperature_2m_min: [50, 45],
  weather_code: [1, null],
  precipitation_sum: [0.4, 0],
  precipitation_probability_max: [20, null],
  uv_index_max: [5, null],
  sunrise: [localIso(new Date(2026, 8, 24, 7)), localIso(new Date(2026, 8, 25, 8))],
  sunset: [localIso(new Date(2026, 8, 24, 19)), localIso(new Date(2026, 8, 25, 18))],
};

function forecastWith(hourly: Partial<OpenMeteoResponse["hourly"]>): OpenMeteoResponse {
  const hours = Array.from({ length: 30 }, (_, i) => localIso(new Date(2026, 8, 24, 21 + i)));
  return {
    latitude: 0,
    longitude: 0,
    current: {
      time: hours[0],
      temperature_2m: null,
      relative_humidity_2m: null,
      apparent_temperature: null,
      weather_code: null,
      wind_speed_10m: null,
      wind_direction_10m: null,
      wind_gusts_10m: null,
      dew_point_2m: null,
      surface_pressure: null,
    },
    hourly: {
      time: hours,
      temperature_2m: hours.map(() => 60),
      precipitation_probability: hours.map(() => 10),
      precipitation: hours.map(() => 0.01),
      weather_code: hours.map(() => 0),
      wind_speed_10m: hours.map(() => 5),
      ...hourly,
    },
    daily,
  };
}

describe("missing values never become numbers", () => {
  it("keeps a null temperature null and renders it as a dash", () => {
    expect(displayTemperature(null, "fahrenheit")).toBeNull();
    expect(formatDegrees(null)).toBe("—");
    expect(formatDegrees(displayTemperature(212, "celsius"))).toBe("100°");
  });

  it("reports an unknown condition instead of cloudy for a missing weather code", () => {
    expect(getCumulusCondition(null, false)).toBe("unknown");
    expect(getIconKind(undefined, false)).toBe("unknown");
    expect(getIconKind(0, false)).toBe("sun");
  });

  it("shows hail for WMO 96/99 rather than the generic storm icon", () => {
    expect(getIconKind(96, false)).toBe("hail");
    expect(getIconKind(99, false)).toBe("hail");
    expect(getIconKind(95, false)).toBe("storm");
  });
});

describe("day and night per hour", () => {
  it("uses each hour's own sunrise and sunset", () => {
    // Tomorrow 9 AM is after today's sunset but in tomorrow's daylight.
    expect(isNightAt(new Date(2026, 8, 25, 9), daily)).toBe(false);
    expect(isNightAt(new Date(2026, 8, 25, 7), daily)).toBe(true);
    expect(isNightAt(new Date(2026, 8, 24, 20), daily)).toBe(true);
  });

  it("marks hourly icons with the right night flag", () => {
    const hours = hourlyView(forecastWith({}), "fahrenheit", today.getTime());
    const nineAmTomorrow = hours.find((h) => h.label === "9a");
    expect(nineAmTomorrow?.night).toBe(false);
    expect(nineAmTomorrow?.icon).toBe("sun");
    expect(hours[0].label).toBe("NOW");
    expect(hours[0].night).toBe(true);
  });
});

describe("precipitation card", () => {
  it("sums the same 24 hours the strip shows, not the calendar day", () => {
    expect(precipitationNext24h(forecastWith({}), today.getTime())).toBeCloseTo(0.24);
  });

  it("is unknown when every hour in the window is missing", () => {
    const hours = forecastWith({}).hourly.time;
    const forecast = forecastWith({ precipitation: hours.map(() => null) });
    expect(precipitationNext24h(forecast, today.getTime())).toBeNull();
  });
});

describe("7-day rows", () => {
  it("keeps missing highs and conditions unknown and ranges only known values", () => {
    const days = dailyView(forecastWith({}), "fahrenheit", today);
    expect(days[1]).toMatchObject({ hi: null, lo: 45, icon: "unknown", chance: null });
    expect(weekRange(days)).toEqual({ lo: 45, hi: 70 });
    expect(weekRange([])).toBeNull();
  });
});

describe("nowcast headline", () => {
  const minutely = (precipitation: (number | null)[]) => ({
    time: precipitation.map((_, i) => localIso(new Date(today.getTime() + i * 15 * 60_000))),
    precipitation,
  });

  it("announces rain starting from the first known wet quarter", () => {
    expect(nowcastHeadline(minutely([0, 0.2, 0.2, 0]), today.getTime())?.headline).toBe(
      "Rain starts in 15 min",
    );
  });

  it("does not treat unknown quarters as rain or as dry", () => {
    expect(nowcastHeadline(minutely([null, null, null]), today.getTime())).toBeNull();
  });
});

describe("next-hour banner source", () => {
  const minutelyWet = {
    time: Array.from({ length: 4 }, (_, i) => localIso(new Date(today.getTime() + i * 15 * 60_000))),
    precipitation: [0, 0.2, 0.2, 0],
  };
  const radar = (mmPerHour: number[]) => ({
    status: "ok" as const,
    points: mmPerHour.map((mm, i) => ({
      timestamp: "",
      lead_minutes: (i + 1) * 5,
      dbz: null,
      precipitation_mm_h: mm,
    })),
  });

  it("prefers the radar nowcast so Home and the Nowcast tab agree", () => {
    // The model says rain soon; radar says dry. Radar wins, as on the Nowcast tab.
    expect(nextHourBanner(radar(Array(12).fill(0)), minutelyWet, today.getTime())).toBeNull();
    expect(nextHourBanner(radar([0, 0, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5]), undefined, today.getTime())?.headline).toMatch(
      /^Rain starts in \d+ min$/,
    );
  });

  it("falls back to the model series only without a usable radar nowcast", () => {
    expect(nextHourBanner(undefined, minutelyWet, today.getTime())?.headline).toBe("Rain starts in 15 min");
    expect(
      nextHourBanner({ status: "unavailable", points: [] }, minutelyWet, today.getTime())?.headline,
    ).toBe("Rain starts in 15 min");
  });
});
