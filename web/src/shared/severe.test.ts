import { describe, expect, it } from "vitest";
import { SEVERE_CREDIT, SEVERE_LAYERS, formatSevereReading, isSevereLayer, severeLayer } from "./severe";
import { LAYER_PALETTE_KEY } from "./serverPalettes";

describe("severe layers", () => {
  it("lists the three layers with a hint and a palette key each", () => {
    expect(SEVERE_LAYERS.map((l) => l.id)).toEqual(["hail-size", "rotation-track", "echo-top"]);
    for (const l of SEVERE_LAYERS) {
      expect(l.hint.length).toBeGreaterThan(10);
      expect(LAYER_PALETTE_KEY[l.id]).toBeTruthy();
    }
    expect(SEVERE_CREDIT).toBe("NOAA MRMS");
  });

  it("recognises only severe layers", () => {
    expect(isSevereLayer("hail-size")).toBe(true);
    expect(isSevereLayer("radar")).toBe(false);
    expect(severeLayer("echo-top")?.unit).toBe("kft");
  });

  it("formats readouts", () => {
    expect(formatSevereReading("hail-size", 1.75)).toBe("1.75 in hail");
    expect(formatSevereReading("hail-size", 0)).toBe("No hail");
    expect(formatSevereReading("rotation-track", 0.0123)).toBe("0.012 /s");
    expect(formatSevereReading("echo-top", 42.4)).toBe("42 kft");
    expect(formatSevereReading("radar", 5)).toBeNull();
  });
});
