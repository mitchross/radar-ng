/**
 * Vector overlays drawn above the radar: SPC severe outlook areas, NWS warning
 * polygons, tracked storm cells, lightning and tropical cyclones. Each keeps its last payload so a map
 * style switch re-adds it without a refetch.
 */
import type { Feature, FeatureCollection } from "geojson";
import type { GeoJSONSource, Map as MLMap, MapGeoJSONFeature } from "maplibre-gl";
import { escapeHtml as esc } from "./html";
import { activeSpc, SPC_OVERLAYS, spcColorExpression, spcTagExpression, spcProduct, spcStyle } from "./shared/spc";
import { warningTagExpression } from "./shared/warningTags";
import {
  advance,
  distanceKm,
  incomingStorm,
  isTracked,
  stormConeFeatures,
  stormLayerOpacity,
  stormTrackTimeSec,
  TRACK_MINUTES,
  type IncomingStorm,
  type StormCell,
} from "./shared/stormCone";

export { advance, distanceKm, incomingStorm, isTracked };
export type { StormCell };
export type Threat = IncomingStorm;

export type OverlayId = "watches" | "discussions" | "outlook" | "warnings" | "storms" | "lightning" | "tropical";
type FC = FeatureCollection;

const FONT = ["Noto Sans Regular"];
const EMPTY: FC = { type: "FeatureCollection", features: [] };
const KMH_TO_MPH = 0.621371;

export const OVERLAYS: { id: OverlayId; name: string; hint: string; refreshMs: number }[] = [
  ...SPC_OVERLAYS,
  { id: "outlook", name: "Severe outlook", hint: "SPC day-1 thunderstorm risk areas", refreshMs: 10 * 60_000 },
  { id: "warnings", name: "Warnings", hint: "NWS storm-based warning polygons", refreshMs: 60_000 },
  { id: "storms", name: "Storm tracks", hint: "Tracked cells with 60-minute paths", refreshMs: 60_000 },
  { id: "lightning", name: "Lightning", hint: "Strikes in the last 15 minutes", refreshMs: 30_000 },
  { id: "tropical", name: "Hurricanes", hint: "NHC cones, tracks and forecast points", refreshMs: 10 * 60_000 },
];

export const WARNING_COLORS: [string, string][] = [
  ["Tornado Warning", "#ff1f3d"],
  ["Severe Thunderstorm Warning", "#ffa500"],
  ["Flash Flood Warning", "#00c853"],
  ["Flood Warning", "#2e8b57"],
  ["Special Marine Warning", "#ff8c00"],
  ["Extreme Wind Warning", "#ff00ff"],
  ["Snow Squall Warning", "#c71585"],
  ["Dust Storm Warning", "#c8a064"],
  ["Flood Advisory", "#00ff7f"],
  ["Special Weather Statement", "#f5deb3"],
];
const warningColor = ["match", ["get", "event"], ...WARNING_COLORS.flat(), "#9aa4be"] as unknown as string;

// ---------- data shaping (pure; unit tested) ----------

export function parseStorms(fc: FC): StormCell[] {
  return fc.features.flatMap((f) => {
    if (f.geometry.type !== "Point") return [];
    const p = (f.properties ?? {}) as Record<string, any>;
    const v = p.tracking_vector ?? {};
    const [lon, lat] = f.geometry.coordinates;
    return [{
      id: Number(p.cell_id),
      lon,
      lat,
      peakDbz: Number(p.peak_dbz ?? 0),
      areaKm2: Number(p.area_km2 ?? 0),
      speedMph: Number(v.speed_kmh ?? 0) * KMH_TO_MPH,
      bearing: Number(v.bearing_deg ?? 0),
      confidence: Number(p.tracking_confidence ?? 0),
      eastKmh: Number(v.east_kmh ?? 0),
      northKmh: Number(v.north_kmh ?? 0),
      history: Array.isArray(p.track_history) ? p.track_history : [],
    }];
  });
}

export function stormGeoJSON(cells: StormCell[]): { cells: FC; tracks: FC; ticks: FC; past: FC; cones: FC } {
  const tracks: Feature[] = [];
  const ticks: Feature[] = [];
  const past: Feature[] = [];
  for (const c of cells) {
    if (c.history.length >= 2) {
      past.push({ type: "Feature", properties: {}, geometry: { type: "LineString", coordinates: c.history.map((h) => [h[1], h[2]]) } });
    }
    if (!isTracked(c)) continue;
    const pts = [[c.lon, c.lat], ...TRACK_MINUTES.map((m) => advance(c.lon, c.lat, c.eastKmh, c.northKmh, m))];
    tracks.push({ type: "Feature", properties: { severe: c.peakDbz >= 55 }, geometry: { type: "LineString", coordinates: pts } });
    TRACK_MINUTES.forEach((m, i) =>
      ticks.push({ type: "Feature", properties: { label: String(m) }, geometry: { type: "Point", coordinates: pts[i + 1] } }),
    );
  }
  return {
    cells: {
      type: "FeatureCollection",
      features: cells.map((c) => ({
        type: "Feature",
        properties: { id: c.id, dbz: c.peakDbz, label: `${Math.round(c.peakDbz)}` },
        geometry: { type: "Point", coordinates: [c.lon, c.lat] },
      })),
    },
    tracks: { type: "FeatureCollection", features: tracks },
    ticks: { type: "FeatureCollection", features: ticks },
    past: { type: "FeatureCollection", features: past },
    cones: { type: "FeatureCollection", features: stormConeFeatures(cells) as Feature[] },
  };
}

export function lightningWithAge(fc: FC, nowSec: number): FC {
  return {
    type: "FeatureCollection",
    features: fc.features.map((f) => ({
      ...f,
      properties: { ...(f.properties ?? {}), age_s: Math.max(0, nowSec - Number(f.properties?.time ?? nowSec)) },
    })),
  };
}

export const compass = (deg: number) => ["N", "NE", "E", "SE", "S", "SW", "W", "NW"][Math.round(((deg % 360) + 360) % 360 / 45) % 8];

// ---------- map wiring ----------

const SOURCES: Record<OverlayId, string[]> = {
  watches: ["ov-watches"],
  discussions: ["ov-discussions"],
  outlook: ["ov-outlook"],
  warnings: ["ov-warnings"],
  storms: ["ov-storm-cones", "ov-storm-past", "ov-storm-tracks", "ov-storm-ticks", "ov-storm-cells"],
  lightning: ["ov-lightning"],
  tropical: ["ov-tropical"],
};

/** Storm layers that fade together: [paint property, full-strength value] per layer. */
const STORM_FADE: Record<string, [string, number][]> = {
  "ov-storm-cone-fill": [["fill-opacity", 0.16]],
  "ov-storm-cone-line": [["line-opacity", 0.8]],
  "ov-storm-past": [["line-opacity", 1]],
  "ov-storm-track-casing": [["line-opacity", 0.6]],
  "ov-storm-tracks": [["line-opacity", 1]],
  "ov-storm-ticks": [["circle-opacity", 1], ["circle-stroke-opacity", 1]],
  "ov-storm-tick-labels": [["text-opacity", 1]],
  "ov-storm-cells": [["circle-opacity", 1], ["circle-stroke-opacity", 1]],
  "ov-storm-labels": [["text-opacity", 1]],
};

export class Overlays {
  private data: Record<string, FC> = {};
  private timers = new Map<OverlayId, number>();
  private pulse = 0;
  cells: StormCell[] = [];
  /** Epoch seconds of the scan the tracks were built from. */
  trackSec: number | null = null;
  private frameSec: number | null = null;
  onStorms: () => void = () => {};
  /** Replay shows only its own warnings; every live feed stops. */
  private replay = false;

  constructor(private map: MLMap, public enabled: Set<OverlayId>) {}

  /** (Re)add every source and layer; call after each style.load. */
  install() {
    for (const ids of Object.values(SOURCES)) for (const id of ids) this.map.addSource(id, { type: "geojson", data: this.data[id] ?? EMPTY });
    const m = this.map;
    const vis = (id: OverlayId) => ((this.replay ? id === "warnings" : this.enabled.has(id)) ? "visible" : "none") as "visible" | "none";

    // SPC ships its own category colors; under the warnings, which are more urgent.
    m.addLayer({ id: "ov-outlook-fill", type: "fill", source: "ov-outlook", layout: { visibility: vis("outlook") }, paint: { "fill-color": ["coalesce", ["get", "fill"], "#c1e9c1"] as never, "fill-opacity": 0.28 } });
    m.addLayer({ id: "ov-outlook-line", type: "line", source: "ov-outlook", layout: { visibility: vis("outlook") }, paint: { "line-color": ["coalesce", ["get", "stroke"], "#55bb55"] as never, "line-width": 1.5 } });
    for (const { id } of SPC_OVERLAYS) {
      m.addLayer({ id: `ov-${id}-hit`, type: "fill", source: `ov-${id}`, layout: { visibility: vis(id) }, paint: { "fill-opacity": 0 } });
      m.addLayer({ id: `ov-${id}-line`, type: "line", source: `ov-${id}`, layout: { visibility: vis(id) }, paint: { "line-color": spcColorExpression() as never, "line-width": 2.5 } });
      m.addLayer({ id: `ov-${id}-labels`, type: "symbol", source: `ov-${id}`, layout: { visibility: vis(id), "text-field": spcTagExpression() as never, "text-font": FONT, "text-size": 12, "text-padding": 4 }, paint: { "text-color": "#ffffff", "text-halo-color": "#111827", "text-halo-width": 1.6 } });
    }
    m.addLayer({ id: "ov-warnings-fill", type: "fill", source: "ov-warnings", layout: { visibility: vis("warnings") }, paint: { "fill-color": warningColor, "fill-opacity": 0.14 } });
    m.addLayer({ id: "ov-warnings-line", type: "line", source: "ov-warnings", layout: { visibility: vis("warnings") }, paint: { "line-color": warningColor, "line-width": ["interpolate", ["linear"], ["zoom"], 4, 1.5, 9, 3] } });
    // Type tag (TOR/SVR/FFW...) so colour is not the only signal; hidden when zoomed far out to avoid clutter.
    m.addLayer({ id: "ov-warnings-labels", type: "symbol", source: "ov-warnings", minzoom: 5.5, layout: { visibility: vis("warnings"), "text-field": warningTagExpression() as never, "text-font": FONT, "text-size": 12, "text-padding": 4, "symbol-placement": "point" }, paint: { "text-color": "#ffffff", "text-halo-color": "#111827", "text-halo-width": 1.6 } });

    m.addLayer({ id: "ov-tropical-cone", type: "fill", source: "ov-tropical", filter: ["==", ["get", "kind"], "cone"], layout: { visibility: vis("tropical") }, paint: { "fill-color": "#ffffff", "fill-opacity": 0.22 } });
    m.addLayer({ id: "ov-tropical-cone-line", type: "line", source: "ov-tropical", filter: ["==", ["get", "kind"], "cone"], layout: { visibility: vis("tropical") }, paint: { "line-color": "#ffffff", "line-width": 1.5, "line-dasharray": [3, 2] } });
    m.addLayer({ id: "ov-tropical-track", type: "line", source: "ov-tropical", filter: ["==", ["get", "kind"], "track"], layout: { visibility: vis("tropical"), "line-cap": "round" }, paint: { "line-color": "#111827", "line-width": 2.5 } });
    const catColor = ["step", ["coalesce", ["get", "category"], 0], "#4aa3ff", 1, "#ffd23f", 2, "#ff9f1c", 3, "#ff4d4d", 4, "#d61f69", 5, "#9b30ff"];
    m.addLayer({ id: "ov-tropical-points", type: "circle", source: "ov-tropical", filter: ["==", ["get", "kind"], "forecast_point"], layout: { visibility: vis("tropical") }, paint: { "circle-radius": 7, "circle-color": catColor as never, "circle-stroke-color": "#111827", "circle-stroke-width": 1.5 } });
    m.addLayer({ id: "ov-tropical-point-labels", type: "symbol", source: "ov-tropical", filter: ["==", ["get", "kind"], "forecast_point"], layout: { visibility: vis("tropical"), "text-field": ["coalesce", ["to-string", ["get", "category"]], ["get", "storm_type"], ""], "text-font": FONT, "text-size": 10, "text-allow-overlap": true }, paint: { "text-color": "#111827" } });
    m.addLayer({ id: "ov-tropical-position", type: "circle", source: "ov-tropical", filter: ["==", ["get", "kind"], "position"], layout: { visibility: vis("tropical") }, paint: { "circle-radius": 11, "circle-color": catColor as never, "circle-stroke-color": "#ffffff", "circle-stroke-width": 3 } });
    m.addLayer({ id: "ov-tropical-name", type: "symbol", source: "ov-tropical", filter: ["==", ["get", "kind"], "position"], layout: { visibility: vis("tropical"), "text-field": ["concat", ["get", "name"], ["case", ["to-boolean", ["get", "category"]], ["concat", " · Cat ", ["to-string", ["get", "category"]]], ""]], "text-font": FONT, "text-size": 13, "text-offset": [0, 1.6], "text-anchor": "top" }, paint: { "text-color": "#ffffff", "text-halo-color": "#111827", "text-halo-width": 1.6 } });

    m.addLayer({ id: "ov-storm-cone-fill", type: "fill", source: "ov-storm-cones", minzoom: 5, layout: { visibility: vis("storms") }, paint: { "fill-color": ["case", ["get", "severe"], "#ff3b6b", "#ffffff"] as never, "fill-opacity": 0.16 } });
    m.addLayer({ id: "ov-storm-cone-line", type: "line", source: "ov-storm-cones", minzoom: 5, layout: { visibility: vis("storms") }, paint: { "line-color": ["case", ["get", "severe"], "#ff3b6b", "#ffffff"] as never, "line-width": 1, "line-dasharray": [3, 2], "line-opacity": 0.8 } });
    m.addLayer({ id: "ov-storm-past", type: "line", source: "ov-storm-past", minzoom: 6, layout: { visibility: vis("storms") }, paint: { "line-color": "#6b7280", "line-width": 1.5, "line-dasharray": [1, 1.5] } });
    m.addLayer({ id: "ov-storm-track-casing", type: "line", source: "ov-storm-tracks", minzoom: 5, layout: { visibility: vis("storms"), "line-cap": "round" }, paint: { "line-color": "#111827", "line-width": 4.5, "line-opacity": 0.6 } });
    m.addLayer({ id: "ov-storm-tracks", type: "line", source: "ov-storm-tracks", minzoom: 5, layout: { visibility: vis("storms"), "line-cap": "round" }, paint: { "line-color": ["case", ["get", "severe"], "#ff3b6b", "#ffffff"], "line-width": 2 } });
    m.addLayer({ id: "ov-storm-ticks", type: "circle", source: "ov-storm-ticks", minzoom: 6, layout: { visibility: vis("storms") }, paint: { "circle-radius": 3, "circle-color": "#ffffff", "circle-stroke-color": "#111827", "circle-stroke-width": 1 } });
    m.addLayer({ id: "ov-storm-tick-labels", type: "symbol", source: "ov-storm-ticks", minzoom: 7, layout: { visibility: vis("storms"), "text-field": ["get", "label"], "text-font": FONT, "text-size": 10, "text-offset": [0, 1], "text-anchor": "top" }, paint: { "text-color": "#ffffff", "text-halo-color": "#111827", "text-halo-width": 1.2 } });
    const dbzColor = ["step", ["get", "dbz"], "#ffe14d", 50, "#ff8a00", 55, "#ff2d2d", 60, "#ff3bff"];
    // Continental view shows only severe cores; every cell appears from regional zoom.
    m.addLayer({ id: "ov-storm-cells", type: "circle", source: "ov-storm-cells", filter: ["any", [">=", ["get", "dbz"], 55], [">=", ["zoom"], 6]] as never, layout: { visibility: vis("storms") }, paint: { "circle-radius": ["interpolate", ["linear"], ["zoom"], 4, 3, 8, 7], "circle-color": dbzColor as never, "circle-stroke-color": "#111827", "circle-stroke-width": 1.5 } });
    m.addLayer({ id: "ov-storm-labels", type: "symbol", source: "ov-storm-cells", minzoom: 6.5, filter: [">=", ["get", "dbz"], 50], layout: { visibility: vis("storms"), "text-field": ["get", "label"], "text-font": FONT, "text-size": 11, "text-offset": [0, -1.3] }, paint: { "text-color": "#ffffff", "text-halo-color": "#111827", "text-halo-width": 1.4 } });

    const ageColor = ["step", ["get", "age_s"], "#ffffff", 120, "#ffe14d", 300, "#ff9f1c", 600, "#ff4d4d"];
    m.addLayer({ id: "ov-lightning-pulse", type: "circle", source: "ov-lightning", filter: ["<", ["get", "age_s"], 90], layout: { visibility: vis("lightning") }, paint: { "circle-radius": 10, "circle-color": "rgba(0,0,0,0)", "circle-stroke-color": "#ffffff", "circle-stroke-width": 1.5, "circle-stroke-opacity": 0.8 } });
    m.addLayer({ id: "ov-lightning", type: "circle", source: "ov-lightning", layout: { visibility: vis("lightning") }, paint: { "circle-radius": ["interpolate", ["linear"], ["zoom"], 4, 1.8, 9, 4], "circle-color": ageColor as never, "circle-stroke-color": "#111827", "circle-stroke-width": 0.8, "circle-opacity": ["interpolate", ["linear"], ["get", "age_s"], 0, 1, 900, 0.35] } });
    this.applyStormFade();
  }

  /** Layer ids a click should hit, most specific first. */
  clickable(): string[] {
    const ids: string[] = [];
    if (this.replay) return this.map.getLayer("ov-warnings-fill") ? ["ov-warnings-fill"] : [];
    if (this.enabled.has("tropical")) ids.push("ov-tropical-position", "ov-tropical-points");
    if (this.enabled.has("storms")) ids.push("ov-storm-cells");
    if (this.enabled.has("warnings")) ids.push("ov-warnings-fill");
    for (const { id } of SPC_OVERLAYS) if (this.enabled.has(id)) ids.push(`ov-${id}-hit`);
    return ids.filter((id) => this.map.getLayer(id));
  }

  /** Enter or leave replay: hide live overlays, drop their data, restart live feeds on leaving. */
  setReplay(on: boolean) {
    this.replay = on;
    for (const t of this.timers.values()) window.clearInterval(t);
    this.timers.clear();
    for (const ids of Object.values(SOURCES)) for (const id of ids) this.set(id, EMPTY);
    this.cells = [];
    this.trackSec = null;
    for (const o of OVERLAYS) this.applyVisibility(o.id);
    this.onStorms();
    if (!on) this.start(false);
  }

  /** Replay warnings in force at the playhead, in the live warning shape. */
  setReplayWarnings(fc: FC) {
    this.set("ov-warnings", fc);
  }

  private applyVisibility(id: OverlayId) {
    const show = this.replay ? id === "warnings" : this.enabled.has(id);
    for (const layer of this.map.getStyle().layers ?? []) {
      if (layer.id.startsWith(`ov-${id === "storms" ? "storm" : id}`)) this.map.setLayoutProperty(layer.id, "visibility", show ? "visible" : "none");
    }
  }

  setEnabled(id: OverlayId, on: boolean) {
    on ? this.enabled.add(id) : this.enabled.delete(id);
    this.applyVisibility(id);
    this.schedule(id);
  }

  start(first = true) {
    for (const o of OVERLAYS) this.schedule(o.id);
    if (!first) return;
    this.animatePulse();
    window.setInterval(() => this.expireSpc(), 15_000);
  }

  private schedule(id: OverlayId) {
    if (this.replay) return;
    window.clearInterval(this.timers.get(id));
    this.timers.delete(id);
    // Storms also feed the "incoming storm" card, so they refresh even when hidden.
    if (!this.enabled.has(id) && id !== "storms") return;
    const spec = OVERLAYS.find((o) => o.id === id)!;
    void this.refresh(id);
    this.timers.set(id, window.setInterval(() => void this.refresh(id), spec.refreshMs));
  }

  /** Tell the storm layers which radar frame is on screen; they fade when it is far from the track time. */
  setFrameTime(sec: number | null) {
    this.frameSec = sec;
    this.applyStormFade();
  }

  private applyStormFade() {
    const k = stormLayerOpacity(this.frameSec, this.trackSec);
    for (const [layer, props] of Object.entries(STORM_FADE)) {
      if (!this.map.getLayer(layer)) continue;
      for (const [prop, base] of props) this.map.setPaintProperty(layer, prop as never, (base * k) as never);
    }
  }

  private set(id: string, fc: FC) {
    this.data[id] = fc;
    (this.map.getSource(id) as GeoJSONSource | undefined)?.setData(fc);
  }

  private expireSpc() {
    for (const { id } of SPC_OVERLAYS) {
      const key = `ov-${id}`;
      if (this.data[key]) this.set(key, activeSpc(this.data[key], Date.now()));
    }
  }

  async refresh(id: OverlayId) {
    if (this.replay) return;
    this.expireSpc();
    const url = { watches: "/api/spc/watches", discussions: "/api/spc/discussions", outlook: "/api/outlooks?day=1", warnings: "/api/alerts/map", storms: "/api/storms", lightning: "/api/lightning", tropical: "/api/tropical" }[id];
    let fc: FC;
    try {
      const res = await fetch(url);
      if (this.replay) return;
      if (!res.ok) return;
      fc = (await res.json()) as FC;
      if (!Array.isArray(fc.features)) return;
    } catch {
      return;
    }
    if (id === "storms") {
      this.cells = parseStorms(fc);
      const g = stormGeoJSON(this.cells);
      this.set("ov-storm-cells", g.cells);
      this.set("ov-storm-tracks", g.tracks);
      this.set("ov-storm-ticks", g.ticks);
      this.set("ov-storm-past", g.past);
      this.set("ov-storm-cones", g.cones);
      this.trackSec = stormTrackTimeSec(fc as { timestamp?: unknown });
      this.applyStormFade();
      this.onStorms();
    } else if (id === "lightning") {
      this.set("ov-lightning", lightningWithAge(fc, Date.now() / 1000));
    } else {
      this.set(SOURCES[id][0], id === "watches" || id === "discussions" ? activeSpc(fc, Date.now()) : fc);
    }
  }

  private animatePulse() {
    const tick = () => {
      if (this.enabled.has("lightning") && this.map.getLayer("ov-lightning-pulse")) {
        this.pulse = (this.pulse + 0.035) % 1;
        this.map.setPaintProperty("ov-lightning-pulse", "circle-radius", 4 + this.pulse * 14);
        this.map.setPaintProperty("ov-lightning-pulse", "circle-stroke-opacity", 0.9 * (1 - this.pulse));
      }
      window.setTimeout(() => requestAnimationFrame(tick), 50);
    };
    requestAnimationFrame(tick);
  }
}

// ---------- popups ----------

const until = (iso: unknown) => (iso ? new Date(String(iso)).toLocaleString([], { weekday: "short", hour: "numeric", minute: "2-digit" }) : "");

export function describeFeature(f: MapGeoJSONFeature, cells: StormCell[]): string {
  const p = f.properties as Record<string, any>;
  if (f.layer.id.startsWith("ov-watches-") || f.layer.id.startsWith("ov-discussions-")) {
    const product = spcProduct(p, Date.now());
    if (!product) return "";
    const style = spcStyle(product.type);
    const rows = product.probabilities.map((r) => `<tr><td>${esc(r.label)}</td><td>${esc(r.value)}</td></tr>`).join("");
    return `<div class="readout spc-detail"><b style="color:${style.color}">${esc(style.name)} #${product.number}</b><small>Issued ${esc(until(product.issued))} · Until ${esc(until(product.expires))}</small><p>${esc(product.geometry_note)}</p><pre>${esc(product.text)}</pre>${rows ? `<table aria-label="Watch probabilities">${rows}</table>` : ""}<small>NOAA/NWS Storm Prediction Center</small></div>`;
  }
  if (f.layer.id === "ov-storm-cells") {
    const c = cells.find((x) => x.id === Number(p.id));
    if (!c) return "";
    const motion = isTracked(c) ? `Moving ${compass(c.bearing)} at ${Math.round(c.speedMph)} mph` : "Motion not yet tracked";
    const hail = c.peakDbz >= 60 ? '<div class="pill danger">Hail likely</div>' : c.peakDbz >= 55 ? '<div class="pill warn">Hail possible</div>' : "";
    return `<div class="readout"><b>STORM CELL</b><div class="v">${Math.round(c.peakDbz)} dBZ peak</div>${hail}<small>${motion} · ${Math.round(c.areaKm2).toLocaleString()} km²</small></div>`;
  }
  if (f.layer.id.startsWith("ov-warnings")) {
    const color = WARNING_COLORS.find(([e]) => e === p.event)?.[1] ?? "#9aa4be";
    return `<div class="readout"><b style="color:${color}">${esc(p.event).toUpperCase()}</b><div class="v">${esc(p.areaDesc).split(";").slice(0, 3).join(", ")}</div><small>${p.ends || p.expires ? `Until ${esc(until(p.ends ?? p.expires))}` : esc(p.severity)}</small></div>`;
  }
  if (f.layer.id === "ov-outlook-fill") {
    return `<div class="readout"><b style="color:${esc(p.stroke)}">SEVERE OUTLOOK · ${esc(p.label)}</b><div class="v">${esc(p.name)}</div><small>Today · until ${esc(until(p.expire))}</small></div>`;
  }
  if (f.layer.id === "ov-tropical-position") {
    const cat = p.category ? `Category ${esc(p.category)} hurricane` : esc(p.classification);
    const move = p.movement_mph ? ` · moving ${compass(Number(p.movement_dir_deg))} at ${Math.round(p.movement_mph)} mph` : "";
    return `<div class="readout"><b>${esc(p.name).toUpperCase()}</b><div class="v">${cat}</div><small>${esc(p.wind_mph ?? "—")} mph winds · ${p.pressure_mb ? `${Math.round(p.pressure_mb)} mb` : ""}${move}</small></div>`;
  }
  if (f.layer.id === "ov-tropical-points") {
    return `<div class="readout"><b>${esc(p.name).toUpperCase()} · +${Math.round(Number(p.tau_h ?? 0))}h</b><div class="v">${esc(p.wind_mph ?? "—")} mph</div><small>${esc(p.label)}</small></div>`;
  }
  return "";
}
