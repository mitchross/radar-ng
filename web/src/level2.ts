/** Level II close-up on the web map: one session, per-sweep raster layers over MRMS, own server only. */
import type { Map as MLMap } from "maplibre-gl";
import { escapeHtml } from "./html";
import {
  fallbackCopy, level2TileUrl, Level2Session, provenance, viewBounds,
  type Level2Frame, type Level2Palette, type Level2Snapshot, type Level2View,
} from "./shared/level2";

export interface Level2Hooks {
  palette(): Level2Palette;
  /** Observed radar is selected and no replay is showing. */
  active(): boolean;
  opacity(): number;
  /** The snapshot key changed: rebuild the timeline. */
  changed(): void;
  /** Level II tiles are laid just under the labels, above every radar frame. */
  beforeId(): string | undefined;
}

export class Level2Layer {
  readonly session: Level2Session;
  private ids = new Map<string, string>();
  private viewKey = "";
  private lastKey = "";
  private shown: string | null = null;

  constructor(private map: MLMap, private hooks: Level2Hooks) {
    this.session = new Level2Session({
      request: async (path, init) => {
        const res = await fetch(path, { method: init.method, body: init.body, signal: init.signal, headers: init.body ? { "content-type": "application/json" } : undefined });
        let json: unknown = null;
        try {
          json = await res.json();
        } catch {
          json = null;
        }
        return { status: res.status, json };
      },
    });
    this.session.subscribe(() => this.onSession());
    map.on("error", (e) => {
      const id = (e as { sourceId?: string }).sourceId;
      if (id && id.startsWith("l2-")) this.session.reportTileFailure();
    });
  }

  snapshot(): Level2Snapshot {
    return this.session.snapshot();
  }

  /** Camera, palette or layer changed. */
  sync(): void {
    const b = this.map.getBounds();
    const c = this.map.getCenter();
    this.session.update({
      centre: { lat: c.lat, lon: c.lng },
      zoom: this.map.getZoom(),
      bounds: [b.getWest(), b.getSouth(), b.getEast(), b.getNorth()],
      palette: this.hooks.palette(),
      active: this.hooks.active(),
    });
  }

  private onSession() {
    const snap = this.session.snapshot();
    if (snap.key === this.lastKey) return;
    const next = snap.key.split("|");
    const prev = this.lastKey.split("|");
    this.lastKey = snap.key;
    // Only the fade changed (index 2): repaint without rebuilding the timeline.
    if (prev.length === next.length && next.every((p, i) => i === 2 || p === prev[i])) this.paint();
    else this.hooks.changed();
  }

  /** Make exactly the current view's frames exist as layers, at opacity 0. */
  private ensure(view: Level2View) {
    const key = `${view.viewId}.${view.revision}`;
    if (key === this.viewKey && view.frames.every((f) => this.ids.has(f.path) && this.map.getLayer(this.ids.get(f.path)!))) return;
    this.clear();
    this.viewKey = key;
    const bounds = viewBounds(view);
    view.frames.forEach((f, i) => {
      const id = `l2-${key}-${i}`;
      const url = `${location.origin}${level2TileUrl(view, f)}`;
      this.map.addSource(id, { type: "raster", tiles: [url], tileSize: 256, minzoom: view.sourceZoom, maxzoom: view.sourceZoom, bounds });
      this.map.addLayer(
        { id, type: "raster", source: id, paint: { "raster-opacity": 0, "raster-fade-duration": 0, "raster-resampling": "linear" } },
        this.hooks.beforeId(),
      );
      this.ids.set(f.path, id);
    });
  }

  /** Show one sweep (or none) over the MRMS frame; every other sweep keeps prefetching at 0. */
  show(frame: Level2Frame | null): void {
    const snap = this.session.snapshot();
    if (!snap.view) {
      this.clear();
      return;
    }
    this.ensure(snap.view);
    for (const id of this.ids.values()) if (this.map.getLayer(id)) this.map.moveLayer(id, this.hooks.beforeId());
    this.shown = frame?.path ?? null;
    this.paint();
  }

  private paint() {
    const snap = this.session.snapshot();
    for (const [path, id] of this.ids) {
      if (this.map.getLayer(id)) this.map.setPaintProperty(id, "raster-opacity", path === this.shown && snap.view ? this.hooks.opacity() * snap.opacity : 0);
    }
  }

  clear(): void {
    for (const id of this.ids.values()) {
      if (this.map.getLayer(id)) this.map.removeLayer(id);
      if (this.map.getSource(id)) this.map.removeSource(id);
    }
    this.ids.clear();
    this.viewKey = "";
    this.shown = null;
  }

  /** "MRMS · Level II unavailable" with a short reason, or null while nothing is wrong. */
  fallback(): { title: string; reason: string } | null {
    const snap = this.session.snapshot();
    return snap.unavailable ? fallbackCopy(snap.reason) : null;
  }

  /** One line plus a details list for the selected sweep at the map centre. */
  provenanceHtml(frame: Level2Frame, timeZone?: string): { line: string; range: string; panel: string } {
    const c = this.map.getCenter();
    const p = provenance(frame, { lat: c.lat, lon: c.lng }, timeZone);
    return {
      line: p.headline,
      range: p.groundRange,
      panel: p.rows.map((r) => `<div><dt>${escapeHtml(r.label)}</dt><dd>${escapeHtml(r.value)}</dd></div>`).join(""),
    };
  }
}
