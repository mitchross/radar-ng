import * as maplibregl from "maplibre-gl";
import type { Map as MLMap, StyleSpecification } from "maplibre-gl";
import "maplibre-gl/dist/maplibre-gl.css";
import "./style.css";
import { buildFrames, nowIndex, offsetLabel, playbackSequence, type Frame, type Manifest, type Zoom } from "./frames";
import {
  aqiLabel,
  CONDITION_LABEL,
  conditionOf,
  describeDbz,
  fetchAlerts,
  fetchForecast,
  fetchManifest,
  iconSvg,
  readPoint,
  reversePlace,
  searchPlaces,
  usAqiFromPm25,
  type Forecast,
  type Place,
} from "./weather";

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

const LEGENDS: Record<string, { title: string; stops: [string, string][] }> = {
  radar: { title: "Precipitation", stops: [["Light", "#3bc77a"], ["Moderate", "#ff9f2e"], ["Heavy", "#ff4040"], ["Severe", "#d02058"], ["Hail", "#b24bff"]] },
  "air-quality": { title: "Air quality (PM2.5)", stops: [["Good", "#00c800"], ["Moderate", "#f5d500"], ["Sensitive", "#ff7e00"], ["Unhealthy", "#ff3c00"], ["V. unh.", "#8f3f97"], ["Hazard", "#7e0023"]] },
  ozone: { title: "Ozone", stops: [["Good", "#00c800"], ["Moderate", "#f5d500"], ["Sensitive", "#ff7e00"], ["Unhealthy", "#ff3c00"], ["V. unh.", "#8f3f97"], ["Hazard", "#7e0023"]] },
};

const DEFAULT_PLACE: Place = { name: "Grand Rapids", admin1: "Michigan", latitude: 42.9634, longitude: -85.6681 };
const TICK_MS = 500;
const PREFETCH_AHEAD = 5;
const MAX_CACHED_FRAMES = 24;
const MANIFEST_POLL_MS = 60_000;
const OVERLAY_OPACITY = 0.78;

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
  zoom: "1h" as Zoom,
  manifest: null as Manifest | null,
  frames: [] as Frame[],
  seq: [] as number[],
  pos: 0,
  playing: false,
  timer: 0 as number | undefined,
  forecast: null as Forecast | null,
};

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
  if (typeof style.glyphs === "string") style.glyphs = abs(style.glyphs);
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
  hash: false,
});
map.addControl(new maplibregl.NavigationControl({ showCompass: false }), "bottom-right");

/** True between a style's "style.load" and the next setStyle; overlays can be added only then. */
let styleReady = false;

/** Frames added as raster layers, keyed by tile path; most recently used last. */
const cached = new Map<string, string>();
let shownKey: string | null = null;

function frameKey(f: Frame) {
  return `${f.source}|${f.path}`;
}

function firstLabelLayer(): string | undefined {
  return map.getStyle().layers?.find((l) => l.type === "symbol")?.id;
}

function ensureFrame(f: Frame): string {
  const key = frameKey(f);
  const existing = cached.get(key);
  if (existing) {
    cached.delete(key);
    cached.set(key, existing);
    return existing;
  }
  const id = `wx-${cached.size}-${Math.random().toString(36).slice(2, 8)}`;
  const path = f.path.split("/").map(encodeURIComponent).join("/");
  map.addSource(id, {
    type: "raster",
    tiles: [`${location.origin}/tiles/${f.source}/classic/${path}/{z}/{x}/{y}.png`],
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
      paint: { "raster-opacity": 0, "raster-fade-duration": 0, "raster-resampling": "linear" },
    },
    firstLabelLayer(),
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

function showCurrentFrame() {
  renderTimeline();
  if (!styleReady || state.seq.length === 0) return;
  const frame = state.frames[state.seq[state.pos]];
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
  map.setPaintProperty(id, "raster-opacity", OVERLAY_OPACITY);
  if (shownKey && shownKey !== key) {
    const prev = cached.get(shownKey);
    if (prev && map.getLayer(prev)) map.setPaintProperty(prev, "raster-opacity", 0);
  }
  shownKey = key;
  evict(keep);
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
  el.style.cssText = "width:14px;height:14px;border-radius:7px;background:#3478f6;border:3px solid #fff;box-shadow:0 1px 4px rgba(0,0,0,.35)";
  placeMarker = new maplibregl.Marker({ element: el }).setLngLat([state.place.longitude, state.place.latitude]).addTo(map);
}

// ---------- frames & playback ----------

function rebuildFrames(keepTime?: number) {
  if (!state.manifest) return;
  const now = nowSec();
  state.frames = buildFrames(state.manifest, state.layer, now);
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

function setPlaying(on: boolean) {
  state.playing = on && state.seq.length > 1;
  clearInterval(state.timer);
  if (state.playing) {
    state.timer = window.setInterval(() => {
      state.pos = (state.pos + 1) % state.seq.length;
      showCurrentFrame();
    }, TICK_MS);
  }
  $("play").setAttribute("aria-label", state.playing ? "Pause radar animation" : "Play radar animation");
  $("play-icon").innerHTML = state.playing
    ? '<path d="M7 5h4v14H7zM13 5h4v14h-4z" fill="currentColor"/>'
    : '<path d="M8 5v14l11-7z" fill="currentColor"/>';
}

async function refreshManifest() {
  try {
    const keepTime = state.frames[state.seq[state.pos]]?.time;
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

// ---------- rendering: timeline, legend, chips ----------

const TIME_FMT = new Intl.DateTimeFormat(undefined, { weekday: "long", hour: "numeric", minute: "2-digit" });

function renderTimeline() {
  const frame = state.frames[state.seq[state.pos]];
  const slider = $<HTMLInputElement>("slider");
  slider.value = String(state.pos);
  if (!frame) {
    $("frame-title").textContent = "No frames yet";
    return;
  }
  const label = offsetLabel(frame.time, nowSec());
  const mode = label === "Now" ? "Now" : `${frame.time > nowSec() ? "Forecast" : "Past"} ${label}`;
  const source = frame.source === "nowcast" ? " · nowcast" : frame.source === "radar-hrrr" ? " · HRRR model" : "";
  const layerName = LAYERS.find((l) => l.id === state.layer)?.name ?? "Radar";
  $("frame-title").textContent = `${layerName} · ${mode}`;
  $("frame-time").textContent = `${TIME_FMT.format(new Date(frame.time * 1000))}${source}`;
  slider.setAttribute("aria-valuetext", `${mode}, ${TIME_FMT.format(new Date(frame.time * 1000))}`);
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
  const spec = LEGENDS[state.layer];
  const el = $("legend");
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
    ${alerts.length ? `<div class="section">${alerts.slice(0, 3).map((a) => `<div class="alert"><strong>${escapeHtml(a.event)}</strong><span>${a.ends ? `Until ${new Date(a.ends).toLocaleString([], { weekday: "short", hour: "numeric", minute: "2-digit" })}` : escapeHtml(a.severity)}</span></div>`).join("")}</div>` : ""}
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

function renderResults() {
  const list = $("search-results");
  list.hidden = results.length === 0;
  list.innerHTML = results
    .map((p, i) => `<li role="option" data-i="${i}" aria-selected="${i === activeResult}">${escapeHtml(p.name)}<small>${escapeHtml(p.admin1 ?? "")}</small></li>`)
    .join("");
}

$("search-input").addEventListener("input", (e) => {
  const q = (e.target as HTMLInputElement).value.trim();
  searchAbort?.abort();
  if (q.length < 2) {
    results = [];
    renderResults();
    return;
  }
  searchAbort = new AbortController();
  const signal = searchAbort.signal;
  window.setTimeout(async () => {
    if (signal.aborted) return;
    try {
      results = await searchPlaces(q, signal);
      activeResult = results.length ? 0 : -1;
      renderResults();
    } catch {
      /* superseded or offline */
    }
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
});

$("search-results").addEventListener("click", (e) => {
  const li = (e.target as HTMLElement).closest("li");
  if (li) choose(results[Number(li.dataset.i)]);
});

function choose(p: Place) {
  results = [];
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
  clearFrames();
  renderChips();
  rebuildFrames();
});

$("styles").addEventListener("click", (e) => {
  const b = (e.target as HTMLElement).closest<HTMLButtonElement>("button[data-style]");
  if (b && b.dataset.style !== state.mapStyle) void applyMapStyle(b.dataset.style as MapStyleId);
});

$("zoom").addEventListener("click", (e) => {
  const b = (e.target as HTMLElement).closest<HTMLButtonElement>("button[data-zoom]");
  if (!b || b.dataset.zoom === state.zoom) return;
  state.zoom = b.dataset.zoom as Zoom;
  renderChips();
  rebuildFrames(state.frames[state.seq[state.pos]]?.time);
});

$("play").addEventListener("click", () => setPlaying(!state.playing));

$("slider").addEventListener("input", (e) => {
  setPlaying(false);
  state.pos = Number((e.target as HTMLInputElement).value);
  showCurrentFrame();
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

document.addEventListener("keydown", (e) => {
  if ((e.target as HTMLElement).closest("input")) return;
  if (e.key === " ") {
    e.preventDefault();
    setPlaying(!state.playing);
  } else if (e.key === "ArrowRight" || e.key === "ArrowLeft") {
    setPlaying(false);
    const n = state.seq.length;
    if (!n) return;
    state.pos = (state.pos + (e.key === "ArrowRight" ? 1 : -1) + n) % n;
    showCurrentFrame();
  }
});

// Click the map: the value of the shown frame at that point.
const popup = new maplibregl.Popup({ closeButton: true, maxWidth: "240px" });
map.on("click", async (e) => {
  const frame = state.frames[state.seq[state.pos]];
  if (!frame) return;
  const { lat, lng } = e.lngLat;
  popup.setLngLat(e.lngLat).setHTML('<div class="readout"><small>Reading…</small></div>').addTo(map);
  const value = await readPoint(state.layer, frame.source, frame.timestamp, lat, lng).catch(() => null);
  const title = frame.source === "nowcast" ? "NOWCAST" : frame.source === "radar-hrrr" ? "HRRR FORECAST" : (LAYERS.find((l) => l.id === state.layer)?.name ?? "").toUpperCase();
  let text = "No data here";
  if (value != null) {
    if (state.layer === "radar") text = `${describeDbz(value)} <small>${Math.round(value)} dBZ</small>`;
    else if (state.layer === "air-quality") text = `AQI ${usAqiFromPm25(value)} · ${aqiLabel(usAqiFromPm25(value))} <small>${Math.round(value)} µg/m³</small>`;
    else if (state.layer === "ozone") text = `${Math.round(value)} ppb`;
    else text = `${value.toFixed(1)}`;
  }
  popup.setHTML(`<div class="readout"><b>${title}</b><div class="v">${text}</div><small>${lat.toFixed(3)}, ${lng.toFixed(3)}</small></div>`);
});

// ---------- boot ----------

// Every style (first load and each switch) needs the overlays put back.
map.on("style.load", () => {
  styleReady = true;
  addPlaceMarker();
  showCurrentFrame();
});

(async () => {
  document.documentElement.dataset.map = state.mapStyle;
  renderChips();
  map.setStyle(await loadStyle(state.mapStyle));
  await refreshManifest();
  void renderForecast();
  window.setInterval(refreshManifest, MANIFEST_POLL_MS);
})();
