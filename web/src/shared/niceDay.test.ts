import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";
import { niceDayRows, niceDaySummary, scoreHour, type NiceDayHour, type NiceDayHourly, type NiceDayMode } from "./niceDay";

interface Vector {
  name: string;
  kind: string;
  mode: NiceDayMode;
  hour: NiceDayHour;
  score: number | null;
  word: string | null;
}
const { vectors }: { vectors: Vector[] } = JSON.parse(readFileSync(new URL("./testdata/niceDay.vectors.json", import.meta.url), "utf8"));

describe("niceDay vectors", () => {
  it.each(vectors.map((v) => [`${v.mode}: ${v.name}`, v] as const))("%s", (_, v) => {
    const r = scoreHour(v.hour, v.mode);
    expect(r?.score ?? null).toBe(v.score);
    expect(r?.word ?? null).toBe(v.word);
  });

  it("cover hot, cold, windy, wet, humid and missing hours in both modes", () => {
    for (const mode of ["niceDay", "bonfire"] as const) {
      const kinds = new Set(vectors.filter((v) => v.mode === mode).map((v) => v.kind));
      for (const k of ["hot", "cold", "windy", "wet", "humid", "missing"]) expect(kinds, `${mode} ${k}`).toContain(k);
    }
  });
});

describe("scoreHour rules", () => {
  const calm: NiceDayHour = { tempF: 55, feelsF: 55, windMph: 3, rainChance: 0, rainIn: 0, humidity: 50 };
  it("makes Bonfire Do Not above 20 mph sustained", () => {
    expect(scoreHour({ ...calm, windMph: 20.1 }, "bonfire")?.word).toBe("Do Not");
    expect(scoreHour({ ...calm, windMph: 20 }, "bonfire")?.word).not.toBe("Do Not");
  });
  it("prefers feels-like over air temperature", () => {
    expect(scoreHour({ tempF: 70, feelsF: 30 })?.score).toBeLessThan(scoreHour({ tempF: 70 })!.score);
  });
  it("treats NaN as missing", () => {
    expect(scoreHour({ tempF: Number.NaN })).toBeNull();
  });
});

function day(date: string, hours: number[], at: (h: number) => NiceDayHour) {
  return hours.map((h) => ({ time: `${date}T${String(h).padStart(2, "0")}:00`, ...at(h) }));
}
function hourly(rows: ({ time: string } & NiceDayHour)[]): NiceDayHourly {
  return {
    time: rows.map((r) => r.time),
    temperature_2m: rows.map((r) => r.tempF ?? null),
    apparent_temperature: rows.map((r) => r.feelsF ?? null),
    wind_speed_10m: rows.map((r) => r.windMph ?? null),
    precipitation_probability: rows.map((r) => r.rainChance ?? null),
    precipitation: rows.map((r) => r.rainIn ?? null),
    relative_humidity_2m: rows.map((r) => r.humidity ?? null),
  };
}
const all = Array.from({ length: 24 }, (_, h) => h);

describe("niceDaySummary", () => {
  // Cold morning, nice 1-4 PM, calm evening that turns windy at 22:00.
  const today = day("2026-11-17", all, (h) => ({
    tempF: h >= 13 && h < 16 ? 70 : 45,
    windMph: h >= 22 ? 25 : h >= 18 ? 3 : 8,
    rainChance: 0,
    rainIn: 0,
    humidity: 50,
  }));
  const tomorrow = day("2026-11-18", all, () => ({ tempF: 68, windMph: 4, rainChance: 0, rainIn: 0, humidity: 50 }));
  const h = hourly([...today, ...tomorrow]);

  it("finds today's best run and tonight's Bonfire", () => {
    const s = niceDaySummary(h, 6, "2026-11-17");
    expect(s.best).toMatchObject({ word: "Great", startHour: 13, endHour: 16, tomorrow: false });
    expect(s.bonfire).toMatchObject({ word: "Great", startHour: 18, endHour: 22, tomorrow: false });
    expect(niceDayRows(s)).toEqual([
      { label: "Nice Day today", word: "Great", when: "Best 1 PM–4 PM" },
      { label: "Bonfire tonight", word: "Great", when: "Best 6 PM–10 PM" },
    ]);
  });

  it("moves to tomorrow once today's window has passed", () => {
    const s = niceDaySummary(h, 23, "2026-11-17");
    expect(s.best).toMatchObject({ startHour: 8, endHour: 20, tomorrow: true });
    expect(s.bonfire).toMatchObject({ startHour: 23, endHour: 24, tomorrow: false });
    expect(niceDaySummary(h, 24, "2026-11-17").bonfire).toMatchObject({ tomorrow: true });
    expect(niceDayRows(niceDaySummary(h, 24, "2026-11-17"))[1].label).toBe("Bonfire tomorrow night");
  });

  it("gives no rows when the hours cannot be scored", () => {
    const s = niceDaySummary({ time: h.time }, 0, "2026-11-17");
    expect(s).toEqual({ best: null, bonfire: null });
    expect(niceDayRows(s)).toEqual([]);
  });
});
