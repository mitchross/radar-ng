/**
 * Apple-Weather-style vertical legend: layer-aware title + labeled color
 * scale. Sits top-left over the map. Replaces the thin horizontal dBZ strip.
 */
import { View, Text, StyleSheet } from "react-native";
import { LinearGradient } from "expo-linear-gradient";
import type { LayerType } from "../../types/weather";
import { useWeatherStore } from "../../stores/useWeatherStore";
import { MAP_CHROME_MAX_FONT_SCALE } from "../../lib/constants";
import { useMapChromeInsets } from "../../hooks/useMapChromeInsets";
import { MapChromeSurface } from "../ui/MapChromeSurface";
import { useServerPalettes } from "../../hooks/useServerPalettes";
import { precipTypeRamps, serverLegend } from "../../lib/serverPalettes";
import { useManifestQuery } from "../../hooks/useManifest";
import { useReplayManifestQuery } from "../../hooks/useReplay";
import { SEVERE_CREDIT, isSevereLayer } from "../../../../web/src/shared/severe";
import { winterAsOf, winterLayer } from "../../../../web/src/shared/winterLayers";
import { NICE_DAY_LAYER, niceDayLegend } from "../../../../web/src/shared/niceDayLayer";
import { TEMP_NORMAL_LAYER, tempAnomalyLegend } from "../../../../web/src/shared/tempNormal";
import { STARGAZING_LAYER, stargazingLegend } from "../../../../web/src/shared/stargazingLayer";

// Plain-English tag at the top of the legend telling the user whether
// they're looking at "Now" (live radar), "Soon" (next hour, pysteps
// extrapolation), or "Later" (multi-hour HRRR forecast). Replaces the
// older OBSERVED/NOWCAST/FORECAST jargon nobody asked for.
type SourceTag = "Now" | "Soon" | "Later";

const LAYER_SOURCE: Record<LayerType, SourceTag> = {
  radar: "Now",
  "radar-composite": "Now",
  "radar-hrrr": "Later",
  temperature: "Later",
  "feels-like": "Later",
  dewpoint: "Later",
  wind: "Later",
  "precip-type": "Later",
  cape: "Later",
  "precip-accum": "Later",
  "precip-24h": "Now",
  "hail-size": "Now",
  "rotation-track": "Now",
  "echo-top": "Now",
  "snowfall-forecast": "Later",
  "ice-forecast": "Later",
  "snow-depth": "Now",
  "snowfall-24h": "Now",
  "nice-day": "Later",
  "temp-anomaly": "Later",
  stargazing: "Later",
  "snow-chance-1": "Later",
  "snow-chance-4": "Later",
  "snow-chance-8": "Later",
  cloud: "Later",
  uv: "Later",
  "air-quality": "Now",
  ozone: "Now",
};

const SOURCE_COLOR: Record<SourceTag, string> = {
  Now:   "#52c47a",
  Soon:  "#7ec3ff",
  Later: "#d4a52e",
};

type LegendStop = { label: string; color: string };

interface LegendSpec {
  title: string;
  stops: LegendStop[]; // top → bottom order
}

// Radar tiles ARE reflectivity (dBZ from MRMS), but the user-facing legend
// reads as "Precipitation" with category labels mapped onto our DBZ palette
// Nobody but a meteorologist reads "45 dBZ" intuitively. Stops match the
// dBZ→intensity buckets in `describeDBZ` (lib/inspector.ts) so the legend
// and the eyedropper readout agree.
const PRECIP_STOPS: LegendStop[] = [
  { label: "Hail",     color: "#b24bff" }, // ≥ 55 dBZ
  { label: "Severe",   color: "#d02058" }, // 45–55
  { label: "Heavy",    color: "#ff4040" }, // 35–45
  { label: "Moderate", color: "#ff9f2e" }, // 25–35
  { label: "Light",    color: "#3bc77a" }, // < 25
];

// Inches; the HRRR rain total and the MRMS 24-hour total share the `precip_accum` palette.
const RAIN_TOTAL_STOPS: LegendStop[] = [
  { label: "8+", color: "#50146b" },
  { label: "4", color: "#b01f96" },
  { label: "2", color: "#e63c3c" },
  { label: "1", color: "#ff8c3c" },
  { label: "0.5", color: "#f0dc46" },
  { label: "0.1", color: "#4696ff" },
  { label: "0", color: "#8cc8ff" },
];

const SNOW_CHANCE_LEGEND: LegendSpec = {
  title: "Snow Chance",
  stops: [
    { label: "90%", color: "#f0555a" },
    { label: "50%", color: "#5541af" },
    { label: "20%", color: "#69a5e1" },
    { label: "5%", color: "#bedcf5" },
  ],
};

const LEGENDS: Record<LayerType, LegendSpec> = {
  radar: {
    title: "Radar",
    stops: PRECIP_STOPS,
  },
  "radar-composite": {
    title: "Radar",
    stops: PRECIP_STOPS,
  },
  "radar-hrrr": {
    title: "Radar",
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
  "feels-like": {
    title: "Feels Like",
    stops: [
      { label: "130", color: "#7c1f11" },
      { label: "90", color: "#ff6b28" },
      { label: "60", color: "#ffd042" },
      { label: "30", color: "#7ed957" },
      { label: "0", color: "#4fb8ff" },
      { label: "-40", color: "#3a2c8a" },
    ],
  },
  dewpoint: {
    title: "Dew Point",
    stops: [
      { label: "75+", color: "#a01e8c" },
      { label: "65", color: "#ff9628" },
      { label: "60", color: "#f0d73c" },
      { label: "50", color: "#5ac36e" },
      { label: "40", color: "#d7c382" },
      { label: "10", color: "#966e46" },
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
    title: "Rain, Snow or Ice",
    stops: [
      { label: "Heavy", color: "#ffb04a" },
      { label: "Light", color: "#4fb8ff" },
    ],
  },
  cape: {
    title: "Storm Energy",
    stops: [
      { label: "Severe", color: "#ff4d6d" },
      { label: "Watch", color: "#ff9f2e" },
      { label: "Calm", color: "#3bc77a" },
    ],
  },
  "precip-accum": {
    title: "Rain Total",
    stops: RAIN_TOTAL_STOPS,
  },
  "precip-24h": {
    title: "Past 24 Hr",
    stops: RAIN_TOTAL_STOPS,
  },
  // Fallbacks for servers that send no palettes; serverLegend normally replaces these.
  "hail-size": {
    title: "Hail Size",
    stops: [
      { label: "4+", color: "#6e1e96" },
      { label: "2", color: "#be143c" },
      { label: "1", color: "#ffaa28" },
      { label: "0.5", color: "#3cbe3c" },
    ],
  },
  "rotation-track": {
    title: "Rotation",
    stops: [
      { label: "0.05+", color: "#50147a" },
      { label: "0.02", color: "#d71e32" },
      { label: "0.01", color: "#f55a28" },
      { label: "0.005", color: "#ffc846" },
    ],
  },
  "snowfall-forecast": {
    title: "Snow Total",
    stops: [
      { label: "24+", color: "#e6466e" },
      { label: "12", color: "#be3caa" },
      { label: "6", color: "#08269a" },
      { label: "1", color: "#6baed6" },
    ],
  },
  "ice-forecast": {
    title: "Ice Total",
    stops: [
      { label: "1+", color: "#3c0a50" },
      { label: "0.5", color: "#962882" },
      { label: "0.25", color: "#d23ca0" },
      { label: "0.01", color: "#ffbedc" },
    ],
  },
  "snow-depth": {
    title: "Snow Depth",
    stops: [
      { label: "72+", color: "#ffffff" },
      { label: "24", color: "#3c3c96" },
      { label: "6", color: "#1e5aa5" },
      { label: "1", color: "#c8e6eb" },
    ],
  },
  "snowfall-24h": {
    title: "Past 24 Hr Snow",
    stops: [
      { label: "24+", color: "#e6466e" },
      { label: "12", color: "#be3caa" },
      { label: "6", color: "#08269a" },
      { label: "1", color: "#6baed6" },
    ],
  },
  // Words, not numbers; the card builds this from the server palette when it has one.
  "nice-day": {
    title: NICE_DAY_LAYER.name,
    stops: niceDayLegend(null).map((s) => ({ label: s.word, color: s.color })),
  },
  // Degrees F above or below the normal high, warmest first.
  "temp-anomaly": {
    title: `${TEMP_NORMAL_LAYER.name} (\u00B0F)`,
    stops: tempAnomalyLegend(null).reverse(),
  },
  stargazing: {
    title: STARGAZING_LAYER.name,
    stops: stargazingLegend(null).map((s) => ({ label: s.word, color: s.color })),
  },
  "snow-chance-1": SNOW_CHANCE_LEGEND,
  "snow-chance-4": SNOW_CHANCE_LEGEND,
  "snow-chance-8": SNOW_CHANCE_LEGEND,
  "echo-top": {
    title: "Echo Tops",
    stops: [
      { label: "65+", color: "#be32aa" },
      { label: "45", color: "#ff9632" },
      { label: "25", color: "#3cc878" },
      { label: "5", color: "#78b4ff" },
    ],
  },
  cloud: {
    title: "Clouds",
    stops: [
      { label: "100%", color: "#505064" },
      { label: "75%", color: "#787887" },
      { label: "50%", color: "#a0a0aa" },
      { label: "25%", color: "#c8c8cd" },
      { label: "0%", color: "#e6e6e6" },
    ],
  },
  // WHO UV Index categories; the server palettes (serverLegend) normally replace this.
  uv: {
    title: "UV Index",
    stops: [
      { label: "11+", color: "#6b49c8" },
      { label: "8", color: "#d8001d" },
      { label: "6", color: "#f85900" },
      { label: "3", color: "#f7e400" },
      { label: "1", color: "#289500" },
    ],
  },
  // US AQI category colors (EPA), worst at the top like the other scales.
  "air-quality": {
    title: "Air Quality",
    stops: [
      { label: "Hazard", color: "#7e0023" },
      { label: "V. Unh.", color: "#8f3f97" },
      { label: "Unhealthy", color: "#ff3c00" },
      { label: "Sensitive", color: "#ff7e00" },
      { label: "Moderate", color: "#f5d500" },
      { label: "Good", color: "#00c800" },
    ],
  },
  ozone: {
    title: "Ozone",
    stops: [
      { label: "Hazard", color: "#7e0023" },
      { label: "V. Unh.", color: "#8f3f97" },
      { label: "Unhealthy", color: "#ff3c00" },
      { label: "Sensitive", color: "#ff7e00" },
      { label: "Moderate", color: "#f5d500" },
      { label: "Good", color: "#00c800" },
    ],
  },
};

export function LayerLegendCard({ activeLayer }: { activeLayer: LayerType }) {
  const replaying = useWeatherStore((s) => s.source.kind === "replay");
  // Replay tiles exist only in the classic palette.
  const palette = useWeatherStore((s) => (s.source.kind === "replay" ? "classic" : s.activePalette));
  const { data: palettes } = useServerPalettes();
  const liveLayer = useManifestQuery().data?.layers[activeLayer];
  const replayManifest = useReplayManifestQuery().data;
  const manifestLayer = replaying ? (replayManifest?.layers[activeLayer] as typeof liveLayer) : liveLayer;
  const niceDay = activeLayer === NICE_DAY_LAYER.id;
  const niceDayRanges = (palettes?.[palette] ?? palettes?.classic)?.[NICE_DAY_LAYER.paletteKey]?.ranges;
  const tempNormal = activeLayer === TEMP_NORMAL_LAYER.id;
  const anomalyRanges = (palettes?.[palette] ?? palettes?.classic)?.[TEMP_NORMAL_LAYER.paletteKey]?.ranges;
  const stars = activeLayer === STARGAZING_LAYER.id;
  const starRanges = (palettes?.[palette] ?? palettes?.classic)?.[STARGAZING_LAYER.paletteKey]?.ranges;
  const legend = tempNormal
    ? { title: `${TEMP_NORMAL_LAYER.name} (\u00B0F)`, stops: tempAnomalyLegend(anomalyRanges).reverse() }
    : niceDay
    ? {
        title: NICE_DAY_LAYER.name,
        stops: niceDayLegend(niceDayRanges).map((s) => ({ label: s.word, color: s.color })),
      }
    : stars
    ? {
        title: STARGAZING_LAYER.name,
        stops: stargazingLegend(starRanges).map((s) => ({ label: s.word, color: s.color })),
      }
    : (serverLegend(palettes, palette, activeLayer, 7, manifestLayer?.palette_key) ?? LEGENDS[activeLayer] ?? LEGENDS.radar);
  // The server colors observed radar by precipitation type when the layer says so.
  const typedLayer = manifestLayer?.precip_type === true;
  const typeRamps = typedLayer ? precipTypeRamps(palettes, palette) : null;
  const chrome = useMapChromeInsets();
  // Scalars, not the frame array: the card re-renders only when the tag can
  // change, not on every playback tick.
  const frameSource = useWeatherStore((s) => s.frames[s.currentFrameIndex]?.source ?? null);
  const frameTime = useWeatherStore((s) => s.frames[s.currentFrameIndex]?.time ?? 0);
  const asOf = winterAsOf(activeLayer, frameTime ? new Date(frameTime * 1000).toISOString() : undefined);
  const frameIsLater = useWeatherStore(
    (s) => (s.frames[s.currentFrameIndex]?.time ?? 0) > Math.floor(Date.now() / 1000) + 30 * 60,
  );

  // For the radar layer, the source tag shifts based on which frame the
  // user is sitting on: past = OBSERVED, near-future = NOWCAST, far-future
  // = FORECAST (HRRR). Other layers are statically tagged.
  let source = LAYER_SOURCE[activeLayer] ?? "Now";
  if (activeLayer === "radar" || activeLayer === "radar-hrrr") {
    if (frameSource === "nowcast") source = "Soon";
    else if (frameSource === "radar-hrrr" || frameSource === "radar-gfs") source = "Later";
    else source = "Now";
  } else if (activeLayer === "air-quality" || activeLayer === "ozone") {
    // A single AQM run spans the current hour plus 3 days out, so the tag
    // tracks whichever frame the scrubber is on.
    if (frameIsLater) source = "Later";
  }

  return (
    <View style={[styles.wrap, { top: chrome.belowClose, left: chrome.left }]}>
      <MapChromeSurface style={styles.card} fallbackStyle={styles.cardFill} colorScheme="light">
        <View style={styles.headerRow}>
          <Text maxFontSizeMultiplier={MAP_CHROME_MAX_FONT_SCALE} style={styles.title} numberOfLines={1}>
            {legend.title}
          </Text>
          <View style={[styles.tag, { backgroundColor: SOURCE_COLOR[source] + "33", borderColor: SOURCE_COLOR[source] }]}>
            <Text maxFontSizeMultiplier={MAP_CHROME_MAX_FONT_SCALE} style={[styles.tagText, { color: SOURCE_COLOR[source] }]}>{replaying ? "Past" : source}</Text>
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
              <Text maxFontSizeMultiplier={MAP_CHROME_MAX_FONT_SCALE} key={i} style={styles.label}>
                {s.label}
              </Text>
            ))}
          </View>
        </View>
        {isSevereLayer(activeLayer) && (
          <Text maxFontSizeMultiplier={MAP_CHROME_MAX_FONT_SCALE} style={styles.credit}>
            {SEVERE_CREDIT}
          </Text>
        )}
        {tempNormal && (
          <Text maxFontSizeMultiplier={MAP_CHROME_MAX_FONT_SCALE} style={styles.credit}>
            {TEMP_NORMAL_LAYER.credit}
          </Text>
        )}
        {niceDay && (
          <Text maxFontSizeMultiplier={MAP_CHROME_MAX_FONT_SCALE} style={styles.credit}>
            {NICE_DAY_LAYER.credit}
          </Text>
        )}
        {stars && (
          <Text maxFontSizeMultiplier={MAP_CHROME_MAX_FONT_SCALE} style={styles.credit}>
            Night hours only. {STARGAZING_LAYER.credit} {STARGAZING_LAYER.seeingCredit}
          </Text>
        )}
        {winterLayer(activeLayer) && (
          <Text maxFontSizeMultiplier={MAP_CHROME_MAX_FONT_SCALE} style={styles.credit}>
            {asOf ? `${asOf} · ` : ""}
            {winterLayer(activeLayer)?.credit}
          </Text>
        )}
        {typeRamps && (
          <View style={styles.types}>
            {typeRamps.map((r) => (
              <View key={r.label} style={styles.typeRow}>
                <Text maxFontSizeMultiplier={MAP_CHROME_MAX_FONT_SCALE} style={styles.typeLabel}>
                  {r.label}
                </Text>
                <LinearGradient
                  colors={r.colors as [string, string, ...string[]]}
                  start={{ x: 0, y: 0.5 }}
                  end={{ x: 1, y: 0.5 }}
                  style={styles.typeBar}
                />
              </View>
            ))}
          </View>
        )}
      </MapChromeSurface>
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
    width: 108,
    borderRadius: 16,
    paddingVertical: 9,
    paddingHorizontal: 11,
  },
  cardFill: {
    backgroundColor: CARD_BG,
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
    height: 104,
    gap: 7,
  },
  gradient: {
    width: 8,
    borderRadius: 4,
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
  credit: {
    marginTop: 6,
    fontSize: 8,
    color: "#3a4258",
    fontWeight: "600",
    letterSpacing: 0.4,
  },
  types: {
    marginTop: 8,
    gap: 4,
  },
  typeRow: {
    flexDirection: "row",
    alignItems: "center",
    gap: 6,
  },
  typeLabel: {
    width: 28,
    fontSize: 9,
    color: "#3a4258",
    fontWeight: "600",
  },
  typeBar: {
    flex: 1,
    height: 6,
    borderRadius: 3,
  },
});
