/**
 * Bilinear reconstruction of packed codes, decoded before blending. Hardware LINEAR filtering would
 * average raw codes, and a v2 code is `class * stride + q`, so a rain and a snow texel would blend
 * into a third class. The fragment shader in layer.ts (FRAGMENT) does exactly this per pixel; this
 * TypeScript copy is the tested reference for that math.
 *
 * - The four texels around the sample point are read, clamped to the tile's own block: never a
 *   neighbouring slot block of the atlas. A tile border is a clamp seam (no cross-tile blending).
 * - Each texel decodes to (class, q); q = 0 is missing.
 * - The class is the present texel with the largest bilinear weight (the nearest one when present).
 * - q is the weight-normalised mean over present texels of that class, so a missing or other-class
 *   neighbour never drags the intensity towards 0 or across a type boundary.
 * - Coverage (the summed weight of present texels) fades the alpha with smoothstep(COVERAGE_LO,
 *   COVERAGE_HI), so storm edges fade out over about a texel and small echoes keep their size.
 *
 * In smooth mode (the default; Settings → Stepped bands off) the shader samples a 4x4 cubic B-spline
 * instead (bsplineTexels, same missing-data and class rules) and the mean q is not rounded: the
 * colour is blended between the class's two neighbouring LUT entries (sampleColor), on a smooth LUT.
 */
import { lutBlend } from "../shared/packed";

export interface SmoothSample {
  /** LUT index to color with (class * stride + q, or q for v1); 0 = draw nothing. */
  code: number;
  /** Multiplier for the LUT alpha, 0..1. */
  alpha: number;
}

/** Hermite smoothstep, as in GLSL. */
export function smoothstep(e0: number, e1: number, x: number): number {
  const t = Math.min(1, Math.max(0, (x - e0) / (e1 - e0)));
  return t * t * (3 - 2 * t);
}

/**
 * Coverage fade edges. With these an isolated texel keeps its area (about 1.0 texel², vs 0.67 for
 * 0.25..0.75), and a straight echo edge reaches half opacity 0.1 texel outside nearest's edge.
 */
export const COVERAGE_LO = 0.15;
export const COVERAGE_HI = 0.65;

/**
 * Samples one slot of a tile at (u, v) in 0..1 tile space.
 * `fetch(px, py)` returns the code (0..255) at pixel (px, py) of the slot, 0 <= px, py < size.
 * `stride` is the v2 types stride, or 0 for untyped (v1) layers.
 */
export function smoothSample(fetch: (px: number, py: number) => number, size: number, u: number, v: number, stride: number): SmoothSample {
  const b = blendTexels(fetch, size, u, v, stride);
  if (!b) return { code: 0, alpha: 0 };
  const qi = Math.min(b.qmax, Math.max(1, Math.floor(b.mean + 0.5)));
  return { code: b.base + qi, alpha: smoothstep(COVERAGE_LO, COVERAGE_HI, b.coverage) };
}

/**
 * The pixel colour the fragment shader writes before the layer opacity: premultiplied RGBA, 0..1.
 * Stepped: the LUT entry of smoothSample's rounded code (PR #364's look, the server's band colours).
 * Smooth: a 4x4 cubic B-spline (bsplineTexels), the fractional mean q blended between the two neighbouring LUT entries of the same class
 * (lutBlend), so a smooth LUT gives continuous colour. Both fade by coverage.
 */
export function sampleColor(
  fetch: (px: number, py: number) => number,
  size: number,
  u: number,
  v: number,
  stride: number,
  lut: Uint8Array,
  smooth: boolean,
): [number, number, number, number] {
  const b = smooth ? bsplineTexels(fetch, size, u, v, stride) : blendTexels(fetch, size, u, v, stride);
  if (!b) return [0, 0, 0, 0];
  const fade = smoothstep(COVERAGE_LO, COVERAGE_HI, b.coverage);
  if (smooth) return lutBlend(lut, b.base, b.mean, b.qmax).map((c) => c * fade) as [number, number, number, number];
  const i = b.base + Math.min(b.qmax, Math.max(1, Math.floor(b.mean + 0.5)));
  const a = (lut[i * 4 + 3] / 255) * fade;
  return [(lut[i * 4] / 255) * a, (lut[i * 4 + 1] / 255) * a, (lut[i * 4 + 2] / 255) * a, a];
}

interface Blend {
  /** LUT index of the class's q = 0 (class * stride, or 0 for v1). */
  base: number;
  /** Weight-normalised mean q of the chosen class, unrounded. */
  mean: number;
  qmax: number;
  coverage: number;
}

/** Cubic B-spline weights of the 4 taps around texel fraction t: non-negative, summing to 1. */
export function bspline(t: number): [number, number, number, number] {
  const s = 1 - t;
  const t2 = t * t;
  return [(s * s * s) / 6, (3 * t2 * t - 6 * t2 + 4) / 6, (-3 * t2 * t + 3 * t2 + 3 * t + 1) / 6, (t2 * t) / 6];
}

/**
 * The smooth path's sampling (the shader's smoothMain): a 4x4 cubic B-spline. Missing taps only
 * lower coverage; the class with the most weight wins (ties to the lower class, rain first) and
 * only its taps are averaged.
 */
export function bsplineTexels(fetch: (px: number, py: number) => number, size: number, u: number, v: number, stride: number): Blend | null {
  const fx = u * size - 0.5;
  const fy = v * size - 0.5;
  const ix = Math.floor(fx);
  const iy = Math.floor(fy);
  const wx = bspline(fx - ix);
  const wy = bspline(fy - iy);
  const csum = new Array<number>(8).fill(0);
  const cw = new Array<number>(8).fill(0);
  let coverage = 0;
  for (let y = 0; y < 4; y++) {
    for (let x = 0; x < 4; x++) {
      const c = fetch(clampInt(ix + x - 1, size), clampInt(iy + y - 1, size));
      const k = stride > 0 ? Math.min(Math.floor(c / stride), 7) : 0;
      const q = stride > 0 ? c % stride : c;
      if (q === 0) continue;
      const w = wx[x] * wy[y];
      coverage += w;
      csum[k] += w * q;
      cw[k] += w;
    }
  }
  let best = 0;
  for (let k = 1; k < 8; k++) if (cw[k] > cw[best]) best = k;
  if (cw[best] <= 0) return null;
  return { base: stride > 0 ? best * stride : 0, mean: csum[best] / cw[best], qmax: stride > 0 ? stride - 1 : 255, coverage };
}

function blendTexels(fetch: (px: number, py: number) => number, size: number, u: number, v: number, stride: number): Blend | null {
  const fx = u * size - 0.5;
  const fy = v * size - 0.5;
  const ix = Math.floor(fx);
  const iy = Math.floor(fy);
  const wx = fx - ix;
  const wy = fy - iy;
  const x0 = clampInt(ix, size);
  const x1 = clampInt(ix + 1, size);
  const y0 = clampInt(iy, size);
  const y1 = clampInt(iy + 1, size);
  const codes = [fetch(x0, y0), fetch(x1, y0), fetch(x0, y1), fetch(x1, y1)];
  const weights = [(1 - wx) * (1 - wy), wx * (1 - wy), (1 - wx) * wy, wx * wy];

  let coverage = 0;
  let best = -1;
  let bestW = -1;
  const cls = [0, 0, 0, 0];
  const q = [0, 0, 0, 0];
  for (let k = 0; k < 4; k++) {
    const c = codes[k];
    cls[k] = stride > 0 ? Math.floor(c / stride) : 0;
    q[k] = stride > 0 ? c % stride : c;
    if (q[k] === 0) continue;
    coverage += weights[k];
    if (weights[k] > bestW) {
      bestW = weights[k];
      best = k;
    }
  }
  if (best < 0) return null;

  let sum = 0;
  let wsum = 0;
  for (let k = 0; k < 4; k++) {
    if (q[k] === 0 || cls[k] !== cls[best]) continue;
    sum += weights[k] * q[k];
    wsum += weights[k];
  }
  // wsum > 0 whenever bestW > 0; a weight-0 best (exact texel edge) falls back to its own q.
  const mean = wsum > 0 ? sum / wsum : q[best];
  return { base: stride > 0 ? cls[best] * stride : 0, mean, qmax: stride > 0 ? stride - 1 : 255, coverage };
}

function clampInt(i: number, size: number): number {
  return Math.min(size - 1, Math.max(0, i));
}
