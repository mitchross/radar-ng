import { describe, expect, it } from "vitest";
import contract from "./shared/testdata/snowSpot.contract.json";
import { parseSnowSpot } from "./shared/snowSpot";
import { snowSpotCardHtml, snowSpotSkeletonHtml } from "./snowSpotCard";

const now = Date.parse(contract.now);
const clock = { timeZone: contract.time_zone, utcOffsetSeconds: contract.utc_offset_seconds };

describe("snowSpotCardHtml", () => {
  it("renders the full answer with 48 bars and the credits", () => {
    const html = snowSpotCardHtml(parseSnowSpot(contract.full)!, now, clock);
    expect(html).toContain("SNOW AT YOUR SPOT");
    expect(html).toContain("0.1–5 in by Friday 8 pm");
    expect(html.match(/<span style="height/g)).toHaveLength(48);
    expect(html).toContain("60% chance of 4 in or more");
    expect(html).toContain("NOAA SNODAS");
  });
  it("renders an empty answer without throwing, in the same box as the skeleton", () => {
    const html = snowSpotCardHtml(parseSnowSpot(contract.empty)!, now, clock);
    expect(html).toContain('class="snow-card"');
    expect(snowSpotSkeletonHtml()).toContain('class="snow-card snow-skeleton"');
  });
});
