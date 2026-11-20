import { readFileSync } from "fs";
import { join } from "path";
import { buildLut, buildPackedFrames, buildSmoothLut, packedTileUrl } from "../../src/lib/packed";
import { frameKeyOf, lutHash } from "../../src/lib/inMapRadarTiles";
import { legendGradient, legendIsSmooth } from "../../src/lib/serverPalettes";
import type { ColorTable } from "../../../web/src/shared/serverPalettes";

const table = (id: string) =>
  JSON.parse(readFileSync(join(__dirname, "../../../rust/crates/radar-tiler/assets", `${id}.json`), "utf8")) as Record<string, ColorTable>;
const PALETTES = { classic: table("classic"), muted: table("muted"), vivid: table("vivid") };

const RADAR_META = {
  version: 2,
  tile_size: 512,
  min_zoom: 3,
  max_zoom: 7,
  cols: 2,
  max_slots: 24,
  color_key: "reflectivity",
  scale: 2.5,
  offset: 0,
  types: { stride: 32, classes: ["rain", "snow", "mix", "ice"] },
  url_template: "/api/tiles/{layer}/packed/{bucket}/{z}/{x}/{y}.png?v={v}",
  buckets: [{ id: "20261120_16", v: 3, slots: ["2026-11-20T16:00:00+00:00", "2026-11-20T16:10:00+00:00"] }],
};
const FRAMES = [{ timestamp: "2026-11-20T16:00:00+00:00" }, { timestamp: "2026-11-20T16:10:00+00:00" }];
const LUT_KEY = "reflectivity|2.5|0|32|rain,snow,mix,ice";

describe("app smooth ramps (parity with the web radar)", () => {
  it.each(Object.keys(PALETTES) as (keyof typeof PALETTES)[])("colours %s frames with the web's smooth or stepped table", (id) => {
    const tables = PALETTES[id];
    const smooth = buildPackedFrames(FRAMES, { radar: { packed: RADAR_META } }, tables, "radar", true)!;
    const stepped = buildPackedFrames(FRAMES, { radar: { packed: RADAR_META } }, tables, "radar", false)!;
    expect(smooth.luts[LUT_KEY]).toEqual(buildSmoothLut(tables.reflectivity, 2.5, 0, RADAR_META.types));
    // Stepped is PR #364's table: what buildPackedFrames built before this setting existed.
    expect(stepped.luts[LUT_KEY]).toEqual(buildLut(tables.reflectivity, 2.5, 0, RADAR_META.types));
    expect(buildPackedFrames(FRAMES, { radar: { packed: RADAR_META } }, tables, "radar")!.luts).toEqual(stepped.luts);
  });

  it("recolours on a mode change without refetching: same atlas URLs, new frame key", () => {
    const smooth = buildPackedFrames(FRAMES, { radar: { packed: RADAR_META } }, PALETTES.classic, "radar", true)!;
    const stepped = buildPackedFrames(FRAMES, { radar: { packed: RADAR_META } }, PALETTES.classic, "radar", false)!;
    const a = smooth.locations[1]!;
    const b = stepped.locations[1]!;
    const url = (l: typeof a) => packedTileUrl("https://radar.example", l.meta.url_template, l.layer, l.bucket, l.v, 5, 8, 12);
    expect(url(a)).toBe(url(b));
    const hashA = lutHash(smooth.luts[a.lutKey]);
    const hashB = lutHash(stepped.luts[b.lutKey]);
    expect(hashA).not.toBe(hashB);
    expect(frameKeyOf(a, hashA)).not.toBe(frameKeyOf(b, hashB));
  });

  it("fades the low end in and keeps class ramps apart in the app's table", () => {
    const lut = buildPackedFrames(FRAMES, { radar: { packed: RADAR_META } }, PALETTES.vivid, "radar", true)!.luts[LUT_KEY];
    const first = PALETTES.vivid.reflectivity.ranges![0].rgba;
    // q 2 = 5 dBZ, the floor: half the first band's alpha; q 1 stays transparent.
    expect(lut[2 * 4 + 3]).toBe(Math.round(first[3] * 0.5));
    expect(lut[1 * 4 + 3]).toBe(0);
    // Rain's last code and snow's q 0 are transparent, so a whole-q texel never borrows a neighbour class.
    expect(lut[31 * 4 + 3]).toBe(0);
    expect(lut[32 * 4 + 3]).toBe(0);
  });
});

describe("legend ramps follow the mode", () => {
  it("blends evenly spread colours when smooth", () => {
    expect(legendGradient(["#000000", "#808080", "#ffffff"], false)).toEqual({ colors: ["#000000", "#808080", "#ffffff"] });
  });

  it("draws flat blocks with hard edges when stepped", () => {
    expect(legendGradient(["#000000", "#808080", "#ffffff"], true)).toEqual({
      colors: ["#000000", "#000000", "#808080", "#808080", "#ffffff", "#ffffff"],
      locations: [0, 1 / 3, 1 / 3, 2 / 3, 2 / 3, 1],
    });
  });

  it("is smooth only where the GPU radar draws smooth ramps", () => {
    const on = { steppedBands: false, fastRadar: true, replaying: false, packed: true };
    expect(legendIsSmooth(on)).toBe(true);
    expect(legendIsSmooth({ ...on, steppedBands: true })).toBe(false);
    expect(legendIsSmooth({ ...on, fastRadar: false })).toBe(false);
    expect(legendIsSmooth({ ...on, replaying: true })).toBe(false);
    expect(legendIsSmooth({ ...on, packed: false })).toBe(false);
  });
});
