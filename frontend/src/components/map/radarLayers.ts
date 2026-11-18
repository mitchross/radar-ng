/**
 * The weather layers the map offers, with their symbols. HRRR, composite,
 * CAPE and precip-type are still ingested (and feed the merged radar
 * timeline) but aren't offered: plain-English layers only.
 */
import type { SymbolViewProps } from "expo-symbols";
import type { LayerType } from "../../types/weather";
import { SEVERE_LAYERS, type SevereLayerId } from "../../../../web/src/shared/severe";
import { WINTER_LAYERS, type WinterLayerId } from "../../../../web/src/shared/winterLayers";
import { NICE_DAY_LAYER } from "../../../../web/src/shared/niceDayLayer";
import { STARGAZING_LAYER } from "../../../../web/src/shared/stargazingLayer";

export type LayerIcon = "rain" | "thermo" | "dust" | "wind" | "drop" | "cloud" | "sun" | "ozone" | "hail" | "rotation" | "tower" | "snow" | "ice" | "snowDepth" | "snowPast" | "snowChance" | "niceDay" | "stars";

export const LAYER_OPTIONS: { id: LayerType; name: string; icon: LayerIcon }[] = [
  { id: "radar", name: "Radar", icon: "rain" },
  { id: "temperature", name: "Temperature", icon: "thermo" },
  { id: "feels-like", name: "Feels Like", icon: "thermo" },
  { id: "dewpoint", name: "Dew Point", icon: "drop" },
  { id: "wind", name: "Wind", icon: "wind" },
  { id: "precip-accum", name: "Rain Total", icon: "drop" },
  { id: "precip-24h", name: "Past 24 Hr Precip", icon: "drop" },
  { id: "cloud", name: "Clouds", icon: "cloud" },
  { id: "nice-day", name: NICE_DAY_LAYER.name, icon: "niceDay" },
  { id: "stargazing", name: STARGAZING_LAYER.name, icon: "stars" },
  { id: "uv", name: "UV Index", icon: "sun" },
  { id: "air-quality", name: "Air Quality", icon: "dust" },
  { id: "ozone", name: "Ozone", icon: "ozone" },
];

/**
 * The "Severe" group of the layers sheet (NOAA MRMS). Names and hints come from the shared list the
 * web radar uses; a layer the server has no frames for is not offered.
 */
const SEVERE_ICONS: Record<SevereLayerId, LayerIcon> = {
  "hail-size": "hail",
  "rotation-track": "rotation",
  "echo-top": "tower",
};
export const SEVERE_OPTIONS: { id: LayerType; name: string; icon: LayerIcon; hint: string }[] = SEVERE_LAYERS.map((l) => ({
  id: l.id,
  name: l.name,
  icon: SEVERE_ICONS[l.id],
  hint: l.hint,
}));

/** The "Winter" group (NOAA snow and ice totals, snow depth, past 24 h snow), from the shared list the web radar uses. */
const WINTER_ICONS: Record<WinterLayerId, LayerIcon> = {
  "snowfall-forecast": "snow",
  "ice-forecast": "ice",
  "snow-depth": "snowDepth",
  "snowfall-24h": "snowPast",
  "snow-chance-1": "snowChance",
  "snow-chance-4": "snowChance",
  "snow-chance-8": "snowChance",
};
export const WINTER_OPTIONS: { id: LayerType; name: string; icon: LayerIcon; hint: string; chip?: string }[] = WINTER_LAYERS.map((l) => ({
  id: l.id,
  name: l.name,
  icon: WINTER_ICONS[l.id],
  hint: l.hint,
  chip: l.chip,
}));

export const LAYER_SYMBOLS: Record<LayerIcon, SymbolViewProps["name"]> = {
  rain: { ios: "cloud.rain.fill", android: "rainy" },
  thermo: { ios: "thermometer.medium", android: "thermostat" },
  dust: { ios: "aqi.medium", android: "masks" },
  wind: { ios: "wind", android: "air" },
  drop: { ios: "drop.fill", android: "water_drop" },
  cloud: { ios: "cloud.fill", android: "cloud" },
  sun: { ios: "sun.max.fill", android: "wb_sunny" },
  ozone: { ios: "sun.haze.fill", android: "hexagon" },
  hail: { ios: "cloud.hail.fill", android: "weather_hail" },
  rotation: { ios: "tornado", android: "tornado" },
  tower: { ios: "arrow.up.to.line", android: "vertical_align_top" },
  snow: { ios: "snowflake", android: "ac_unit" },
  ice: { ios: "thermometer.snowflake", android: "severe_cold" },
  snowDepth: { ios: "ruler.fill", android: "straighten" },
  snowPast: { ios: "clock.arrow.circlepath", android: "history" },
  niceDay: { ios: "sun.horizon.fill", android: "nature_people" },
  stars: { ios: "moon.stars.fill", android: "nights_stay" },
  snowChance: { ios: "percent", android: "percent" },
};

/** GOES satellite imagery: a switch drawn under any weather layer, not one of the layers above. */
export const SATELLITE_OVERLAY = {
  name: "Satellite clouds",
  detail: "GOES imagery under the radar, every 10 minutes",
  symbol: { ios: "globe.americas.fill", android: "satellite_alt" } as SymbolViewProps["name"],
};

export function layerOption(id: LayerType) {
  return LAYER_OPTIONS.find((o) => o.id === id) ?? SEVERE_OPTIONS.find((o) => o.id === id) ?? WINTER_OPTIONS.find((o) => o.id === id) ?? LAYER_OPTIONS[0];
}

const REPLAY_LAYER_OPTIONS: { id: LayerType; name: string; icon: LayerIcon }[] = [
  { id: "radar", name: "Radar", icon: "rain" },
  { id: "radar-composite", name: "Composite", icon: "rain" },
  { id: "hail-size", name: "Hail Size", icon: "hail" },
  { id: "rotation-track", name: "Rotation Tracks", icon: "rotation" },
  { id: "echo-top", name: "Echo Tops", icon: "tower" },
  { id: "precip-24h", name: "Past 24 Hr Precip", icon: "drop" },
];

/** The layers a replay case offers, in picker order; nothing the live manifest alone has. */
/** Main layers the server may leave out (off unless HRRR_ENABLED_LAYERS lists them). */
const MANIFEST_GATED: readonly LayerType[] = [NICE_DAY_LAYER.id, STARGAZING_LAYER.id];

/** The main layer list, minus gated layers the live manifest has no frames for. */
export function liveLayerOptions(manifestLayers?: Record<string, { frames?: unknown[]; timestamps?: unknown[] } | undefined>) {
  return LAYER_OPTIONS.filter(
    (o) => !MANIFEST_GATED.includes(o.id) || (manifestLayers?.[o.id]?.frames?.length ?? manifestLayers?.[o.id]?.timestamps?.length ?? 0) > 0,
  );
}

export function replayLayerOptions(caseLayers: readonly string[]) {
  return REPLAY_LAYER_OPTIONS.filter((o) => caseLayers.includes(o.id));
}
