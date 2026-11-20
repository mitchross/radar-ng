import { describe, expect, it } from "vitest";
import { FRAGMENT } from "./layer";
import { COVERAGE_HI, COVERAGE_LO, smoothSample, smoothstep } from "./filter";

const STRIDE = 32;
const RAIN = 0;
const SNOW = 1;
const MIX = 2;
const typed = (cls: number, q: number) => cls * STRIDE + q;

/** A size x size slot from a function of (px, py). */
const slot = (size: number, f: (px: number, py: number) => number) => (px: number, py: number) => {
  expect(px).toBeGreaterThanOrEqual(0);
  expect(py).toBeGreaterThanOrEqual(0);
  expect(px).toBeLessThan(size);
  expect(py).toBeLessThan(size);
  return f(px, py);
};

/** Tile-space coordinate of texel-space x (texel centres are at i + 0.5). */
const uv = (x: number, size: number) => x / size;

describe("smoothSample", () => {
  it("returns the texel itself at texel centres", () => {
    const codes = [typed(RAIN, 5), typed(SNOW, 9), typed(MIX, 31), typed(RAIN, 17)];
    const fetch = slot(2, (x, y) => codes[y * 2 + x]);
    for (let y = 0; y < 2; y++) {
      for (let x = 0; x < 2; x++) {
        expect(smoothSample(fetch, 2, uv(x + 0.5, 2), uv(y + 0.5, 2), STRIDE)).toEqual({ code: codes[y * 2 + x], alpha: 1 });
      }
    }
  });

  it("blends v1 codes linearly between texel centres", () => {
    const fetch = slot(4, (x) => [10, 30, 30, 30][x]);
    expect(smoothSample(fetch, 4, uv(1, 4), 0.5, 0).code).toBe(20);
    expect(smoothSample(fetch, 4, uv(0.75, 4), 0.5, 0).code).toBe(15);
    expect(smoothSample(fetch, 4, uv(1, 4), 0.5, 0).alpha).toBe(1);
  });

  it("keeps the class of the nearest texel and never blends across classes", () => {
    // Rain q=20 on the left half, snow q=4 on the right half.
    const fetch = slot(4, (x) => (x < 2 ? typed(RAIN, 20) : typed(SNOW, 4)));
    const left = smoothSample(fetch, 4, uv(1.9, 4), 0.5, STRIDE);
    const right = smoothSample(fetch, 4, uv(2.1, 4), 0.5, STRIDE);
    expect(left).toEqual({ code: typed(RAIN, 20), alpha: 1 });
    expect(right).toEqual({ code: typed(SNOW, 4), alpha: 1 });
    // Sweep: every result is one of the two input codes, never a third class or a mixed q.
    for (let x = 0; x <= 4; x += 0.05) {
      expect([typed(RAIN, 20), typed(SNOW, 4)]).toContain(smoothSample(fetch, 4, x / 4, 0.37, STRIDE).code);
    }
  });

  it("does not drag intensity towards zero next to missing texels; coverage fades the alpha", () => {
    // One echo column of q=24 at x=1; x=0 and x=2.. are missing.
    const fetch = slot(4, (x) => (x === 1 ? typed(RAIN, 24) : 0));
    // Halfway to the missing neighbour (where nearest sampling had its hard edge).
    const edge = smoothSample(fetch, 4, uv(2, 4), 0.5, STRIDE);
    expect(edge.code).toBe(typed(RAIN, 24));
    expect(edge.alpha).toBeCloseTo(smoothstep(COVERAGE_LO, COVERAGE_HI, 0.5), 6);
    expect(edge.alpha).toBeGreaterThan(0.5);
    expect(edge.alpha).toBeLessThan(1);
    // A quarter texel past the centre: still full intensity, mostly opaque.
    const inner = smoothSample(fetch, 4, uv(1.75, 4), 0.5, STRIDE);
    expect(inner.code).toBe(typed(RAIN, 24));
    expect(inner.alpha).toBe(1);
    // Near the missing texel's centre: faded out.
    const outer = smoothSample(fetch, 4, uv(2.5, 4) - 0.1 / 4, 0.5, STRIDE);
    expect(outer.code).toBe(typed(RAIN, 24));
    expect(outer.alpha).toBe(0);
    // Fully missing neighbourhood draws nothing.
    expect(smoothSample(fetch, 4, uv(3.5, 4), 0.5, STRIDE)).toEqual({ code: 0, alpha: 0 });
  });

  it("keeps an isolated strong cell at full intensity and about its original size", () => {
    const size = 5;
    const fetch = slot(size, (x, y) => (x === 2 && y === 2 ? typed(RAIN, 31) : 0));
    let area = 0;
    const n = 100;
    for (let j = 0; j < n; j++) {
      for (let i = 0; i < n; i++) {
        const s = smoothSample(fetch, size, (1.5 + (2 * (i + 0.5)) / n) / size, (1.5 + (2 * (j + 0.5)) / n) / size, STRIDE);
        if (s.alpha > 0) expect(s.code).toBe(typed(RAIN, 31));
        area += s.alpha * (2 / n) ** 2;
      }
    }
    // Nearest sampling drew exactly one texel; the faded footprint is within 10% of that.
    expect(area).toBeGreaterThan(0.9);
    expect(area).toBeLessThan(1.1);
  });

  it("stays inside the slot's own atlas block, even at the tile border", () => {
    // A 2x1 block atlas (cols = 2): this slot's block is empty, the neighbouring block is full.
    const tile = 8;
    const width = tile * 2;
    const atlas = new Uint8Array(width * tile);
    for (let y = 0; y < tile; y++) for (let x = tile; x < width; x++) atlas[y * width + x] = typed(SNOW, 30);
    atlas[3 * width + (tile - 1)] = typed(RAIN, 12); // last column of our block
    const reads: number[] = [];
    const fetch = (px: number, py: number) => {
      const ax = 0 + px; // block origin x0 = 0
      reads.push(ax);
      return atlas[py * width + ax];
    };
    for (const u of [0, 0.5, 0.99, 0.999999, 1]) {
      for (const v of [0, 0.4, 3.5 / tile, 1]) {
        const s = smoothSample(fetch, tile, u, v, STRIDE);
        expect([0, typed(RAIN, 12)]).toContain(s.code);
      }
    }
    expect(Math.max(...reads)).toBe(tile - 1);
    expect(Math.min(...reads)).toBe(0);
    // At the right border the edge texel is clamped (a seam), not blended with the next block.
    expect(smoothSample(fetch, tile, 1, 3.5 / tile, STRIDE)).toEqual({ code: typed(RAIN, 12), alpha: 1 });
  });

  it("clamps the blended q to the class's code range", () => {
    const fetch = slot(2, () => typed(MIX, 31));
    expect(smoothSample(fetch, 2, 0.3, 0.8, STRIDE).code).toBe(typed(MIX, 31));
    const v1 = slot(2, () => 255);
    expect(smoothSample(v1, 2, 0.3, 0.8, 0).code).toBe(255);
  });
});

describe("packed fragment shader", () => {
  it("mirrors smoothSample: four clamped fetches, per-class mean, coverage fade", () => {
    expect(FRAGMENT).toContain(`smoothstep(${COVERAGE_LO}, ${COVERAGE_HI}, coverage)`);
    expect(FRAGMENT).toContain("clamp(ivec2(i), ivec2(0), ivec2(u_size - 1))");
    expect(FRAGMENT).toContain("clamp(ivec2(i) + 1, ivec2(0), ivec2(u_size - 1))");
    expect(FRAGMENT).toContain("cls[k] != cls[best]");
    expect(FRAGMENT.match(/code\(/g)?.length).toBe(5); // the helper plus four fetches
  });
});
