import { View, StyleSheet } from "react-native";
import { WeatherMap } from "../../components/map/WeatherMap";
import { RadarOverlay } from "../../components/map/RadarOverlay";
import { TimeSlider } from "../../components/timeline/TimeSlider";
import { PlayButton } from "../../components/timeline/PlayButton";
import { ForecastSheet } from "../../components/forecast/ForecastSheet";
import { AlertBanner } from "../../components/alerts/AlertBanner";
import { useLocation } from "../../hooks/useLocation";
import { useManifest } from "../../hooks/useManifest";

export default function MapScreen() {
  useLocation();
  useManifest();

  return (
    <View style={styles.container}>
      <WeatherMap>
        <RadarOverlay />
      </WeatherMap>
      <AlertBanner />
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
  container: {
    flex: 1,
    backgroundColor: "#000",
  },
  timelineBar: {
    flexDirection: "row",
    alignItems: "center",
    backgroundColor: "rgba(26, 26, 46, 0.9)",
    paddingBottom: 16,
  },
  sliderContainer: {
    flex: 1,
  },
});
