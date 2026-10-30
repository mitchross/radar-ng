import { describe, expect, it } from "vitest";
import { offsetLabel } from "./offsetLabel";

const NOW = 1_800_000_000;

describe("offsetLabel", () => {
  it("reads within 5 minutes of now as Now, in either direction", () => {
    expect(offsetLabel(NOW, NOW)).toBe("Now");
    expect(offsetLabel(NOW + 5 * 60, NOW)).toBe("Now");
    expect(offsetLabel(NOW - 5 * 60, NOW)).toBe("Now");
    expect(offsetLabel(NOW + 6 * 60, NOW)).toBe("+6m");
    expect(offsetLabel(NOW - 6 * 60, NOW)).toBe("−6m");
  });

  it("switches to hours past 90 minutes", () => {
    expect(offsetLabel(NOW + 89 * 60, NOW)).toBe("+89m");
    expect(offsetLabel(NOW + 90 * 60, NOW)).toBe("+2h");
    expect(offsetLabel(NOW - 4 * 3600, NOW)).toBe("−4h");
  });
});
