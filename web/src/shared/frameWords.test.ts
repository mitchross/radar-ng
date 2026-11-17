import { describe, expect, it } from "vitest";
import { FUTURE_RADAR, MODEL_GUIDANCE, forecastLine, forecastTitle, forecastWords } from "./frameWords";

const NOW = 1_800_000_000;
const H = 3600;

describe("forecastWords", () => {
  it("calls storm-scale frames future radar up to 18 h", () => {
    expect(forecastWords("nowcast", NOW + 600, NOW)).toBe(FUTURE_RADAR);
    expect(forecastWords("radar-hrrr", NOW + 18 * H, NOW)).toBe(FUTURE_RADAR);
  });

  it("calls every frame past 18 h, and all of GFS, model guidance", () => {
    expect(forecastWords("radar-hrrr", NOW + 18 * H + 60, NOW)).toBe(MODEL_GUIDANCE);
    expect(forecastWords("radar-gfs", NOW + 17 * H, NOW)).toBe(MODEL_GUIDANCE);
    expect(MODEL_GUIDANCE).toBe("Precipitation forecast · model guidance");
    expect(MODEL_GUIDANCE.toLowerCase()).not.toContain("future radar");
  });

  it("has no words for observed or past frames", () => {
    expect(forecastWords("radar", NOW + H, NOW)).toBeNull();
    expect(forecastWords("radar-hrrr", NOW - H, NOW)).toBeNull();
  });

  it("names the model on the timeline line and shortens for the inspector", () => {
    expect(forecastLine("radar-hrrr", NOW + 2 * H, NOW)).toBe("Future radar · HRRR");
    expect(forecastLine("radar-gfs", NOW + 40 * H, NOW)).toBe("Precipitation forecast · model guidance · GFS");
    expect(forecastTitle("radar-hrrr", NOW + 2 * H, NOW)).toBe("FUTURE RADAR");
    expect(forecastTitle("radar-gfs", NOW + 40 * H, NOW)).toBe("MODEL GUIDANCE");
    expect(forecastTitle("radar", NOW, NOW)).toBeNull();
  });
});
