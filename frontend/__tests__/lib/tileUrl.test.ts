import { buildSelfHostedTileUrl } from "../../src/lib/tileUrl";

describe("buildSelfHostedTileUrl", () => {
  it("builds tile URL with default palette", () => {
    const url = buildSelfHostedTileUrl(
      "https://radar-ng-api.vanillax.me",
      "temperature",
      "2026-04-23T10:00:00+00:00",
    );
    expect(url).toBe(
      "https://radar-ng-api.vanillax.me/tiles/temperature/classic/2026-04-23T10:00:00+00:00/{z}/{x}/{y}.png",
    );
  });

  it("respects a non-default palette", () => {
    const url = buildSelfHostedTileUrl("http://s", "radar", "T", "vivid");
    expect(url).toBe("http://s/tiles/radar/vivid/T/{z}/{x}/{y}.png");
  });

  it("accepts the synthesized 'nowcast' layer key for forecast frames", () => {
    const url = buildSelfHostedTileUrl("http://s", "nowcast", "T", "muted");
    expect(url).toBe("http://s/tiles/nowcast/muted/T/{z}/{x}/{y}.png");
  });
});
