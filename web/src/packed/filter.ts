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
 */

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
  if (best < 0) return { code: 0, alpha: 0 };

  let sum = 0;
  let wsum = 0;
  for (let k = 0; k < 4; k++) {
    if (q[k] === 0 || cls[k] !== cls[best]) continue;
    sum += weights[k] * q[k];
    wsum += weights[k];
  }
  // wsum > 0 whenever bestW > 0; a weight-0 best (exact texel edge) falls back to its own q.
  const mean = wsum > 0 ? sum / wsum : q[best];
  const qmax = stride > 0 ? stride - 1 : 255;
  const qi = Math.min(qmax, Math.max(1, Math.floor(mean + 0.5)));
  return { code: (stride > 0 ? cls[best] * stride : 0) + qi, alpha: smoothstep(COVERAGE_LO, COVERAGE_HI, coverage) };
}

function clampInt(i: number, size: number): number {
  return Math.min(size - 1, Math.max(0, i));
}
