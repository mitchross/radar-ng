import { View, Text, TouchableOpacity, StyleSheet } from "react-native";
import { SafeAreaView } from "react-native-safe-area-context";
import { WeatherMap } from "../../components/map/WeatherMap";
import { RadarOverlay } from "../../components/map/RadarOverlay";
import { WeatherLayerOverlay } from "../../components/map/WeatherLayerOverlay";
import { AlertPolygon } from "../../components/map/AlertPolygon";
import { TimelineBar } from "../../components/timeline/TimelineBar";
import { RadarFABs } from "../../components/map/RadarFABs";
import { AlertBanner } from "../../components/alerts/AlertBanner";
import { useManifest } from "../../hooks/useManifest";
import { useWeatherStore } from "../../stores/useWeatherStore";

export default function RadarScreen() {
  useManifest();

  const activeLayer = useWeatherStore((s) => s.activeLayer);
  const visibleOverlays = useWeatherStore((s) => s.visibleOverlays);
  const radarOpacity = useWeatherStore((s) => s.radarOpacity);
  const dataSource = useWeatherStore((s) => s.dataSource);

  return (
    <View style={styles.container}>
      <WeatherMap>
        {activeLayer === "radar" && <RadarOverlay />}
        {activeLayer === "temperature" && dataSource === "selfhosted" && (
          <WeatherLayerOverlay layerId="temperature" opacity={radarOpacity} />
        )}
        {activeLayer === "precip-type" && dataSource === "selfhosted" && (
          <WeatherLayerOverlay layerId="precip-type" opacity={radarOpacity} />
        )}
        {visibleOverlays.has("wind") && dataSource === "selfhosted" && (
          <WeatherLayerOverlay layerId="wind" opacity={0.6} />
        )}
        {visibleOverlays.has("cape") && dataSource === "selfhosted" && (
          <WeatherLayerOverlay layerId="cape" opacity={0.5} />
        )}
        <AlertPolygon />
      </WeatherMap>

      {/* Top: alert banner */}
      <AlertBanner />

      {/* Right: CARROT-style FABs */}
      <RadarFABs />

      {/* Bottom: timeline bar */}
      <TimelineBar />
    </View>
  );
}

const styles = StyleSheet.create({
  container: {
    flex: 1,
    backgroundColor: "#f0f0f0",
  },
});
