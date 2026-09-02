import { create } from "zustand";
import type { RadarFrame, TemperatureUnit, MapStyle, LayerType, MapProjection, Palette, TimelineMode } from "../types/weather";
import type { LocationMode, SelectedPlace } from "../types/location";
import { DEFAULTS, RADAR, SELF_HOSTED } from "../lib/constants";
import { getString, setString } from "../lib/storage";
import {
  parseMapProjection,
  parseMapStyle,
  parsePalette,
  parseServerUrl,
  parseTimelineMode,
  parseViewMode,
  type ViewMode,
} from "../lib/persistedPrefs";
import type { AppearanceMode } from "../theme/weatherClearTheme";

interface WeatherState {
  frames: RadarFrame[];
  currentFrameIndex: number;
  isPlaying: boolean;
  playbackSpeed: number;
  latitude: number | null;
  longitude: number | null;
  locationMode: LocationMode;
  selectedPlace: SelectedPlace | null;
  devicePlace: SelectedPlace | null;
  radarOpacity: number;
  radarVisible: boolean;
  activeLayer: LayerType;
  temperatureUnit: TemperatureUnit;
  mapStyle: MapStyle;
  mapProjection: MapProjection;
  activePalette: Palette;
  timelineMode: TimelineMode;
  // Power-user overlays — off by default. Lightning + storm-cell dots
  // overwhelm the radar view for casual users; opt-in via this flag.
  extrasVisible: boolean;
  serverUrl: string;
  viewMode: ViewMode;
  appearanceMode: AppearanceMode;

  setFrames: (frames: RadarFrame[]) => void;
  setCurrentFrameIndex: (index: number) => void;
  setIsPlaying: (playing: boolean) => void;
  togglePlaying: () => void;
  setPlaybackSpeed: (speed: number) => void;
  setLocation: (lat: number, lon: number) => void;
  setSelectedPlace: (place: SelectedPlace) => void;
  setDevicePlace: (place: SelectedPlace | null) => void;
  useDeviceLocation: () => void;
  setRadarOpacity: (opacity: number) => void;
  setTemperatureUnit: (unit: TemperatureUnit) => void;
  setMapStyle: (style: MapStyle) => void;
  setMapProjection: (projection: MapProjection) => void;
  setActivePalette: (palette: Palette) => void;
  setTimelineMode: (mode: TimelineMode) => void;
  toggleExtras: () => void;
  setActiveLayer: (layer: LayerType) => void;
  setServerUrl: (url: string) => void;
  setViewMode: (mode: ViewMode) => void;
  setAppearanceMode: (mode: AppearanceMode) => void;
}

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

const DEFAULT_PLACE: SelectedPlace = {
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

export const useWeatherStore = create<WeatherState>()((set, get) => ({
  frames: [],
  currentFrameIndex: -1,
  isPlaying: false,
  playbackSpeed: DEFAULTS.PLAYBACK_FPS,
  latitude: initialResolvedLocationMode === "city" && initialSelectedPlace ? initialSelectedPlace.latitude : DEFAULTS.LATITUDE,
  longitude: initialResolvedLocationMode === "city" && initialSelectedPlace ? initialSelectedPlace.longitude : DEFAULTS.LONGITUDE,
  locationMode: initialResolvedLocationMode,
  selectedPlace: initialSelectedPlace,
  devicePlace: null,
  radarOpacity: RADAR.DEFAULT_OPACITY,
  radarVisible: true,
  activeLayer: "radar" as LayerType,
  temperatureUnit: "fahrenheit",
  // Persisted strings are parsed, not cast: a stale/garbage value would otherwise
  // index MAP_STYLES_SELFHOSTED[undefined] and throw at WeatherMap mount.
  mapStyle: parseMapStyle(getString("mapStyle", "light")),
  mapProjection: parseMapProjection(getString("mapProjection", "flat")),
  activePalette: parsePalette(getString("activePalette", "classic")),
  // Default to "forecast" so the timeline shows past + nowcast + HRRR future
  // as one merged stream out of the box. Less UI to flip, less to explain.
  timelineMode: parseTimelineMode(getString("timelineMode", "forecast")),
  extrasVisible: getString("extrasVisible", "0") === "1",
  serverUrl: parseServerUrl(getString("serverUrl", SELF_HOSTED.DEFAULT_URL), SELF_HOSTED.DEFAULT_URL),
  viewMode: parseViewMode(getString("viewMode", "simple")),
  appearanceMode: parseAppearanceMode(getString("appearanceMode", "system")),

  setFrames: (frames) => set({ frames }),
  setCurrentFrameIndex: (index) => set({ currentFrameIndex: index }),
  setIsPlaying: (playing) => set({ isPlaying: playing }),
  togglePlaying: () => set((s) => ({ isPlaying: !s.isPlaying })),
  setPlaybackSpeed: (speed) => set({ playbackSpeed: speed }),
  setLocation: (lat, lon) => set({ latitude: lat, longitude: lon }),
  setSelectedPlace: (place) => {
    setString("locationMode", "city");
    setString("selectedPlace", JSON.stringify(place));
    set({
      locationMode: "city",
      selectedPlace: place,
      latitude: place.latitude,
      longitude: place.longitude,
    });
  },
  setDevicePlace: (place) => set({ devicePlace: place }),
  useDeviceLocation: () => {
    setString("locationMode", "device");
    set({ locationMode: "device" });
  },
  setRadarOpacity: (opacity) => set({ radarOpacity: opacity }),
  setTemperatureUnit: (unit) => set({ temperatureUnit: unit }),
  setMapStyle: (style) => {
    setString("mapStyle", style);
    set({ mapStyle: style });
  },
  setMapProjection: (projection) => {
    setString("mapProjection", projection);
    set({ mapProjection: projection });
  },
  setActivePalette: (palette) => {
    setString("activePalette", palette);
    set({ activePalette: palette });
  },
  setTimelineMode: (mode) => {
    setString("timelineMode", mode);
    set({ timelineMode: mode });
  },
  toggleExtras: () => set((s) => {
    const next = !s.extrasVisible;
    setString("extrasVisible", next ? "1" : "0");
    return { extrasVisible: next };
  }),
  setActiveLayer: (layer) => set({ activeLayer: layer }),
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
