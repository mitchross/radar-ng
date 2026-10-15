/**
 * The GOES satellite layer: one raster layer per frame, drawn under the radar.
 * The frame shown is the one at or before the timeline time; frames ahead in
 * playback load at opacity 0, as the radar frames do.
 */
import type { Map as MLMap } from "maplibre-gl";
import { SATELLITE_OPACITY, satelliteIndexAt, satelliteTileUrl, type SatelliteFrame } from "./shared/satellite";

const MAX_CACHED = 12;

export class SatelliteLayer {
  enabled: boolean;
  private frames: SatelliteFrame[] = [];
  /** Layer ids keyed by frame path; most recently used last. */
  private cached = new Map<string, string>();
  private shown: string | null = null;

  constructor(
    private readonly map: MLMap,
    enabled: boolean,
    /** The id to insert under: the lowest radar layer, or the first label layer. */
    private readonly beforeId: () => string | undefined,
  ) {
    this.enabled = enabled;
  }

  get available(): boolean {
    return this.frames.length > 0;
  }

  setFrames(frames: SatelliteFrame[]) {
    this.frames = frames;
  }

  setEnabled(on: boolean) {
    this.enabled = on;
    if (!on) this.clear();
  }

  /** Show the frame for `timeSec` and preload the frames for `aheadSec`. Call only while the style is loaded. */
  show(timeSec: number, aheadSec: readonly number[]) {
    if (!this.enabled) return;
    const frame = this.frames[satelliteIndexAt(this.frames, timeSec)];
    const keep = new Set<string>();
    for (const t of aheadSec) {
      const f = this.frames[satelliteIndexAt(this.frames, t)];
      if (f) {
        this.ensure(f);
        keep.add(f.path);
      }
    }
    const id = frame ? this.ensure(frame) : null;
    if (frame) keep.add(frame.path);
    if (id) this.map.setPaintProperty(id, "raster-opacity", SATELLITE_OPACITY);
    if (this.shown && this.shown !== id && this.map.getLayer(this.shown)) this.map.setPaintProperty(this.shown, "raster-opacity", 0);
    this.shown = id;
    this.evict(keep);
  }

  /** True when the frame for `timeSec` has its tiles, or nothing is to be drawn. */
  isReady(timeSec: number): boolean {
    const frame = this.enabled ? this.frames[satelliteIndexAt(this.frames, timeSec)] : undefined;
    const id = frame && this.cached.get(frame.path);
    return !id || this.map.isSourceLoaded(id);
  }

  /** Forget every layer, e.g. before a style switch removes them. */
  clear() {
    for (const id of this.cached.values()) {
      if (this.map.getLayer(id)) this.map.removeLayer(id);
      if (this.map.getSource(id)) this.map.removeSource(id);
    }
    this.cached.clear();
    this.shown = null;
  }

  private ensure(f: SatelliteFrame): string {
    const existing = this.cached.get(f.path);
    if (existing) {
      this.cached.delete(f.path);
      this.cached.set(f.path, existing);
      return existing;
    }
    const id = `sat-${Math.random().toString(36).slice(2, 10)}`;
    this.map.addSource(id, {
      type: "raster",
      tiles: [satelliteTileUrl(location.origin, f.path)],
      tileSize: 256,
      maxzoom: f.maxZoom,
      attribution: "GOES imagery: NASA GIBS, NOAA",
    });
    this.map.addLayer(
      {
        id,
        type: "raster",
        source: id,
        paint: { "raster-opacity": 0, "raster-fade-duration": 0, "raster-resampling": "linear" },
      },
      this.beforeId(),
    );
    this.cached.set(f.path, id);
    return id;
  }

  private evict(keep: Set<string>) {
    for (const [path, id] of this.cached) {
      if (this.cached.size <= MAX_CACHED) break;
      if (keep.has(path)) continue;
      this.map.removeLayer(id);
      this.map.removeSource(id);
      this.cached.delete(path);
    }
  }
}
