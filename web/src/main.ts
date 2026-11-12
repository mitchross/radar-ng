import { pointOutlookHtml } from "./outlookReadout";
import * as maplibregl from "maplibre-gl";
import type { Map as MLMap, StyleSpecification } from "maplibre-gl";
// MapLibre picks its worker file by name at runtime, which a bundler can't
// follow; importing it explicitly bundles the worker (and its shared chunk).
import workerUrl from "maplibre-gl/dist/maplibre-gl-worker.mjs?worker&url";
import "maplibre-gl/dist/maplibre-gl.css";
import "./style.css";
import { ANALYTICS_AVAILABLE, analyticsEnabled, initAnalytics, setAnalyticsEnabled, track } from "./analytics";
import { buildFrames, isForecastSource, longZoomLabel, nowIndex, offsetLabel, playbackSequence, refreshKeepTime, sameFrames, type Frame, type Manifest, type Zoom } from "./frames";
import { escapeHtml } from "./html";
import { bannerHtml, fetchReplayCatalogue, fetchReplayManifest, fetchReplayWarnings, pickerHtml } from "./replay";
import type { ReplayCase } from "./shared/replayApi";
import type { ReplayWarningHistory } from "./shared/replayWarnings";
import {
  fullCaseSequence, isReplaySource, lookaheadPositions, LIVE_SOURCE, REPLAY_LAYERS, REPLAY_PALETTE, replayFrameNote, replayClockLabel, replayFrames,
  replayTileUrl, replayTimeLabel, replayWarningCollection, replayWarningCopy, replayWarningKey, sourceKey, SourceGuard, type ReplayManifest, type SourceContext,
} from "./shared/replaySource";
import { compass, describeFeature, incomingStorm, OVERLAYS, Overlays, WARNING_COLORS, type OverlayId } from "./overlays";
import { css, LEGEND_TICKS, PALETTES, typeRamps, type PaletteId } from "./palettes";
import { fetchServerPalettes, layerLegend, uvCategory } from "./layerLegends";
import type { ServerPalettes } from "./shared/serverPalettes";
import {
  aqiLabel,
  conditionOf,
  describeDbz,
  fetchAlerts,
  fetchForecast,
  explainAlert,
  fetchBriefing,
  fetchManifest,
  forecastHourLabel,
  iconSvg,
  readPoint,
  reversePlace,
  searchPlaces,
  usAqiFromPm25,
  type Forecast,
  type Place,
} from "./weather";
import { WindLayer } from "./wind";
import { SatelliteLayer } from "./satellite";
import { forecastTimeMs, isNightAt } from "./shared/forecastTime";
import { satelliteFrames } from "./shared/satellite";
import { PackedRadarLayer, type PackedFrameRef } from "./packed/layer";
import { interpolateNowcast, nextHourHeadline, nowcastVerdict } from "./nowcast";
import { CONDITION_HEADLINES, conditionGradient, getCumulusCondition } from "./shared/conditions";
import { describeNowcastSkill } from "./shared/nowcastSkill";
import { SEVERE_CREDIT, SEVERE_LAYERS, formatSevereReading, severeLayer } from "./shared/severe";
import { RAIN_ALERT_LEAD_OPTIONS, RAIN_START_IN_HR } from "./shared/rainAlerts";
import type { RadarNowcastResponse } from "./shared/nowcastTypes";
import { BrowserRainAlerts } from "./rainAlerts";
import { BrowserStormNotice } from "./stormNotice";
import { BrowserAlertNotifications } from "./alertNotifications";
import { ALERT_PREFERENCES, parseAlertPreference, parseAlertBounds, type AlertBounds } from "./shared/alertLevels";
import { approachCopy } from "./shared/stormCone";
import { startPeriodic } from "./refresh";
import { observedFrameTime, radarNoticeCopy, radarStatus } from "./radarStatus";
import { Level2Layer } from "./level2";
import { closeUpFrames, scanClock, scanFull } from "./shared/level2";
import { bootLocationStep, cardAfterGeoError, LOCATION_CARD_COPY, locateClickStep, queryLocationPermission, type LocationCardKind } from "./location";

const rainAlerts = new BrowserRainAlerts();
const stormNotice = new BrowserStormNotice();
const nwsNotifications = new BrowserAlertNotifications(rainAlerts, openNotificationRadar);

function openNotificationRadar(bounds: AlertBounds | null): void {
  toggleSheet(false);
  if (bounds) map.fitBounds([[bounds[0], bounds[1]], [bounds[2], bounds[3]]], { padding: 60, maxZoom: 11 });
}

if ("serviceWorker" in navigator) navigator.serviceWorker.addEventListener("message", (event) => {
  if (event.data?.kind === "nws-alert") openNotificationRadar(parseAlertBounds(event.data.bounds));
});
/** iOS Safari exposes Notification only to pages installed on the Home Screen. */
const IOS_BROWSER = /iPhone|iPad|iPod/.test(navigator.userAgent) && !matchMedia("(display-mode: standalone)").matches;

maplibregl.setWorkerUrl(workerUrl);
// Default is 16. Tiles share one HTTP/3 connection, and each costs a ~140 ms tunnel round trip.
maplibregl.setMaxParallelImageRequests(48);
let serverPalettes: ServerPalettes | null = null;
const serverPalettesReady = fetchServerPalettes();
void serverPalettesReady.then((p) => {
  serverPalettes = p;
  renderLegend();
});
void initAnalytics();

// ---------- configuration ----------

const LAYERS: { id: string; name: string; needs: string }[] = [
  { id: "radar", name: "Radar", needs: "radar" },
  { id: "temperature", name: "Temperature", needs: "temperature" },
  { id: "feels-like", name: "Feels like", needs: "feels-like" },
  { id: "dewpoint", name: "Dew point", needs: "dewpoint" },
  { id: "wind", name: "Wind", needs: "wind" },
  { id: "precip-accum", name: "Rain total", needs: "precip-accum" },
  { id: "precip-24h", name: "Past 24 hr precip", needs: "precip-24h" },
  { id: "cloud", name: "Clouds", needs: "cloud" },
  { id: "uv", name: "UV index", needs: "uv" },
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
// Forecast matches the API's max-age; NWS alerts change faster.
const FORECAST_REFRESH_MS = 15 * 60_000;
const ALERTS_REFRESH_MS = 5 * 60_000;

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
  satellite: stored<boolean>("rng.satellite", false),
  ai: stored<boolean>("rng.ai", true),
  zoom: "1h" as Zoom,
  manifest: null as Manifest | null,
  /** Explicit data source: the live feeds, or one pinned replay case revision. */
  source: LIVE_SOURCE as SourceContext,
  replay: null as { case: ReplayCase; manifest: ReplayManifest | null; warnings: ReplayWarningHistory | null; error: string | null } | null,
  /** True until the first manifest poll settles. */
  manifestLoading: true,
  manifestFailed: false,
  frames: [] as Frame[],
  seq: [] as number[],
  pos: 0,
  playing: false,
  timer: 0 as number | undefined,
  forecast: null as Forecast | null,
};
if (!PALETTES[state.palette]) state.palette = "classic";

const nowSec = () => Math.floor(Date.now() / 1000);
/** Switching source aborts every in-flight request of the old source. */
const sourceGuard = new SourceGuard();
const replaying = () => isReplaySource(state.source);
const activeManifest = (): Manifest | null => (replaying() ? ((state.replay?.manifest as Manifest | null) ?? null) : state.manifest);
const $ = <T extends HTMLElement>(id: string) => document.getElementById(id) as T;

// ---------- map ----------

async function loadStyle(id: MapStyleId): Promise<StyleSpecification> {
  const res = await fetch(MAP_STYLES[id].url);
  if (!res.ok) throw new Error(`${MAP_STYLES[id].url}: ${res.status}`);
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
/** Packed data tiles (docs/reference/packed-tiles.md); `?raster=1` turns them off. */
const packed = new URLSearchParams(location.search).get("raster") === "1" ? null : new PackedRadarLayer(state.palette, state.opacity);

/** GOES imagery goes under the lowest radar layer (raster frame or packed), else under the labels. */
const satellite = new SatelliteLayer(map, state.satellite, () => map.getStyle().layers?.find((l) => l.id.startsWith("wx-") || l.id === packed?.id)?.id ?? radarBeforeId());

/** Level II close-up: only while Observed radar is the live layer; everything else is MRMS as before. */
const level2 = new Level2Layer(map, {
  palette: () => state.palette,
  active: () => state.layer === "radar" && !replaying(),
  opacity: () => state.opacity,
  beforeId: () => radarBeforeId(),
  changed: () => {
    const was = closeUpOn;
    rebuildFrames(was && level2.snapshot().view ? currentFrame()?.time : undefined);
    renderLevel2();
  },
});
let closeUpOn = false;

/** True between a style's "style.load" and the next setStyle; overlays can be added only then. */
let styleReady = false;

/** Frames added as raster layers, keyed by tile path; most recently used last. */
const cached = new Map<string, string>();
let shownKey: string | null = null;

function frameKey(f: Frame) {
  return `${sourceKey(state.source)}|${f.source}|${replaying() ? REPLAY_PALETTE : state.palette}|${f.path}`;
}

/** Radar sits under the basemap labels and every vector overlay. */
function radarBeforeId(): string | undefined {
  return map.getStyle().layers?.find((l) => l.type === "symbol" || l.id.startsWith("ov-"))?.id;
}

/** The packed slot reference for a frame, or null when the frame uses raster tiles. */
function packedRef(f: Frame): PackedFrameRef | null {
  return !replaying() && packed?.covers(f.source, f.timestamp) ? { layer: f.source, timestamp: f.timestamp } : null;
}

function ensurePackedLayer() {
  if (packed && !packed.failed && !map.getLayer(packed.id)) map.addLayer(packed, radarBeforeId());
}

/** Hands the manifest and the playback order to the packed layer. */
function syncPacked() {
  if (!packed) return;
  packed.setManifest(replaying() ? null : state.manifest);
  packed.setSequence(state.seq.flatMap((i) => packedRef(state.frames[i]) ?? []));
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
  const template = state.replay?.manifest?.tile_url_template;
  const tiles = replaying() && template ? replayTileUrl(`${location.origin}${template}`, f.source, REPLAY_PALETTE, f.path) : `${location.origin}/tiles/${f.source}/${state.palette}/${path}/{z}/{x}/{y}.png`;
  map.addSource(id, {
    type: "raster",
    tiles: [tiles],
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
  level2.clear();
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
  syncReplayWarnings();
  if (!styleReady || state.seq.length === 0) return;
  const frame = currentFrame();
  if (!frame) return;
  overlays.setFrameTime(frame.time);
  ensurePackedLayer();
  const keep = new Set<string>();
  for (const at of lookaheadPositions(state.pos, state.seq.length, PREFETCH_AHEAD)) {
    const f = state.frames[state.seq[at]];
    if (f && !f.gap && !packedRef(f)) {
      ensureFrame(f);
      keep.add(frameKey(f));
    }
  }
  const ref = packedRef(frame);
  packed?.setFrame(ref);
  const key = ref || frame.gap ? null : frameKey(frame);
  if (key) map.setPaintProperty(cached.get(key)!, "raster-opacity", state.opacity);
  if (shownKey && shownKey !== key) {
    const prev = cached.get(shownKey);
    if (prev && map.getLayer(prev)) map.setPaintProperty(prev, "raster-opacity", 0);
  }
  shownKey = key;
  evict(keep);
  level2.sync();
  level2.show(currentFrame()?.level2 ?? null);
  renderLevel2();
  syncSatellite();
  if (wind.enabled && !replaying()) void syncWind();
}

/** The satellite frame for the playhead, with the next PREFETCH_AHEAD timeline frames preloading. */
function syncSatellite() {
  const frame = currentFrame();
  if (!styleReady || !frame || !satellite.enabled || replaying()) return;
  const ahead = Array.from({ length: PREFETCH_AHEAD }, (_, k) => state.frames[state.seq[(state.pos + k + 1) % state.seq.length]]?.time ?? frame.time);
  satellite.show(frame.time, ahead);
}

function nextFrameReady(): boolean {
  const f = state.frames[state.seq[(state.pos + 1) % state.seq.length]];
  if (f?.gap) return true;
  if (f && !replaying() && !satellite.isReady(f.time)) return false;
  const ref = f && packedRef(f);
  if (ref) return packed!.isFrameReady(ref);
  const id = f && cached.get(frameKey(f));
  return !id || map.isSourceLoaded(id);
}

async function applyMapStyle(id: MapStyleId) {
  state.mapStyle = id;
  store("rng.mapStyle", id);
  document.documentElement.dataset.map = id;
  let style: StyleSpecification;
  try {
    style = await loadStyle(id);
  } catch (err) {
    console.error(err);
    toast("Couldn't load the map style");
    return;
  }
  clearFrames();
  satellite.clear();
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

/** The case layer on screen: composite is the radar layer's second product. */
const replayLayer = () => (state.layer === "radar" && state.product === "radar-composite" ? "radar-composite" : state.layer);

/** Live frames: the normal MRMS timeline, or the last six Level II sweeps over their MRMS frames. */
function liveTimeline(manifest: Manifest, now: number): { frames: Frame[]; seq: number[]; closeUp: boolean } {
  const base = buildFrames(manifest, state.layer, now, state.product);
  const view = state.layer === "radar" ? level2.snapshot().view : null;
  if (view) {
    const observed = base.filter((f) => !isForecastSource(f.source) && f.time <= now);
    const frames = closeUpFrames(view, observed).map((c): Frame =>
      c.gap
        ? { time: c.time, timestamp: c.level2.timestamp, path: "", source: "radar", maxZoom: 7, gap: true, level2: c.level2 }
        : { ...observed[c.mrmsIndex], time: c.time, level2: c.level2 },
    );
    return { frames, seq: frames.map((_, i) => i), closeUp: true };
  }
  return { frames: base, seq: playbackSequence(base, state.zoom, now), closeUp: false };
}

function rebuildFrames(keepTime?: number) {
  const inReplay = replaying();
  if (inReplay ? !state.replay?.manifest : !state.manifest) return;
  const now = nowSec();
  if (inReplay) {
    state.frames = replayFrames(state.replay!.manifest!, replayLayer());
    state.seq = fullCaseSequence(state.frames.length);
    closeUpOn = false;
  } else {
    const t = liveTimeline(state.manifest!, now);
    state.frames = t.frames;
    state.seq = t.seq;
    closeUpOn = t.closeUp;
  }
  document.body.classList.toggle("closeup", closeUpOn);
  const newest = closeUpOn ? state.frames[state.frames.length - 1]?.time : state.frames[nowIndex(state.frames, now)]?.time;
  const target = keepTime ?? (inReplay ? state.frames[0]?.time : newest);
  let best = 0;
  state.seq.forEach((idx, p) => {
    if (Math.abs(state.frames[idx].time - (target ?? now)) < Math.abs(state.frames[state.seq[best]].time - (target ?? now))) best = p;
  });
  state.pos = best;
  syncPacked();
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
  if (replaying()) return;
  const ticket = sourceGuard.ticket();
  try {
    const keepTime = refreshKeepTime(currentFrame(), latestObserved(), state.playing);
    const hadManifest = state.manifest !== null;
    const chipsBefore = availabilityKey();
    const fresh = await fetchManifest(ticket.signal);
    if (!ticket.current()) return;
    state.manifest = fresh;
    state.manifestFailed = false;
    satellite.setFrames(satelliteFrames(state.manifest));
    // renderChips also rebuilds the open settings sheet, so skip it unless availability changed.
    if (!hadManifest || availabilityKey() !== chipsBefore) renderChips();
    if (!LAYERS.some((l) => l.id === state.layer && layerAvailable(l.needs)) && !(severeLayer(state.layer) && layerAvailable(state.layer))) state.layer = "radar";
    const now = nowSec();
    const { frames, seq } = liveTimeline(state.manifest, now);
    if (hadManifest && sameFrames(state.frames, frames) && seq.length === state.seq.length && seq.every((v, i) => v === state.seq[i])) {
      // Packed bucket versions change while the frame list stays the same.
      syncPacked();
      syncSatellite();
      renderAxis();
    } else rebuildFrames(keepTime);
  } catch (err) {
    if (!ticket.current()) return;
    state.manifestFailed = true;
    console.error(err);
  } finally {
    state.manifestLoading = false;
    renderRadarStatus();
  }
}

let noticeKey = "";

/** The freshness chip and, when the chip alone can't explain it, the notice with Retry. */
function renderRadarStatus() {
  if (replaying()) {
    $("live").hidden = true;
    $("radar-notice").hidden = true;
    return;
  }
  const status = radarStatus({
    frameTimeSeconds: observedFrameTime(state.manifest, nowSec()),
    refreshFailed: state.manifestFailed,
    loading: state.manifestLoading,
  });
  const chip = $("live");
  chip.hidden = false;
  chip.dataset.tone = status.tone;
  chip.textContent = status.label;
  chip.setAttribute("aria-label", status.accessibilityLabel);
  const notice = $("radar-notice");
  // The boot error card already explains a dead API.
  const copy = $("boot-error").hidden ? radarNoticeCopy(status, state.frames.length > 0) : null;
  notice.hidden = !copy;
  const key = copy ? `${copy.title}|${copy.retry}` : "";
  if (!copy || key === noticeKey) return;
  noticeKey = key;
  notice.setAttribute("role", copy.retry ? "alert" : "status");
  notice.innerHTML =
    `<span class="${copy.retry ? "mark" : "spinner"}" aria-hidden="true"></span>` +
    `<div><b>${escapeHtml(copy.title)}</b><p>${escapeHtml(copy.message)}</p></div>` +
    (copy.retry ? `<button id="radar-retry" class="retry" aria-label="Retry loading radar">Retry</button>` : "");
}

/** The layers and products the chips and sheet offer. */
function availabilityKey(): string {
  return [...LAYERS.map((l) => l.needs), ...SEVERE_LAYERS.map((l) => l.id), "radar-composite", "satellite"].filter(layerAvailable).join();
}

function layerAvailable(key: string) {
  if (replaying() && !(REPLAY_LAYERS as readonly string[]).includes(key)) return false;
  const layer = activeManifest()?.layers[key];
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
const SOURCE_SUFFIX: Record<string, string> = {
  nowcast: " · nowcast",
  "radar-hrrr": " · HRRR model",
  "radar-gfs": " · GFS model",
  "radar-composite": " · composite",
};
const SOURCE_TITLE: Record<string, string> = {
  nowcast: "NOWCAST",
  "radar-hrrr": "HRRR FORECAST",
  "radar-gfs": "GFS FORECAST",
  "radar-composite": "COMPOSITE",
};

function latestObserved(): Frame | undefined {
  for (let i = state.frames.length - 1; i >= 0; i--) {
    const f = state.frames[i];
    if (!isForecastSource(f.source) && f.time <= nowSec()) return f;
  }
  return undefined;
}

function renderTimeline() {
  const frame = currentFrame();
  const slider = $<HTMLInputElement>("slider");
  slider.value = String(state.pos);
  if (replaying()) return renderReplayTimeline(frame, slider);
  if (!frame) {
    $("frame-title").textContent = "No frames yet";
    return;
  }
  if (closeUpOn && frame.level2) {
    $("frame-title").textContent = "Radar · Level II close-up";
    const note = frame.gap ? " · no MRMS underlay within 10 min" : "";
    $("frame-time").textContent = `Scan ${scanFull(frame.level2.scanEndMs)}${note}`;
    slider.setAttribute("aria-valuetext", `Level II scan ${scanClock(frame.level2.scanEndMs)}${note}`);
    return;
  }
  const label = offsetLabel(frame.time, nowSec());
  const mode = label === "Now" ? "Now" : `${frame.time > nowSec() ? "Forecast" : "Past"} ${label}`;
  const source = SOURCE_SUFFIX[frame.source] ?? "";
  const layerName = LAYERS.find((l) => l.id === state.layer)?.name ?? severeLayer(state.layer)?.name ?? "Radar";
  $("frame-title").textContent = `${layerName} · ${mode}`;
  $("frame-time").textContent = `${TIME_FMT.format(new Date(frame.time * 1000))}${source}`;
  slider.setAttribute("aria-valuetext", `${mode}, ${TIME_FMT.format(new Date(frame.time * 1000))}`);
}

function renderAxis() {
  if (replaying()) return renderReplayAxis();
  const n = state.seq.length;
  const now = nowSec();
  if (closeUpOn) {
    $("track-segments").innerHTML = n ? `<span class="past" style="left:0;width:100%"></span>` : "";
    $("axis").innerHTML = [0, 0.25, 0.5, 0.75, 1].map((f) => `<span>${escapeHtml(scanClock(state.frames[state.seq[Math.round(f * (n - 1))]].time * 1000))}</span>`).join("");
    return;
  }
  const last = Math.max(1, n - 1);
  const pct = (p: number) => `${(p / last) * 100}%`;
  const lastAtOrBefore = (sec: number) => {
    let p = -1;
    while (p + 1 < n && state.frames[state.seq[p + 1]].time <= sec) p++;
    return p;
  };
  const nowP = Math.max(0, lastAtOrBefore(now));
  const ncP = Math.max(nowP, lastAtOrBefore(now + 3600));
  const gfsAt = state.seq.findIndex((i) => state.frames[i].source === "radar-gfs");
  const longP = gfsAt < 0 ? last : Math.max(ncP, gfsAt - 1);
  $("track-segments").innerHTML = n
    ? `<span class="past" style="left:0;width:${pct(nowP)}"></span>` +
      `<span class="nowcast" style="left:${pct(nowP)};width:calc(${pct(ncP)} - ${pct(nowP)})"></span>` +
      `<span class="model" style="left:${pct(ncP)};width:calc(${pct(longP)} - ${pct(ncP)})"></span>` +
      (gfsAt < 0 ? "" : `<span class="model long" style="left:${pct(longP)};right:0"></span>`) +
      `<span class="now-marker" style="left:${pct(nowP)}"></span>`
    : "";
  const long = document.querySelector<HTMLButtonElement>('#zoom button[data-zoom="48h"]');
  if (long) long.textContent = longZoomLabel(state.frames, now);
  $("axis").innerHTML = n
    ? [0, 0.25, 0.5, 0.75, 1]
        .map((f) => {
          const l = offsetLabel(state.frames[state.seq[Math.round(f * (n - 1))]].time, now);
          return `<span class="${l === "Now" ? "now-label" : ""}">${l}</span>`;
        })
        .join("")
    : "";
}

/** The one-line hint and source credit under a severe layer's legend; empty for other layers. */
function severeNote(layer: string): string {
  const info = severeLayer(layer);
  return info ? `<div class="legend-note">${escapeHtml(info.hint)} · ${SEVERE_CREDIT}</div>` : "";
}

function renderLegend() {
  const el = $("legend");
  if (state.layer === "radar") {
    const bands = PALETTES[replaying() ? REPLAY_PALETTE : state.palette].bands;
    const lo = bands[0].min;
    const hi = 75;
    const pos = (dbz: number) => `${((dbz - lo) / (hi - lo)) * 100}%`;
    el.hidden = false;
    el.innerHTML =
      `<div class="legend-title"><span>Precipitation</span><span class="legend-tag">${PALETTES[state.palette].name}</span></div>` +
      `<div class="legend-bar" style="background:linear-gradient(90deg,${bands.map((b, i) => `${css(b.rgba)} ${pos(b.min)} ${pos(bands[i + 1]?.min ?? hi)}`).join(",")})"></div>` +
      `<div class="legend-ticks">${LEGEND_TICKS.map(([dbz, name]) => `<span style="left:${pos(dbz)}">${name}</span>`).join("")}</div>`;
    if (closeUpOn) el.innerHTML += `<div class="legend-note">Level II close-up: one radar's reflectivity aloft, not surface rain. Same colours as MRMS.</div>`;
    // Observed radar is colored by precipitation type when the server says so.
    if (!replaying() && state.manifest?.layers.radar?.precip_type === true) {
      el.innerHTML +=
        `<div class="legend-types">` +
        typeRamps(state.palette)
          .map((r) => `<div class="legend-type"><span>${r.label}</span><div class="legend-type-bar" style="background:${r.gradient}"></div></div>`)
          .join("") +
        `</div>`;
    }
    return;
  }
  const table = layerLegend(serverPalettes, replaying() ? REPLAY_PALETTE : state.palette, state.layer, activeManifest()?.layers[state.layer]?.palette_key);
  if (table) {
    el.hidden = false;
    el.innerHTML =
      `<div class="legend-title">${table.title}</div>` +
      `<div class="legend-bar" style="background:${table.gradient}"></div>` +
      `<div class="legend-labels">${table.labels.map((l) => `<span>${l}</span>`).join("")}</div>` +
      severeNote(state.layer);
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
  if (replaying()) return renderReplaySheet();
  const seg = (group: string, items: [string, string, string?][], current: string) =>
    `<div class="segmented wide" role="radiogroup" data-group="${group}">${items
      .map(([id, name, hint]) => `<button role="radio" data-value="${id}" aria-checked="${id === current}"${hint ? ` title="${hint}"` : ""}>${name}</button>`)
      .join("")}</div>`;
  const composite = layerAvailable("radar-composite");
  const severe = SEVERE_LAYERS.filter((l) => layerAvailable(l.id));
  const severeHint = severeLayer(state.layer)?.hint ?? "Hail, storm rotation and storm height";
  const toggles = [
    ...OVERLAYS.map((o) => ({ id: o.id, name: o.name, hint: o.hint, on: state.overlays.has(o.id) })),
    ...(layerAvailable("satellite") ? [{ id: "satellite", name: "Satellite clouds", hint: "GOES imagery under the radar, every 10 minutes", on: state.satellite }] : []),
    { id: "wind", name: "Wind flow", hint: "Animated HRRR surface wind", on: state.wind },
    { id: "ai", name: "AI narration", hint: "Local-LLM summaries; hides itself if the model is down", on: state.ai },
  ];
  $("sheet-body").innerHTML = `
    <div class="sheet-section"><div class="section-label">RADAR</div>
      ${composite ? seg("product", (Object.keys(PRODUCTS) as ProductId[]).map((id) => [id, PRODUCTS[id].name, PRODUCTS[id].hint]), state.product) : ""}
      <p class="hint">${PRODUCTS[state.product].hint}</p>
      ${seg("palette", (Object.keys(PALETTES) as PaletteId[]).map((id) => [id, PALETTES[id].name, id === "muted" ? "Colorblind-safe (viridis)" : undefined]), state.palette)}
      <p class="hint">Muted is the colorblind-safe palette (viridis). Warning outlines are also labelled TOR, SVR or FFW.</p>
      <label class="range-row"><span>Opacity</span><input id="opacity" type="range" min="0.3" max="1" step="0.05" value="${state.opacity}" aria-label="Radar opacity" /></label>
    </div>
    ${severe.length ? `<div class="sheet-section"><div class="section-label">SEVERE</div>
      ${seg("severe", severe.map((l) => [l.id, l.name, l.hint]), state.layer)}
      <p class="hint">${escapeHtml(severeHint)} · ${SEVERE_CREDIT}</p>
    </div>` : ""}
    <div class="sheet-section"><div class="section-label">OVERLAYS</div>
      ${toggles.map((t) => `<label class="toggle"><span><b>${t.name}</b><small>${t.hint}</small></span><input type="checkbox" data-overlay="${t.id}" ${t.on ? "checked" : ""} /><i></i></label>`).join("")}
    </div>
    <div class="sheet-section"><div class="section-label">NWS ALERTS</div>
      <label class="range-row"><span>NWS alert interruptions</span><select id="nws-alert-level" aria-label="NWS alert interruptions">${ALERT_PREFERENCES.map(p => `<option value="${p.value}" ${p.value === nwsNotifications.preference ? "selected" : ""}>${p.label}</option>`).join("")}</select></label>
      <button id="test-nws-alert" type="button">Send test alert</button>
      <p class="hint">Chime: watches/advisories · Warning: warnings · Critical: tornadoes, emergencies, destructive thunderstorms. Browser/system sound settings apply. Keep this page open; this is not a replacement for official warnings.</p>
    </div>
    <div class="sheet-section"><div class="section-label">RAIN ALERTS</div>
      <label class="toggle"><span><b>Rain starting soon</b><small>Local notifications from your radar forecast</small></span><input id="rain-alerts" type="checkbox" ${rainAlerts.enabled ? "checked" : ""} ${rainAlerts.supported ? "" : "disabled"} /><i></i></label>
      <label class="range-row"><span>Notify me before rain</span><select id="rain-alert-lead" aria-label="Rain alert lead time">${RAIN_ALERT_LEAD_OPTIONS.map((lead) => `<option value="${lead}" ${lead === rainAlerts.leadMinutes ? "selected" : ""}>${lead} min</option>`).join("")}</select></label>
      <p class="hint" role="status">${escapeHtml(rainAlerts.supported ? rainAlerts.message : IOS_BROWSER ? "On iPhone, add this page to your Home Screen (Share → Add to Home Screen) to get rain alerts." : "Notifications are unavailable in this browser. Use the app for rain alerts.")}</p>
    </div>
    <div class="sheet-section"><div class="section-label">STORM NOTICE</div>
      <label class="toggle"><span><b>Strong storm heading my way</b><small>Radar estimate for 55+ dBZ cells within 10 km in 30 min. Not an NWS warning. On this device only</small></span><input id="storm-notice" type="checkbox" ${stormNotice.enabled ? "checked" : ""} /><i></i></label>
    </div>
    ${ANALYTICS_AVAILABLE ? `<div class="sheet-section"><div class="section-label">PRIVACY</div>
      <label class="toggle"><span><b>Usage analytics</b><small>Self-hosted PostHog: anonymous events and session replay. Off sends nothing.</small></span><input type="checkbox" id="analytics" ${analyticsEnabled() ? "checked" : ""} /><i></i></label>
    </div>` : ""}
    <div class="sheet-section"><div class="section-label">KEYBOARD</div>
      <p class="hint"><kbd>Space</kbd> play · <kbd>←</kbd><kbd>→</kbd> step · <kbd>W</kbd> warnings · <kbd>S</kbd> storms · <kbd>L</kbd> lightning · <kbd>H</kbd> hurricanes</p>
    </div>`;
}

// ---------- forecast panel ----------

const deg = (v: number | null | undefined) => (v == null ? "—" : `${Math.round(v)}°`);

function hourIndex(times: string[], utcOffsetSeconds?: number): number {
  const cutoff = Date.now() - 30 * 60_000;
  const i = times.findIndex((t) => forecastTimeMs(t, utcOffsetSeconds) >= cutoff);
  return Math.max(0, i);
}

/** `refresh` re-renders in place: it keeps the scroll position, the briefing, and the old data if the fetch fails. */
async function renderForecast(refresh = false) {
  const body = $("panel-body");
  const { latitude: lat, longitude: lon } = state.place;
  let fc: Forecast;
  try {
    fc = await fetchForecast(lat, lon);
  } catch {
    if (refresh) return;
    body.innerHTML = `<p class="muted">Can't reach the forecast API.</p><button id="forecast-retry" class="retry">Retry</button>`;
    return;
  }
  const aq = await airQualityNow(lat, lon);
  // A place chosen while this was in flight owns the panel now.
  if (lat !== state.place.latitude || lon !== state.place.longitude) return;
  state.forecast = fc;
  const c = fc.current;
  const cond = conditionOf(c.weather_code);
  const sky = getCumulusCondition(c.weather_code, isNightAt(Date.now(), fc.daily, fc.utc_offset_seconds));
  const start = hourIndex(fc.hourly.time, fc.utc_offset_seconds);
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
  const fmtHour = (t: string, i: number) => (i === 0 ? "Now" : forecastHourLabel(t, fc.utc_offset_seconds));
  const dayName = (d: string, i: number) => (i === 0 ? "Today" : new Date(`${d}T00:00:00`).toLocaleDateString([], { weekday: "short" }));
  const inHg = c.surface_pressure == null ? null : (c.surface_pressure / 33.8639).toFixed(2);
  const aqi = aq.pm25 == null ? null : usAqiFromPm25(aq.pm25);

  const scrollTop = body.scrollTop;
  const hourlyLeft = body.querySelector(".hourly")?.scrollLeft ?? 0;
  const briefing = document.getElementById("briefing");
  body.innerHTML = `
    <h1 class="place">${escapeHtml(state.place.name)}${state.place.admin1 ? `<span class="muted" style="font-size:15px;font-family:var(--ui)"> · ${escapeHtml(state.place.admin1)}</span>` : ""}</h1>
    <div class="now">
      <div>
        <div class="now-cond">${CONDITION_HEADLINES[sky]}</div>
        <div class="now-temp">${deg(c.temperature_2m)}</div>
      </div>
      ${iconSvg(cond, 84)}
    </div>
    <div class="now-meta">Feels ${deg(c.apparent_temperature)} · H ${deg(fc.daily.temperature_2m_max[0])} L ${deg(fc.daily.temperature_2m_min[0])}</div>
    <div id="briefing" class="briefing" hidden></div>
    <div id="threat" class="threat" hidden></div>
    <div id="alerts" class="section" hidden></div>
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
  body.scrollTop = scrollTop;
  // CSS picks the light or dark stops from the basemap, like the other tokens.
  const gradient = (dark: boolean) => `linear-gradient(180deg, ${conditionGradient(sky, dark).join(", ")})`;
  $("panel").style.setProperty("--cond-light", gradient(false));
  $("panel").style.setProperty("--cond-dark", gradient(true));
  body.querySelector(".hourly")!.scrollLeft = hourlyLeft;
  renderThreat();
  void renderAlerts();
  if (refresh && briefing && !briefing.hidden) {
    $("briefing").hidden = false;
    $("briefing").innerHTML = briefing.innerHTML;
  } else void renderBriefing();
  void renderNextHour();
}

/** NWS alerts for the place, polled on their own cadence; a failed poll keeps the last result. */
async function renderAlerts() {
  const el = document.getElementById("alerts");
  const { latitude, longitude } = state.place;
  const alerts = await fetchAlerts(latitude, longitude).catch(() => null);
  if (!alerts || latitude !== state.place.latitude || longitude !== state.place.longitude) return;
  void nwsNotifications.check(alerts.flatMap(a => a.feature ? [a.feature] : []));
  if (!el || !document.body.contains(el)) return;
  el.hidden = alerts.length === 0;
  el.innerHTML = alerts.slice(0, 3).map((a) => `<div class="alert"><strong>${escapeHtml(a.event)}</strong><span>${a.ends ? `Until ${new Date(a.ends).toLocaleString([], { weekday: "short", hour: "numeric", minute: "2-digit" })}` : escapeHtml(a.severity)}</span></div>`).join("");
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
  const headline = nextHourHeadline(minutes);
  const peak = Math.max(0.1, ...minutes);
  // Same call to action as the app's rain banner: rain within the hour and alerts not on yet.
  const offerAlerts = !rainAlerts.enabled && rainAlerts.supported && minutes.some((v) => v > RAIN_START_IN_HR);
  el.hidden = false;
  el.innerHTML =
    `<div class="section-label">NEXT HOUR</div>` +
    (headline ? `<p class="nexthour-headline">${escapeHtml(headline)}</p>` : "") +
    `<p class="nexthour-verdict">${escapeHtml(nowcastVerdict(minutes))}</p>` +
    (offerAlerts ? `<button id="notify-me" class="notify">Notify me before rain</button>` : "") +
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
  const place: [number, number] = [state.place.longitude, state.place.latitude];
  const approaches = stormNotice.check(overlays.cells, place);
  const t = incomingStorm(overlays.cells, place);
  if (!t) {
    el.hidden = true;
    return;
  }
  const what = t.peakDbz >= 60 ? "Hail-producing storm" : t.peakDbz >= 50 ? "Heavy storm" : "Rain";
  const when = t.minutes === 0 ? "over you now" : `arriving in ~${t.minutes} min`;
  el.hidden = false;
  el.className = `threat ${t.peakDbz >= 55 ? "severe" : ""}`;
  const near = approaches[0];
  const notice = near ? `<span class="threat-notice"><b>${escapeHtml(approachCopy(near).title)}.</b> ${escapeHtml(approachCopy(near).body)}</span>` : "";
  el.innerHTML = `<strong>${what} ${when}</strong><span>${Math.round(t.peakDbz)} dBZ · moving ${compass(t.bearing)} at ${Math.round(t.speedMph)} mph</span>${notice}`;
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
  renderLocationCard(null);
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
  // The region is enough to see where people look; the place name stays out of analytics.
  track("place_chosen", { region: p.admin1 });
  results = [];
  searchStatus = null;
  renderResults();
  $<HTMLInputElement>("search-input").value = "";
  setPlace(p);
}

$("panel-body").addEventListener("click", (e) => {
  const t = e.target as HTMLElement;
  if (t.closest("#forecast-retry")) void renderForecast();
  else if (t.closest("#notify-me")) void enableRainAlerts();
});

/** The sheet toggle and the next-hour button share one path, so the hint and card stay in step. */
async function enableRainAlerts() {
  await rainAlerts.setEnabled(true);
  if (!rainAlerts.enabled) toast(rainAlerts.message);
  renderSheet();
  void renderNextHour();
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
  if (!replaying()) store("rng.layer", state.layer);
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
  if (show) $("replay-sheet").hidden = true;
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
    if (!replaying()) store("rng.product", value);
    track("radar_product_changed", { product: value });
    rebuildFrames(currentFrame()?.time);
  } else if (group === "severe" && value !== state.layer) {
    state.layer = value;
    if (!replaying()) store("rng.layer", state.layer);
    track("layer_changed", { layer: state.layer });
    clearFrames();
    renderChips();
    rebuildFrames();
    return;
  } else if (group === "palette" && value !== state.palette) {
    state.palette = value as PaletteId;
    store("rng.palette", value);
    packed?.setPalette(state.palette);
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
  packed?.setOpacity(state.opacity);
  const id = shownKey && cached.get(shownKey);
  if (id) map.setPaintProperty(id, "raster-opacity", state.opacity);
});

function setOverlay(id: OverlayId | "wind" | "ai" | "satellite", on: boolean) {
  track("overlay_toggled", { overlay: id, on });
  if (id === "satellite") {
    state.satellite = on;
    store("rng.satellite", on);
    satellite.setEnabled(on);
    syncSatellite();
  } else if (id === "ai") {
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
  if (el.id === "nws-alert-level") {
    void nwsNotifications.setPreference(parseAlertPreference(el.value)).then(granted => {
      if (!granted) toast("Notifications are blocked or unavailable. Check browser permissions.");
    });
  } else if (el.id === "rain-alerts") {
    el.disabled = true;
    if (el.checked) void enableRainAlerts();
    else void rainAlerts.setEnabled(false).then(() => {
      renderSheet();
      void renderNextHour();
    });
  } else if (el.id === "storm-notice") {
    void stormNotice.setEnabled(el.checked).then(renderThreat);
  } else if (el.id === "rain-alert-lead") {
    rainAlerts.setLead(el.value);
    void renderNextHour();
  } else if (el.id === "analytics") {
    void setAnalyticsEnabled(el.checked);
  } else if (el.id === "opacity") {
    // "input" fires on every drag tick; persist once on release.
    store("rng.opacity", state.opacity);
  }
  if (el.dataset.overlay) setOverlay(el.dataset.overlay as OverlayId | "wind" | "ai" | "satellite", el.checked);
});

function locate() {
  if (!navigator.geolocation) return toast("Location isn't available in this browser");
  navigator.geolocation.getCurrentPosition(
    async (pos) => {
      const { latitude, longitude } = pos.coords;
      const place = await reversePlace(latitude, longitude).catch(() => null);
      setPlace({ name: place?.name ?? "My location", admin1: place?.admin1, latitude, longitude });
    },
    (err) => {
      const card = cardAfterGeoError(err.code);
      if (card) renderLocationCard(card, true);
      else toast("Couldn't find your location");
    },
    { enableHighAccuracy: false, timeout: 10_000 },
  );
}

/** The pre-prompt before the browser's location dialog, or the recovery card after a denial. */
function renderLocationCard(kind: LocationCardKind | null, focus = false) {
  const el = $("location-card");
  el.hidden = !kind;
  if (!kind) return;
  const copy = LOCATION_CARD_COPY[kind];
  el.innerHTML =
    `<b>${copy.title}</b><p>${escapeHtml(copy.body(state.place.name))}</p><div class="actions">` +
    (copy.allow ? `<button data-action="allow" class="primary">Allow location</button>` : "") +
    `<button data-action="city" class="secondary">Choose a city</button></div>`;
  if (focus) el.querySelector<HTMLButtonElement>("button")?.focus();
}

$("location-card").addEventListener("click", (e) => {
  const action = (e.target as HTMLElement).closest<HTMLButtonElement>("button[data-action]")?.dataset.action;
  if (action === "allow") locate();
  else if (action === "city") {
    renderLocationCard(null);
    $("search-input").focus();
  }
});

$("locate").addEventListener("click", async () => {
  const step = locateClickStep(await queryLocationPermission());
  if (step === "locate") locate();
  else renderLocationCard(step, true);
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
  } else if (OVERLAY_KEYS[e.key.toLowerCase()] && !replaying()) {
    const id = OVERLAY_KEYS[e.key.toLowerCase()];
    setOverlay(id, !state.overlays.has(id));
    toast(`${OVERLAYS.find((o) => o.id === id)!.name} ${state.overlays.has(id) ? "on" : "off"}`);
  }
});

// Click the map: an overlay feature if one is under the cursor, else the shown frame's value there.
const popup = new maplibregl.Popup({ closeButton: true, maxWidth: "260px" });
let clickSeq = 0;
map.on("click", async (e) => {
  const click = ++clickSeq;
  if (replaying()) return showReplayWarningPopup(e);
  const outlook = pointOutlookHtml(e.lngLat.lng, e.lngLat.lat);
  const layers = overlays.clickable();
  const hit = layers.length ? map.queryRenderedFeatures(e.point, { layers })[0] : undefined;
  if (hit) {
    const html = describeFeature(hit, overlays.cells);
    if (html) {
      const alertId = hit.layer.id.startsWith("ov-warnings") && state.ai ? String(hit.properties?.id ?? "") : "";
      popup.setLngLat(e.lngLat).setHTML(`${html}${alertId ? '<div class="explain muted">✦ Explaining…</div>' : ""}<div class="outlook-slot">Loading SPC outlook…</div>`).addTo(map);
      void outlook.then((result) => {
        if (click !== clickSeq) return;
        const slot = popup.getElement()?.querySelector(".outlook-slot");
        if (slot) slot.innerHTML = result;
      });
      track("overlay_inspected", { layer: hit.layer.id });
      if (alertId) void fillExplanation(alertId);
      return;
    }
  }
  const frame = currentFrame();
  if (!frame) {
    popup.setLngLat(e.lngLat).setHTML('<div class="outlook-slot">Loading SPC outlook…</div>').addTo(map);
    const html = await outlook;
    if (click === clickSeq) popup.setHTML(html);
    return;
  }
  const { lat, lng } = e.lngLat;
  popup.setLngLat(e.lngLat).setHTML('<div class="readout"><small>Reading…</small></div>').addTo(map);
  const value = await readPoint(state.layer, frame.source, frame.timestamp, lat, lng).catch(() => null);
  if (click !== clickSeq) return; // a newer click owns the popup now
  track("point_inspected", { layer: state.layer, source: frame.source, has_value: value != null });
  const title = SOURCE_TITLE[frame.source] ?? (LAYERS.find((l) => l.id === state.layer)?.name ?? severeLayer(state.layer)?.name ?? "").toUpperCase();
  let text = "No data here";
  if (value != null) {
    if (state.layer === "radar") text = `${describeDbz(value)} <small>${Math.round(value)} dBZ</small>`;
    else if (state.layer === "air-quality") text = `AQI ${usAqiFromPm25(value)} · ${aqiLabel(usAqiFromPm25(value))} <small>${Math.round(value)} µg/m³</small>`;
    else if (state.layer === "ozone") text = `${Math.round(value)} ppb`;
    else if (["temperature", "feels-like", "dewpoint"].includes(state.layer)) text = `${Math.round(value)}°F`;
    else if (state.layer === "wind") text = `${Math.round(value)} mph`;
    else if (state.layer === "cloud") text = `${Math.round(value)}%`;
    else if (state.layer === "precip-accum" || state.layer === "precip-24h") text = `${value.toFixed(2)} in`;
    else if (state.layer === "uv") text = `UV ${Math.round(value)} · ${uvCategory(value)}`;
    else if (severeLayer(state.layer)) text = formatSevereReading(state.layer, value) ?? `${value.toFixed(1)}`;
    else text = `${value.toFixed(1)}`;
  }
  popup.setHTML(`<div class="readout"><b>${title}</b><div class="v">${text}</div><small>${lat.toFixed(3)}, ${lng.toFixed(3)}</small></div><div class="outlook-slot">Loading SPC outlook…</div>`);
  const outlookHtml = await outlook;
  if (click !== clickSeq) return;
  const slot = popup.getElement()?.querySelector(".outlook-slot");
  if (slot) slot.innerHTML = outlookHtml;
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

for (const id of ["ov-storm-cells", "ov-warnings-fill", "ov-tropical-position", "ov-tropical-points", "ov-watches-hit", "ov-discussions-hit"]) {
  map.on("mouseenter", id, () => (map.getCanvas().style.cursor = "pointer"));
  map.on("mouseleave", id, () => (map.getCanvas().style.cursor = ""));
}

// ---------- replay ----------

let l2PanelKey = "";

/** The Level II provenance line and panel for the shown sweep, and the MRMS fallback chip. */
function renderLevel2() {
  const fb = level2.fallback();
  const chip = $("level2-fallback");
  chip.hidden = !fb;
  if (fb) chip.textContent = `${fb.title} · ${fb.reason}`;
  const note = $("level2-note");
  const frame = closeUpOn && !replaying() ? currentFrame()?.level2 : undefined;
  note.hidden = !frame;
  if (!frame) {
    $("level2-panel").hidden = true;
    l2PanelKey = "";
    return;
  }
  const p = level2.provenanceHtml(frame);
  const key = `${frame.path}|${p.line}|${p.range}`;
  if (key === l2PanelKey) return;
  l2PanelKey = key;
  $("level2-line").textContent = p.line;
  $("level2-range").textContent = p.range;
  $("level2-rows").innerHTML = p.panel;
}

$("level2-note").addEventListener("click", () => {
  const panel = $("level2-panel");
  panel.hidden = !panel.hidden;
  $("level2-note").setAttribute("aria-expanded", String(!panel.hidden));
});

map.on("zoom", () => level2.sync());
map.on("moveend", () => {
  level2.sync();
  renderLevel2();
});

const EMPTY_FC = { type: "FeatureCollection" as const, features: [] };
let liveLayer = state.layer;
let replayWarnKey = "";
let replayCases: ReplayCase[] = [];

const replayZone = () => state.replay?.case.timezone ?? "UTC";

/** Absolute historical date, time and zone; never "Now". */
function renderReplayTimeline(frame: Frame | undefined, slider: HTMLInputElement) {
  const layerName = LAYERS.find((l) => l.id === state.layer)?.name ?? severeLayer(state.layer)?.name ?? "Radar";
  if (!frame) {
    $("frame-title").textContent = state.replay?.error ? "Replay unavailable" : "Loading replay…";
    $("frame-time").innerHTML = "&nbsp;";
    return;
  }
  const label = replayTimeLabel(frame.time, replayZone());
  const note = replayFrameNote(frame, replayZone());
  const product = state.layer === "radar" && state.product === "radar-composite" ? " · composite" : "";
  $("frame-title").textContent = `${layerName}${product} · Historical replay`;
  $("frame-time").textContent = note ? `${label} · ${note}` : label;
  slider.setAttribute("aria-valuetext", `Historical replay, ${label}`);
}

function renderReplayAxis() {
  const n = state.seq.length;
  $("track-segments").innerHTML = n ? `<span class="past" style="left:0;right:0"></span>` : "";
  $("axis").innerHTML = n
    ? [0, 0.25, 0.5, 0.75, 1].map((f) => `<span>${escapeHtml(replayClockLabel(state.frames[state.seq[Math.round(f * (n - 1))]].time, replayZone()))}</span>`).join("")
    : "";
}

function renderReplaySheet() {
  const seg = (group: string, items: [string, string, string?][], current: string) =>
    `<div class="segmented wide" role="radiogroup" data-group="${group}">${items
      .map(([id, name, hint]) => `<button role="radio" data-value="${id}" aria-checked="${id === current}"${hint ? ` title="${hint}"` : ""}>${name}</button>`)
      .join("")}</div>`;
  const severe = SEVERE_LAYERS.filter((l) => layerAvailable(l.id));
  $("sheet-body").innerHTML = `
    <div class="sheet-section"><div class="section-label">RADAR</div>
      ${layerAvailable("radar-composite") ? seg("product", (Object.keys(PRODUCTS) as ProductId[]).map((id) => [id, PRODUCTS[id].name, PRODUCTS[id].hint]), state.product) : ""}
      <p class="hint">${PRODUCTS[state.product].hint}</p>
      <label class="range-row"><span>Opacity</span><input id="opacity" type="range" min="0.3" max="1" step="0.05" value="${state.opacity}" aria-label="Radar opacity" /></label>
    </div>
    ${severe.length ? `<div class="sheet-section"><div class="section-label">SEVERE</div>
      ${seg("severe", severe.map((l) => [l.id, l.name, l.hint]), state.layer)}
      <p class="hint">${escapeHtml(severeLayer(state.layer)?.hint ?? "Hail, storm rotation and storm height")} · ${SEVERE_CREDIT}</p>
    </div>` : ""}
    <div class="sheet-section"><p class="hint">Replay shows archived radar and the warnings in force at each moment, in the classic palette. Live overlays, forecasts and point readings are off until you return to live.</p></div>`;
}

function renderReplayBanner() {
  const el = $("replay-banner");
  el.hidden = !state.replay;
  if (state.replay) el.innerHTML = bannerHtml(state.replay.case, state.replay.error);
}

/** Draw the warning versions in force at the playhead; only a changed set redraws. */
function syncReplayWarnings() {
  const r = state.replay;
  const frame = currentFrame();
  if (!replaying() || !r || !frame) return;
  const fc = replayWarningCollection(r.warnings, frame.time * 1000);
  const key = replayWarningKey(fc);
  if (key === replayWarnKey) return;
  replayWarnKey = key;
  overlays.setReplayWarnings(fc as never);
}

function showReplayWarningPopup(e: maplibregl.MapMouseEvent) {
  const layers = overlays.clickable();
  const hit = layers.length ? map.queryRenderedFeatures(e.point, { layers })[0] : undefined;
  if (!hit) return popup.remove();
  const p = hit.properties as Record<string, unknown>;
  const copy = replayWarningCopy(p, replayZone());
  const color = WARNING_COLORS.find(([ev]) => ev === p.event)?.[1] ?? "#9aa4be";
  popup
    .setLngLat(e.lngLat)
    .setHTML(
      `<div class="readout"><b style="color:${color}">${escapeHtml(copy.title).toUpperCase()}</b>${copy.badge ? `<div class="pill danger">${escapeHtml(copy.badge)}</div>` : ""}` +
        `<div class="v">${escapeHtml(copy.lines[0] ?? "")}</div><small>${escapeHtml(copy.lines.slice(1).join(" · "))}</small></div>`,
    )
    .addTo(map);
}

const paddedBounds = (b: [number, number, number, number]): [[number, number], [number, number]] => {
  const dx = (b[2] - b[0]) / 2;
  const dy = (b[3] - b[1]) / 2;
  return [[b[0] - dx, b[1] - dy], [b[2] + dx, b[3] + dy]];
};

async function enterReplay(c: ReplayCase) {
  sourceGuard.cancel();
  const ticket = sourceGuard.ticket();
  if (!replaying()) liveLayer = state.layer;
  setPlaying(false);
  popup.remove();
  toggleReplaySheet(false);
  toggleSheet(false);
  state.source = { kind: "replay", caseId: c.id, revision: c.revision };
  state.replay = { case: c, manifest: null, warnings: null, error: null };
  level2.sync();
  closeUpOn = false;
  document.body.classList.remove("closeup");
  state.frames = [];
  state.seq = [];
  state.pos = 0;
  replayWarnKey = "";
  if (!(REPLAY_LAYERS as readonly string[]).includes(state.layer)) state.layer = "radar";
  clearFrames();
  syncPacked();
  satellite.setEnabled(false);
  wind.setEnabled(false);
  overlays.setReplay(true);
  overlays.setReplayWarnings(EMPTY_FC);
  document.body.classList.add("replay");
  map.setMaxBounds(null);
  map.fitBounds([[c.bounds[0], c.bounds[1]], [c.bounds[2], c.bounds[3]]], { padding: 40, duration: 0 });
  map.setMaxBounds(paddedBounds(c.bounds));
  renderReplayBanner();
  renderRadarStatus();
  renderChips();
  renderLegend();
  renderTimeline();
  renderAxis();
  try {
    const [manifest, warnings] = await Promise.all([fetchReplayManifest(c, ticket.signal), fetchReplayWarnings(c, ticket.signal)]);
    if (!ticket.current()) return;
    state.replay.manifest = manifest;
    state.replay.warnings = warnings;
    renderChips();
    rebuildFrames();
  } catch (err) {
    if (!ticket.current()) return;
    console.error(err);
    state.replay.error = "Couldn't load this replay. The live radar is not shown in its place.";
    renderReplayBanner();
    renderTimeline();
  }
}

/** Leave replay: drop every case frame, then show live only after a fresh manifest arrives. */
async function returnToLive() {
  sourceGuard.cancel();
  setPlaying(false);
  popup.remove();
  state.source = LIVE_SOURCE;
  state.replay = null;
  state.frames = [];
  state.seq = [];
  state.pos = 0;
  state.manifest = null;
  state.manifestLoading = true;
  state.layer = liveLayer;
  replayWarnKey = "";
  clearFrames();
  level2.sync();
  document.body.classList.remove("replay");
  map.setMaxBounds(null);
  overlays.setReplay(false);
  satellite.setEnabled(state.satellite);
  wind.setEnabled(state.wind);
  syncPacked();
  renderReplayBanner();
  renderChips();
  renderLegend();
  renderTimeline();
  renderAxis();
  renderRadarStatus();
  await refreshManifest();
  if (!replaying()) showCurrentFrame();
}

function toggleReplaySheet(open?: boolean) {
  const sheet = $("replay-sheet");
  const show = open ?? sheet.hidden;
  sheet.hidden = !show;
  $("storms").setAttribute("aria-expanded", String(show));
  if (!show) return;
  toggleSheet(false);
  void loadCatalogue();
}

let catalogueTicket = 0;
async function loadCatalogue() {
  const mine = ++catalogueTicket;
  const body = $("replay-body");
  body.innerHTML = `<p class="hint">Loading famous storms…</p>`;
  try {
    const { cases } = await fetchReplayCatalogue();
    if (mine !== catalogueTicket) return;
    replayCases = cases;
    body.innerHTML = pickerHtml(cases);
  } catch (err) {
    if (mine !== catalogueTicket) return;
    console.error(err);
    body.innerHTML = `<p class="hint" role="alert">Couldn't load the replay list.</p><button id="replay-list-retry" class="retry" type="button">Retry</button>`;
  }
}

$("storms").addEventListener("click", () => toggleReplaySheet());
$("replay-close").addEventListener("click", () => toggleReplaySheet(false));
$("replay-body").addEventListener("click", (e) => {
  const t = e.target as HTMLElement;
  if (t.closest("#replay-list-retry")) return void loadCatalogue();
  const b = t.closest<HTMLButtonElement>("button[data-case]");
  const c = b && replayCases.find((x) => x.id === b.dataset.case && x.revision === b.dataset.revision);
  if (c) void enterReplay(c);
});
$("replay-banner").addEventListener("click", (e) => {
  const t = e.target as HTMLElement;
  if (t.closest("#return-live")) void returnToLive();
  else if (t.closest("#replay-retry") && state.replay) void enterReplay(state.replay.case);
});

// ---------- boot ----------

// Every style (first load and each switch) needs the overlays put back.
map.on("style.load", () => {
  styleReady = true;
  overlays.install();
  addPlaceMarker();
  showCurrentFrame();
});

overlays.onStorms = renderThreat;

/** The basemap style is the one boot request that cannot be retried later; without it the page is blank. */
async function boot() {
  $("boot-error").hidden = true;
  renderRadarStatus();
  try {
    map.setStyle(await loadStyle(state.mapStyle));
  } catch (err) {
    console.error(err);
    $("boot-error").hidden = false;
    $("panel-body").innerHTML = '<p class="muted">The radar API is unreachable.</p>';
    renderRadarStatus();
    return;
  }
  // Only reflectivity has a built-in table; other layers turn packed once the server tables arrive.
  if (packed)
    void serverPalettesReady.then((p) => {
      if (!p) return;
      packed.setServerPalettes(p);
      syncPacked();
      showCurrentFrame();
    });
  await refreshManifest();
  const alertBounds = new URL(location.href).searchParams.get("alertBounds");
  if (alertBounds) {
    try { openNotificationRadar(parseAlertBounds(JSON.parse(alertBounds))); } catch { /* invalid link */ }
    const url = new URL(location.href);
    url.searchParams.delete("alertBounds");
    history.replaceState(null, "", url);
  }
  overlays.start();
  if (rainAlerts.enabled) void rainAlerts.connect();
  if (state.wind) setOverlay("wind", true);
  void renderForecast();
  startPeriodic(() => void refreshManifest(), MANIFEST_POLL_MS);
  // Keep "Now" and the radar age honest between manifest polls.
  startPeriodic(() => {
    if (replaying()) return;
    renderTimeline();
    renderRadarStatus();
  }, 30_000);
  startPeriodic(() => void renderBriefing(), 10 * 60_000);
  startPeriodic(() => void renderNextHour(), 60_000);
  startPeriodic(() => void renderForecast(true), FORECAST_REFRESH_MS);
  startPeriodic(() => void renderAlerts(), ALERTS_REFRESH_MS);
}

document.addEventListener("click", (event) => {
  if (!(event.target as HTMLElement).closest("#test-nws-alert")) return;
  void nwsNotifications.test().then(ok => toast(ok ? "Test alert sent" : "Notifications are blocked or unavailable. Check browser permissions."));
});

$("boot-retry").addEventListener("click", () => void boot());
$("radar-notice").addEventListener("click", (e) => {
  const b = (e.target as HTMLElement).closest<HTMLButtonElement>("#radar-retry");
  if (!b) return;
  b.disabled = true;
  noticeKey = "";
  void refreshManifest();
});
document.documentElement.dataset.map = state.mapStyle;
renderChips();
renderRadarStatus();
const hasStoredPlace = (() => {
  try {
    return localStorage.getItem("rng.place") !== null;
  } catch {
    return false;
  }
})();
void queryLocationPermission().then((permission) => {
  const step = bootLocationStep(permission, hasStoredPlace);
  if (step === "locate") locate();
  else if (step === "card") renderLocationCard("undetermined");
});
void boot();
