import { create } from "zustand";
import type { RadarFrame, TemperatureUnit, MapStyle, LayerType, Palette } from "../types/weather";
import type { LocationMode, SelectedPlace } from "../types/location";
import { DEFAULTS, RADAR, SELF_HOSTED } from "../lib/constants";
import { getString, setString } from "../lib/storage";
import {
  parseMapStyle,
  parsePalette,
  parseServerUrl,
  parseViewMode,
  parseOpacity,
  parsePlaybackFps,
  type ViewMode,
} from "../lib/persistedPrefs";
import type { AppearanceMode } from "../theme/weatherClearTheme";
import type { PlaybackWindow } from "../lib/radarCarousel";
import { refreshKeepTime, resnapFrameIndex, sameFrames } from "../lib/frameIndex";
import { roundCoord } from "../lib/coordinates";
import { DEFAULT_RAIN_ALERT_LEAD, parseRainAlertLead, type RainAlertLeadMinutes } from "../lib/rainAlerts";
import { parseDeviceFix, statusForFix, type LocationStatus } from "../lib/locationStatus";

import { parseAlertPreference, type AlertPreference } from "../lib/alertLevels";
import { LIVE_SOURCE, isReplaySource, sourceKey, type SourceContext } from "../lib/replaySource";
import type { ReplayCase } from "../lib/replayApi";
import { replayGuard } from "../lib/replayClient";
import { layerForCase } from "../lib/replayState";

interface WeatherState {
  alertPreference: AlertPreference;
  setAlertPreference: (value: AlertPreference) => void;
  /** Where frames, tiles and warnings come from; replay never mixes with live. */
  source: SourceContext;
  /** The catalogue case behind a replay source (title, zone, bounds, layers); null when live. */
  replayCase: ReplayCase | null;
  /** Epoch ms of the last return to live; live frames wait for a manifest fetched after it. */
  liveReturnedAt: number | null;
  frames: RadarFrame[];
  currentFrameIndex: number;
  isPlaying: boolean;
  playbackSpeed: number;
  // Inclusive index range the timeline loops over (1h/48h zoom); null = all frames.
  // The raster carousel prefetches inside it, including the loop wrap.
  playbackWindow: PlaybackWindow | null;
  latitude: number | null;
  longitude: number | null;
  locationMode: LocationMode;
  selectedPlace: SelectedPlace | null;
  devicePlace: SelectedPlace | null;
  /** Where device-mode coordinates come from; drives honest location labels. */
  locationStatus: LocationStatus;
  /** Epoch ms of the device fix currently shown, or null. */
  lastFixAt: number | null;
  /** Bumped when the map should recenter on the active location (user intent, not GPS drift). */
  recenterNonce: number;
  /** One-shot request for the radar map to frame these bounds [west, south, east, north]. */
  focusBounds: [number, number, number, number] | null;
  radarOpacity: number;
  /** Draw radar frames that have packed tiles on the GPU (InMapPackedRadar); off = PNG rasters. */
  fastRadar: boolean;
  activeLayer: LayerType;
  temperatureUnit: TemperatureUnit;
  mapStyle: MapStyle;
  activePalette: Palette;
  // Power-user overlays, off by default. Lightning + storm-cell dots
  // overwhelm the radar view for casual users; opt-in via this flag.
  extrasVisible: boolean;
  spcWatchesVisible: boolean;
  spcDiscussionsVisible: boolean;
  /** GOES satellite imagery under the radar (manifest `satellite` layer). */
  satelliteVisible: boolean;
  /** Optional local-LLM briefing card; the card hides itself whenever the model is unavailable. */
  aiNarration: boolean;
  /** "Rain starts in N min" local notifications planned on this phone from the radar nowcast. */
  rainAlertsEnabled: boolean;
  stormNoticeEnabled: boolean;
  /** How many minutes ahead of the first wet minute the notification shows. */
  rainAlertLeadMinutes: RainAlertLeadMinutes;
  serverUrl: string;
  viewMode: ViewMode;
  appearanceMode: AppearanceMode;

  /** `forSource` drops a late publish from a source that is no longer active. */
  setFrameTimeline: (frames: RadarFrame[], fallbackIndex: number, forSource?: SourceContext) => void;
  enterReplay: (replayCase: ReplayCase) => void;
  returnToLive: () => void;
  setCurrentFrameIndex: (index: number) => void;
  setIsPlaying: (playing: boolean) => void;
  togglePlaying: () => void;
  setPlaybackSpeed: (speed: number) => void;
  setPlaybackWindow: (window: PlaybackWindow | null) => void;
  setLocation: (lat: number, lon: number) => void;
  setSelectedPlace: (place: SelectedPlace) => void;
  setDevicePlace: (place: SelectedPlace | null) => void;
  useDeviceLocation: () => void;
  applyDeviceFix: (lat: number, lon: number, at: number) => void;
  applyLocationFailure: (reason: "denied" | "undetermined" | "unavailable") => void;
  requestRecenter: () => void;
  setFocusBounds: (bounds: [number, number, number, number] | null) => void;
  setRadarOpacity: (opacity: number) => void;
  setFastRadar: (on: boolean) => void;
  setTemperatureUnit: (unit: TemperatureUnit) => void;
  setMapStyle: (style: MapStyle) => void;
  setActivePalette: (palette: Palette) => void;
  toggleExtras: () => void;
  setSpcVisible: (kind: "watches" | "discussions", on: boolean) => void;
  setSatelliteVisible: (on: boolean) => void;
  setAiNarration: (on: boolean) => void;
  setRainAlertsEnabled: (on: boolean) => void;
  setStormNoticeEnabled: (on: boolean) => void;
  setRainAlertLeadMinutes: (minutes: RainAlertLeadMinutes) => void;
  setActiveLayer: (layer: LayerType) => void;
  setServerUrl: (url: string) => void;
  setViewMode: (mode: ViewMode) => void;
  setAppearanceMode: (mode: AppearanceMode) => void;
}

const LAST_DEVICE_FIX_KEY = "lastDeviceFix";

function parseLocationMode(value: string): LocationMode {
  return value === "city" ? "city" : "device";
}

function parseSelectedPlace(value: string): SelectedPlace | null {
  if (!value) return null;
  try {
    const parsed = JSON.parse(value) as SelectedPlace;
    if (
      typeof parsed.id === "number" &&
      typeof parsed.name === "string" &&
      typeof parsed.latitude === "number" &&
      typeof parsed.longitude === "number"
    ) {
      return parsed;
    }
  } catch {}
  return null;
}

function parseAppearanceMode(value: string): AppearanceMode {
  return value === "light" || value === "dark" ? value : "system";
}

export const DEFAULT_PLACE: SelectedPlace = {
  id: 4994358,
  name: "Grand Rapids",
  latitude: DEFAULTS.LATITUDE,
  longitude: DEFAULTS.LONGITUDE,
  admin1: "Michigan",
  country: "United States",
};

const initialLocationMode = parseLocationMode(getString("locationMode", "device"));
const initialSelectedPlace = parseSelectedPlace(getString("selectedPlace", "")) ?? DEFAULT_PLACE;
const initialResolvedLocationMode: LocationMode =
  initialLocationMode === "city" && initialSelectedPlace ? "city" : "device";
// Device mode starts from the last fix this device saw, never from the fallback city.
const persistedFix = parseDeviceFix(getString(LAST_DEVICE_FIX_KEY, ""));
const initialCoords =
  initialResolvedLocationMode === "city"
    ? { latitude: initialSelectedPlace.latitude, longitude: initialSelectedPlace.longitude }
    : { latitude: persistedFix?.latitude ?? null, longitude: persistedFix?.longitude ?? null };

/** Same ~110 m cell: a new fix inside it is GPS jitter, not a move. */
const sameCell = (a: number | null, b: number) => a !== null && roundCoord(a, 3) === roundCoord(b, 3);

export const useWeatherStore = create<WeatherState>()((set, get) => ({
  alertPreference: parseAlertPreference(getString("alertPreference", "")),
  setAlertPreference: (value) => {
    setString("alertPreference", value);
    set({ alertPreference: value });
  },
  source: LIVE_SOURCE,
  replayCase: null,
  liveReturnedAt: null,
  frames: [],
  currentFrameIndex: -1,
  isPlaying: false,
  playbackSpeed: parsePlaybackFps(getString("playbackSpeed", ""), DEFAULTS.PLAYBACK_FPS),
  playbackWindow: null,
  latitude: initialCoords.latitude,
  longitude: initialCoords.longitude,
  locationMode: initialResolvedLocationMode,
  selectedPlace: initialSelectedPlace,
  devicePlace: null,
  locationStatus: persistedFix ? "last-known" : "locating",
  lastFixAt: persistedFix?.at ?? null,
  recenterNonce: 0,
  focusBounds: null,
  radarOpacity: parseOpacity(getString("radarOpacity", String(RADAR.DEFAULT_OPACITY)), RADAR.DEFAULT_OPACITY),
  fastRadar: getString("fastRadar", "1") === "1",
  activeLayer: "radar" as LayerType,
  temperatureUnit: getString("temperatureUnit", "fahrenheit") === "celsius" ? "celsius" : "fahrenheit",
  // Persisted strings are parsed, not cast: a stale/garbage value would otherwise
  // index MAP_STYLES_SELFHOSTED[undefined] and throw at WeatherMap mount.
  mapStyle: parseMapStyle(getString("mapStyle", "light")),
  activePalette: parsePalette(getString("activePalette", "classic")),
  // Default to "forecast" so the timeline shows past + nowcast + HRRR future
  // as one merged stream out of the box. Less UI to flip, less to explain.
  spcWatchesVisible: getString("spcWatchesVisible", "0") === "1",
  spcDiscussionsVisible: getString("spcDiscussionsVisible", "0") === "1",
  extrasVisible: getString("extrasVisible", "0") === "1",
  satelliteVisible: getString("satelliteVisible", "0") === "1",
  aiNarration: getString("aiNarration", "1") === "1",
  rainAlertsEnabled: getString("rainAlertsEnabled", "0") === "1",
  stormNoticeEnabled: getString("stormNoticeEnabled", "0") === "1",
  rainAlertLeadMinutes: parseRainAlertLead(getString("rainAlertLeadMinutes", ""), DEFAULT_RAIN_ALERT_LEAD),
  serverUrl: parseServerUrl(getString("serverUrl", SELF_HOSTED.DEFAULT_URL), SELF_HOSTED.DEFAULT_URL),
  viewMode: parseViewMode(getString("viewMode", "simple")),
  appearanceMode: parseAppearanceMode(getString("appearanceMode", "system")),

  setFrameTimeline: (frames, fallbackIndex, forSource) =>
    set((state) => {
      if (forSource && sourceKey(forSource) !== sourceKey(state.source)) return state;
      // An unchanged poll keeps the same array so subscribers see no new timeline.
      if (sameFrames(state.frames, frames)) return state;
      if (frames.length === 0) return { frames, currentFrameIndex: -1 };
      if (isReplaySource(state.source)) {
        // Replay keeps the UTC playhead across layers; base times are shared, so it resnaps exactly.
        const at = state.frames[state.currentFrameIndex]?.time ?? null;
        const idx = at === null ? -1 : resnapFrameIndex(frames, state.currentFrameIndex, at);
        return { frames, currentFrameIndex: idx === -1 ? 0 : idx };
      }

      const previousTime = refreshKeepTime(
        state.frames,
        state.currentFrameIndex,
        state.isPlaying,
        Math.floor(Date.now() / 1000),
      );
      const preservedIndex = previousTime === null
        ? -1
        : resnapFrameIndex(frames, state.currentFrameIndex, previousTime);
      const safeFallback = fallbackIndex >= 0 && fallbackIndex < frames.length ? fallbackIndex : 0;
      return {
        frames,
        currentFrameIndex: preservedIndex === -1 ? safeFallback : preservedIndex,
      };
    }),
  enterReplay: (replayCase) => {
    replayGuard.cancel();
    set((s) => ({
      source: { kind: "replay", caseId: replayCase.id, revision: replayCase.revision },
      replayCase,
      liveReturnedAt: null,
      frames: [],
      currentFrameIndex: -1,
      isPlaying: false,
      playbackWindow: null,
      activeLayer: layerForCase(s.activeLayer, replayCase.layers),
      focusBounds: replayCase.bounds,
    }));
  },
  returnToLive: () => {
    replayGuard.cancel();
    set((s) => ({
      source: LIVE_SOURCE,
      replayCase: null,
      liveReturnedAt: Date.now(),
      frames: [],
      currentFrameIndex: -1,
      isPlaying: false,
      playbackWindow: null,
      activeLayer: s.activeLayer === "radar-composite" ? "radar" : s.activeLayer,
    }));
  },
  setCurrentFrameIndex: (index) => set({ currentFrameIndex: index }),
  setIsPlaying: (playing) => set({ isPlaying: playing }),
  togglePlaying: () => set((s) => ({ isPlaying: !s.isPlaying })),
  setPlaybackSpeed: (speed) => {
    const fps = parsePlaybackFps(String(speed), DEFAULTS.PLAYBACK_FPS);
    setString("playbackSpeed", String(fps));
    set({ playbackSpeed: fps });
  },
  setPlaybackWindow: (window) =>
    set((s) =>
      s.playbackWindow?.start === window?.start &&
      s.playbackWindow?.end === window?.end &&
      sameSequence(s.playbackWindow?.sequence, window?.sequence)
        ? s
        : { playbackWindow: window },
    ),
  setLocation: (lat, lon) => set({ latitude: lat, longitude: lon }),
  setSelectedPlace: (place) => {
    setString("locationMode", "city");
    setString("selectedPlace", JSON.stringify(place));
    set((s) => ({
      locationMode: "city",
      selectedPlace: place,
      latitude: place.latitude,
      longitude: place.longitude,
      recenterNonce: s.recenterNonce + 1,
    }));
  },
  setDevicePlace: (place) => set({ devicePlace: place }),
  useDeviceLocation: () => {
    setString("locationMode", "device");
    const fix = parseDeviceFix(getString(LAST_DEVICE_FIX_KEY, ""));
    // Never keep showing the chosen city's coordinates under a "My Location" label.
    set((s) => ({
      locationMode: "device",
      latitude: fix?.latitude ?? null,
      longitude: fix?.longitude ?? null,
      locationStatus: fix ? "last-known" : "locating",
      lastFixAt: fix?.at ?? null,
      recenterNonce: s.recenterNonce + 1,
    }));
  },
  applyDeviceFix: (lat, lon, at) => {
    const state = get();
    if (state.locationMode !== "device") return;
    setString(LAST_DEVICE_FIX_KEY, JSON.stringify({ latitude: lat, longitude: lon, at }));
    const firstFix = state.latitude === null;
    const moved = !(sameCell(state.latitude, lat) && sameCell(state.longitude, lon));
    set({
      ...(moved ? { latitude: lat, longitude: lon } : {}),
      locationStatus: statusForFix(at),
      lastFixAt: at,
      ...(firstFix ? { recenterNonce: state.recenterNonce + 1 } : {}),
    });
  },
  applyLocationFailure: (reason) => {
    const state = get();
    if (state.locationMode !== "device") return;
    // A timeout keeps any fix we already have; a denial must stop using the device's position.
    if (reason === "unavailable" && state.latitude !== null && state.locationStatus !== "denied") {
      set({ locationStatus: "last-known" });
      return;
    }
    if (reason !== "unavailable") setString(LAST_DEVICE_FIX_KEY, "");
    set({
      latitude: DEFAULT_PLACE.latitude,
      longitude: DEFAULT_PLACE.longitude,
      locationStatus: reason,
      lastFixAt: null,
      devicePlace: null,
      recenterNonce: state.recenterNonce + 1,
    });
  },
  requestRecenter: () => set((s) => ({ recenterNonce: s.recenterNonce + 1 })),
  setFocusBounds: (bounds) => set({ focusBounds: bounds }),
  setRadarOpacity: (opacity) => {
    setString("radarOpacity", String(opacity));
    set({ radarOpacity: opacity });
  },
  setFastRadar: (on) => {
    setString("fastRadar", on ? "1" : "0");
    set({ fastRadar: on });
  },
  setTemperatureUnit: (unit) => {
    setString("temperatureUnit", unit);
    set({ temperatureUnit: unit });
  },
  setMapStyle: (style) => {
    setString("mapStyle", style);
    set({ mapStyle: style });
  },
  setActivePalette: (palette) => {
    setString("activePalette", palette);
    set({ activePalette: palette });
  },
  toggleExtras: () => set((s) => {
    const next = !s.extrasVisible;
    setString("extrasVisible", next ? "1" : "0");
    return { extrasVisible: next };
  }),
  setSpcVisible: (kind, on) => {
    const key = kind === "watches" ? "spcWatchesVisible" : "spcDiscussionsVisible";
    setString(key, on ? "1" : "0");
    set({ [key]: on });
  },
  setSatelliteVisible: (on) => {
    setString("satelliteVisible", on ? "1" : "0");
    set({ satelliteVisible: on });
  },
  setAiNarration: (on) => {
    setString("aiNarration", on ? "1" : "0");
    set({ aiNarration: on });
  },
  setRainAlertsEnabled: (on) => {
    setString("rainAlertsEnabled", on ? "1" : "0");
    set({ rainAlertsEnabled: on });
  },
  setStormNoticeEnabled: (on) => {
    setString("stormNoticeEnabled", on ? "1" : "0");
    set({ stormNoticeEnabled: on });
  },
  setRainAlertLeadMinutes: (minutes) => {
    setString("rainAlertLeadMinutes", String(minutes));
    set({ rainAlertLeadMinutes: minutes });
  },
  setActiveLayer: (layer) =>
    set((s) => (isReplaySource(s.source) && !s.replayCase?.layers.includes(layer) ? s : { activeLayer: layer })),
  setServerUrl: (url) => {
    const next = parseServerUrl(url, get().serverUrl);
    setString("serverUrl", next);
    set({ serverUrl: next });
  },
  setViewMode: (mode) => {
    setString("viewMode", mode);
    set({ viewMode: mode });
  },
  setAppearanceMode: (mode) => {
    setString("appearanceMode", mode);
    set({ appearanceMode: mode });
  },
}));

function sameSequence(a?: readonly number[], b?: readonly number[]): boolean {
  if (a === b) return true;
  if (!a || !b || a.length !== b.length) return false;
  return a.every((v, i) => v === b[i]);
}
