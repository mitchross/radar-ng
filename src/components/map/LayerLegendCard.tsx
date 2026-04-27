/**
 * Apple-Weather-style vertical legend — layer-aware title + labeled color
 * scale. Sits top-left over the map. Replaces the thin horizontal dBZ strip.
 */
import { View, Text, StyleSheet } from "react-native";
import { LinearGradient } from "expo-linear-gradient";
import type { LayerType } from "../../types/weather";
import { cumulus } from "../../lib/cumulusTheme";
import { useWeatherStore } from "../../stores/useWeatherStore";

// Source-of-truth tag shown at the top of the legend so the user knows
// whether they're staring at observed radar (live MRMS), nowcast (pysteps
// extrapolation), or a model forecast (HRRR). Clearer than a separate badge
// because the legend is what people look at to understand what they see.
const LAYER_SOURCE: Record<LayerType, "OBSERVED" | "NOWCAST" | "FORECAST"> = {
  radar: "OBSERVED",
  "radar-composite": "OBSERVED",
  "radar-hrrr": "FORECAST",
  temperature: "FORECAST",
  wind: "FORECAST",
  "precip-type": "FORECAST",
  cape: "FORECAST",
  "precip-accum": "FORECAST",
  cloud: "FORECAST",
};

const SOURCE_COLOR: Record<"OBSERVED" | "NOWCAST" | "FORECAST", string> = {
  OBSERVED: "#52c47a",
  NOWCAST: "#7ec3ff",
  FORECAST: "#d4a52e",
};

type LegendStop = { label: string; color: string };

interface LegendSpec {
  title: string;
  stops: LegendStop[]; // top → bottom order
}

// Radar tiles ARE reflectivity (dBZ from MRMS), but the user-facing legend
// reads as "Precipitation" with category labels mapped onto our DBZ palette
// — nobody but a meteorologist reads "45 dBZ" intuitively. Stops match the
// dBZ→intensity buckets in `describeDBZ` (lib/inspector.ts) so the legend
// and the eyedropper readout agree.
const PRECIP_STOPS: LegendStop[] = [
  { label: "Hail",     color: "#b24bff" }, // ≥ 55 dBZ
  { label: "Severe",   color: "#d02058" }, // 45–55
  { label: "Heavy",    color: "#ff4040" }, // 35–45
  { label: "Moderate", color: "#ff9f2e" }, // 25–35
  { label: "Light",    color: "#3bc77a" }, // < 25
];

const LEGENDS: Record<LayerType, LegendSpec> = {
  radar: {
    title: "Precipitation",
    stops: PRECIP_STOPS,
  },
  "radar-composite": {
    title: "Composite Reflectivity",
    stops: PRECIP_STOPS,
  },
  "radar-hrrr": {
    title: "Precipitation (Forecast)",
    stops: PRECIP_STOPS,
  },
  temperature: {
    title: "Temperature",
    stops: [
      { label: "130", color: "#7c1f11" },
      { label: "90", color: "#ff6b28" },
      { label: "60", color: "#ffd042" },
      { label: "30", color: "#7ed957" },
      { label: "0", color: "#4fb8ff" },
      { label: "-40", color: "#3a2c8a" },
    ],
  },
  wind: {
    title: "Wind (mph)",
    stops: [
      { label: "75", color: "#4a7ad4" },
      { label: "50", color: "#7fa7e8" },
      { label: "25", color: "#b3cef0" },
      { label: "0", color: "#e5eef8" },
    ],
  },
  "precip-type": {
    title: "Precip Type",
    stops: [
      { label: "Extreme", color: "#ffe57a" },
      { label: "Heavy", color: "#ffb04a" },
      { label: "Moderate", color: "#b98cff" },
      { label: "Light", color: "#4fb8ff" },
    ],
  },
  cape: {
    title: "CAPE (J/kg)",
    stops: [
      { label: "4000", color: "#b24bff" },
      { label: "2500", color: "#ff4d6d" },
      { label: "1500", color: "#ff9f2e" },
      { label: "500", color: "#3bc77a" },
      { label: "0", color: "#2c3854" },
    ],
  },
  "precip-accum": {
    title: "Rainfall (in/hr)",
    stops: [
      { label: "8+", color: "#50146b" },
      { label: "4", color: "#b01f96" },
      { label: "2", color: "#e63c3c" },
      { label: "1", color: "#ff8c3c" },
      { label: "0.5", color: "#f0dc46" },
      { label: "0.1", color: "#4696ff" },
      { label: "0", color: "#8cc8ff" },
    ],
  },
  cloud: {
    title: "Cloud Cover",
    stops: [
      { label: "100%", color: "#505064" },
      { label: "75%", color: "#787887" },
      { label: "50%", color: "#a0a0aa" },
      { label: "25%", color: "#c8c8cd" },
      { label: "0%", color: "#e6e6e6" },
    ],
  },
};

export function LayerLegendCard({ activeLayer }: { activeLayer: LayerType }) {
  const legend = LEGENDS[activeLayer] ?? LEGENDS.radar;
  const timelineMode = useWeatherStore((s) => s.timelineMode);
  const currentFrameIndex = useWeatherStore((s) => s.currentFrameIndex);
  const frames = useWeatherStore((s) => s.frames);

  // For the radar layer, the source tag shifts based on which frame the
  // user is sitting on: past = OBSERVED, near-future = NOWCAST, far-future
  // = FORECAST (HRRR). Other layers are statically tagged.
  let source = LAYER_SOURCE[activeLayer] ?? "OBSERVED";
  if ((activeLayer === "radar" || activeLayer === "radar-hrrr") && timelineMode === "forecast") {
    const frame = frames[currentFrameIndex];
    if (frame?.source === "nowcast") source = "NOWCAST";
    else if (frame?.source === "radar-hrrr") source = "FORECAST";
    else source = "OBSERVED";
  }

  return (
    <View style={styles.wrap}>
      <View style={styles.card}>
        <View style={styles.headerRow}>
          <Text style={styles.title} numberOfLines={1}>
            {legend.title}
          </Text>
          <View style={[styles.tag, { backgroundColor: SOURCE_COLOR[source] + "33", borderColor: SOURCE_COLOR[source] }]}>
            <Text style={[styles.tagText, { color: SOURCE_COLOR[source] }]}>{source}</Text>
          </View>
        </View>
        <View style={styles.scaleRow}>
          <LinearGradient
            colors={legend.stops.map((s) => s.color) as [string, string, ...string[]]}
            start={{ x: 0.5, y: 0 }}
            end={{ x: 0.5, y: 1 }}
            style={styles.gradient}
          />
          <View style={styles.labels}>
            {legend.stops.map((s, i) => (
              <Text key={i} style={styles.label}>
                {s.label}
              </Text>
            ))}
          </View>
        </View>
      </View>
      <Text style={styles.attribution}>Map Data</Text>
    </View>
  );
}

const CARD_BG = "rgba(255,255,255,0.9)";

const styles = StyleSheet.create({
  wrap: {
    position: "absolute",
    left: 12,
    top: 112,
    zIndex: 12,
  },
  card: {
    width: 120,
    backgroundColor: CARD_BG,
    borderRadius: 14,
    paddingVertical: 10,
    paddingHorizontal: 12,
    shadowColor: "#000",
    shadowOpacity: 0.15,
    shadowRadius: 8,
    shadowOffset: { width: 0, height: 2 },
    elevation: 3,
    borderWidth: 1,
    borderColor: "rgba(10,20,40,0.06)",
  },
  headerRow: {
    flexDirection: "row",
    alignItems: "center",
    justifyContent: "space-between",
    marginBottom: 8,
    gap: 6,
  },
  title: {
    color: "#1a2030",
    fontSize: 11,
    fontWeight: "600",
    flexShrink: 1,
  },
  tag: {
    borderRadius: 6,
    borderWidth: 1,
    paddingHorizontal: 5,
    paddingVertical: 1,
  },
  tagText: {
    fontSize: 8,
    fontWeight: "800",
    letterSpacing: 0.6,
  },
  scaleRow: {
    flexDirection: "row",
    height: 140,
    gap: 8,
  },
  gradient: {
    width: 10,
    borderRadius: 5,
  },
  labels: {
    flex: 1,
    justifyContent: "space-between",
    paddingVertical: 0,
  },
  label: {
    fontSize: 10,
    color: "#3a4258",
    fontVariant: ["tabular-nums"],
    fontWeight: "500",
  },
  attribution: {
    color: cumulus.inkMuted,
    fontSize: 9,
    marginTop: 4,
    marginLeft: 6,
    textDecorationLine: "underline",
  },
});
