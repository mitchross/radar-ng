import { create } from "zustand";
import type { RadarFrame, TemperatureUnit, MapStyle } from "../types/weather";
import { DEFAULTS, RADAR } from "../lib/constants";

interface WeatherState {
  // Timeline
  frames: RadarFrame[];
  currentFrameIndex: number;
  isPlaying: boolean;
  playbackSpeed: number;

  // Location
  latitude: number | null;
  longitude: number | null;

  // Layers
  radarOpacity: number;
  radarVisible: boolean;

  // Settings
  temperatureUnit: TemperatureUnit;
  mapStyle: MapStyle;

  // Actions
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

  temperatureUnit: "fahrenheit",
  mapStyle: "light",

  setFrames: (frames) => set({ frames }),
  setCurrentFrameIndex: (index) => set({ currentFrameIndex: index }),
  setIsPlaying: (playing) => set({ isPlaying: playing }),
  togglePlaying: () => set((s) => ({ isPlaying: !s.isPlaying })),
  setPlaybackSpeed: (speed) => set({ playbackSpeed: speed }),
  setLocation: (lat, lon) => set({ latitude: lat, longitude: lon }),
  setRadarOpacity: (opacity) => set({ radarOpacity: opacity }),
  setRadarVisible: (visible) => set({ radarVisible: visible }),
  setTemperatureUnit: (unit) => set({ temperatureUnit: unit }),
  setMapStyle: (style) => set({ mapStyle: style }),
  nextFrame: () => {
    const { frames, currentFrameIndex } = get();
    if (frames.length === 0) return;
    set({ currentFrameIndex: (currentFrameIndex + 1) % frames.length });
  },
}));
