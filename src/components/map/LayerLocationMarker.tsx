/**
 * Apple-Weather-style "My Location" marker — pill shows a layer-aware value
 * with a triangular tail pointing down to the user dot on the map.
 *
 *   Wind:        stacked "<dir> / <mph> / MPH"
 *   Temperature: big number
 *   AQI:         big number (placeholder until AQI layer lands)
 *   Radar/etc:   falls back to current temperature
 */
import { View, Text, StyleSheet } from "react-native";
import MapLibreGL from "@maplibre/maplibre-react-native";
import { useWeatherStore } from "../../stores/useWeatherStore";
import { useForecast } from "../../hooks/useForecast";
import { cumulus, getWindDirection } from "../../lib/cumulusTheme";
import type { LayerType } from "../../types/weather";

export function LayerLocationMarker() {
  const latitude = useWeatherStore((s) => s.latitude);
  const longitude = useWeatherStore((s) => s.longitude);
  const activeLayer = useWeatherStore((s) => s.activeLayer);
  const temperatureUnit = useWeatherStore((s) => s.temperatureUnit);
  const { data: forecast } = useForecast();

  if (latitude == null || longitude == null) return null;

  const body = renderBody(activeLayer, forecast, temperatureUnit);

  return (
    <MapLibreGL.MarkerView
      coordinate={[longitude, latitude]}
      anchor={{ x: 0.5, y: 1 }}
      allowOverlap
    >
      <View style={styles.wrap} pointerEvents="none">
        <View style={styles.pill}>{body}</View>
        <View style={styles.tail} />
        <View style={styles.dot} />
        <Text style={styles.label}>My Location</Text>
      </View>
    </MapLibreGL.MarkerView>
  );
}

function renderBody(
  layer: LayerType,
  forecast: ReturnType<typeof useForecast>["data"],
  unit: "fahrenheit" | "celsius",
) {
  if (layer === "wind") {
    const mph = forecast?.current?.wind_speed_10m;
    const deg = forecast?.current?.wind_direction_10m;
    if (mph == null || deg == null) return <Text style={styles.value}>{"\u2014"}</Text>;
    return (
      <>
        <Text style={styles.windDir}>{getWindDirection(deg)}</Text>
        <Text style={[styles.value, { color: cumulus.rain }]}>{Math.round(mph)}</Text>
        <Text style={styles.unit}>MPH</Text>
      </>
    );
  }

  if (layer === "cape") {
    return (
      <>
        <Text style={styles.value}>{"\u2014"}</Text>
        <Text style={styles.unit}>J/KG</Text>
      </>
    );
  }

  const t = forecast?.current?.temperature_2m;
  if (t == null) return <Text style={styles.value}>{"\u2014"}</Text>;
  // Tile-server's /api/forecast already returns Fahrenheit (open-meteo
  // upstream is configured with `temperature_unit=fahrenheit`). Home tab
  // does the same passthrough \u2014 earlier `t * 9/5 + 32` here was a double
  // conversion that displayed 131\u00b0 for what should read 55\u00b0. The `unit`
  // store flag is currently cosmetic; honoring it requires a server-side
  // unit param, out of scope here.
  return <Text style={styles.value}>{Math.round(t)}</Text>;
}

const PILL_BG = "rgba(255,255,255,0.98)";
const PILL_BORDER = "rgba(10,20,40,0.08)";

const styles = StyleSheet.create({
  wrap: { alignItems: "center" },
  pill: {
    backgroundColor: PILL_BG,
    borderRadius: 18,
    paddingHorizontal: 12,
    paddingVertical: 6,
    minWidth: 52,
    alignItems: "center",
    shadowColor: "#000",
    shadowOpacity: 0.15,
    shadowRadius: 6,
    shadowOffset: { width: 0, height: 2 },
    elevation: 4,
    borderWidth: 1,
    borderColor: PILL_BORDER,
  },
  tail: {
    width: 0,
    height: 0,
    borderLeftWidth: 6,
    borderRightWidth: 6,
    borderTopWidth: 7,
    borderLeftColor: "transparent",
    borderRightColor: "transparent",
    borderTopColor: PILL_BG,
    marginTop: -1,
  },
  dot: {
    width: 9,
    height: 9,
    borderRadius: 5,
    backgroundColor: cumulus.rain,
    borderWidth: 2,
    borderColor: "#fff",
    marginTop: 2,
    shadowColor: "#000",
    shadowOpacity: 0.25,
    shadowRadius: 2,
    shadowOffset: { width: 0, height: 1 },
    elevation: 3,
  },
  label: {
    marginTop: 2,
    fontSize: 11,
    fontWeight: "600",
    color: "#1a2030",
    textShadowColor: "rgba(255,255,255,0.8)",
    textShadowRadius: 3,
  },
  value: {
    fontSize: 18,
    fontWeight: "700",
    color: "#1a2030",
    fontVariant: ["tabular-nums"],
    lineHeight: 20,
  },
  windDir: {
    fontSize: 10,
    fontWeight: "700",
    color: "#4c5770",
    letterSpacing: 0.6,
  },
  unit: {
    fontSize: 9,
    fontWeight: "700",
    color: "#4c5770",
    letterSpacing: 0.6,
  },
});
