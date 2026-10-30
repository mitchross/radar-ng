/**
 * MapLibre custom layer that draws packed data tiles (docs/reference/packed-tiles.md): one atlas
 * texture per (layer, bucket, tile), colored on the GPU through a 256-entry table.
 * Changing frame, palette or opacity re-renders without any network request.
 * Mercator only: under globe projection the layer draws nothing.
 */
import type { CustomLayerInterface, CustomRenderMethodInput, Map as MLMap } from "maplibre-gl";
import type { PaletteId } from "../palettes";
import { colorTable, type ServerPalettes } from "./colorTables";
import {
  atlasSize,
  buildLut,
  locateFrame,
  packedTileUrl,
  parsePackedMeta,
  slotAddress,
  tileCover,
  tileMatrix,
  type FrameLocation,
  type PackedMeta,
  type TileId,
} from "./format";

export interface PackedFrameRef {
  /** The manifest layer the frame belongs to (radar, radar-composite, nowcast, ...). */
  layer: string;
  timestamp: string;
}

interface Entry {
  /** null: the server had no tile (204) or the fetch failed. */
  tex: WebGLTexture | null;
  /** Slots the tile holds at least: the bucket's slot count when it was requested. */
  slots: number;
  bytes: number;
  /** Set on failure: refetch after this time. */
  retryAt?: number;
}

interface Request {
  key: string;
  url: string;
  slots: number;
}

const CONCURRENCY = 6;
/** GPU budget for atlases; a full 24-slot atlas is 1024 x 2048 (8 MB as RGBA). */
const MAX_BYTES = 384 * 2 ** 20;
/** Prefetch stops once the loop ahead of the playhead needs this much. */
const PREFETCH_BYTES = 256 * 2 ** 20;
const MAX_ENTRIES = 1000;
const MAX_COVER_TILES = 36;
const UPLOADS_PER_FRAME = 4;
const RETRY_MS = 30_000;
const MOVE_THROTTLE_MS = 150;

const VERTEX = `#version 300 es
uniform mat4 u_matrix;
uniform vec4 u_uv;
in vec2 a_pos;
out vec2 v_uv;
void main() {
  v_uv = u_uv.xy + a_pos * u_uv.zw;
  gl_Position = u_matrix * vec4(a_pos, 0.0, 1.0);
}`;

const FRAGMENT = `#version 300 es
precision highp float;
precision highp int;
uniform sampler2D u_atlas;
uniform sampler2D u_lut;
uniform ivec2 u_origin;
uniform int u_channel;
uniform int u_size;
uniform float u_opacity;
in vec2 v_uv;
out vec4 fragColor;
void main() {
  ivec2 p = clamp(ivec2(floor(v_uv * float(u_size))), ivec2(0), ivec2(u_size - 1));
  vec3 t = texelFetch(u_atlas, u_origin + p, 0).rgb;
  float c = u_channel == 0 ? t.r : (u_channel == 1 ? t.g : t.b);
  int q = int(c * 255.0 + 0.5);
  if (q == 0) discard;
  vec4 color = texelFetch(u_lut, ivec2(q, 0), 0);
  float a = color.a * u_opacity;
  if (a <= 0.0) discard;
  fragColor = vec4(color.rgb * a, a);
}`;

type Uniform = "u_matrix" | "u_uv" | "u_atlas" | "u_lut" | "u_origin" | "u_channel" | "u_size" | "u_opacity";

const tileKey = (layer: string, bucket: string, t: TileId) => `${layer}|${bucket}|${t.z}/${t.x}/${t.y}`;

export class PackedRadarLayer implements CustomLayerInterface {
  readonly id = "packed-radar";
  readonly type = "custom" as const;
  readonly renderingMode = "2d" as const;
  /** True when WebGL setup failed; callers fall back to raster frames. */
  failed = false;

  private map: MLMap | null = null;
  private gl: WebGL2RenderingContext | null = null;
  private program: WebGLProgram | null = null;
  private vao: WebGLVertexArrayObject | null = null;
  private buffer: WebGLBuffer | null = null;
  private uniforms = {} as Record<Uniform, WebGLUniformLocation | null>;

  private metas: Record<string, PackedMeta> = {};
  /** layer -> timestamp -> slot, rebuilt per manifest. */
  private index = new Map<string, Map<string, FrameLocation | null>>();
  private server: ServerPalettes | null = null;
  private sequence: PackedFrameRef[] = [];
  private frame: PackedFrameRef | null = null;
  private luts = new Map<string, WebGLTexture>();

  /** Tile atlases, least recently drawn first. */
  private entries = new Map<string, Entry>();
  private bytes = 0;
  private wanted = new Set<string>();
  private queue: Request[] = [];
  private inflight = new Map<string, AbortController>();
  private uploads: { req: Request; bitmap: ImageBitmap }[] = [];
  private moveTimer: number | undefined;

  constructor(
    private palette: PaletteId,
    private opacity: number,
  ) {}

  // ---------- public API ----------

  /** Reads `layers.<layer>.packed` from a manifest; unusable metadata is ignored. */
  setManifest(manifest: { layers: Record<string, { packed?: unknown }> } | null) {
    const metas: Record<string, PackedMeta> = {};
    for (const [layer, entry] of Object.entries(manifest?.layers ?? {})) {
      const meta = parsePackedMeta(entry.packed);
      if (meta) metas[layer] = meta;
    }
    this.metas = metas;
    this.index = new Map();
    this.schedule();
    this.repaint();
  }

  setServerPalettes(server: ServerPalettes | null) {
    this.server = server;
    this.dropLuts();
    this.repaint();
  }

  /** True when this layer can draw the frame: packed metadata, a slot for it, and a color table. */
  covers(layer: string, timestamp: string): boolean {
    const meta = this.metas[layer];
    return !this.failed && Boolean(meta && colorTable(this.server, this.palette, meta.color_key) && this.locate(layer, timestamp));
  }

  /** The playback order of the packed frames; prefetch follows it from the current frame. */
  setSequence(refs: PackedFrameRef[]) {
    this.sequence = refs;
    this.schedule();
  }

  setFrame(ref: PackedFrameRef | null) {
    if (ref?.layer === this.frame?.layer && ref?.timestamp === this.frame?.timestamp) return;
    this.frame = ref;
    this.schedule();
    this.repaint();
  }

  setOpacity(opacity: number) {
    this.opacity = opacity;
    this.repaint();
  }

  setPalette(palette: PaletteId) {
    this.palette = palette;
    this.repaint();
  }

  /** Every tile in view for the frame's bucket is settled (drawn, empty, or failed). */
  isFrameReady(ref: PackedFrameRef): boolean {
    const meta = this.metas[ref.layer];
    const loc = this.locate(ref.layer, ref.timestamp);
    if (!meta || !loc || !this.map) return true;
    for (const t of this.cover(meta)) {
      const e = this.entries.get(tileKey(ref.layer, loc.bucket, t));
      // A failed tile counts as settled so playback does not stall on it.
      if (!e || (e.slots <= loc.slot && e.retryAt === undefined)) return false;
    }
    return true;
  }

  destroy() {
    for (const ctrl of this.inflight.values()) ctrl.abort();
    this.inflight.clear();
    this.queue = [];
    for (const u of this.uploads) u.bitmap.close();
    this.uploads = [];
    if (this.map?.getLayer(this.id)) this.map.removeLayer(this.id);
    const gl = this.gl;
    if (gl) for (const e of this.entries.values()) if (e.tex) gl.deleteTexture(e.tex);
    this.entries.clear();
    this.bytes = 0;
    this.dropLuts();
    this.gl = null;
  }

  // ---------- CustomLayerInterface ----------

  onAdd(map: MLMap, gl: WebGL2RenderingContext) {
    this.map = map;
    this.gl = gl;
    try {
      this.program = linkProgram(gl, VERTEX, FRAGMENT);
    } catch (err) {
      console.error("packed radar: WebGL setup failed, using raster tiles", err);
      this.failed = true;
      return;
    }
    for (const name of ["u_matrix", "u_uv", "u_atlas", "u_lut", "u_origin", "u_channel", "u_size", "u_opacity"] as Uniform[]) {
      this.uniforms[name] = gl.getUniformLocation(this.program, name);
    }
    this.buffer = gl.createBuffer();
    this.vao = gl.createVertexArray();
    gl.bindVertexArray(this.vao);
    gl.bindBuffer(gl.ARRAY_BUFFER, this.buffer);
    gl.bufferData(gl.ARRAY_BUFFER, new Float32Array([0, 0, 1, 0, 0, 1, 1, 1]), gl.STATIC_DRAW);
    const loc = gl.getAttribLocation(this.program, "a_pos");
    gl.enableVertexAttribArray(loc);
    gl.vertexAttribPointer(loc, 2, gl.FLOAT, false, 0, 0);
    gl.bindVertexArray(null);
    map.on("move", this.onMove);
    // After the caller's setFrame in the same task, so the current bucket is fetched first.
    queueMicrotask(() => this.schedule());
  }

  onRemove(map: MLMap, gl: WebGL2RenderingContext) {
    map.off("move", this.onMove);
    window.clearTimeout(this.moveTimer);
    this.moveTimer = undefined;
    if (this.program) gl.deleteProgram(this.program);
    if (this.vao) gl.deleteVertexArray(this.vao);
    if (this.buffer) gl.deleteBuffer(this.buffer);
    this.program = this.vao = this.buffer = null;
    // Atlases and tables stay: a style switch re-adds the layer on the same GL context.
  }

  render(gl: WebGL2RenderingContext, options: CustomRenderMethodInput) {
    if (!this.program || !this.vao) return;
    this.uploadPending(gl);
    const ref = this.frame;
    if (!ref || this.map?.getProjection()?.type === "globe") return;
    const meta = this.metas[ref.layer];
    const loc = this.locate(ref.layer, ref.timestamp);
    if (!meta || !loc) return;
    const lut = this.lut(gl, meta);
    if (!lut) return;

    const { channel, x0, y0 } = slotAddress(loc.slot, meta.cols, meta.tile_size);
    gl.useProgram(this.program);
    gl.bindVertexArray(this.vao);
    gl.activeTexture(gl.TEXTURE1);
    gl.bindTexture(gl.TEXTURE_2D, lut);
    gl.uniform1i(this.uniforms.u_lut, 1);
    gl.activeTexture(gl.TEXTURE0);
    gl.uniform1i(this.uniforms.u_atlas, 0);
    gl.uniform2i(this.uniforms.u_origin, x0, y0);
    gl.uniform1i(this.uniforms.u_channel, channel);
    gl.uniform1i(this.uniforms.u_size, meta.tile_size);
    gl.uniform1f(this.uniforms.u_opacity, this.opacity);

    const main = options.defaultProjectionData.mainMatrix;
    for (const t of this.cover(meta)) {
      const hit = this.drawable(ref.layer, loc, meta, t);
      if (!hit) continue;
      const d = 2 ** (t.z - hit.tile.z);
      gl.bindTexture(gl.TEXTURE_2D, hit.tex);
      gl.uniformMatrix4fv(this.uniforms.u_matrix, false, tileMatrix(main, t));
      gl.uniform4f(this.uniforms.u_uv, (t.x - hit.tile.x * d) / d, (t.y - hit.tile.y * d) / d, 1 / d, 1 / d);
      gl.drawArrays(gl.TRIANGLE_STRIP, 0, 4);
    }
    gl.bindVertexArray(null);
  }

  // ---------- drawing ----------

  /** The tile's own atlas, else the nearest loaded ancestor (drawn as a sub-rectangle). */
  private drawable(layer: string, loc: FrameLocation, meta: PackedMeta, t: TileId): { tex: WebGLTexture; tile: TileId } | null {
    for (let z = t.z; z >= meta.min_zoom; z--) {
      const d = 2 ** (t.z - z);
      const tile = { z, x: Math.floor(t.x / d), y: Math.floor(t.y / d) };
      const key = tileKey(layer, loc.bucket, tile);
      const e = this.entries.get(key);
      if (!e || e.slots <= loc.slot) continue;
      // A 204 means no data anywhere under that tile; a failure lets an ancestor stand in.
      if (!e.tex) {
        if (e.retryAt === undefined) return null;
        continue;
      }
      this.entries.delete(key);
      this.entries.set(key, e);
      return { tex: e.tex, tile };
    }
    return null;
  }

  private lut(gl: WebGL2RenderingContext, meta: PackedMeta): WebGLTexture | null {
    const key = `${this.palette}|${meta.color_key}|${meta.scale}|${meta.offset}|${meta.types?.stride ?? 0}|${meta.types?.classes.join(",") ?? ""}`;
    const existing = this.luts.get(key);
    if (existing) return existing;
    const table = colorTable(this.server, this.palette, meta.color_key);
    if (!table) return null;
    const tex = gl.createTexture();
    gl.bindTexture(gl.TEXTURE_2D, tex);
    gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA8, 256, 1, 0, gl.RGBA, gl.UNSIGNED_BYTE, buildLut(table, meta.scale, meta.offset, meta.types));
    setNearest(gl);
    this.luts.set(key, tex);
    return tex;
  }

  private dropLuts() {
    if (this.gl) for (const tex of this.luts.values()) this.gl.deleteTexture(tex);
    this.luts.clear();
  }

  private cover(meta: PackedMeta): TileId[] {
    const map = this.map;
    if (!map) return [];
    const b = map.getBounds();
    return tileCover({ west: b.getWest(), south: b.getSouth(), east: b.getEast(), north: b.getNorth() }, map.getZoom(), meta.min_zoom, meta.max_zoom, MAX_COVER_TILES);
  }

  private locate(layer: string, timestamp: string): FrameLocation | null {
    let byTime = this.index.get(layer);
    if (!byTime) this.index.set(layer, (byTime = new Map()));
    let loc = byTime.get(timestamp);
    if (loc === undefined) byTime.set(timestamp, (loc = locateFrame(this.metas[layer], timestamp)));
    return loc;
  }

  private repaint() {
    this.map?.triggerRepaint();
  }

  private onMove = () => {
    if (this.moveTimer !== undefined) return;
    this.moveTimer = window.setTimeout(() => {
      this.moveTimer = undefined;
      this.schedule();
    }, MOVE_THROTTLE_MS);
  };

  // ---------- fetching ----------

  /** Buckets in playback order from the current frame, each once. */
  private bucketOrder(): { layer: string; loc: FrameLocation }[] {
    const seq = this.sequence;
    const cur = this.frame;
    const i = cur ? seq.findIndex((r) => r.layer === cur.layer && r.timestamp === cur.timestamp) : -1;
    const refs = i >= 0 ? [...seq.slice(i), ...seq.slice(0, i)] : cur ? [cur, ...seq] : seq;
    const seen = new Set<string>();
    const out: { layer: string; loc: FrameLocation }[] = [];
    for (const r of refs) {
      const loc = this.locate(r.layer, r.timestamp);
      if (!loc || seen.has(`${r.layer}|${loc.bucket}`)) continue;
      seen.add(`${r.layer}|${loc.bucket}`);
      out.push({ layer: r.layer, loc });
    }
    return out;
  }

  /** Recomputes which atlases the view and loop need, aborts the rest, and starts fetches. */
  private schedule() {
    if (!this.map || this.failed) return;
    const now = Date.now();
    this.wanted = new Set();
    this.queue = [];
    let budget = 0;
    for (const { layer, loc } of this.bucketOrder()) {
      if (budget > PREFETCH_BYTES) break;
      const meta = this.metas[layer];
      const tiles = this.cover(meta);
      const { width, height } = atlasSize(loc.slots, meta.cols, meta.tile_size);
      budget += width * height * 4 * tiles.length;
      for (const t of tiles) {
        const key = tileKey(layer, loc.bucket, t);
        this.wanted.add(key);
        const e = this.entries.get(key);
        if (e && (e.retryAt === undefined ? e.slots >= loc.slots : e.retryAt > now)) continue;
        if (this.inflight.has(key)) continue;
        this.queue.push({ key, url: packedTileUrl(meta.url_template, layer, loc.bucket, t, loc.v), slots: loc.slots });
      }
    }
    for (const [key, ctrl] of this.inflight) {
      if (this.wanted.has(key)) continue;
      ctrl.abort();
      this.inflight.delete(key);
    }
    this.pump();
  }

  private pump() {
    while (this.inflight.size < CONCURRENCY && this.queue.length) {
      const req = this.queue.shift()!;
      if (this.wanted.has(req.key) && !this.inflight.has(req.key)) void this.fetchTile(req);
    }
  }

  private async fetchTile(req: Request) {
    const ctrl = new AbortController();
    this.inflight.set(req.key, ctrl);
    try {
      const res = await fetch(req.url, { signal: ctrl.signal });
      if (res.status === 204) this.store(req, null);
      else if (!res.ok) throw new Error(`${req.url}: ${res.status}`);
      else {
        const bitmap = await createImageBitmap(await res.blob(), { premultiplyAlpha: "none", colorSpaceConversion: "none" });
        if (ctrl.signal.aborted) bitmap.close();
        else this.uploads.push({ req, bitmap });
      }
    } catch (err) {
      if (!ctrl.signal.aborted) {
        console.warn("packed radar tile failed", err);
        this.store(req, null, Date.now() + RETRY_MS);
      }
    } finally {
      if (this.inflight.get(req.key) === ctrl) this.inflight.delete(req.key);
      this.pump();
      this.repaint();
    }
  }

  /** GL work happens only inside render, where MapLibre resets its cached GL state afterwards. */
  private uploadPending(gl: WebGL2RenderingContext) {
    const batch = this.uploads.splice(0, UPLOADS_PER_FRAME);
    for (const { req, bitmap } of batch) {
      const tex = gl.createTexture();
      gl.bindTexture(gl.TEXTURE_2D, tex);
      // RGB8 keeps all three slots; ImageBitmap uploads ignore the UNPACK_* premultiply/flip flags.
      gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGB8, gl.RGB, gl.UNSIGNED_BYTE, bitmap);
      setNearest(gl);
      this.store(req, tex, undefined, bitmap.width * bitmap.height * 4);
      bitmap.close();
    }
    if (this.uploads.length) this.repaint();
    if (batch.length) this.evict(gl);
  }

  private store(req: Request, tex: WebGLTexture | null, retryAt?: number, bytes = 0) {
    const old = this.entries.get(req.key);
    // A failed refetch keeps the older atlas, which still holds its slots.
    if (old?.tex && !tex) {
      if (retryAt !== undefined) {
        this.entries.delete(req.key);
        this.entries.set(req.key, { ...old, retryAt });
        return;
      }
    }
    if (old?.tex) {
      this.gl?.deleteTexture(old.tex);
      this.bytes -= old.bytes;
    }
    this.entries.delete(req.key);
    this.entries.set(req.key, { tex, slots: req.slots, bytes, retryAt });
    this.bytes += bytes;
  }

  private evict(gl: WebGL2RenderingContext) {
    for (const [key, e] of this.entries) {
      if (this.bytes <= MAX_BYTES && this.entries.size <= MAX_ENTRIES) break;
      if (this.wanted.has(key)) continue;
      if (e.tex) gl.deleteTexture(e.tex);
      this.bytes -= e.bytes;
      this.entries.delete(key);
    }
  }
}

function setNearest(gl: WebGL2RenderingContext) {
  gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, gl.NEAREST);
  gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, gl.NEAREST);
  gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_S, gl.CLAMP_TO_EDGE);
  gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_T, gl.CLAMP_TO_EDGE);
}

function linkProgram(gl: WebGL2RenderingContext, vs: string, fs: string): WebGLProgram {
  const compile = (type: number, src: string) => {
    const s = gl.createShader(type)!;
    gl.shaderSource(s, src);
    gl.compileShader(s);
    if (!gl.getShaderParameter(s, gl.COMPILE_STATUS)) throw new Error(gl.getShaderInfoLog(s) ?? "shader compile failed");
    return s;
  };
  const p = gl.createProgram();
  const v = compile(gl.VERTEX_SHADER, vs);
  const f = compile(gl.FRAGMENT_SHADER, fs);
  gl.attachShader(p, v);
  gl.attachShader(p, f);
  gl.linkProgram(p);
  gl.deleteShader(v);
  gl.deleteShader(f);
  if (!gl.getProgramParameter(p, gl.LINK_STATUS)) throw new Error(gl.getProgramInfoLog(p) ?? "program link failed");
  return p;
}
