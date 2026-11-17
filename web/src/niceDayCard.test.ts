import { describe, expect, it } from "vitest";
import { niceDayCardHtml } from "./niceDayCard";

const time = Array.from({ length: 24 }, (_, h) => `2026-11-17T${String(h).padStart(2, "0")}:00`);

describe("niceDayCardHtml", () => {
  it("shows both rows and says the index is ours", () => {
    const html = niceDayCardHtml({ time, temperature_2m: time.map(() => 60), wind_speed_10m: time.map(() => 3) }, 9, "2026-11-17");
    expect(html).toContain("Nice Day today");
    expect(html).toContain("Bonfire tonight");
    expect(html).toContain("not an official one");
  });

  it("renders nothing when no hour can be scored", () => {
    expect(niceDayCardHtml({ time }, 0, "2026-11-17")).toBe("");
  });
});
