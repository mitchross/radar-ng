import * as maplibregl from "maplibre-gl";
import type { Map as MLMap, StyleSpecification } from "maplibre-gl";
// MapLibre picks its worker file by name at runtime, which a bundler can't
// follow; importing it explicitly bundles the worker (and its shared chunk).
import workerUrl from "maplibre-gl/dist/maplibre-gl-worker.mjs?worker&url";
import "maplibre-gl/dist/maplibre-gl.css";
import "./style.css";
import { initAnalytics, track } from "./analytics";
import { buildFrames, nowIndex, offsetLabel, playbackSequence, type Frame, type Manifest, type Zoom } from "./frames";
import { compass, describeFeature, incomingStorm, OVERLAYS, Overlays, type OverlayId } from "./overlays";
import { css, LEGEND_TICKS, PALETTES, type PaletteId } from "./palettes";
import {
  aqiLabel,
  CONDITION_LABEL,
  conditionOf,
  describeDbz,
  fetchAlerts,
  fetchForecast,
  explainAlert,
  fetchBriefing,
  fetchManifest,
  iconSvg,
  readPoint,
  reversePlace,
  searchPlaces,
  usAqiFromPm25,
  type Forecast,
  type Place,
} from "./weather";
import { WindLayer } from "./wind";
import { interpolateNowcast, nowcastVerdict } from "./nowcast";
import { describeNowcastSkill } from "./shared/nowcastSkill";
import { RAIN_ALERT_LEAD_OPTIONS } from "./shared/rainAlerts";
import type { RadarNowcastResponse } from "./shared/nowcastTypes";
import { BrowserRainAlerts } from "./rainAlerts";

const rainAlerts = new BrowserRainAlerts();

maplibregl.setWorkerUrl(workerUrl);
void initAnalytics();

// ---------- configuration ----------

const LAYERS: { id: string; name: string; needs: string }[] = [
  { id: "radar", name: "Radar", needs: "radar" },
  { id: "temperature", name: "Temperature", needs: "temperature" },
  { id: "wind", name: "Wind", needs: "wind" },
  { id: "precip-accum", name: "Rain total", needs: "precip-accum" },
  { id: "cloud", name: "Clouds", needs: "cloud" },
  { id: "air-quality", name: "Air quality", needs: "air-quality" },
  { id: "ozone", name: "Ozone", needs: "ozone" },
];

const MAP_STYLES = {
  light: { name: "Light", url: "/basemap/styles/positron.json" },
  dark: { name: "Dark", url: "/basemap/styles/dark-matter.json" },
  satellite: { name: "Satellite", url: "/basemap/styles/satellite.json" },
} as const;
type MapStyleId = keyof typeof MAP_STYLES;

const PRODUCTS = {
  radar: { name: "Base", hint: "Lowest scan — what's reaching the ground" },
  "radar-composite": { name: "Composite", hint: "Strongest echo in the column — storm tops and hail cores" },
} as const;
type ProductId = keyof typeof PRODUCTS;

const LEGENDS: Record<string, { title: string; stops: [string, string][] }> = {
  "air-quality": { title: "Air quality (PM2.5)", stops: [["Good", "#00c800"], ["Moderate", "#f5d500"], ["Sensitive", "#ff7e00"], ["Unhealthy", "#ff3c00"], ["V. unh.", "#8f3f97"], ["Hazard", "#7e0023"]] },
  ozone: { title: "Ozone", stops: [["Good", "#00c800"], ["Moderate", "#f5d500"], ["Sensitive", "#ff7e00"], ["Unhealthy", "#ff3c00"], ["V. unh.", "#8f3f97"], ["Hazard", "#7e0023"]] },
};

const SPEEDS = [0.5, 1, 2, 4];
const DEFAULT_PLACE: Place = { name: "Grand Rapids", admin1: "Michigan", latitude: 42.9634, longitude: -85.6681 };
const BASE_TICK_MS = 450;
const LOOP_DWELL_MS = 1400;
const FADE_MS = 220;
const PREFETCH_AHEAD = 6;
const MAX_CACHED_FRAMES = 28;
const MANIFEST_POLL_MS = 60_000;

// ---------- state ----------

const stored = <T,>(key: string, fallback: T): T => {
  try {
    const v = localStorage.getItem(key);
    return v ? (JSON.parse(v) as T) : fallback;
  } catch {
    return fallback;
  }
};
const store = (key: string, value: unknown) => {
  try {
    localStorage.setItem(key, JSON.stringify(value));
  } catch {
    /* private mode */
  }
};

const state = {
  place: stored<Place>("rng.place", DEFAULT_PLACE),
  mapStyle: stored<MapStyleId>("rng.mapStyle", "light"),
  layer: stored<string>("rng.layer", "radar"),
  palette: stored<PaletteId>("rng.palette", "classic"),
  product: stored<ProductId>("rng.product", "radar"),
  opacity: stored<number>("rng.opacity", 0.8),
  speed: stored<number>("rng.speed", 1),
  overlays: new Set<OverlayId>(stored<OverlayId[]>("rng.overlays", ["warnings", "storms", "tropical"])),
  wind: stored<boolean>("rng.wind", false),
  ai: stored<boolean>("rng.ai", true),
  zoom: "1h" as Zoom,
  manifest: null as Manifest | null,
  frames: [] as Frame[],
  seq: [] as number[],
  pos: 0,
  playing: false,
  timer: 0 as number | undefined,
  forecast: null as Forecast | null,
};
if (!PALETTES[state.palette]) state.palette = "classic";

const nowSec = () => Math.floor(Date.now() / 1000);
const $ = <T extends HTMLElement>(id: string) => document.getElementById(id) as T;

// ---------- map ----------

async function loadStyle(id: MapStyleId): Promise<StyleSpecification> {
  const res = await fetch(MAP_STYLES[id].url);
  const style = (await res.json()) as StyleSpecification & { glyphs?: string; sprite?: string };
  // The bundled styles use server-relative paths; MapLibre needs absolute URLs.
  const abs = (u: string) => (u.startsWith("http") ? u : `${location.origin}${u}`);
  for (const src of Object.values(style.sources) as { tiles?: string[]; url?: string }[]) {
    if (src.tiles) src.tiles = src.tiles.map(abs);
    if (src.url) src.url = abs(src.url);
  }
  // Overlay labels need glyphs; the satellite style ships without any.
  style.glyphs = abs(typeof style.glyphs === "string" ? style.glyphs : "/basemap/fonts/{fontstack}/{range}.pbf");
  if (typeof style.sprite === "string") style.sprite = abs(style.sprite);
  return style;
}

const map: MLMap = new maplibregl.Map({
  container: "map",
  style: { version: 8, sources: {}, layers: [] },
  center: [state.place.longitude, state.place.latitude],
  zoom: 6.5,
  minZoom: 4,
  maxZoom: 11,
  attributionControl: { compact: true },
  hash: "map",
});
map.addControl(new maplibregl.NavigationControl({ showCompass: false }), "bottom-right");

const overlays = new Overlays(map, state.overlays);
const wind = new WindLayer(map);

/** True between a style's "style.load" and the next setStyle; overlays can be added only then. */
let styleReady = false;

/** Frames added as raster layers, keyed by tile path; most recently used last. */
const cached = new Map<string, string>();
let shownKey: string | null = null;

function frameKey(f: Frame) {
  return `${f.source}|${state.palette}|${f.path}`;
}

/** Radar sits under the basemap labels and every vector overlay. */
function radarBeforeId(): string | undefined {
  return map.getStyle().layers?.find((l) => l.type === "symbol" || l.id.startsWith("ov-"))?.id;
}

function ensureFrame(f: Frame): string {
  const key = frameKey(f);
  const existing = cached.get(key);
  if (existing) {
    cached.delete(key);
    cached.set(key, existing);
    return existing;
  }
  const id = `wx-${Math.random().toString(36).slice(2, 10)}`;
  const path = f.path.split("/").map(encodeURIComponent).join("/");
  map.addSource(id, {
    type: "raster",
    tiles: [`${location.origin}/tiles/${f.source}/${state.palette}/${path}/{z}/{x}/{y}.png`],
    tileSize: 256,
    minzoom: 4,
    maxzoom: f.maxZoom,
  });
  map.addLayer(
    {
      id,
      type: "raster",
      source: id,
      // Opacity 0 still loads tiles: frames ahead of playback prefetch this way.
      paint: {
        "raster-opacity": 0,
        "raster-opacity-transition": { duration: FADE_MS, delay: 0 },
        "raster-fade-duration": 0,
        "raster-resampling": "linear",
      },
    },
    radarBeforeId(),
  );
  cached.set(key, id);
  return id;
}

function evict(keep: Set<string>) {
  for (const [key, id] of cached) {
    if (cached.size <= MAX_CACHED_FRAMES) break;
    if (keep.has(key)) continue;
    map.removeLayer(id);
    map.removeSource(id);
    cached.delete(key);
  }
}

function clearFrames() {
  for (const id of cached.values()) {
    if (map.getLayer(id)) map.removeLayer(id);
    if (map.getSource(id)) map.removeSource(id);
  }
  cached.clear();
  shownKey = null;
}

function currentFrame(): Frame | undefined {
  return state.frames[state.seq[state.pos]];
}

function showCurrentFrame() {
  renderTimeline();
  if (!styleReady || state.seq.length === 0) return;
  const frame = currentFrame();
  if (!frame) return;
  const keep = new Set<string>();
  for (let k = 0; k <= PREFETCH_AHEAD; k++) {
    const f = state.frames[state.seq[(state.pos + k) % state.seq.length]];
    if (f) {
      ensureFrame(f);
      keep.add(frameKey(f));
    }
  }
  const key = frameKey(frame);
  const id = cached.get(key)!;
  map.setPaintProperty(id, "raster-opacity", state.opacity);
  if (shownKey && shownKey !== key) {
    const prev = cached.get(shownKey);
    if (prev && map.getLayer(prev)) map.setPaintProperty(prev, "raster-opacity", 0);
  }
  shownKey = key;
  evict(keep);
  if (wind.enabled) void syncWind();
}

function nextFrameReady(): boolean {
  const f = state.frames[state.seq[(state.pos + 1) % state.seq.length]];
  const id = f && cached.get(frameKey(f));
  return !id || map.isSourceLoaded(id);
}

async function applyMapStyle(id: MapStyleId) {
  state.mapStyle = id;
  store("rng.mapStyle", id);
  document.documentElement.dataset.map = id;
  const style = await loadStyle(id);
  clearFrames();
  styleReady = false;
  // The persistent "style.load" handler re-adds the frames on the new style.
  map.setStyle(style);
  renderChips();
}

let placeMarker: maplibregl.Marker | null = null;
function addPlaceMarker() {
  placeMarker?.remove();
  const el = document.createElement("div");
  el.className = "place-dot";
  placeMarker = new maplibregl.Marker({ element: el }).setLngLat([state.place.longitude, state.place.latitude]).addTo(map);
}

// ---------- frames & playback ----------

function rebuildFrames(keepTime?: number) {
  if (!state.manifest) return;
  const now = nowSec();
  state.frames = buildFrames(state.manifest, state.layer, now, state.product);
  state.seq = playbackSequence(state.frames, state.zoom, now);
  const target = keepTime ?? state.frames[nowIndex(state.frames, now)]?.time;
  let best = 0;
  state.seq.forEach((idx, p) => {
    if (Math.abs(state.frames[idx].time - (target ?? now)) < Math.abs(state.frames[state.seq[best]].time - (target ?? now))) best = p;
  });
  state.pos = best;
  const slider = $<HTMLInputElement>("slider");
  slider.max = String(Math.max(0, state.seq.length - 1));
  renderLegend();
  renderAxis();
  showCurrentFrame();
}

function scheduleTick(delay: number, waited = 0) {
  window.clearTimeout(state.timer);
  state.timer = window.setTimeout(() => {
    if (!state.playing) return;
    // Hold on a frame whose tiles are still loading (up to ~1.5 s) instead of flashing an empty map.
    if (!nextFrameReady() && waited < 1500) return scheduleTick(100, waited + 100);
    state.pos = (state.pos + 1) % state.seq.length;
    showCurrentFrame();
    const atEnd = state.pos === state.seq.length - 1;
    scheduleTick(atEnd ? LOOP_DWELL_MS : BASE_TICK_MS / state.speed);
  }, delay);
}

function setPlaying(on: boolean) {
  state.playing = on && state.seq.length > 1;
  window.clearTimeout(state.timer);
  if (state.playing) scheduleTick(BASE_TICK_MS / state.speed);
  $("play").setAttribute("aria-label", state.playing ? "Pause radar animation" : "Play radar animation");
  $("play-icon").innerHTML = state.playing
    ? '<path d="M7 5h4v14H7zM13 5h4v14h-4z" fill="currentColor"/>'
    : '<path d="M8 5v14l11-7z" fill="currentColor"/>';
}

async function refreshManifest() {
  try {
    const keepTime = currentFrame()?.time;
    state.manifest = await fetchManifest();
    renderChips();
    if (!LAYERS.some((l) => l.id === state.layer && layerAvailable(l.needs))) state.layer = "radar";
    rebuildFrames(state.playing ? undefined : keepTime);
  } catch (err) {
    toast("Couldn't reach the radar server");
    console.error(err);
  }
}

function layerAvailable(key: string) {
  const layer = state.manifest?.layers[key];
  return Boolean(layer && ((layer.frames?.length ?? 0) > 0 || (layer.timestamps?.length ?? 0) > 0));
}

// ---------- wind ----------

/** The HRRR wind hour for the shown frame; the API falls back to its newest grid. */
function windTimestamp(): string | null {
  const frames = state.manifest?.layers.wind?.frames ?? [];
  const at = (currentFrame()?.time ?? nowSec()) * 1000;
  let best: string | null = null;
  let gap = Infinity;
  for (const f of frames) {
    const g = Math.abs(Date.parse(f.timestamp) - at);
    if (g < gap) [best, gap] = [f.timestamp, g];
  }
  return best;
}

async function syncWind() {
  const ok = await wind.load(windTimestamp());
  if (!ok && wind.enabled) {
    wind.setEnabled(false);
    state.wind = false;
    renderSheet();
    toast("Wind particles need HRRR wind — it isn't being ingested yet");
  }
}

// ---------- rendering: timeline, legend, chips, sheet ----------

const TIME_FMT = new Intl.DateTimeFormat(undefined, { weekday: "long", hour: "numeric", minute: "2-digit" });

function latestObserved(): Frame | undefined {
  for (let i = state.frames.length - 1; i >= 0; i--) {
    const f = state.frames[i];
    if (f.source !== "nowcast" && f.source !== "radar-hrrr" && f.time <= nowSec()) return f;
  }
  return undefined;
}

function renderTimeline() {
  const frame = currentFrame();
  const slider = $<HTMLInputElement>("slider");
  slider.value = String(state.pos);
  if (!frame) {
    $("frame-title").textContent = "No frames yet";
    return;
  }
  const label = offsetLabel(frame.time, nowSec());
  const mode = label === "Now" ? "Now" : `${frame.time > nowSec() ? "Forecast" : "Past"} ${label}`;
  const source = frame.source === "nowcast" ? " · nowcast" : frame.source === "radar-hrrr" ? " · HRRR model" : frame.source === "radar-composite" ? " · composite" : "";
  const layerName = LAYERS.find((l) => l.id === state.layer)?.name ?? "Radar";
  $("frame-title").textContent = `${layerName} · ${mode}`;
  $("frame-time").textContent = `${TIME_FMT.format(new Date(frame.time * 1000))}${source}`;
  slider.setAttribute("aria-valuetext", `${mode}, ${TIME_FMT.format(new Date(frame.time * 1000))}`);
  const live = latestObserved();
  const pill = $("live");
  if (state.layer === "radar" && live) {
    const ageMin = Math.max(0, Math.round((nowSec() - live.time) / 60));
    pill.hidden = false;
    pill.classList.toggle("stale", ageMin > 10);
    pill.textContent = frame === live ? `Live · ${ageMin} min ago` : `Live ${ageMin}m`;
  } else pill.hidden = true;
}

function renderAxis() {
  const n = state.seq.length;
  const now = nowSec();
  const last = Math.max(1, n - 1);
  const pct = (p: number) => `${(p / last) * 100}%`;
  const lastAtOrBefore = (sec: number) => {
    let p = -1;
    while (p + 1 < n && state.frames[state.seq[p + 1]].time <= sec) p++;
    return p;
  };
  const nowP = Math.max(0, lastAtOrBefore(now));
  const ncP = Math.max(nowP, lastAtOrBefore(now + 3600));
  $("track-segments").innerHTML = n
    ? `<span class="past" style="left:0;width:${pct(nowP)}"></span>` +
      `<span class="nowcast" style="left:${pct(nowP)};width:calc(${pct(ncP)} - ${pct(nowP)})"></span>` +
      `<span class="model" style="left:${pct(ncP)};right:0"></span>` +
      `<span class="now-marker" style="left:${pct(nowP)}"></span>`
    : "";
  $("axis").innerHTML = n
    ? [0, 0.25, 0.5, 0.75, 1]
        .map((f) => {
          const l = offsetLabel(state.frames[state.seq[Math.round(f * last)]].time, now);
          return `<span class="${l === "Now" ? "now-label" : ""}">${l}</span>`;
        })
        .join("")
    : "";
}

function renderLegend() {
  const el = $("legend");
  if (state.layer === "radar") {
    const bands = PALETTES[state.palette].bands;
    const lo = bands[0].min;
    const hi = 75;
    const pos = (dbz: number) => `${((dbz - lo) / (hi - lo)) * 100}%`;
    el.hidden = false;
    el.innerHTML =
      `<div class="legend-title"><span>Precipitation</span><span class="legend-tag">${PALETTES[state.palette].name}</span></div>` +
      `<div class="legend-bar" style="background:linear-gradient(90deg,${bands.map((b, i) => `${css(b.rgba)} ${pos(b.min)} ${pos(bands[i + 1]?.min ?? hi)}`).join(",")})"></div>` +
      `<div class="legend-ticks">${LEGEND_TICKS.map(([dbz, name]) => `<span style="left:${pos(dbz)}">${name}</span>`).join("")}</div>`;
    return;
  }
  const spec = LEGENDS[state.layer];
  if (!spec) {
    el.hidden = true;
    return;
  }
  el.hidden = false;
  el.innerHTML =
    `<div class="legend-title">${spec.title}</div>` +
    `<div class="legend-bar" style="background:linear-gradient(90deg,${spec.stops.map((s) => s[1]).join(",")})"></div>` +
    `<div class="legend-labels">${[spec.stops[0], spec.stops[spec.stops.length - 1]].map((s) => `<span>${s[0]}</span>`).join("")}</div>`;
}

function renderChips() {
  $("layers").innerHTML = LAYERS.filter((l) => layerAvailable(l.needs))
    .map((l) => `<button role="radio" data-layer="${l.id}" aria-checked="${l.id === state.layer}">${l.name}</button>`)
    .join("");
  $("styles").innerHTML = (Object.keys(MAP_STYLES) as MapStyleId[])
    .map((id) => `<button role="radio" data-style="${id}" aria-checked="${id === state.mapStyle}">${MAP_STYLES[id].name}</button>`)
    .join("");
  document.querySelectorAll<HTMLButtonElement>("#zoom button").forEach((b) => b.setAttribute("aria-checked", String(b.dataset.zoom === state.zoom)));
  $("speed").textContent = `${state.speed}×`;
  renderSheet();
}

function renderSheet() {
  const seg = (group: string, items: [string, string, string?][], current: string) =>
    `<div class="segmented wide" role="radiogroup" data-group="${group}">${items
      .map(([id, name, hint]) => `<button role="radio" data-value="${id}" aria-checked="${id === current}"${hint ? ` title="${hint}"` : ""}>${name}</button>`)
      .join("")}</div>`;
  const composite = layerAvailable("radar-composite");
  const toggles = [
    ...OVERLAYS.map((o) => ({ id: o.id, name: o.name, hint: o.hint, on: state.overlays.has(o.id) })),
    { id: "wind", name: "Wind flow", hint: "Animated HRRR surface wind", on: state.wind },
    { id: "ai", name: "AI narration", hint: "Local-LLM summaries; hides itself if the model is down", on: state.ai },
  ];
  $("sheet-body").innerHTML = `
    <div class="sheet-section"><div class="section-label">RADAR</div>
      ${composite ? seg("product", (Object.keys(PRODUCTS) as ProductId[]).map((id) => [id, PRODUCTS[id].name, PRODUCTS[id].hint]), state.product) : ""}
      <p class="hint">${PRODUCTS[state.product].hint}</p>
      ${seg("palette", (Object.keys(PALETTES) as PaletteId[]).map((id) => [id, PALETTES[id].name]), state.palette)}
      <label class="range-row"><span>Opacity</span><input id="opacity" type="range" min="0.3" max="1" step="0.05" value="${state.opacity}" aria-label="Radar opacity" /></label>
    </div>
    <div class="sheet-section"><div class="section-label">OVERLAYS</div>
      ${toggles.map((t) => `<label class="toggle"><span><b>${t.name}</b><small>${t.hint}</small></span><input type="checkbox" data-overlay="${t.id}" ${t.on ? "checked" : ""} /><i></i></label>`).join("")}
    </div>
    <div class="sheet-section"><div class="section-label">RAIN ALERTS</div>
      <label class="toggle"><span><b>Rain starting soon</b><small>Local notifications from your radar forecast</small></span><input id="rain-alerts" type="checkbox" ${rainAlerts.enabled ? "checked" : ""} ${rainAlerts.supported ? "" : "disabled"} /><i></i></label>
      <label class="range-row"><span>Notify me before rain</span><select id="rain-alert-lead" aria-label="Rain alert lead time">${RAIN_ALERT_LEAD_OPTIONS.map((lead) => `<option value="${lead}" ${lead === rainAlerts.leadMinutes ? "selected" : ""}>${lead} min</option>`).join("")}</select></label>
      <p class="hint" role="status">${escapeHtml(rainAlerts.supported ? rainAlerts.message : "Notifications are unavailable in this browser. Use the app for rain alerts.")}</p>
    </div>
    <div class="sheet-section"><div class="section-label">KEYBOARD</div>
      <p class="hint"><kbd>Space</kbd> play · <kbd>←</kbd><kbd>→</kbd> step · <kbd>W</kbd> warnings · <kbd>S</kbd> storms · <kbd>L</kbd> lightning · <kbd>H</kbd> hurricanes</p>
    </div>`;
}

// ---------- forecast panel ----------

const deg = (v: number | null | undefined) => (v == null ? "—" : `${Math.round(v)}°`);

function hourIndex(times: string[]): number {
  const cutoff = Date.now() - 30 * 60_000;
  const i = times.findIndex((t) => Date.parse(t) >= cutoff);
  return Math.max(0, i);
}

async function renderForecast() {
  const body = $("panel-body");
  const { latitude: lat, longitude: lon } = state.place;
  let fc: Forecast;
  try {
    fc = await fetchForecast(lat, lon);
  } catch {
    body.innerHTML = '<p class="muted">Forecast unavailable right now.</p>';
    return;
  }
  state.forecast = fc;
  const [alerts, aq] = await Promise.all([fetchAlerts(lat, lon).catch(() => []), airQualityNow(lat, lon)]);
  const c = fc.current;
  const cond = conditionOf(c.weather_code);
  const start = hourIndex(fc.hourly.time);
  const hours = Array.from({ length: 24 }, (_, k) => start + k).filter((i) => i < fc.hourly.time.length);
  const rain = hours.map((i) => fc.hourly.precipitation[i] ?? 0);
  const rainTotal = rain.reduce((a, b) => a + b, 0);
  const peak = Math.max(0.05, ...rain);
  const days = fc.daily.time.slice(0, 7);
  const lows = fc.daily.temperature_2m_min.slice(0, 7).filter((v): v is number => v != null);
  const highs = fc.daily.temperature_2m_max.slice(0, 7).filter((v): v is number => v != null);
  const wLo = Math.min(...lows);
  const wHi = Math.max(...highs);
  const span = Math.max(1, wHi - wLo);
  const fmtHour = (t: string, i: number) => (i === 0 ? "Now" : new Date(t).toLocaleTimeString([], { hour: "numeric" }).replace(" ", "").toLowerCase());
  const dayName = (d: string, i: number) => (i === 0 ? "Today" : new Date(`${d}T00:00:00`).toLocaleDateString([], { weekday: "short" }));
  const inHg = c.surface_pressure == null ? null : (c.surface_pressure / 33.8639).toFixed(2);
  const aqi = aq.pm25 == null ? null : usAqiFromPm25(aq.pm25);

  body.innerHTML = `
    <h1 class="place">${escapeHtml(state.place.name)}${state.place.admin1 ? `<span class="muted" style="font-size:15px;font-family:var(--ui)"> · ${escapeHtml(state.place.admin1)}</span>` : ""}</h1>
    <div class="now">
      <div>
        <div class="now-cond">${CONDITION_LABEL[cond]}</div>
        <div class="now-temp">${deg(c.temperature_2m)}</div>
      </div>
      ${iconSvg(cond, 84)}
    </div>
    <div class="now-meta">Feels ${deg(c.apparent_temperature)} · H ${deg(fc.daily.temperature_2m_max[0])} L ${deg(fc.daily.temperature_2m_min[0])}</div>
    <div id="briefing" class="briefing" hidden></div>
    <div id="threat" class="threat" hidden></div>
    ${alerts.length ? `<div class="section">${alerts.slice(0, 3).map((a) => `<div class="alert"><strong>${escapeHtml(a.event)}</strong><span>${a.ends ? `Until ${new Date(a.ends).toLocaleString([], { weekday: "short", hour: "numeric", minute: "2-digit" })}` : escapeHtml(a.severity)}</span></div>`).join("")}</div>` : ""}
    <div id="nexthour" class="section" hidden></div>
    <div class="section">
      <div class="section-label">HOURLY</div>
      <div class="hourly">${hours.map((i, k) => `<div class="hour ${k === 0 ? "now-hour" : ""}"><b>${fmtHour(fc.hourly.time[i], k)}</b>${iconSvg(conditionOf(fc.hourly.weather_code[i]), 22)}<div class="t">${deg(fc.hourly.temperature_2m[i])}</div></div>`).join("")}</div>
    </div>
    <div class="section">
      <div class="section-label" style="display:flex;justify-content:space-between"><span>RAIN · NEXT 24H</span><span>${rainTotal.toFixed(2)}″</span></div>
      <div class="rainbars">${rain.map((v) => `<i class="${v ? "" : "dry"}" style="height:${v ? Math.max(8, (v / peak) * 100) : 6}%"></i>`).join("")}</div>
    </div>
    <div class="section">
      <div class="section-label">7-DAY</div>
      ${days.map((d, i) => {
        const lo = fc.daily.temperature_2m_min[i];
        const hi = fc.daily.temperature_2m_max[i];
        const left = lo == null ? 0 : ((lo - wLo) / span) * 100;
        const width = lo == null || hi == null ? 0 : ((hi - lo) / span) * 100;
        return `<div class="day"><b>${dayName(d, i)}</b>${iconSvg(conditionOf(fc.daily.weather_code[i]), 22)}<span class="lo">${deg(lo)}</span><div class="bar"><span style="left:${left}%;width:${width}%"></span></div><span class="hi">${deg(hi)}</span></div>`;
      }).join("")}
    </div>
    <div class="section">
      <div class="section-label">CONDITIONS</div>
      <div class="tiles">
        <div class="tile"><b>WIND</b><div class="v">${c.wind_speed_10m == null ? "—" : Math.round(c.wind_speed_10m)}<small>mph</small></div><div class="c">${c.wind_gusts_10m == null ? "" : `Gusts ${Math.round(c.wind_gusts_10m)}`}</div></div>
        <div class="tile"><b>HUMIDITY</b><div class="v">${c.relative_humidity_2m == null ? "—" : Math.round(c.relative_humidity_2m)}<small>%</small></div><div class="c">Dew point ${deg(c.dew_point_2m)}</div></div>
        <div class="tile"><b>PRESSURE</b><div class="v">${inHg ?? "—"}<small>inHg</small></div></div>
        <div class="tile"><b>RAIN TODAY</b><div class="v">${(fc.daily.precipitation_sum[0] ?? 0).toFixed(2)}<small>in</small></div><div class="c">Tomorrow ${(fc.daily.precipitation_sum[1] ?? 0).toFixed(2)}″</div></div>
        ${aqi == null ? "" : `<div class="tile" style="grid-column:1/-1"><b>AIR QUALITY</b><div class="v">${aqi}<small>AQI</small></div><div class="c">${aqiLabel(aqi)} · PM2.5 ${Math.round(aq.pm25!)} µg/m³${aq.ozone == null ? "" : ` · Ozone ${Math.round(aq.ozone)} ppb`}</div></div>`}
      </div>
    </div>`;
  renderThreat();
  void renderBriefing();
  void renderNextHour();
}

/** Minute-by-minute rain for the next hour from the radar nowcast (hidden when unavailable). */
async function renderNextHour() {
  const el = document.getElementById("nexthour");
  if (!el && !rainAlerts.enabled) return;
  const { latitude, longitude } = state.place;
  let nowcast: RadarNowcastResponse | null = null;
  try {
    const res = await fetch(`/api/nowcast/${latitude.toFixed(2)}/${longitude.toFixed(2)}`, { signal: AbortSignal.timeout(10_000) });
    if (res.ok) nowcast = await res.json() as RadarNowcastResponse;
  } catch {
    /* no radar nowcast: leave the section hidden */
  }
  // A previous location or panel render must not replace a newer plan.
  if (latitude !== state.place.latitude || longitude !== state.place.longitude || (el && !document.body.contains(el))) return;
  rainAlerts.sync(nowcast, state.place.name);
  const points = nowcast?.points ?? [];
  if (!el) return;
  el.hidden = true;
  if (!points.length || !document.body.contains(el)) return;
  const skill = describeNowcastSkill(nowcast?.skill);
  const minutes = interpolateNowcast(points);
  const peak = Math.max(0.1, ...minutes);
  el.hidden = false;
  el.innerHTML =
    `<div class="section-label">NEXT HOUR</div><p class="nexthour-verdict">${escapeHtml(nowcastVerdict(minutes))}</p>` +
    (skill ? `<p class="hint">${escapeHtml(skill.sentence)}${skill.beatsPersistence === false ? " · no better than a still radar" : ""}</p>` : "") +
    `<div class="minutebars">${minutes.map((v) => `<i class="${v > 0.01 ? "" : "dry"}" style="height:${v > 0.01 ? Math.max(8, (v / peak) * 100) : 6}%"></i>`).join("")}</div>` +
    `<div class="minute-axis"><span>Now</span><span>15m</span><span>30m</span><span>45m</span><span>60m</span></div>`;
}

let briefingAbort: AbortController | null = null;

/** Optional LLM narration: the card stays hidden unless a briefing actually arrives. */
async function renderBriefing() {
  briefingAbort?.abort();
  const el = document.getElementById("briefing");
  if (!el) return;
  if (!state.ai) {
    el.hidden = true;
    return;
  }
  briefingAbort = new AbortController();
  const b = await fetchBriefing(state.place, briefingAbort.signal);
  if (!b || !document.body.contains(el)) return;
  el.hidden = false;
  el.innerHTML = `<div class="briefing-head"><span class="spark">✦</span><strong>${escapeHtml(b.headline || "Right now")}</strong></div><p>${escapeHtml(b.body)}</p><small>AI summary · ${escapeHtml(b.model ?? "local model")}</small>`;
}

/** "Storm arriving in ~20 min" when a tracked cell's path crosses the chosen place. */
function renderThreat() {
  const el = document.getElementById("threat");
  if (!el) return;
  const t = incomingStorm(overlays.cells, [state.place.longitude, state.place.latitude]);
  if (!t) {
    el.hidden = true;
    return;
  }
  const what = t.peakDbz >= 60 ? "Hail-producing storm" : t.peakDbz >= 50 ? "Heavy storm" : "Rain";
  const when = t.minutes === 0 ? "over you now" : `arriving in ~${t.minutes} min`;
  el.hidden = false;
  el.className = `threat ${t.peakDbz >= 55 ? "severe" : ""}`;
  el.innerHTML = `<strong>${what} ${when}</strong><span>${Math.round(t.peakDbz)} dBZ · moving ${compass(t.bearing)} at ${Math.round(t.speedMph)} mph</span>`;
}

async function airQualityNow(lat: number, lon: number): Promise<{ pm25: number | null; ozone: number | null }> {
  const nearest = (key: string) => {
    const frames = state.manifest?.layers[key]?.frames ?? [];
    let best: { ts: string; gap: number } | null = null;
    for (const f of frames) {
      const gap = Math.abs(Date.parse(f.timestamp) - Date.now());
      if (!best || gap < best.gap) best = { ts: f.timestamp, gap };
    }
    return best && best.gap <= 90 * 60_000 ? best.ts : null;
  };
  const read = async (key: string) => {
    const ts = nearest(key);
    return ts ? readPoint(key, key, ts, lat, lon).catch(() => null) : null;
  };
  const [pm25, ozone] = await Promise.all([read("air-quality"), read("ozone")]);
  return { pm25, ozone };
}

function escapeHtml(s: string) {
  return s.replace(/[&<>"']/g, (ch) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" })[ch]!);
}

function toast(message: string) {
  const el = $("toast");
  el.textContent = message;
  el.hidden = false;
  window.setTimeout(() => (el.hidden = true), 3500);
}

// ---------- place ----------

function setPlace(place: Place, fly = true) {
  state.place = place;
  store("rng.place", place);
  addPlaceMarker();
  if (fly) map.flyTo({ center: [place.longitude, place.latitude], zoom: Math.max(map.getZoom(), 6.5), duration: 900 });
  void renderForecast();
}

// ---------- events ----------

let searchAbort: AbortController | null = null;
let results: Place[] = [];
let activeResult = -1;

/** A status row instead of results: "Searching…", "No places found", or a retry hint. */
let searchStatus: string | null = null;

function renderResults() {
  const list = $("search-results");
  list.hidden = results.length === 0 && !searchStatus;
  if (searchStatus && results.length === 0) {
    list.innerHTML = `<li class="status" aria-disabled="true">${escapeHtml(searchStatus)}</li>`;
    return;
  }
  list.innerHTML = results
    .map((p, i) => `<li role="option" data-i="${i}" aria-selected="${i === activeResult}">${escapeHtml(p.name)}<small>${escapeHtml(p.admin1 ?? "")}</small></li>`)
    .join("");
}

$("search-input").addEventListener("input", (e) => {
  const q = (e.target as HTMLInputElement).value.trim();
  searchAbort?.abort();
  if (q.length < 2) {
    results = [];
    searchStatus = null;
    renderResults();
    return;
  }
  searchAbort = new AbortController();
  const signal = searchAbort.signal;
  window.setTimeout(async () => {
    if (signal.aborted) return;
    results = [];
    searchStatus = "Searching…";
    renderResults();
    try {
      results = await searchPlaces(q, signal);
      activeResult = results.length ? 0 : -1;
      searchStatus = results.length ? null : "No places found";
    } catch {
      if (signal.aborted) return;
      // The geocoder can take several seconds on a cold query; a retry is usually instant.
      searchStatus = "Search is slow right now — press Enter to try again";
    }
    renderResults();
  }, 200);
});

$("search-input").addEventListener("keydown", (e) => {
  if (e.key === "ArrowDown" || e.key === "ArrowUp") {
    e.preventDefault();
    activeResult = Math.max(0, Math.min(results.length - 1, activeResult + (e.key === "ArrowDown" ? 1 : -1)));
    renderResults();
  } else if (e.key === "Escape") {
    results = [];
    renderResults();
  }
});

$("search").addEventListener("submit", (e) => {
  e.preventDefault();
  if (results[activeResult]) choose(results[activeResult]);
  // No results yet (slow or failed search): Enter searches again.
  else $("search-input").dispatchEvent(new Event("input"));
});

$("search-results").addEventListener("click", (e) => {
  const li = (e.target as HTMLElement).closest("li[data-i]");
  if (li instanceof HTMLElement) choose(results[Number(li.dataset.i)]);
});

function choose(p: Place) {
  track("place_chosen", { place: p.name, region: p.admin1 });
  results = [];
  searchStatus = null;
  renderResults();
  $<HTMLInputElement>("search-input").value = "";
  setPlace(p);
}

$("panel-toggle").addEventListener("click", () => {
  const panel = $("panel");
  const collapsed = panel.classList.toggle("collapsed");
  $("panel-toggle").setAttribute("aria-expanded", String(!collapsed));
  $("panel-toggle").setAttribute("aria-label", collapsed ? "Expand forecast" : "Collapse forecast");
});

$("layers").addEventListener("click", (e) => {
  const b = (e.target as HTMLElement).closest<HTMLButtonElement>("button[data-layer]");
  if (!b || b.dataset.layer === state.layer) return;
  state.layer = b.dataset.layer!;
  store("rng.layer", state.layer);
  track("layer_changed", { layer: state.layer });
  clearFrames();
  renderChips();
  rebuildFrames();
});

$("styles").addEventListener("click", (e) => {
  const b = (e.target as HTMLElement).closest<HTMLButtonElement>("button[data-style]");
  if (b && b.dataset.style !== state.mapStyle) {
    track("map_style_changed", { style: b.dataset.style });
    void applyMapStyle(b.dataset.style as MapStyleId);
  }
});

$("zoom").addEventListener("click", (e) => {
  const b = (e.target as HTMLElement).closest<HTMLButtonElement>("button[data-zoom]");
  if (!b || b.dataset.zoom === state.zoom) return;
  state.zoom = b.dataset.zoom as Zoom;
  track("timeline_range_changed", { range: state.zoom });
  renderChips();
  rebuildFrames(currentFrame()?.time);
});

$("speed").addEventListener("click", () => {
  state.speed = SPEEDS[(SPEEDS.indexOf(state.speed) + 1) % SPEEDS.length] ?? 1;
  store("rng.speed", state.speed);
  $("speed").textContent = `${state.speed}×`;
  if (state.playing) scheduleTick(BASE_TICK_MS / state.speed);
});

$("play").addEventListener("click", () => {
  setPlaying(!state.playing);
  track(state.playing ? "playback_started" : "playback_paused", { range: state.zoom, layer: state.layer });
});

$("slider").addEventListener("input", (e) => {
  setPlaying(false);
  state.pos = Number((e.target as HTMLInputElement).value);
  showCurrentFrame();
});

function toggleSheet(open?: boolean) {
  const sheet = $("sheet");
  const show = open ?? sheet.hidden;
  sheet.hidden = !show;
  $("sheet-toggle").setAttribute("aria-expanded", String(show));
}
$("sheet-toggle").addEventListener("click", () => toggleSheet());
$("sheet-close").addEventListener("click", () => toggleSheet(false));

$("sheet-body").addEventListener("click", (e) => {
  const b = (e.target as HTMLElement).closest<HTMLButtonElement>("[data-group] button[data-value]");
  if (!b) return;
  const group = b.closest<HTMLElement>("[data-group]")!.dataset.group;
  const value = b.dataset.value!;
  if (group === "product" && value !== state.product) {
    state.product = value as ProductId;
    store("rng.product", value);
    track("radar_product_changed", { product: value });
    rebuildFrames(currentFrame()?.time);
  } else if (group === "palette" && value !== state.palette) {
    state.palette = value as PaletteId;
    store("rng.palette", value);
    track("palette_changed", { palette: value });
    clearFrames();
    rebuildFrames(currentFrame()?.time);
  }
  renderSheet();
});

$("sheet-body").addEventListener("input", (e) => {
  const el = e.target as HTMLInputElement;
  if (el.id !== "opacity") return;
  state.opacity = Number(el.value);
  store("rng.opacity", state.opacity);
  const id = shownKey && cached.get(shownKey);
  if (id) map.setPaintProperty(id, "raster-opacity", state.opacity);
});

function setOverlay(id: OverlayId | "wind" | "ai", on: boolean) {
  track("overlay_toggled", { overlay: id, on });
  if (id === "ai") {
    state.ai = on;
    store("rng.ai", on);
    void renderBriefing();
  } else if (id === "wind") {
    state.wind = on;
    store("rng.wind", on);
    wind.setEnabled(on);
    if (on) void syncWind();
  } else {
    overlays.setEnabled(id, on);
    store("rng.overlays", [...state.overlays]);
  }
  renderSheet();
}

$("sheet-body").addEventListener("change", (e) => {
  const el = e.target as HTMLInputElement;
  if (el.id === "rain-alerts") {
    el.disabled = true;
    void rainAlerts.setEnabled(el.checked).then(() => {
      renderSheet();
      void renderNextHour();
    });
  } else if (el.id === "rain-alert-lead") {
    rainAlerts.setLead(el.value);
    void renderNextHour();
  }
  if (el.dataset.overlay) setOverlay(el.dataset.overlay as OverlayId | "wind" | "ai", el.checked);
});

$("locate").addEventListener("click", () => {
  if (!navigator.geolocation) return toast("Location isn't available in this browser");
  navigator.geolocation.getCurrentPosition(
    async (pos) => {
      const { latitude, longitude } = pos.coords;
      const place = await reversePlace(latitude, longitude).catch(() => null);
      setPlace({ name: place?.name ?? "My location", admin1: place?.admin1, latitude, longitude });
    },
    () => toast("Location permission was denied"),
    { enableHighAccuracy: false, timeout: 10_000 },
  );
});

const OVERLAY_KEYS: Record<string, OverlayId> = { w: "warnings", s: "storms", l: "lightning", h: "tropical" };

document.addEventListener("keydown", (e) => {
  if ((e.target as HTMLElement).closest("input") || e.metaKey || e.ctrlKey || e.altKey) return;
  const n = state.seq.length;
  if (e.key === " ") {
    e.preventDefault();
    setPlaying(!state.playing);
  } else if ((e.key === "ArrowRight" || e.key === "ArrowLeft") && n) {
    setPlaying(false);
    state.pos = (state.pos + (e.key === "ArrowRight" ? 1 : -1) + n) % n;
    showCurrentFrame();
  } else if (e.key === "Escape") {
    toggleSheet(false);
    popup.remove();
  } else if (OVERLAY_KEYS[e.key.toLowerCase()]) {
    const id = OVERLAY_KEYS[e.key.toLowerCase()];
    setOverlay(id, !state.overlays.has(id));
    toast(`${OVERLAYS.find((o) => o.id === id)!.name} ${state.overlays.has(id) ? "on" : "off"}`);
  }
});

// Click the map: an overlay feature if one is under the cursor, else the shown frame's value there.
const popup = new maplibregl.Popup({ closeButton: true, maxWidth: "260px" });
map.on("click", async (e) => {
  const layers = overlays.clickable();
  const hit = layers.length ? map.queryRenderedFeatures(e.point, { layers })[0] : undefined;
  if (hit) {
    const html = describeFeature(hit, overlays.cells);
    if (html) {
      const alertId = hit.layer.id.startsWith("ov-warnings") && state.ai ? String(hit.properties?.id ?? "") : "";
      popup.setLngLat(e.lngLat).setHTML(alertId ? `${html}<div class="explain muted">✦ Explaining…</div>` : html).addTo(map);
      track("overlay_inspected", { layer: hit.layer.id });
      if (alertId) void fillExplanation(alertId);
      return;
    }
  }
  const frame = currentFrame();
  if (!frame) return;
  const { lat, lng } = e.lngLat;
  popup.setLngLat(e.lngLat).setHTML('<div class="readout"><small>Reading…</small></div>').addTo(map);
  const value = await readPoint(state.layer, frame.source, frame.timestamp, lat, lng).catch(() => null);
  track("point_inspected", { layer: state.layer, source: frame.source, has_value: value != null });
  const title = frame.source === "nowcast" ? "NOWCAST" : frame.source === "radar-hrrr" ? "HRRR FORECAST" : frame.source === "radar-composite" ? "COMPOSITE" : (LAYERS.find((l) => l.id === state.layer)?.name ?? "").toUpperCase();
  let text = "No data here";
  if (value != null) {
    if (state.layer === "radar") text = `${describeDbz(value)} <small>${Math.round(value)} dBZ</small>`;
    else if (state.layer === "air-quality") text = `AQI ${usAqiFromPm25(value)} · ${aqiLabel(usAqiFromPm25(value))} <small>${Math.round(value)} µg/m³</small>`;
    else if (state.layer === "ozone") text = `${Math.round(value)} ppb`;
    else text = `${value.toFixed(1)}`;
  }
  popup.setHTML(`<div class="readout"><b>${title}</b><div class="v">${text}</div><small>${lat.toFixed(3)}, ${lng.toFixed(3)}</small></div>`);
});

/** Adds the LLM's plain-English take to an open warning popup, or quietly removes the placeholder. */
async function fillExplanation(alertId: string) {
  const x = await explainAlert(alertId);
  const slot = popup.getElement()?.querySelector<HTMLElement>(".explain");
  if (!slot) return;
  if (!x) return slot.remove();
  slot.classList.remove("muted");
  slot.innerHTML = `<b>✦ In plain English</b><p>${escapeHtml(x.what)}</p><p><strong>Do:</strong> ${escapeHtml(x.do)}</p>`;
}

for (const id of ["ov-storm-cells", "ov-warnings-fill", "ov-tropical-position", "ov-tropical-points"]) {
  map.on("mouseenter", id, () => (map.getCanvas().style.cursor = "pointer"));
  map.on("mouseleave", id, () => (map.getCanvas().style.cursor = ""));
}

// ---------- boot ----------

// Every style (first load and each switch) needs the overlays put back.
map.on("style.load", () => {
  styleReady = true;
  overlays.install();
  addPlaceMarker();
  showCurrentFrame();
});

overlays.onStorms = renderThreat;

(async () => {
  document.documentElement.dataset.map = state.mapStyle;
  renderChips();
  map.setStyle(await loadStyle(state.mapStyle));
  await refreshManifest();
  overlays.start();
  if (state.wind) setOverlay("wind", true);
  void renderForecast();
  window.setInterval(refreshManifest, MANIFEST_POLL_MS);
  // Keep "Live · N min ago" honest between manifest polls.
  window.setInterval(renderTimeline, 30_000);
  window.setInterval(() => void renderBriefing(), 10 * 60_000);
  window.setInterval(() => void renderNextHour(), 60_000);
})();
