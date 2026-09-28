/**
 * The weather layers the map offers, with their symbols. HRRR, composite,
 * CAPE and precip-type are still ingested (and feed the merged radar
 * timeline) but aren't offered: plain-English layers only.
 */
import type { SymbolViewProps } from "expo-symbols";
import type { LayerType } from "../../types/weather";

export type LayerIcon = "rain" | "thermo" | "dust" | "wind" | "drop" | "cloud" | "ozone";

export const LAYER_OPTIONS: { id: LayerType; name: string; icon: LayerIcon }[] = [
  { id: "radar", name: "Radar", icon: "rain" },
  { id: "temperature", name: "Temperature", icon: "thermo" },
  { id: "wind", name: "Wind", icon: "wind" },
  { id: "precip-accum", name: "Rain Total", icon: "drop" },
  { id: "cloud", name: "Clouds", icon: "cloud" },
  { id: "air-quality", name: "Air Quality", icon: "dust" },
  { id: "ozone", name: "Ozone", icon: "ozone" },
];

export const LAYER_SYMBOLS: Record<LayerIcon, SymbolViewProps["name"]> = {
  rain: { ios: "cloud.rain.fill", android: "rainy" },
  thermo: { ios: "thermometer.medium", android: "thermostat" },
  dust: { ios: "aqi.medium", android: "masks" },
  wind: { ios: "wind", android: "air" },
  drop: { ios: "drop.fill", android: "water_drop" },
  cloud: { ios: "cloud.fill", android: "cloud" },
  ozone: { ios: "sun.haze.fill", android: "hexagon" },
};

export function layerOption(id: LayerType) {
  return LAYER_OPTIONS.find((o) => o.id === id) ?? LAYER_OPTIONS[0];
}
