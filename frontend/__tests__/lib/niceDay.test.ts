import { niceDayRows, niceDaySummary, scoreHour, type NiceDayHour, type NiceDayMode } from "../../src/lib/niceDay";
import { startHourIndex } from "../../src/lib/forecastView";

interface Vector {
  name: string;
  mode: NiceDayMode;
  hour: NiceDayHour;
  score: number | null;
  word: string | null;
}
const { vectors }: { vectors: Vector[] } = require("../../../web/src/shared/testdata/niceDay.vectors.json");

describe("niceDay vectors (same file as web and Rust)", () => {
  it.each(vectors.map((v) => [`${v.mode}: ${v.name}`, v] as const))("%s", (_, v) => {
    const r = scoreHour(v.hour, v.mode);
    expect(r?.score ?? null).toBe(v.score);
    expect(r?.word ?? null).toBe(v.word);
  });
});

describe("niceDaySummary on the app's forecast shape", () => {
  it("reads the Open-Meteo hourly block the Home tab already has", () => {
    const time = Array.from({ length: 24 }, (_, h) => `2026-11-17T${String(h).padStart(2, "0")}:00`);
    const hourly = {
      time,
      temperature_2m: time.map(() => 70),
      precipitation_probability: time.map(() => 0),
      precipitation: time.map(() => 0),
      weather_code: time.map(() => 0),
      wind_speed_10m: time.map((_, h) => (h >= 18 ? 25 : 5)),
    };
    const now = Date.UTC(2026, 10, 17, 14, 0) + 5 * 3600_000;
    const start = startHourIndex(hourly.time, now, -5 * 3600);
    expect(start).toBe(14);
    expect(niceDayRows(niceDaySummary(hourly, start, "2026-11-17"))).toEqual([
      { label: "Nice Day today", word: "Great", when: "Best 2 PM–6 PM" },
      { label: "Bonfire tonight", word: "Do Not", when: "Best 6 PM–12 AM" },
    ]);
  });
});
