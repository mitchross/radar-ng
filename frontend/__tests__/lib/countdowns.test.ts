import { countdownRows, holidaysForYear } from "../../src/lib/holidays";
import { clockLabel, sunTimes } from "../../src/lib/astro";

// The app re-exports the shared modules; this proves they resolve and run under the app's toolchain.
describe("app countdown re-exports", () => {
  it("pins Thanksgiving and the Detroit Christmas sunset", () => {
    expect(holidaysForYear(2028).find((h) => h.id === "thanksgiving")?.dateKey).toBe("2028-11-23");
    const set = sunTimes("2026-12-25", 42.3314, -83.0458).set!;
    expect([set - 60_000, set, set + 60_000].map((t) => clockLabel(t, undefined, -18000))).toContain("5:05 PM");
  });

  it("builds card rows", () => {
    const rows = countdownRows("2026-11-17", { lat: 42.3314, lon: -83.0458, utcOffsetSeconds: -18000 });
    expect(rows.map((r) => r.inLabel)).toEqual(["in 9 days", "in 38 days", "in 45 days"]);
  });
});
