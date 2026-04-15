import { buildRadarTileUrl } from "../../src/lib/tileUrl";

describe("buildRadarTileUrl", () => {
  const host = "https://tilecache.rainviewer.com";
  const frame = { time: 1776213000, path: "/v2/radar/abc123" };

  it("builds correct tile URL with defaults", () => {
    const url = buildRadarTileUrl(host, frame);
    expect(url).toBe(
      "https://tilecache.rainviewer.com/v2/radar/abc123/256/{z}/{x}/{y}/6/1_1.png"
    );
  });

  it("respects custom tile size", () => {
    const url = buildRadarTileUrl(host, frame, { size: 512 });
    expect(url).toContain("/512/{z}");
  });

  it("respects smooth=false and snow=false", () => {
    const url = buildRadarTileUrl(host, frame, { smooth: false, snow: false });
    expect(url).toMatch(/\/0_0\.png$/);
  });
});
