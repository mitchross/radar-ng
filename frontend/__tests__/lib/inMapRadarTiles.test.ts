import {
  commitGen,
  diskBytes,
  expandCover,
  frameDir,
  frameKeyOf,
  frameStatus,
  genDir,
  IN_MAP_DIR,
  inMapRootUri,
  lutHash,
  planGen,
  priorityOrder,
  selectCapEvictions,
  selectCleanup,
  slotSpec,
  tileKey,
  tilePath,
  tileUrlTemplate,
  type FrameDisk,
} from "../../src/lib/inMapRadarTiles";
import type { TileId } from "../../src/lib/packed";

const grid = (z: number, x0: number, x1: number, y0: number, y1: number): TileId[] => {
  const out: TileId[] = [];
  for (let y = y0; y <= y1; y++) for (let x = x0; x <= x1; x++) out.push({ z, x, y });
  return out;
};

const diskOf = (gen: number, tiles: TileId[], bytes = 100): FrameDisk => ({
  gen,
  tiles: new Map(tiles.map((t) => [tileKey(t), bytes])),
});

describe("paths and URL templates", () => {
  it("lays tiles out as <layer>/<frameKey>/<gen>/{z}/{x}/{y}.png under radar-inmap", () => {
    const key = frameKeyOf({ bucket: "2026-05-01T12", v: 3, slot: 7 }, "abcd1234");
    expect(key).toBe("2026-05-01T12.v3.s7.abcd1234");
    expect(frameDir("radar", key)).toBe(`radar/${key}`);
    expect(genDir("radar", key, 2)).toBe(`radar/${key}/2`);
    expect(tilePath("radar", key, 2, { z: 6, x: 14, y: 23 })).toBe(`radar/${key}/2/6/14/23.png`);
  });

  it("builds a file:// template from the cache directory URI", () => {
    const root = inMapRootUri("file:///var/mobile/Containers/Data/Application/ABC/Library/Caches/");
    expect(root).toBe(`file:///var/mobile/Containers/Data/Application/ABC/Library/Caches/${IN_MAP_DIR}/`);
    expect(inMapRootUri("file:///cache")).toBe(`file:///cache/${IN_MAP_DIR}/`);
    expect(tileUrlTemplate(root, "radar", "k", 4)).toBe(`${root}radar/k/4/{z}/{x}/{y}.png`);
  });

  it("keeps odd bucket and layer ids to one safe path segment", () => {
    const key = frameKeyOf({ bucket: "a/b:c d", v: 1, slot: 0 }, "00000000");
    expect(key).not.toMatch(/[/: ]/);
    expect(frameDir("radar/hrrr", key).split("/")).toHaveLength(2);
  });

  it("changes the frame key when the palette (LUT bytes) changes", () => {
    const a = new Uint8Array(1024).fill(7);
    const b = new Uint8Array(1024).fill(7);
    b[400] = 8;
    expect(lutHash(a)).toBe(lutHash(new Uint8Array(a)));
    expect(lutHash(a)).not.toBe(lutHash(b));
    expect(lutHash(a)).toHaveLength(8);
  });
});

describe("expandCover", () => {
  it("keeps the cover first and adds a one-tile ring clamped to the world", () => {
    const cover = grid(5, 10, 11, 4, 5);
    const out = expandCover(cover);
    expect(out.slice(0, cover.length)).toEqual(cover);
    expect(out).toHaveLength(16);
    expect(new Set(out.map(tileKey)).size).toBe(16);
    const edge = expandCover(grid(2, 0, 0, 0, 0));
    expect(edge.map(tileKey).sort()).toEqual(["2/0/0", "2/0/1", "2/1/0", "2/1/1"]);
  });

  it("is empty for an empty cover", () => {
    expect(expandCover([])).toEqual([]);
  });
});

describe("planGen", () => {
  const cover = grid(6, 20, 21, 30, 31);

  it("plans gen 1 with every cover and ring tile for a frame with nothing on disk", () => {
    const plan = planGen(undefined, cover)!;
    expect(plan.gen).toBe(1);
    expect(plan.reuse).toEqual([]);
    expect(plan.render).toEqual(plan.want);
    expect(plan.want).toHaveLength(16);
  });

  it("needs no new gen while the cover and its ring are on disk", () => {
    const first = planGen(undefined, cover)!;
    const disk = commitGen(undefined, first, new Map(first.render.map((t) => [tileKey(t), 50])));
    expect(planGen(disk, cover)).toBeNull();
    // A smaller cover inside the same area is covered too.
    expect(planGen(disk, grid(6, 20, 21, 30, 30))).toBeNull();
  });

  it("bumps the gen when a pan makes the ring reach past what is on disk, reusing the rest", () => {
    const first = planGen(undefined, cover)!;
    const disk = commitGen(undefined, first, new Map(first.render.map((t) => [tileKey(t), 50])));
    const panned = grid(6, 21, 22, 30, 31);
    const plan = planGen(disk, panned)!;
    expect(plan.gen).toBe(2);
    expect(plan.want).toHaveLength(16);
    // Columns 20..22 of the new ring (19..23) were on disk only for x <= 22.
    expect(plan.reuse.length).toBeGreaterThan(0);
    expect(plan.render.every((t) => !disk.tiles.has(tileKey(t)))).toBe(true);
    expect(plan.reuse.length + plan.render.length).toBe(plan.want.length);
    expect(plan.render.map((t) => t.x).every((x) => x >= 22)).toBe(true);
  });

  it("reuses nothing across a zoom change", () => {
    const first = planGen(undefined, cover)!;
    const disk = commitGen(undefined, first, new Map(first.render.map((t) => [tileKey(t), 50])));
    const plan = planGen(disk, grid(7, 40, 41, 60, 61))!;
    expect(plan.gen).toBe(2);
    expect(plan.reuse).toEqual([]);
    expect(plan.render).toEqual(plan.want);
  });

  it("returns null with no cover", () => {
    expect(planGen(undefined, [])).toBeNull();
  });
});

describe("commitGen", () => {
  it("holds the new gen's tiles with sizes from the copy or the write, and drops the rest", () => {
    const old = diskOf(1, grid(6, 0, 3, 0, 0), 10);
    const plan = planGen(old, grid(6, 2, 3, 0, 0))!;
    const written = new Map(plan.render.map((t) => [tileKey(t), 99]));
    const next = commitGen(old, plan, written);
    expect(next.gen).toBe(2);
    expect(next.tiles.get("6/2/0")).toBe(10);
    expect([...next.tiles.keys()].sort()).toEqual(plan.want.map(tileKey).sort());
    expect(diskBytes(next)).toBeGreaterThan(0);
    expect(diskBytes(undefined)).toBe(0);
  });
});

describe("frame readiness", () => {
  const cover = grid(6, 20, 21, 30, 31);

  it("is pending with nothing on disk, ready with the cover, stale after the viewport moves", () => {
    expect(frameStatus(undefined, cover)).toBe("pending");
    const plan = planGen(undefined, cover)!;
    const disk = commitGen(undefined, plan, new Map(plan.render.map((t) => [tileKey(t), 1])));
    expect(frameStatus(disk, cover)).toBe("ready");
    expect(frameStatus(disk, grid(6, 30, 31, 30, 31))).toBe("stale");
  });
});

describe("priorityOrder", () => {
  it("colors the current frame first, then outward, newer first on a tie", () => {
    expect(priorityOrder([0, 1, 2, 3, 4, 5], 3)).toEqual([3, 4, 2, 5, 1, 0]);
    expect(priorityOrder([0, 1, 2], 2)).toEqual([2, 1, 0]);
  });

  it("starts from the nearest frame when the current one is not in the list", () => {
    expect(priorityOrder([4, 5], 9)).toEqual([5, 4]);
  });
});

describe("slotSpec", () => {
  const root = "file:///cache/radar-inmap/";
  const frame = { layer: "radar", frameKey: "k", minZoom: 4, maxZoom: 7 };

  it("gives a frame with nothing on disk an inert source: hidden layer, zoom 22, no real path", () => {
    const spec = slotSpec(3, frame, undefined, root);
    expect(spec.visible).toBe(false);
    expect(spec.minZoom).toBe(22);
    expect(spec.maxZoom).toBe(22);
    expect(spec.tileUrl).not.toContain("/radar/k/");
    expect(spec.sourceId).toContain("pending");
  });

  it("points a ready frame at its gen with the packed zoom range, and changes ids with the gen", () => {
    const g1 = slotSpec(3, frame, diskOf(1, grid(6, 0, 0, 0, 0)), root);
    const g2 = slotSpec(3, frame, diskOf(2, grid(6, 0, 0, 0, 0)), root);
    expect(g1.visible).toBe(true);
    expect(g1.tileUrl).toBe(`${root}radar/k/1/{z}/{x}/{y}.png`);
    expect(g1.minZoom).toBe(4);
    expect(g1.maxZoom).toBe(7);
    expect(g2.tileUrl).toBe(`${root}radar/k/2/{z}/{x}/{y}.png`);
    expect(g2.sourceId).not.toBe(g1.sourceId);
    expect(g2.layerId).not.toBe(g1.layerId);
  });

  it("never reuses an id between a pending and a ready frame", () => {
    expect(slotSpec(0, frame, undefined, root).sourceId).not.toBe(slotSpec(0, frame, diskOf(1, []), root).sourceId);
  });
});

describe("selectCleanup", () => {
  it("deletes frames no longer wanted and gens a source no longer reads", () => {
    const listing = [
      { layer: "radar", frameKey: "old", gens: [1] },
      { layer: "radar", frameKey: "cur", gens: [1, 2, 3] },
      { layer: "radar-hrrr", frameKey: "h", gens: [4] },
    ];
    const keep = new Map<string, ReadonlySet<number>>([
      ["radar/cur", new Set([3])],
      ["radar-hrrr/h", new Set([4])],
    ]);
    expect(selectCleanup(listing, keep).sort()).toEqual(["radar/cur/1", "radar/cur/2", "radar/old"]);
  });

  it("keeps a gen that is being written", () => {
    const listing = [{ layer: "radar", frameKey: "cur", gens: [2, 3] }];
    expect(selectCleanup(listing, new Map([["radar/cur", new Set([2, 3])]]))).toEqual([]);
  });

  it("deletes everything when nothing is wanted", () => {
    expect(selectCleanup([{ layer: "radar", frameKey: "a", gens: [1] }], new Map())).toEqual(["radar/a"]);
  });
});

describe("selectCapEvictions", () => {
  const entries = [
    { key: "cur", bytes: 40, rank: 0 },
    { key: "near", bytes: 40, rank: 1 },
    { key: "far", bytes: 40, rank: 5 },
    { key: "farthest", bytes: 40, rank: 9 },
  ];

  it("evicts nothing under the cap", () => {
    expect(selectCapEvictions(entries, 160)).toEqual([]);
  });

  it("evicts the frames furthest from the current one first until the total fits", () => {
    expect(selectCapEvictions(entries, 100)).toEqual(["farthest", "far"]);
    expect(selectCapEvictions(entries, 120)).toEqual(["farthest"]);
  });

  it("never evicts protected frames, even over the cap", () => {
    expect(selectCapEvictions(entries, 0, new Set(["cur", "near", "far"]))).toEqual(["farthest"]);
  });
});
