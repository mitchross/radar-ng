import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";
import { buildLut, buildRampLut, buildSmoothLut, lutBlend, SMOOTH_FLOOR_ALPHA, type PackedTypes } from "../shared/packed";
import type { ColorTable } from "../shared/serverPalettes";
import { bspline, sampleColor, smoothSample } from "./filter";
import { FRAGMENT } from "./layer";

const PALETTE_IDS = ["classic", "muted", "vivid"] as const;
const assets = Object.fromEntries(
  PALETTE_IDS.map((id) => [id, JSON.parse(readFileSync(new URL(`../../../rust/crates/radar-tiler/assets/${id}.json`, import.meta.url), "utf8")) as Record<string, ColorTable>]),
);
/** The live manifest's quantization per packed colour key (docs/reference/packed-tiles.md). */
const QUANT: Record<string, { scale: number; offset: number }> = {
  reflectivity: { scale: 2.5, offset: 0 },
  temperature: { scale: 1, offset: -80 },
  dewpoint: { scale: 5, offset: -80 },
  wind_speed: { scale: 5, offset: -5 },
  humidity: { scale: 10, offset: -10 },
  cape: { scale: 500, offset: 0 },
  precip_accum: { scale: 0.05, offset: 0 },
  cloud_cover: { scale: 5, offset: 0 },
  pm25: { scale: 1, offset: 0.5 },
  ozone: { scale: 1, offset: 0 },
};
const TYPES: PackedTypes = { stride: 32, classes: ["rain", "snow", "mix", "ice"] };
const at = (lut: Uint8Array, i: number) => Array.from(lut.slice(i * 4, i * 4 + 4));

/** Every (palette, colour key) the packed renderer draws, with typed reflectivity. */
function cases() {
  const out: { name: string; table: ColorTable; scale: number; offset: number; types?: PackedTypes }[] = [];
  for (const id of PALETTE_IDS) {
    for (const [key, q] of Object.entries(QUANT)) {
      const table = assets[id][key];
      if (!table?.ranges?.length) continue;
      out.push({ name: `${id}/${key}`, table, ...q });
      if (key === "reflectivity") out.push({ name: `${id}/${key}/typed`, table, ...q, types: TYPES });
    }
  }
  return out;
}

describe("smooth ramp LUT", () => {
  it("covers the packed colour keys of every palette", () => {
    expect(cases().length).toBe(PALETTE_IDS.length * (Object.keys(QUANT).length + 1));
  });

  it("stepped mode is PR #364's table, byte for byte", () => {
    for (const c of cases()) expect(buildRampLut(c.table, c.scale, c.offset, c.types, false)).toEqual(buildLut(c.table, c.scale, c.offset, c.types));
  });

  it("is transparent exactly where the stepped table is", () => {
    for (const c of cases()) {
      const stepped = buildLut(c.table, c.scale, c.offset, c.types);
      const smooth = buildSmoothLut(c.table, c.scale, c.offset, c.types);
      for (let i = 0; i < 256; i++) expect([c.name, i, smooth[i * 4 + 3] === 0]).toEqual([c.name, i, stepped[i * 4 + 3] === 0]);
    }
  });

  it("keeps the band colour at each band start and blends towards the next band inside it", () => {
    const table = assets.classic.reflectivity;
    const lut = buildSmoothLut(table, 2.5, 0);
    const r = table.ranges!;
    // 30 dBZ (q 12) starts a band: its stepped colour exactly.
    expect(at(lut, 12)).toEqual(r[5].rgba);
    // 32.5 dBZ (q 13) is halfway from the 30 band's green to the 35 band's yellow.
    expect(at(lut, 13)).toEqual(r[5].rgba.map((v, k) => Math.round((v + r[6].rgba[k]) / 2)));
    // The top band (65-75) does not blend into anything.
    expect(at(lut, 27)).toEqual(r[12].rgba);
    expect(at(lut, 29)).toEqual(r[12].rgba);
  });

  it("matches the stepped colour at every band start above the floor, for every palette", () => {
    for (const c of cases()) {
      if (c.types) continue;
      const stepped = buildLut(c.table, c.scale, c.offset);
      const smooth = buildSmoothLut(c.table, c.scale, c.offset);
      const starts = new Set(c.table.ranges!.slice(1).map((r) => r.min));
      for (let q = 1; q < 256; q++) {
        const v = Math.round((c.offset + q * c.scale) * 1e6) / 1e6;
        if (starts.has(v) && stepped[q * 4 + 3] > 0) expect([c.name, q, at(smooth, q)]).toEqual([c.name, q, at(stepped, q)]);
      }
    }
  });

  it("fades in from the floor instead of cutting in at full band alpha", () => {
    const table = assets.classic.reflectivity;
    const lut = buildSmoothLut(table, 2.5, 0);
    const first = table.ranges![0].rgba;
    // q 1 = 2.5 dBZ is below no_data_below (5): transparent, as stepped.
    expect(at(lut, 1)).toEqual([0, 0, 0, 0]);
    // 5 dBZ starts at half the band's alpha, 7.5 dBZ halfway to the 10 dBZ band, 10 dBZ is that band.
    expect(at(lut, 2)).toEqual([...first.slice(0, 3), Math.round(first[3] * SMOOTH_FLOOR_ALPHA)]);
    expect(lut[3 * 4 + 3]).toBe(Math.round((first[3] * SMOOTH_FLOOR_ALPHA + table.ranges![1].rgba[3]) / 2));
    expect(at(lut, 4)).toEqual(table.ranges![1].rgba);
    // Snow, mix and ice fade from their own first band too.
    const typed = buildSmoothLut(table, 2.5, 0, TYPES);
    for (const [cls, name] of [[1, "snow"], [2, "mix"], [3, "ice"]] as const) {
      const a = table.types![name]!.ranges![0].rgba[3];
      expect(typed[(cls * 32 + 2) * 4 + 3]).toBe(Math.round(a * SMOOTH_FLOOR_ALPHA));
    }
  });

  it("does not fade full-coverage fields whose no_data_below is not the ramp's floor", () => {
    const table = assets.classic.temperature;
    expect(table.no_data_below).toBe(-999);
    const lut = buildSmoothLut(table, 1, -80);
    const q = -40 + 80; // the lowest band starts at -40 °F
    expect(at(lut, q)).toEqual(table.ranges![0].rgba);
    const dew = assets.classic.dewpoint;
    expect(at(buildSmoothLut(dew, 5, -80), (-20 + 80) / 5)).toEqual(dew.ranges![0].rgba);
  });

  it("keeps each precipitation class on its own ramp", () => {
    const table = assets.vivid.reflectivity;
    const typed = buildSmoothLut(table, 2.5, 0, TYPES);
    const rain = buildSmoothLut(table, 2.5, 0);
    for (let q = 1; q < 32; q++) expect(at(typed, q)).toEqual(at(rain, q));
    for (const [cls, name] of [[1, "snow"], [2, "mix"], [3, "ice"]] as const) {
      const own = buildSmoothLut({ ranges: table.types![name]!.ranges, no_data_below: table.no_data_below }, 2.5, 0);
      // The class ramp alone; no_data_below is the rain ramp's rule, so start above it.
      for (let q = 2; q < 32; q++) expect([name, q, at(typed, cls * 32 + q)]).toEqual([name, q, at(own, q)]);
      expect(at(typed, cls * 32)).toEqual([0, 0, 0, 0]);
    }
  });
});

describe("lutBlend and sampleColor (the shader's smooth path)", () => {
  const table = assets.classic.reflectivity;
  const smoothLut = buildSmoothLut(table, 2.5, 0, TYPES);
  const steppedLut = buildLut(table, 2.5, 0, TYPES);
  const premul = (lut: Uint8Array, i: number) => {
    const a = lut[i * 4 + 3] / 255;
    return [(lut[i * 4] / 255) * a, (lut[i * 4 + 1] / 255) * a, (lut[i * 4 + 2] / 255) * a, a];
  };

  it("is the entry itself at whole q", () => {
    for (let q = 1; q < 32; q++) expect(lutBlend(smoothLut, 32, q, 31)).toEqual(premul(smoothLut, 32 + q));
  });

  it("never reads past the class's last code into the next class", () => {
    // Rain q 31 is transparent and snow q 1 (code 33) is not; a blend near 31 stays rain.
    const near = lutBlend(smoothLut, 0, 30.9, 31);
    near.forEach((x, k) => expect(x).toBeCloseTo(premul(smoothLut, 30)[k] * 0.1 + premul(smoothLut, 31)[k] * 0.9, 12));
    expect(lutBlend(smoothLut, 0, 31, 31)).toEqual(premul(smoothLut, 31));
    expect(lutBlend(smoothLut, 32, 40, 31)).toEqual(premul(smoothLut, 63));
    expect(lutBlend(smoothLut, 32, 0.2, 31)).toEqual(premul(smoothLut, 33));
  });

  it("stepped sampleColor is PR #364's colour: the rounded code's entry times the coverage fade", () => {
    const codes = [12, 13, 33, 0, 20, 21, 50, 14, 9];
    const fetch = (x: number, y: number) => codes[y * 3 + x];
    for (let i = 0; i <= 30; i++) {
      const u = i / 30;
      const v = (30 - i) / 30;
      const s = smoothSample(fetch, 3, u, v, 32);
      const c = sampleColor(fetch, 3, u, v, 32, steppedLut, false);
      const want = s.code ? premul(steppedLut, s.code).map((x) => x * s.alpha) : [0, 0, 0, 0];
      c.forEach((x, k) => expect(x).toBeCloseTo(want[k], 12));
    }
  });

  it("smooth sampleColor gives continuous colour between texels of one class", () => {
    // Rain q 12 (30 dBZ, green) next to rain q 14 (35 dBZ, yellow): stepped jumps, smooth glides.
    const fetch = (x: number) => (x < 1 ? 12 : 14);
    let prev = sampleColor(fetch, 2, 0.25, 0.5, 32, smoothLut, true);
    let maxStep = 0;
    for (let i = 1; i <= 100; i++) {
      const c = sampleColor(fetch, 2, 0.25 + (0.5 * i) / 100, 0.5, 32, smoothLut, true);
      maxStep = Math.max(maxStep, ...c.map((x, k) => Math.abs(x - prev[k])));
      prev = c;
    }
    expect(maxStep).toBeLessThan(0.02);
    // Across the edge the colour runs from q 12's towards q 14's (the B-spline keeps some of each).
    const left = sampleColor(fetch, 2, 0.25, 0.5, 32, smoothLut, true);
    const right = sampleColor(fetch, 2, 0.75, 0.5, 32, smoothLut, true);
    const mid = sampleColor(fetch, 2, 0.5, 0.5, 32, smoothLut, true);
    expect(lutBlend(smoothLut, 0, 13, 31).map((x) => expect.closeTo(x, 9))).toEqual(mid);
    expect(left[0]).toBeLessThan(mid[0]);
    expect(mid[0]).toBeLessThan(right[0]);
  });

  it("never blends across a rain/snow edge", () => {
    const fetch = (x: number) => (x < 1 ? 20 : 32 + 20);
    for (let i = 0; i <= 50; i++) {
      const u = i / 50;
      const c = sampleColor(fetch, 2, u, 0.5, 32, smoothLut, true);
      // At u = 0.5 the weights tie and the first texel (rain) wins, as in the shader.
      const own = u <= 0.5 ? premul(smoothLut, 20) : premul(smoothLut, 52);
      c.forEach((x, k) => expect(x).toBeCloseTo(own[k], 12));
    }
  });
});

describe("B-spline sampling (smooth mode)", () => {
  it("has non-negative weights summing to 1", () => {
    for (let i = 0; i <= 20; i++) {
      const w = bspline(i / 20);
      expect(w.every((x) => x >= 0)).toBe(true);
      expect(w.reduce((a, b) => a + b)).toBeCloseTo(1, 12);
    }
  });

  it("is the texel value over a uniform field and fades isolated speckle", () => {
    const smoothLut = buildSmoothLut(assets.classic.reflectivity, 2.5, 0, TYPES);
    const premul12 = lutBlend(smoothLut, 0, 12, 31);
    for (let i = 0; i <= 10; i++) {
      const c = sampleColor(() => 12, 8, i / 10, 0.3, 32, smoothLut, true);
      c.forEach((x, k) => expect(x).toBeCloseTo(premul12[k], 12));
    }
    // One echo texel in a 5x5 empty field: centre coverage (2/3)^2, so partly faded, not cut.
    const fetch = (x: number, y: number) => (x === 2 && y === 2 ? 20 : 0);
    const centre = sampleColor(fetch, 5, 0.5, 0.5, 32, smoothLut, true);
    const full = lutBlend(smoothLut, 0, 20, 31);
    expect(centre[3]).toBeGreaterThan(0.4 * full[3]);
    expect(centre[3]).toBeLessThan(full[3]);
  });
});

describe("packed fragment shader smooth path", () => {
  it("mirrors bsplineTexels and lutBlend; stepped path kept", () => {
    expect(FRAGMENT).toContain("vec4 bspline(float t)");
    expect(FRAGMENT).toContain("for (int k = 1; k < 8; k++) if (cw[k] > cw[best]) best = k;");
    expect(FRAGMENT).toContain("uniform int u_smooth;");
    expect(FRAGMENT).toContain("float m = clamp(csum[best] / cw[best], 1.0, float(qmax));");
    expect(FRAGMENT).toContain("int hi = min(lo + 1, qmax);");
    expect(FRAGMENT).toContain("mix(vec4(c0.rgb * c0.a, c0.a), vec4(c1.rgb * c1.a, c1.a), m - float(lo))");
    // PR #364's stepped lines, unchanged.
    expect(FRAGMENT).toContain("int qi = clamp(int(floor(mean + 0.5)), 1, qmax);");
    expect(FRAGMENT).toContain("fragColor = vec4(color.rgb * a, a);");
  });
});
