import { describe, expect, it } from "vitest";
import { WARNING_TAGS, warningTag, warningTagExpression } from "./warningTags";

describe("warningTag", () => {
  it("tags the headline warning types", () => {
    expect(warningTag("Tornado Warning")).toBe("TOR");
    expect(warningTag("Severe Thunderstorm Warning")).toBe("SVR");
    expect(warningTag("Flash Flood Warning")).toBe("FFW");
  });

  it("gives no tag to other types and missing events", () => {
    for (const e of ["Flood Advisory", "Special Weather Statement", "Tornado Watch", "", null, undefined]) {
      expect(warningTag(e)).toBe("");
    }
  });

  it("keeps tags short and unique", () => {
    const tags = WARNING_TAGS.map(([, t]) => t);
    expect(new Set(tags).size).toBe(tags.length);
    for (const t of tags) expect(t).toMatch(/^[A-Z]{3}$/);
  });

  it("builds a match expression covering every tagged event with an empty fallback", () => {
    const expr = warningTagExpression();
    expect(expr.slice(0, 2)).toEqual(["match", ["get", "event"]]);
    expect(expr.slice(2, -1)).toEqual(WARNING_TAGS.flat());
    expect(expr[expr.length - 1]).toBe("");
  });
});
