import { View, StyleSheet } from "react-native";
import { WeatherMap } from "../../components/map/WeatherMap";
import { RadarOverlay } from "../../components/map/RadarOverlay";
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
    </View>
  );
}

const styles = StyleSheet.create({
  container: {
    flex: 1,
    backgroundColor: "#000",
  },
});
