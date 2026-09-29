/**
 * Vector overlays drawn above the radar: NWS warning polygons, tracked storm
 * cells, lightning and tropical cyclones. Each keeps its last payload so a map
 * style switch re-adds it without a refetch.
 */
import type { Feature, FeatureCollection } from "geojson";
import type { GeoJSONSource, Map as MLMap, MapGeoJSONFeature } from "maplibre-gl";

export type OverlayId = "warnings" | "storms" | "lightning" | "tropical";
type FC = FeatureCollection;

const FONT = ["Noto Sans Regular"];
const EMPTY: FC = { type: "FeatureCollection", features: [] };
const KMH_TO_MPH = 0.621371;
const TRACK_MINUTES = [15, 30, 45, 60];
const MIN_TRACK_CONFIDENCE = 0.4;

export const OVERLAYS: { id: OverlayId; name: string; hint: string; refreshMs: number }[] = [
  { id: "warnings", name: "Warnings", hint: "NWS storm-based warning polygons", refreshMs: 60_000 },
  { id: "storms", name: "Storm tracks", hint: "Tracked cells with 60-minute paths", refreshMs: 60_000 },
  { id: "lightning", name: "Lightning", hint: "Strikes in the last 15 minutes", refreshMs: 30_000 },
  { id: "tropical", name: "Hurricanes", hint: "NHC cones, tracks and forecast points", refreshMs: 10 * 60_000 },
];

const WARNING_COLORS: [string, string][] = [
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

export interface StormCell {
  id: number;
  lon: number;
  lat: number;
  peakDbz: number;
  areaKm2: number;
  speedMph: number;
  bearing: number;
  confidence: number;
  eastKmh: number;
  northKmh: number;
  history: [number, number, number][];
}

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

export function advance(lon: number, lat: number, eastKmh: number, northKmh: number, minutes: number): [number, number] {
  const h = minutes / 60;
  return [lon + (eastKmh * h) / Math.max(1e-6, 111.32 * Math.cos((lat * Math.PI) / 180)), lat + (northKmh * h) / 110.574];
}

// History-backed fits only: one-step vectors from older servers were noise.
export const isTracked = (c: StormCell) =>
  c.history.length >= 3 && c.confidence >= MIN_TRACK_CONFIDENCE && c.speedMph >= 3 && c.speedMph <= 90;

export function stormGeoJSON(cells: StormCell[]): { cells: FC; tracks: FC; ticks: FC; past: FC } {
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
  };
}

const R_EARTH_KM = 6371.0088;
export function distanceKm(a: [number, number], b: [number, number]): number {
  const toR = Math.PI / 180;
  const dLat = (b[1] - a[1]) * toR;
  const dLon = (b[0] - a[0]) * toR;
  const h = Math.sin(dLat / 2) ** 2 + Math.cos(a[1] * toR) * Math.cos(b[1] * toR) * Math.sin(dLon / 2) ** 2;
  return 2 * R_EARTH_KM * Math.asin(Math.min(1, Math.sqrt(h)));
}

export interface Threat {
  minutes: number;
  peakDbz: number;
  speedMph: number;
  bearing: number;
}

/** The soonest tracked cell whose next-hour path passes within `radiusKm` of a place. */
export function incomingStorm(cells: StormCell[], place: [number, number], radiusKm = 12): Threat | null {
  let best: Threat | null = null;
  for (const c of cells) {
    if (distanceKm([c.lon, c.lat], place) <= radiusKm) {
      if (!best || best.minutes > 0) best = { minutes: 0, peakDbz: c.peakDbz, speedMph: c.speedMph, bearing: c.bearing };
      continue;
    }
    if (!isTracked(c)) continue;
    for (let m = 5; m <= 60; m += 5) {
      if (distanceKm(advance(c.lon, c.lat, c.eastKmh, c.northKmh, m), place) <= radiusKm) {
        if (!best || m < best.minutes) best = { minutes: m, peakDbz: c.peakDbz, speedMph: c.speedMph, bearing: c.bearing };
        break;
      }
    }
  }
  return best;
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
  warnings: ["ov-warnings"],
  storms: ["ov-storm-past", "ov-storm-tracks", "ov-storm-ticks", "ov-storm-cells"],
  lightning: ["ov-lightning"],
  tropical: ["ov-tropical"],
};

export class Overlays {
  private data: Record<string, FC> = {};
  private timers = new Map<OverlayId, number>();
  private pulse = 0;
  cells: StormCell[] = [];
  onStorms: () => void = () => {};

  constructor(private map: MLMap, public enabled: Set<OverlayId>) {}

  /** (Re)add every source and layer; call after each style.load. */
  install() {
    for (const ids of Object.values(SOURCES)) for (const id of ids) this.map.addSource(id, { type: "geojson", data: this.data[id] ?? EMPTY });
    const m = this.map;
    const vis = (id: OverlayId) => (this.enabled.has(id) ? "visible" : "none") as "visible" | "none";

    m.addLayer({ id: "ov-warnings-fill", type: "fill", source: "ov-warnings", layout: { visibility: vis("warnings") }, paint: { "fill-color": warningColor, "fill-opacity": 0.14 } });
    m.addLayer({ id: "ov-warnings-line", type: "line", source: "ov-warnings", layout: { visibility: vis("warnings") }, paint: { "line-color": warningColor, "line-width": ["interpolate", ["linear"], ["zoom"], 4, 1.5, 9, 3] } });

    m.addLayer({ id: "ov-tropical-cone", type: "fill", source: "ov-tropical", filter: ["==", ["get", "kind"], "cone"], layout: { visibility: vis("tropical") }, paint: { "fill-color": "#ffffff", "fill-opacity": 0.22 } });
    m.addLayer({ id: "ov-tropical-cone-line", type: "line", source: "ov-tropical", filter: ["==", ["get", "kind"], "cone"], layout: { visibility: vis("tropical") }, paint: { "line-color": "#ffffff", "line-width": 1.5, "line-dasharray": [3, 2] } });
    m.addLayer({ id: "ov-tropical-track", type: "line", source: "ov-tropical", filter: ["==", ["get", "kind"], "track"], layout: { visibility: vis("tropical"), "line-cap": "round" }, paint: { "line-color": "#111827", "line-width": 2.5 } });
    const catColor = ["step", ["coalesce", ["get", "category"], 0], "#4aa3ff", 1, "#ffd23f", 2, "#ff9f1c", 3, "#ff4d4d", 4, "#d61f69", 5, "#9b30ff"];
    m.addLayer({ id: "ov-tropical-points", type: "circle", source: "ov-tropical", filter: ["==", ["get", "kind"], "forecast_point"], layout: { visibility: vis("tropical") }, paint: { "circle-radius": 7, "circle-color": catColor as never, "circle-stroke-color": "#111827", "circle-stroke-width": 1.5 } });
    m.addLayer({ id: "ov-tropical-point-labels", type: "symbol", source: "ov-tropical", filter: ["==", ["get", "kind"], "forecast_point"], layout: { visibility: vis("tropical"), "text-field": ["coalesce", ["to-string", ["get", "category"]], ["get", "storm_type"], ""], "text-font": FONT, "text-size": 10, "text-allow-overlap": true }, paint: { "text-color": "#111827" } });
    m.addLayer({ id: "ov-tropical-position", type: "circle", source: "ov-tropical", filter: ["==", ["get", "kind"], "position"], layout: { visibility: vis("tropical") }, paint: { "circle-radius": 11, "circle-color": catColor as never, "circle-stroke-color": "#ffffff", "circle-stroke-width": 3 } });
    m.addLayer({ id: "ov-tropical-name", type: "symbol", source: "ov-tropical", filter: ["==", ["get", "kind"], "position"], layout: { visibility: vis("tropical"), "text-field": ["concat", ["get", "name"], ["case", ["to-boolean", ["get", "category"]], ["concat", " · Cat ", ["to-string", ["get", "category"]]], ""]], "text-font": FONT, "text-size": 13, "text-offset": [0, 1.6], "text-anchor": "top" }, paint: { "text-color": "#ffffff", "text-halo-color": "#111827", "text-halo-width": 1.6 } });

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
  }

  /** Layer ids a click should hit, most specific first. */
  clickable(): string[] {
    const ids: string[] = [];
    if (this.enabled.has("tropical")) ids.push("ov-tropical-position", "ov-tropical-points");
    if (this.enabled.has("storms")) ids.push("ov-storm-cells");
    if (this.enabled.has("warnings")) ids.push("ov-warnings-fill");
    return ids.filter((id) => this.map.getLayer(id));
  }

  setEnabled(id: OverlayId, on: boolean) {
    on ? this.enabled.add(id) : this.enabled.delete(id);
    for (const layer of this.map.getStyle().layers ?? []) {
      if (layer.id.startsWith(`ov-${id === "storms" ? "storm" : id}`)) this.map.setLayoutProperty(layer.id, "visibility", on ? "visible" : "none");
    }
    this.schedule(id);
  }

  start() {
    for (const o of OVERLAYS) this.schedule(o.id);
    this.animatePulse();
  }

  private schedule(id: OverlayId) {
    window.clearInterval(this.timers.get(id));
    this.timers.delete(id);
    // Storms also feed the "incoming storm" card, so they refresh even when hidden.
    if (!this.enabled.has(id) && id !== "storms") return;
    const spec = OVERLAYS.find((o) => o.id === id)!;
    void this.refresh(id);
    this.timers.set(id, window.setInterval(() => void this.refresh(id), spec.refreshMs));
  }

  private set(id: string, fc: FC) {
    this.data[id] = fc;
    (this.map.getSource(id) as GeoJSONSource | undefined)?.setData(fc);
  }

  async refresh(id: OverlayId) {
    const url = { warnings: "/api/alerts/map", storms: "/api/storms", lightning: "/api/lightning", tropical: "/api/tropical" }[id];
    let fc: FC;
    try {
      const res = await fetch(url);
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
      this.onStorms();
    } else if (id === "lightning") {
      this.set("ov-lightning", lightningWithAge(fc, Date.now() / 1000));
    } else {
      this.set(SOURCES[id][0], fc);
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

const esc = (s: unknown) => String(s ?? "").replace(/[&<>"']/g, (ch) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" })[ch]!);
const until = (iso: unknown) => (iso ? new Date(String(iso)).toLocaleString([], { weekday: "short", hour: "numeric", minute: "2-digit" }) : "");

export function describeFeature(f: MapGeoJSONFeature, cells: StormCell[]): string {
  const p = f.properties as Record<string, any>;
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
  if (f.layer.id === "ov-tropical-position") {
    const cat = p.category ? `Category ${p.category} hurricane` : esc(p.classification);
    const move = p.movement_mph ? ` · moving ${compass(Number(p.movement_dir_deg))} at ${Math.round(p.movement_mph)} mph` : "";
    return `<div class="readout"><b>${esc(p.name).toUpperCase()}</b><div class="v">${cat}</div><small>${p.wind_mph ?? "—"} mph winds · ${p.pressure_mb ? `${Math.round(p.pressure_mb)} mb` : ""}${move}</small></div>`;
  }
  if (f.layer.id === "ov-tropical-points") {
    return `<div class="readout"><b>${esc(p.name).toUpperCase()} · +${Math.round(Number(p.tau_h ?? 0))}h</b><div class="v">${p.wind_mph ?? "—"} mph</div><small>${esc(p.label)}</small></div>`;
  }
  return "";
}
