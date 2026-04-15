import { View, StyleSheet } from "react-native";
import { WeatherMap } from "../../components/map/WeatherMap";
import { RadarOverlay } from "../../components/map/RadarOverlay";
import { WeatherLayerOverlay } from "../../components/map/WeatherLayerOverlay";
import { AlertPolygon } from "../../components/map/AlertPolygon";
import { TimeSlider } from "../../components/timeline/TimeSlider";
import { PlayButton } from "../../components/timeline/PlayButton";
import { ForecastSheet } from "../../components/forecast/ForecastSheet";
import { AlertBanner } from "../../components/alerts/AlertBanner";
import { LayerPicker } from "../../components/layers/LayerPicker";
import { useLocation } from "../../hooks/useLocation";
import { useManifest } from "../../hooks/useManifest";
import { useWeatherStore } from "../../stores/useWeatherStore";

export default function MapScreen() {
  useLocation();
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

      <AlertBanner />
      <LayerPicker />

      <View style={styles.timelineBar}>
        <PlayButton />
        <View style={styles.sliderContainer}>
          <TimeSlider />
        </View>
      </View>
      <ForecastSheet />
    </View>
  );
}

const styles = StyleSheet.create({
  container: { flex: 1, backgroundColor: "#000" },
  timelineBar: {
    flexDirection: "row",
    alignItems: "center",
    backgroundColor: "rgba(26, 26, 46, 0.9)",
    paddingBottom: 16,
  },
  sliderContainer: { flex: 1 },
});
