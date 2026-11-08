import { describe, expect, it } from "vitest";
import type { Feature, FeatureCollection, Polygon } from "geojson";
import { activeSpc, containsPoint, outlookAt, outlookDays, spcProduct, spcTagExpression, spcColorExpression } from "./spc";

const shell = [[0, 0], [10, 0], [10, 10], [0, 10], [0, 0]];
const hole = [[3, 3], [7, 3], [7, 7], [3, 7], [3, 3]];
const geometry: Polygon = { type: "Polygon", coordinates: [shell, hole] };
const now = Date.parse("2026-11-08T00:00:00Z");
const risk = (label: string, expire = "2026-11-09T00:00:00Z"): Feature => ({
  type: "Feature", geometry, properties: { label, name: `${label} risk`, expire },
});
const fc = (...features: Feature[]): FeatureCollection => ({ type: "FeatureCollection", features });

describe("SPC point outlooks", () => {
  it("handles Polygon and MultiPolygon holes and boundaries", () => {
    for (const shape of [geometry, { type: "MultiPolygon" as const, coordinates: [geometry.coordinates, [[[20, 20], [21, 20], [21, 21], [20, 20]]]] }]) {
      expect(containsPoint(shape, [1, 1])).toBe(true);
      expect(containsPoint(shape, [5, 5])).toBe(false);
      expect(containsPoint(shape, [0, 5])).toBe(true);
      expect(containsPoint(shape, [3, 5])).toBe(false);
      expect(containsPoint(shape, [11, 5])).toBe(false);
    }
    expect(containsPoint({ type: "MultiPolygon", coordinates: [geometry.coordinates, [[[20, 20], [22, 20], [22, 22], [20, 20]]]] }, [21.5, 21])).toBe(true);
  });

  it("takes the highest overlapping current category regardless of feature order", () => {
    const data = fc(risk("SLGT"), risk("HIGH", "2026-11-07T00:00:00Z"), risk("ENH"), risk("TSTM"));
    expect(outlookAt(data, [1, 1], now)).toEqual({ status: "risk", text: "ENH · ENH risk" });
    expect(outlookAt(fc(...data.features.reverse()), [1, 1], now).text).toBe("ENH · ENH risk");
    expect(outlookAt(data, [5, 5], now).status).toBe("none");
  });

  it("distinguishes absent, invalid and expired data from no outlined risk for all three days", () => {
    expect(outlookAt(null, [1, 1], now).status).toBe("unavailable");
    expect(outlookAt(fc(), [1, 1], now).status).toBe("unavailable");
    expect(outlookAt(fc(risk("SLGT", "bad")), [1, 1], now).status).toBe("unavailable");
    expect(outlookAt(fc(risk("SLGT", "2026-11-08T00:00:00Z")), [1, 1], now).status).toBe("expired");
    expect(outlookDays([fc(risk("MRGL")), null, fc(risk("HIGH", "2026-11-07T00:00:00Z"))], [1, 1], now))
      .toEqual(["Day 1: MRGL · MRGL risk", "Day 2: Outlook unavailable", "Day 3: Outlook expired"]);
    expect(outlookAt(fc(risk("TSTM")), [1, 1], now).text).toBe("TSTM · TSTM risk");
    expect(outlookAt(fc(risk("TSTM")), [15, 15], now).text).toBe("No outlined risk");
  });
});

describe("SPC products", () => {
  it("drops expired and undated polygons even from a cached response", () => {
    const product = (expires?: string): Feature => ({ type: "Feature", geometry, properties: { expires } });
    expect(activeSpc(fc(product("2026-11-09T00:00:00Z"), product("2026-11-08T00:00:00Z"), product("bad"), product()), now).features).toHaveLength(1);
    expect(activeSpc({ type: "FeatureCollection" } as FeatureCollection, now).features).toEqual([]);
  });

  it("reads MapLibre serialized probabilities and rejects expired selection", () => {
    const p = { id: "2026-ww-0001", number: 1, type: "tornado", text: "Full text", expires: "2026-11-09T00:00:00Z", probabilities: '[{"label":"Tornadoes","value":"<5%"}]' };
    expect(spcProduct(p, now)?.probabilities).toEqual([{ label: "Tornadoes", value: "<5%" }]);
    expect(spcProduct({ ...p, probabilities: "invalid" }, now)?.probabilities).toEqual([]);
    expect(spcProduct(p, now + 86_400_000)).toBeNull();
    expect(spcProduct({ ...p, type: "unknown" }, now)).toBeNull();
    expect(spcTagExpression()).toContain("TOR WATCH");
    expect(spcTagExpression()).toContain("SVR WATCH");
    expect(spcTagExpression()).toContain("MD");
    expect(new Set(spcColorExpression().filter((s) => typeof s === "string" && s.startsWith("#"))).size).toBe(3);
  });
});
