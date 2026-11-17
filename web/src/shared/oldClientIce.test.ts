/**
 * An app or web build from master (three classes, before ice) meeting a server with four. The
 * decoder below is a frozen copy of master's packed.ts, so this fails if the format change breaks
 * old clients.
 */
import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";
import type { ColorTable } from "./serverPalettes";
import { buildLut as masterBuildLut, parsePackedMeta as masterParse } from "./testdata/packed.master-d96e36f";

const reflectivity = (id: string): ColorTable =>
  JSON.parse(readFileSync(new URL(`../../../rust/crates/radar-tiler/assets/${id}.json`, import.meta.url), "utf8")).reflectivity;

const rgba = (lut: Uint8Array, code: number) => Array.from(lut.slice(code * 4, code * 4 + 4));

/** The packed metadata a four-class server sends (radar-ingest packed.rs, `TYPE_CLASSES`). */
const serverMeta = {
  version: 2,
  tile_size: 512,
  min_zoom: 3,
  max_zoom: 6,
  cols: 2,
  max_slots: 24,
  color_key: "reflectivity",
  scale: 2.5,
  offset: 0,
  url_template: "/tiles/packed/{layer}/{bucket}/{z}/{x}/{y}.png?v={v}",
  buckets: [{ id: "2026-11-17T18:00:00Z", v: 1, slots: ["2026-11-17T18:00:38+00:00"] }],
  types: { stride: 32, classes: ["rain", "snow", "mix", "ice"] },
};

/** Ice at 30-35 dBZ: class 3, q 12. */
const ICE_30 = 3 * 32 + 12;

describe("three-class client on a four-class server", () => {
  const meta = masterParse(serverMeta)!;

  it("accepts the metadata and keeps the fourth class", () => {
    expect(meta).not.toBeNull();
    expect(meta.types?.classes).toEqual(["rain", "snow", "mix", "ice"]);
  });

  it("draws ice cells with the server's ice ramp", () => {
    for (const id of ["classic", "muted", "vivid"]) {
      const table = reflectivity(id);
      const lut = masterBuildLut(table, meta.scale, meta.offset, meta.types);
      expect(rgba(lut, ICE_30)).toEqual(table.types!.ice.ranges[5].rgba);
      expect(rgba(lut, 3 * 32 + 29)).toEqual(table.types!.ice.ranges[6].rgba); // 72.5 dBZ, top band
      expect(rgba(lut, ICE_30)[3]).toBeGreaterThan(0);
    }
  });

  it("still draws ice (in rain colours) on its offline fallback table, which has no ice ramp", () => {
    const table = reflectivity("classic");
    const { ice: _ice, ...threeRamps } = table.types!;
    const offline: ColorTable = { ...table, types: threeRamps };
    const lut = masterBuildLut(offline, meta.scale, meta.offset, meta.types);
    expect(rgba(lut, ICE_30)).toEqual(table.ranges[5].rgba);
  });
});
