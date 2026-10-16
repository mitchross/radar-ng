import { satelliteFrames, satelliteIndexAt, satelliteTileUrl } from "../../src/lib/satellite";
import { parseSelfHostedManifest } from "../../src/lib/manifest";

// The tile server's satellite layer, as /api/manifest.json sends it.
const manifest = {
  schema_version: 2,
  tile_url_template: "/tiles/{layer}/{palette}/{path}/{z}/{x}/{y}.png",
  updated_at: "2026-10-15T22:40:00+00:00",
  layers: {
    satellite: {
      frames: ["22:00", "22:10"].map((hm) => ({
        timestamp: `2026-10-15T${hm}:00+00:00`,
        path: `2026-10-15T${hm}:00Z`,
        palettes: [],
        kind: "observation",
        source: "goes-geocolor",
        max_zoom: 7,
      })),
      timestamps: ["2026-10-15T22:00:00+00:00", "2026-10-15T22:10:00+00:00"],
      latest: "2026-10-15T22:10:00+00:00",
      palettes: [],
      title: "GOES GeoColor satellite",
      kind: "satellite",
    },
  },
};

describe("satellite layer in the app", () => {
  it("passes manifest validation and maps timeline times to frames", () => {
    const parsed = parseSelfHostedManifest(manifest);
    const frames = satelliteFrames(parsed);
    expect(frames.map((f) => f.path)).toEqual(["2026-10-15T22:00:00Z", "2026-10-15T22:10:00Z"]);
    const at = (iso: string) => satelliteIndexAt(frames, Date.parse(iso) / 1000);
    expect(at("2026-10-15T22:09:00Z")).toBe(0);
    expect(at("2026-10-15T22:52:00Z")).toBe(1);
    expect(at("2026-10-16T02:00:00Z")).toBe(-1);
  });

  it("builds the proxied tile URL on the configured server", () => {
    expect(satelliteTileUrl("https://radar.example", "2026-10-15T22:10:00Z")).toBe(
      "https://radar.example/tiles/satellite/2026-10-15T22:10:00Z/{z}/{x}/{y}.png",
    );
  });
});
