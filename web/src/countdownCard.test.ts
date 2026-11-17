import { describe, expect, it } from "vitest";
import { countdownCardHtml } from "./countdownCard";

describe("countdownCardHtml", () => {
  it("lists the next holidays with days to go and sunset", () => {
    const html = countdownCardHtml("2026-11-17", { lat: 42.3314, lon: -83.0458, timeZone: "America/Detroit" });
    expect(html).toContain("COMING UP");
    expect(html).toContain("Thanksgiving");
    expect(html).toContain("in 9 days");
    expect(html).toContain("Fri, Dec 25 · Sunset 5:05 PM");
    expect(html).toContain("New Year&#39;s Day");
  });
});
