import { readFileSync } from "fs";
import { join } from "path";
import {
  bucketsInPlaybackOrder,
  buildLut,
  buildPackedFrames,
  locateFrame,
  packedTileUrl,
  packedTileZoom,
  parsePackedMeta,
  playbackOrder,
  rasterCarouselView,
  slotAddress,
  tileCover,
  tileScreenRect,
  worldPixel,
  type PackedColorTable,
} from "../../src/lib/packed";

const classic = JSON.parse(
  readFileSync(join(__dirname, "../../../rust/crates/radar-tiler/assets/classic.json"), "utf8"),
) as Record<string, PackedColorTable>;

const rgbaAt = (lut: Uint8Array, q: number) => Array.from(lut.slice(q * 4, q * 4 + 4));

/** Inverse of worldPixel, for placing the camera on a known world pixel. */
function lngLat(x: number, y: number, zoom: number) {
  const world = 512 * Math.pow(2, zoom);
  return {
    lon: (x / world) * 360 - 180,
    lat: (Math.atan(Math.sinh(Math.PI * (1 - (2 * y) / world))) * 180) / Math.PI,
  };
}

const META = {
  version: 1,
  tile_size: 512,
  min_zoom: 3,
  max_zoom: 6,
  cols: 2,
  max_slots: 24,
  color_key: "reflectivity",
  categorical: false,
  scale: 0.5,
  offset: -32,
  url_template: "/tiles/packed/{layer}/{bucket}/{z}/{x}/{y}.png?v={v}",
  buckets: [
    { id: "2026-10-15T18:00:00Z", v: 15, slots: ["2026-10-15T18:00:38+00:00", "2026-10-15T18:02:38+00:00"] },
    { id: "2026-10-15T18:30:00Z", v: 7, slots: ["2026-10-15T18:30:38+00:00", "2026-10-15T18:32:38+00:00"] },
  ],
};

describe("buildLut", () => {
  const lut = buildLut(classic.reflectivity, 0.5, -32);

  it("is 256 RGBA entries", () => {
    expect(lut).toBeInstanceOf(Uint8Array);
    expect(lut.length).toBe(1024);
  });

  it("colors q=144 (40 dBZ) with the classic 40-45 band", () => {
    expect(rgbaAt(lut, 144)).toEqual([255, 170, 0, 230]);
  });

  it("decodes production reflectivity codes (2.5 dBZ, offset 0) into the server's band", () => {
    const prod = buildLut(classic.reflectivity, 2.5, 0);
    expect(rgbaAt(prod, 16)).toEqual([255, 170, 0, 230]);
    expect(rgbaAt(prod, 17)).toEqual([255, 170, 0, 230]);
    expect(rgbaAt(prod, 18)).not.toEqual([255, 170, 0, 230]);
    expect(rgbaAt(prod, 1)).toEqual([0, 0, 0, 0]);
  });

  it("keeps q=0 transparent", () => {
    expect(rgbaAt(lut, 0)).toEqual([0, 0, 0, 0]);
  });

  it("applies no_data_below and half-open ranges", () => {
    expect(rgbaAt(lut, 73)).toEqual([0, 0, 0, 0]); // 4.5 dBZ < no_data_below 5
    expect(rgbaAt(lut, 74)).toEqual([110, 185, 225, 115]); // 5 dBZ opens [5, 10)
    expect(rgbaAt(lut, 83)).toEqual([110, 185, 225, 115]); // 9.5
    expect(rgbaAt(lut, 84)).toEqual([90, 200, 195, 155]); // 10 belongs to [10, 15)
  });

  it("leaves values in no range transparent", () => {
    expect(rgbaAt(lut, 213)).toEqual([180, 0, 220, 250]); // 74.5 in [65, 75)
    expect(rgbaAt(lut, 214)).toEqual([0, 0, 0, 0]); // 75 is past the last range
  });

  it("lets a later overlapping range win and leaves gaps transparent", () => {
    const table: PackedColorTable = {
      ranges: [
        { min: 0, max: 10, rgba: [1, 2, 3, 4] },
        { min: 5, max: 8, rgba: [9, 9, 9, 9] },
        { min: 20, max: 30, rgba: [7, 7, 7, 7] },
      ],
    };
    const t = buildLut(table, 1, 0);
    expect(rgbaAt(t, 4)).toEqual([1, 2, 3, 4]);
    expect(rgbaAt(t, 6)).toEqual([9, 9, 9, 9]);
    expect(rgbaAt(t, 8)).toEqual([1, 2, 3, 4]);
    expect(rgbaAt(t, 15)).toEqual([0, 0, 0, 0]);
    expect(rgbaAt(t, 30)).toEqual([0, 0, 0, 0]);
  });

  it("defaults no_data_below to -999", () => {
    const table: PackedColorTable = { ranges: [{ min: -2000, max: 2000, rgba: [5, 5, 5, 5] }] };
    expect(rgbaAt(buildLut(table, 1, -1001), 1)).toEqual([0, 0, 0, 0]); // -1000
    expect(rgbaAt(buildLut(table, 1, -1000), 1)).toEqual([5, 5, 5, 5]); // -999
  });

  it("is all transparent without a table", () => {
    expect(buildLut(undefined, 0.5, -32).every((b) => b === 0)).toBe(true);
  });
});

describe("slotAddress", () => {
  it("maps slot 4 to block 1, channel G, the second column", () => {
    expect(slotAddress(4, 2, 512)).toEqual({ block: 1, channel: 1, x0: 512, y0: 0 });
  });

  it("lays blocks out row-major, 2 wide", () => {
    expect(slotAddress(0)).toEqual({ block: 0, channel: 0, x0: 0, y0: 0 });
    expect(slotAddress(6)).toEqual({ block: 2, channel: 0, x0: 0, y0: 512 });
    expect(slotAddress(23)).toEqual({ block: 7, channel: 2, x0: 512, y0: 1536 });
  });
});

describe("locateFrame", () => {
  it("finds a frame's bucket, version and slot by timestamp", () => {
    expect(locateFrame(META, "2026-10-15T18:32:38+00:00")).toEqual({ bucket: "2026-10-15T18:30:00Z", v: 7, slot: 1 });
  });

  it("matches the same instant written with Z", () => {
    expect(locateFrame(META, "2026-10-15T18:00:38Z")).toEqual({ bucket: "2026-10-15T18:00:00Z", v: 15, slot: 0 });
  });

  it("returns null for frames without a slot", () => {
    expect(locateFrame(META, "2026-10-15T20:00:00+00:00")).toBeNull();
    expect(locateFrame(null, "2026-10-15T18:00:38+00:00")).toBeNull();
  });
});

describe("parsePackedMeta", () => {
  it("accepts the spec example", () => {
    expect(parsePackedMeta(META)?.buckets).toHaveLength(2);
  });

  it("rejects what this client cannot draw", () => {
    expect(parsePackedMeta(undefined)).toBeNull();
    expect(parsePackedMeta({ ...META, version: 2 })).toBeNull();
    expect(parsePackedMeta({ ...META, tile_size: 256 })).toBeNull();
    expect(parsePackedMeta({ ...META, categorical: true })).toBeNull();
    expect(parsePackedMeta({ ...META, scale: 0 })).toBeNull();
    expect(parsePackedMeta({ ...META, buckets: [{ id: "x", v: 1, slots: new Array(25).fill("t") }] })).toBeNull();
  });

  it("accepts typed codes only as v2 with a valid types object", () => {
    const types = { stride: 32, classes: ["rain", "snow", "mix"] };
    expect(parsePackedMeta({ ...META, version: 2, types })?.types).toEqual(types);
    expect(parsePackedMeta({ ...META, version: 2 })).toBeNull();
    expect(parsePackedMeta({ ...META, version: 1, types })).toBeNull();
    expect(parsePackedMeta({ ...META, version: 2, types: { stride: 0, classes: ["rain"] } })).toBeNull();
    expect(parsePackedMeta({ ...META, version: 2, types: { stride: 32, classes: [1] } })).toBeNull();
    expect(parsePackedMeta(META)?.types).toBeUndefined();
  });
});

describe("buildLut with typed codes", () => {
  const table = classic.reflectivity;
  const types = { stride: 32, classes: ["rain", "snow", "mix"] };
  const lut = buildLut(table, 2.5, 0, types);

  it("colors class * 32 + q with the class ramp", () => {
    expect(rgbaAt(lut, 12)).toEqual([0, 140, 0, 220]); // rain 30 dBZ
    expect(rgbaAt(lut, 32 + 12)).toEqual(table.types!.snow!.ranges![5].rgba);
    expect(rgbaAt(lut, 64 + 12)).toEqual(table.types!.mix!.ranges![5].rgba);
    expect(rgbaAt(lut, 64)).toEqual([0, 0, 0, 0]);
    expect(rgbaAt(lut, 96 + 1)).toEqual([0, 0, 0, 0]);
  });

  it("keeps rain codes identical to the untyped table", () => {
    const plain = buildLut(table, 2.5, 0);
    for (let q = 1; q < 32; q++) expect(rgbaAt(lut, q)).toEqual(rgbaAt(plain, q));
  });
});

describe("tileCover", () => {
  const screen = { width: 500, height: 500 };

  it("covers one tile when the screen sits inside it at an integer zoom", () => {
    const c = lngLat(4.5 * 512, 5.5 * 512, 4);
    expect(tileCover({ ...c, zoom: 4 }, screen, 3, 6)).toEqual([{ z: 4, x: 4, y: 5 }]);
  });

  it("uses floor(zoom) and lists the center tile first", () => {
    const c = lngLat(4.5 * 512 * 2, 5.5 * 512 * 2, 5); // center of z4 tile (4,5) at map zoom 5
    const tiles = tileCover({ ...c, zoom: 5.4 }, screen, 3, 6);
    expect(tiles.every((t) => t.z === 5)).toBe(true);
    expect(tiles).toHaveLength(4);
    expect(tiles).toEqual(expect.arrayContaining([{ z: 5, x: 8, y: 10 }, { z: 5, x: 9, y: 11 }]));
  });

  it("overzooms past max_zoom and clamps below min_zoom", () => {
    const c = lngLat(4.5 * 512, 5.5 * 512, 4);
    expect(packedTileZoom(8.7, 3, 6)).toBe(6);
    expect(packedTileZoom(1.2, 3, 6)).toBe(3);
    expect(tileCover({ ...c, zoom: 8.7 }, screen, 3, 6).every((t) => t.z === 6)).toBe(true);
    const low = tileCover({ ...c, zoom: 1.2 }, screen, 3, 6);
    expect(low.every((t) => t.z === 3 && t.x >= 0 && t.x < 8 && t.y >= 0 && t.y < 8)).toBe(true);
  });

  it("covers the rotated screen's bounding box", () => {
    const c = lngLat(16.5 * 512, 16.5 * 512, 5);
    const wide = { width: 1000, height: 100 };
    const north = tileCover({ ...c, zoom: 5, bearing: 0 }, wide, 3, 6);
    const east = tileCover({ ...c, zoom: 5, bearing: 90 }, wide, 3, 6);
    expect(new Set(north.map((t) => t.x)).size).toBeGreaterThan(new Set(north.map((t) => t.y)).size);
    expect(new Set(east.map((t) => t.y)).size).toBeGreaterThan(new Set(east.map((t) => t.x)).size);
  });

  it("is empty before the canvas has a size", () => {
    expect(tileCover({ lon: 0, lat: 0, zoom: 4 }, { width: 0, height: 0 }, 3, 6)).toEqual([]);
  });
});

describe("tileScreenRect", () => {
  it("puts the camera's tile under the screen center with 512-pt tiles at its zoom", () => {
    const c = lngLat(4.5 * 512, 5.5 * 512, 4);
    const r = tileScreenRect({ z: 4, x: 4, y: 5 }, { ...c, zoom: 4 }, { width: 400, height: 800 });
    expect(r.size).toBeCloseTo(512);
    expect(r.x).toBeCloseTo(200 - 256, 6);
    expect(r.y).toBeCloseTo(400 - 256, 6);
  });

  it("scales a parent tile by 2^(zoom - z)", () => {
    const c = lngLat(4.5 * 512, 5.5 * 512, 4);
    expect(tileScreenRect({ z: 3, x: 2, y: 2 }, { ...c, zoom: 4 }, { width: 400, height: 800 }).size).toBeCloseTo(1024);
  });

  it("agrees with worldPixel at the origin", () => {
    expect(worldPixel(-180, 0, 0)).toEqual({ x: 0, y: 256 });
  });
});

describe("packedTileUrl", () => {
  it("fills the manifest template onto the server origin", () => {
    expect(
      packedTileUrl("https://radar.example", META.url_template, "radar", "2026-10-15T18:30:00Z", 7, 5, 8, 11),
    ).toBe("https://radar.example/tiles/packed/radar/2026-10-15T18%3A30%3A00Z/5/8/11.png?v=7");
  });

  it("keeps an absolute template", () => {
    expect(packedTileUrl("https://a", "https://cdn/{layer}/{bucket}/{z}/{x}/{y}?v={v}", "nowcast", "r1.p0", 2, 3, 1, 2)).toBe(
      "https://cdn/nowcast/r1.p0/3/1/2?v=2",
    );
  });
});

describe("playback helpers", () => {
  const P = true;
  const R = false;

  it("playbackOrder prefers the sequence, else the window range, else every frame", () => {
    expect(playbackOrder({ start: 0, end: 4, sequence: [0, 2, 4] }, 5)).toEqual([0, 2, 4]);
    expect(playbackOrder({ start: 1, end: 3 }, 5)).toEqual([1, 2, 3]);
    expect(playbackOrder(null, 3)).toEqual([0, 1, 2]);
  });

  it("rasterCarouselView holds the next raster frame hidden while a packed frame shows", () => {
    expect(rasterCarouselView([P, P, P, R, R], 1, null)).toEqual({
      currentIndex: 3,
      window: { start: 3, end: 4, sequence: [3, 4] },
      visible: false,
    });
  });

  it("rasterCarouselView shows raster frames", () => {
    expect(rasterCarouselView([P, P, P, R, R], 4, null)?.visible).toBe(true);
    expect(rasterCarouselView([P, P, P, R, R], 4, null)?.currentIndex).toBe(4);
  });

  it("rasterCarouselView is null when every frame in the loop is packed", () => {
    expect(rasterCarouselView([P, P, P, R], 1, { start: 0, end: 2, sequence: [0, 1, 2] })).toBeNull();
  });

  it("rasterCarouselView still shows a raster frame scrubbed outside the loop", () => {
    const view = rasterCarouselView([P, P, P, R], 3, { start: 0, end: 2, sequence: [0, 1, 2] });
    expect(view).toEqual({ currentIndex: 3, window: { start: 3, end: 3, sequence: [3] }, visible: true });
  });

  it("bucketsInPlaybackOrder lists distinct buckets from the current frame, wrapping", () => {
    const a = { layer: "radar", bucket: "a", v: 1 };
    const b = { layer: "radar", bucket: "b", v: 1 };
    const n = { layer: "nowcast", bucket: "r", v: 3 };
    const locs = [a, a, b, b, n, null];
    expect(bucketsInPlaybackOrder(locs, [0, 1, 2, 3, 4, 5], 3)).toEqual([b, n, a]);
  });
});

describe("model layers (HRRR, AQM)", () => {
  const palette = (id: string) =>
    JSON.parse(readFileSync(join(__dirname, `../../../rust/crates/radar-tiler/assets/${id}.json`), "utf8")) as Record<
      string,
      Required<PackedColorTable>
    >;
  // The server's table (docs/packed-tiles.md): [scale, offset, floor] and the band edges it cannot hit.
  const TABLE: Record<string, [number, number, number, number[]]> = {
    temperature: [1, -80, -79, []],
    dewpoint: [5, -80, -75, []],
    wind_speed: [5, -5, 0, []],
    humidity: [10, -10, 0, [100.01]],
    cape: [500, 0, 500, []],
    precip_accum: [0.05, 0, 0.01, [20]],
    cloud_cover: [5, 0, 10, [101]],
    pm25: [1, 0.5, 0, [9.1, 20, 45, 90, 175, 500, 20000]],
    ozone: [1, 0, 0, [600]],
  };
  const quantize = (v: number, scale: number, offset: number, floor: number) =>
    Number.isNaN(v) || v < floor ? 0 : Math.max(1, Math.min(255, Math.floor((v - offset) / scale)));

  it.each(Object.keys(TABLE))("colors %s like the PNG tiles, except the listed edges", (key) => {
    const [scale, offset, floor, inexact] = TABLE[key];
    for (const id of ["classic", "muted", "vivid"]) {
      const table = palette(id)[key];
      const lut = buildLut(table, scale, offset);
      const band = (v: number) => {
        if (v < (table.no_data_below ?? -999)) return "0,0,0,0";
        let hit = [0, 0, 0, 0];
        for (const r of table.ranges) if (v >= r.min && v < r.max) hit = r.rgba;
        return hit.join();
      };
      const edges = table.ranges.flatMap((r) => [r.min, r.max]).sort((a, b) => a - b);
      const misses = new Set<number>();
      for (let v = edges[0] - 2 * scale; v <= edges[edges.length - 1] * 1.01; v += Math.min(scale, 1) / 7) {
        const raw = Math.fround(v);
        if (rgbaAt(lut, quantize(raw, scale, offset, floor)).join() !== band(raw)) {
          misses.add([...edges].reverse().find((e) => e <= raw) ?? -Infinity);
        }
      }
      expect([...misses].sort((a, b) => a - b)).toEqual(inexact);
    }
  });

  const temperatureMeta = {
    ...META,
    max_zoom: 5,
    color_key: "temperature",
    scale: 1,
    offset: -80,
    buckets: [
      { id: "20261015_12", v: 4, slots: ["2026-10-15T13:00:00+00:00", ""] },
      { id: "20261015_13", v: 2, slots: ["2026-10-15T15:00:00+00:00", "2026-10-15T14:00:00+00:00"] },
    ],
  };
  const frames = ["2026-10-15T13:00:00+00:00", "2026-10-15T14:00:00+00:00", "2026-10-15T16:00:00+00:00"].map((timestamp) => ({
    timestamp,
  }));

  it("locates a model layer's frames by the active layer and colors them with its key", () => {
    const packed = buildPackedFrames(frames, { temperature: { packed: temperatureMeta } }, classic, "temperature");
    expect(packed?.isPacked).toEqual([true, true, false]);
    expect(packed?.locations[1]).toMatchObject({ layer: "temperature", bucket: "20261015_13", slot: 1, lutKey: "temperature|1|-80" });
    // 60 F is q 140 with offset -80: the classic 55-65 band.
    const lut = packed!.luts["temperature|1|-80"];
    const band = classic.temperature.ranges!.find((r) => 60 >= r.min && 60 < r.max)!;
    expect(rgbaAt(lut, 140)).toEqual(band.rgba);
  });

  it("keeps rasters without a table or for categorical layers", () => {
    expect(buildPackedFrames(frames, { temperature: { packed: temperatureMeta } }, {}, "temperature")).toBeNull();
    const categorical = { ...temperatureMeta, color_key: "precip_type", categorical: true };
    expect(buildPackedFrames(frames, { "precip-type": { packed: categorical } }, classic, "precip-type")).toBeNull();
  });
});
