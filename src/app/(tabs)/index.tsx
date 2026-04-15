import { View, StyleSheet } from "react-native";
import { WeatherMap } from "../../components/map/WeatherMap";
import { RadarOverlay } from "../../components/map/RadarOverlay";
import { WeatherLayerOverlay } from "../../components/map/WeatherLayerOverlay";
import { AlertPolygon } from "../../components/map/AlertPolygon";
import { TimeSlider } from "../../components/timeline/TimeSlider";
import { PlayButton } from "../../components/timeline/PlayButton";
import { ForecastPeek } from "../../components/forecast/ForecastSheet";
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
      {/* Full-bleed map */}
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

      {/* Floating overlays */}
      <AlertBanner />
      <LayerPicker />

      {/* Bottom controls — floating over map */}
      <View style={styles.bottomControls}>
        <ForecastPeek />
        <View style={styles.timelineBar}>
          <PlayButton />
          <View style={styles.sliderContainer}>
            <TimeSlider />
          </View>
        </View>
      </View>
    </View>
  );
}

const styles = StyleSheet.create({
  container: {
    flex: 1,
    backgroundColor: "#000",
  },
  bottomControls: {
    position: "absolute",
    bottom: 0,
    left: 0,
    right: 0,
    paddingBottom: 60, // space for tab bar
  },
  timelineBar: {
    flexDirection: "row",
    alignItems: "center",
    backgroundColor: "rgba(10, 10, 20, 0.85)",
    paddingVertical: 6,
    paddingHorizontal: 4,
  },
  sliderContainer: {
    flex: 1,
  },
});
