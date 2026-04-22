import { create } from "zustand";
import type { RadarFrame, TemperatureUnit, MapStyle, LayerType, DataSource, MapProjection, Palette, TimelineMode } from "../types/weather";
import { DEFAULTS, RADAR, SELF_HOSTED } from "../lib/constants";
import { getString, setString } from "../lib/storage";

interface WeatherState {
  frames: RadarFrame[];
  currentFrameIndex: number;
  isPlaying: boolean;
  playbackSpeed: number;
  latitude: number | null;
  longitude: number | null;
  radarOpacity: number;
  radarVisible: boolean;
  activeLayer: LayerType;
  visibleOverlays: Set<LayerType>;
  temperatureUnit: TemperatureUnit;
  mapStyle: MapStyle;
  mapProjection: MapProjection;
  activePalette: Palette;
  timelineMode: TimelineMode;
  dataSource: DataSource;
  serverUrl: string;

  setFrames: (frames: RadarFrame[]) => void;
  setCurrentFrameIndex: (index: number) => void;
  setIsPlaying: (playing: boolean) => void;
  togglePlaying: () => void;
  setPlaybackSpeed: (speed: number) => void;
  setLocation: (lat: number, lon: number) => void;
  setRadarOpacity: (opacity: number) => void;
  setRadarVisible: (visible: boolean) => void;
  setTemperatureUnit: (unit: TemperatureUnit) => void;
  setMapStyle: (style: MapStyle) => void;
  setMapProjection: (projection: MapProjection) => void;
  setActivePalette: (palette: Palette) => void;
  setTimelineMode: (mode: TimelineMode) => void;
  setActiveLayer: (layer: LayerType) => void;
  toggleOverlay: (layer: LayerType) => void;
  setDataSource: (source: DataSource) => void;
  setServerUrl: (url: string) => void;
  nextFrame: () => void;
}

export const useWeatherStore = create<WeatherState>()((set, get) => ({
  frames: [],
  currentFrameIndex: -1,
  isPlaying: false,
  playbackSpeed: DEFAULTS.PLAYBACK_FPS,
  latitude: null,
  longitude: null,
  radarOpacity: RADAR.DEFAULT_OPACITY,
  radarVisible: true,
  activeLayer: "radar" as LayerType,
  visibleOverlays: new Set<LayerType>(),
  temperatureUnit: "fahrenheit",
  mapStyle: (getString("mapStyle", "light") as MapStyle),
  mapProjection: (getString("mapProjection", "flat") as MapProjection),
  activePalette: (getString("activePalette", "classic") as Palette),
  timelineMode: (getString("timelineMode", "current") as TimelineMode),
  dataSource: (getString("dataSource", "selfhosted") as DataSource),
  serverUrl: getString("serverUrl", SELF_HOSTED.DEFAULT_URL),

  setFrames: (frames) => set({ frames }),
  setCurrentFrameIndex: (index) => set({ currentFrameIndex: index }),
  setIsPlaying: (playing) => set({ isPlaying: playing }),
  togglePlaying: () => set((s) => ({ isPlaying: !s.isPlaying })),
  setPlaybackSpeed: (speed) => set({ playbackSpeed: speed }),
  setLocation: (lat, lon) => set({ latitude: lat, longitude: lon }),
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
  setActiveLayer: (layer) => set({ activeLayer: layer }),
  toggleOverlay: (layer) => set((s) => {
    const next = new Set(s.visibleOverlays);
    if (next.has(layer)) next.delete(layer);
    else next.add(layer);
    return { visibleOverlays: next };
  }),
  setDataSource: (source) => {
    setString("dataSource", source);
    set({ dataSource: source });
  },
  setServerUrl: (url) => {
    setString("serverUrl", url);
    set({ serverUrl: url });
  },
  nextFrame: () => {
    const { frames, currentFrameIndex } = get();
    if (frames.length === 0) return;
    set({ currentFrameIndex: (currentFrameIndex + 1) % frames.length });
  },
}));
