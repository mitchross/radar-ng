/**
 * What both fast-radar engines share: the SkSL palette shader, the packed-tile decoder and the
 * module-level decoded-tile cache. PackedRadarOverlay draws with them on a canvas over the map;
 * InMapPackedRadar draws the same shader into offscreen tiles that MapLibre loads from disk.
 */
import {
  AlphaType,
  ColorType,
  FilterMode,
  MipmapMode,
  Skia,
  TileMode,
  type SkImage,
  type SkShader,
} from "@shopify/react-native-skia";
import { fetchWithTimeout } from "../../lib/api";
import { PACKED_TILE_SIZE } from "../../lib/packed";
import { createTileCache, type LoadResult } from "./packedTileCache";

/** Uniform order: tileOrigin(2), tileScale, blockOrigin(2), channel, tileSize, opacity. */
export const PACKED_SKSL = `
uniform shader atlas;
uniform shader lut;
uniform float2 tileOrigin;
uniform float tileScale;
uniform float2 blockOrigin;
uniform float channel;
uniform float tileSize;
uniform float opacity;

half4 main(float2 pos) {
  float2 texel = clamp(floor((pos - tileOrigin) / tileScale), float2(0.0), float2(tileSize - 1.0));
  float4 t = atlas.eval(blockOrigin + texel + 0.5);
  float q = channel < 0.5 ? t.r : (channel < 1.5 ? t.g : t.b);
  float4 c = lut.eval(float2(floor(q * 255.0 + 0.5) + 0.5, 0.5));
  return half4(c * opacity);
}`;

/** Decoded atlas bytes kept in memory; prefetch stops short of it. */
export const CACHE_BUDGET_BYTES = 192 * 1024 * 1024;

export interface Decoded {
  image: SkImage;
  shader: SkShader;
}

function decode(buf: ArrayBuffer): Decoded | null {
  const lazy = Skia.Image.MakeImageFromEncoded(Skia.Data.fromBytes(new Uint8Array(buf)));
  if (!lazy) return null;
  // Decode now on the JS thread, not on the first draw mid-animation.
  let image = lazy;
  try {
    image = lazy.makeNonTextureImage() ?? lazy;
  } catch {}
  const shader = image.makeShaderOptions(TileMode.Clamp, TileMode.Clamp, FilterMode.Nearest, MipmapMode.None);
  return { image, shader };
}

async function loadTile(url: string): Promise<LoadResult<Decoded>> {
  const res = await fetchWithTimeout(url);
  if (res.status === 204) return { kind: "empty" };
  if (!res.ok) return { kind: "error" };
  const decoded = decode(await res.arrayBuffer());
  if (!decoded) return { kind: "error" };
  const { image } = decoded;
  return { kind: "ready", handle: decoded, height: image.height(), bytes: image.width() * image.height() * 4 };
}

/** One LUT image per (color key, quantization); the bytes are unpremultiplied RGBA. */
export function makeLutShaders(luts: Record<string, Uint8Array>): Record<string, SkShader> {
  const out: Record<string, SkShader> = {};
  for (const [key, bytes] of Object.entries(luts)) {
    const image = Skia.Image.MakeImage(
      { width: 256, height: 1, alphaType: AlphaType.Unpremul, colorType: ColorType.RGBA_8888 },
      Skia.Data.fromBytes(bytes),
      256 * 4,
    );
    if (image) {
      out[key] = image.makeShaderOptions(TileMode.Clamp, TileMode.Clamp, FilterMode.Nearest, MipmapMode.None);
    }
  }
  return out;
}

/** Module-level so tiles survive a remount (tab switch, layer toggle, engine switch). */
export const tiles = createTileCache<Decoded>({
  budgetBytes: CACHE_BUDGET_BYTES,
  maxInFlight: 6,
  errorRetryMs: 30_000,
  maxEntries: 2000,
  now: Date.now,
  load: loadTile,
});

/** Rough decoded size of a bucket's atlas, from its slot count. */
export function atlasBytes(slots: number, cols: number): number {
  const rows = Math.max(1, Math.ceil(Math.ceil(Math.max(1, slots) / 3) / cols));
  return PACKED_TILE_SIZE * cols * PACKED_TILE_SIZE * rows * 4;
}
