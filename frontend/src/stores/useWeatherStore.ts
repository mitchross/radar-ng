import { create } from "zustand";
import type { RadarFrame, TemperatureUnit, MapStyle, LayerType, MapProjection, Palette, TimelineMode } from "../types/weather";
import type { LocationMode, SelectedPlace } from "../types/location";
import { DEFAULTS, RADAR, SELF_HOSTED } from "../lib/constants";
import { getString, setString } from "../lib/storage";

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
  visibleOverlays: Set<LayerType>;
  temperatureUnit: TemperatureUnit;
  mapStyle: MapStyle;
  mapProjection: MapProjection;
  activePalette: Palette;
  timelineMode: TimelineMode;
  // Power-user overlays — off by default. Lightning + storm-cell dots
  // overwhelm the radar view for casual users; opt-in via this flag.
  extrasVisible: boolean;
  serverUrl: string;
  viewMode: "simple" | "advanced";

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
  setRadarVisible: (visible: boolean) => void;
  setTemperatureUnit: (unit: TemperatureUnit) => void;
  setMapStyle: (style: MapStyle) => void;
  setMapProjection: (projection: MapProjection) => void;
  setActivePalette: (palette: Palette) => void;
  setTimelineMode: (mode: TimelineMode) => void;
  toggleExtras: () => void;
  setActiveLayer: (layer: LayerType) => void;
  toggleOverlay: (layer: LayerType) => void;
  setServerUrl: (url: string) => void;
  setViewMode: (mode: "simple" | "advanced") => void;
  nextFrame: () => void;
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
  visibleOverlays: new Set<LayerType>(),
  temperatureUnit: "fahrenheit",
  mapStyle: (getString("mapStyle", "light") as MapStyle),
  mapProjection: (getString("mapProjection", "flat") as MapProjection),
  activePalette: (getString("activePalette", "classic") as Palette),
  // Default to "forecast" so the timeline shows past + nowcast + HRRR future
  // as one merged stream out of the box. Less UI to flip, less to explain.
  timelineMode: (getString("timelineMode", "forecast") as TimelineMode),
  extrasVisible: getString("extrasVisible", "0") === "1",
  serverUrl: getString("serverUrl", SELF_HOSTED.DEFAULT_URL),
  viewMode: (getString("viewMode", "simple") as "simple" | "advanced"),

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
  setRadarVisible: (visible) => set({ radarVisible: visible }),
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
  toggleOverlay: (layer) => set((s) => {
    const next = new Set(s.visibleOverlays);
    if (next.has(layer)) next.delete(layer);
    else next.add(layer);
    return { visibleOverlays: next };
  }),
  setServerUrl: (url) => {
    setString("serverUrl", url);
    set({ serverUrl: url });
  },
  setViewMode: (mode) => {
    setString("viewMode", mode);
    set({ viewMode: mode });
  },
  nextFrame: () => {
    const { frames, currentFrameIndex } = get();
    if (frames.length === 0) return;
    set({ currentFrameIndex: (currentFrameIndex + 1) % frames.length });
  },
}));
