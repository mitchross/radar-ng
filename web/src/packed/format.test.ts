import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";
import type { PaletteId } from "../palettes";
import { colorTable, fallbackTable } from "./colorTables";
import {
  atlasSize,
  buildLut,
  locateFrame,
  packedTileUrl,
  quantize,
  slotAddress,
  tileCover,
  tileMatrix,
  usablePacked,
  type ColorTable,
  type PackedMeta,
} from "./format";

const palette = (id: PaletteId): Record<string, ColorTable> =>
  JSON.parse(readFileSync(new URL(`../../../rust/crates/radar-tiler/assets/${id}.json`, import.meta.url), "utf8"));

const rgba = (lut: Uint8Array, q: number) => Array.from(lut.slice(q * 4, q * 4 + 4));

const meta = (over: Partial<PackedMeta> = {}): PackedMeta => ({
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
    { id: "2026-10-15T18:00:00Z", v: 3, slots: ["2026-10-15T18:00:38+00:00", "2026-10-15T18:02:38+00:00"] },
    { id: "2026-10-15T18:30:00Z", v: 7, slots: ["2026-10-15T18:30:38+00:00", "2026-10-15T18:32:38+00:00", "2026-10-15T18:34:38+00:00"] },
  ],
  ...over,
});

describe("buildLut", () => {
  const classic = palette("classic").reflectivity;
  const lut = buildLut(classic, 0.5, -32);

  it("colors q through the classic reflectivity bands", () => {
    expect(quantize(40, 0.5, -32)).toBe(144);
    expect(rgba(lut, 144)).toEqual([255, 170, 0, 230]); // 40 dBZ: the 40-45 band
    expect(rgba(lut, 143)).toEqual([255, 240, 0, 225]); // 39.5 dBZ: 35-40
    expect(rgba(lut, 74)).toEqual([110, 185, 225, 115]); // 5 dBZ: first band
    expect(rgba(lut, 213)).toEqual([180, 0, 220, 250]); // 74.5 dBZ: top band
  });

  it("leaves q=0, values below no_data_below and values past every range transparent", () => {
    expect(rgba(lut, 0)).toEqual([0, 0, 0, 0]);
    expect(rgba(lut, 73)).toEqual([0, 0, 0, 0]); // 4.5 dBZ < no_data_below 5
    expect(rgba(lut, 214)).toEqual([0, 0, 0, 0]); // 75 dBZ: ranges are half-open
    expect(rgba(lut, 255)).toEqual([0, 0, 0, 0]);
  });

  it("lets a later overlapping range win and leaves gaps transparent", () => {
    const table: ColorTable = {
      ranges: [
        { min: 0, max: 10, rgba: [1, 1, 1, 1] },
        { min: 5, max: 8, rgba: [2, 2, 2, 2] },
        { min: 12, max: 20, rgba: [3, 3, 3, 3] },
      ],
    };
    const l = buildLut(table, 1, 0);
    expect(rgba(l, 4)).toEqual([1, 1, 1, 1]);
    expect(rgba(l, 5)).toEqual([2, 2, 2, 2]);
    expect(rgba(l, 8)).toEqual([1, 1, 1, 1]);
    expect(rgba(l, 10)).toEqual([0, 0, 0, 0]);
    expect(rgba(l, 12)).toEqual([3, 3, 3, 3]);
  });

  it("defaults no_data_below to -999 and compares band edges without float noise", () => {
    const temps = buildLut({ ranges: [{ min: -100, max: 0, rgba: [9, 9, 9, 9] }] }, 1, -80);
    expect(rgba(temps, 1)).toEqual([9, 9, 9, 9]); // -79 °F
    const accum = buildLut({ ranges: [{ min: 0.5, max: 1, rgba: [1, 1, 1, 1] }, { min: 1, max: 2, rgba: [2, 2, 2, 2] }] }, 0.05, 0);
    expect(rgba(accum, 20)).toEqual([2, 2, 2, 2]); // 0.05 * 20 is 1.0000000000000002 in floats
  });

  it("gives an empty table without a color table", () => {
    expect(buildLut(undefined, 0.5, -32).every((v) => v === 0)).toBe(true);
  });

  it("builds the same table from the palettes.ts fallback as from the renderer JSON", () => {
    for (const id of ["classic", "muted", "vivid"] as PaletteId[]) {
      expect(buildLut(fallbackTable(id, "reflectivity"), 0.5, -32)).toEqual(buildLut(palette(id).reflectivity, 0.5, -32));
    }
    expect(fallbackTable("classic", "temperature")).toBeUndefined();
    expect(colorTable({ classic: { temperature: palette("classic").temperature } }, "classic", "temperature")).toBeDefined();
  });
});

describe("quantize", () => {
  it("matches the spec's truncation, floor and clamping", () => {
    expect(quantize(Number.NaN, 2.5, 0, 5)).toBe(0);
    expect(quantize(4.9, 2.5, 0, 5)).toBe(0);
    expect(quantize(44.9, 2.5, 0, 5)).toBe(17);
    expect(quantize(5000, 2.5, 0, 5)).toBe(255);
  });

  it("decodes production reflectivity codes into the server's color band", () => {
    const lut = buildLut(palette("classic").reflectivity, 2.5, 0);
    // 40.0 and 44.9 dBZ (codes 16, 17) are the 40-45 band; 45.0 (code 18) is the next one.
    expect([...lut.slice(16 * 4, 16 * 4 + 4)]).toEqual([255, 170, 0, 230]);
    expect([...lut.slice(17 * 4, 17 * 4 + 4)]).toEqual([255, 170, 0, 230]);
    expect([...lut.slice(18 * 4, 18 * 4 + 4)]).not.toEqual([255, 170, 0, 230]);
    expect([...lut.slice(1 * 4, 1 * 4 + 4)]).toEqual([0, 0, 0, 0]);
  });
});

describe("slotAddress", () => {
  it("lays blocks of three slots out row-major, two blocks wide", () => {
    expect(slotAddress(4)).toEqual({ block: 1, channel: 1, x0: 512, y0: 0 });
    expect(slotAddress(0)).toEqual({ block: 0, channel: 0, x0: 0, y0: 0 });
    expect(slotAddress(6)).toEqual({ block: 2, channel: 0, x0: 0, y0: 512 });
    expect(slotAddress(23)).toEqual({ block: 7, channel: 2, x0: 512, y0: 1536 });
  });

  it("sizes the atlas from the slot count", () => {
    expect(atlasSize(24)).toEqual({ width: 1024, height: 2048 });
    expect(atlasSize(1)).toEqual({ width: 1024, height: 512 });
    expect(atlasSize(7)).toEqual({ width: 1024, height: 1024 });
  });
});

describe("locateFrame", () => {
  it("finds the bucket, version and slot by timestamp", () => {
    expect(locateFrame(meta(), "2026-10-15T18:32:38+00:00")).toEqual({ bucket: "2026-10-15T18:30:00Z", v: 7, slot: 1, slots: 3 });
    expect(locateFrame(meta(), "2026-10-15T18:00:38+00:00")).toEqual({ bucket: "2026-10-15T18:00:00Z", v: 3, slot: 0, slots: 2 });
  });

  it("falls back to the instant when the string differs", () => {
    expect(locateFrame(meta(), "2026-10-15T18:34:38Z")?.slot).toBe(2);
  });

  it("returns null for frames without a slot", () => {
    expect(locateFrame(meta(), "2026-10-15T19:00:00+00:00")).toBeNull();
    expect(locateFrame(undefined, "2026-10-15T18:32:38+00:00")).toBeNull();
  });

  it("accepts only v1 continuous metadata", () => {
    expect(usablePacked(meta())).toBe(true);
    expect(usablePacked(meta({ categorical: true }))).toBe(false);
    expect(usablePacked(meta({ version: 2 }))).toBe(false);
    expect(usablePacked(undefined)).toBe(false);
  });
});

describe("tileCover", () => {
  const michigan = { west: -88, south: 41, east: -82, north: 46 };

  it("covers the view at clamp(floor(zoom), min, max)", () => {
    const tiles = tileCover(michigan, 6.5, 3, 6);
    expect(new Set(tiles.map((t) => t.z))).toEqual(new Set([6]));
    expect(tiles.map((t) => `${t.x}/${t.y}`).sort()).toEqual(["16/22", "16/23", "17/22", "17/23"]);
    expect(tileCover(michigan, 9.2, 3, 6)[0].z).toBe(6);
    expect(tileCover(michigan, 1.5, 3, 6)).toEqual([{ z: 3, x: 2, y: 2 }]);
  });

  it("clamps longitudes past the antimeridian into the world", () => {
    const tiles = tileCover({ west: -200, south: 50, east: -170, north: 60 }, 3, 3, 6);
    expect(Math.min(...tiles.map((t) => t.x))).toBe(0);
    expect(tileCover({ west: 190, south: 0, east: 200, north: 10 }, 3, 3, 6)).toEqual([]);
  });

  it("steps down a zoom when the cover is too large", () => {
    const conus = { west: -125, south: 24, east: -66, north: 50 };
    expect(tileCover(conus, 6, 3, 6).length).toBeGreaterThan(36);
    const capped = tileCover(conus, 6, 3, 6, 36);
    expect(capped.length).toBeLessThanOrEqual(36);
    expect(capped[0].z).toBeLessThan(6);
  });
});

describe("packedTileUrl", () => {
  it("fills the template and encodes the bucket id", () => {
    expect(packedTileUrl(meta().url_template, "radar", "2026-10-15T18:30:00Z", { z: 5, x: 8, y: 11 }, 7)).toBe(
      "/tiles/packed/radar/2026-10-15T18%3A30%3A00Z/5/8/11.png?v=7",
    );
    expect(packedTileUrl(meta().url_template, "nowcast", "20261015T1830.p1", { z: 3, x: 2, y: 2 }, 1)).toBe("/tiles/packed/nowcast/20261015T1830.p1/3/2/2.png?v=1");
  });
});

describe("tileMatrix", () => {
  it("maps tile-local 0..1 into mercator 0..1 through the main matrix", () => {
    const identity = [1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1];
    const m = tileMatrix(identity, { z: 1, x: 1, y: 0 });
    const apply = (x: number, y: number) => [m[0] * x + m[4] * y + m[12], m[1] * x + m[5] * y + m[13]];
    expect(apply(0, 0)).toEqual([0.5, 0]);
    expect(apply(1, 1)).toEqual([1, 0.5]);
  });
});
