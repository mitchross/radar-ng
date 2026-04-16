/**
 * CARROT-style floating action buttons for the radar screen.
 * Clean, minimal, white-on-blue design matching CARROT Weather's radar FABs.
 */
import { View, TouchableOpacity, StyleSheet } from "react-native";
import { useWeatherStore } from "../../stores/useWeatherStore";

// View-based icons matching CARROT's radar FABs

function LocationArrowIcon() {
  return (
    <View style={iconStyles.locationContainer}>
      {/* Arrow pointing up-right (location icon) */}
      <View style={iconStyles.locationArrow} />
      <View style={iconStyles.locationArrowStem} />
    </View>
  );
}

function LayersIcon({ active }: { active?: boolean }) {
  const color = active ? "#fff" : "#444";
  return (
    <View style={iconStyles.layersContainer}>
      <View style={[iconStyles.layerLine1, { backgroundColor: color }]} />
      <View style={[iconStyles.layerLine2, { backgroundColor: color }]} />
      <View style={[iconStyles.layerLine3, { backgroundColor: color }]} />
    </View>
  );
}

function MapStyleIcon() {
  return (
    <View style={iconStyles.mapStyleContainer}>
      <View style={iconStyles.mapSquare}>
        <View style={iconStyles.mapFold} />
      </View>
    </View>
  );
}

export function RadarFABs() {
  const mapStyle = useWeatherStore((s) => s.mapStyle);
  const setMapStyle = useWeatherStore((s) => s.setMapStyle);
  const radarVisible = useWeatherStore((s) => s.radarVisible);
  const setRadarVisible = useWeatherStore((s) => s.setRadarVisible);

  return (
    <View style={styles.container}>
      {/* Location button */}
      <TouchableOpacity style={styles.fab} activeOpacity={0.7}>
        <LocationArrowIcon />
      </TouchableOpacity>

      {/* Layers toggle (radar on/off) */}
      <TouchableOpacity
        style={[styles.fab, radarVisible && styles.fabActive]}
        onPress={() => setRadarVisible(!radarVisible)}
        activeOpacity={0.7}
      >
        <LayersIcon active={radarVisible} />
      </TouchableOpacity>

      {/* Map style toggle */}
      <TouchableOpacity
        style={styles.fab}
        onPress={() => setMapStyle(mapStyle === "dark" ? "light" : "dark")}
        activeOpacity={0.7}
      >
        <MapStyleIcon />
      </TouchableOpacity>
    </View>
  );
}

const styles = StyleSheet.create({
  container: {
    position: "absolute",
    right: 12,
    top: 100,
    gap: 10,
    zIndex: 50,
  },
  fab: {
    width: 44,
    height: 44,
    borderRadius: 22,
    backgroundColor: "rgba(255,255,255,0.92)",
    justifyContent: "center",
    alignItems: "center",
    shadowColor: "#000",
    shadowOffset: { width: 0, height: 2 },
    shadowOpacity: 0.15,
    shadowRadius: 4,
    elevation: 4,
  },
  fabActive: {
    backgroundColor: "#2196F3",
  },
});

const iconStyles = StyleSheet.create({
  // Location arrow
  locationContainer: {
    width: 20,
    height: 20,
    alignItems: "center",
    justifyContent: "center",
  },
  locationArrow: {
    width: 0,
    height: 0,
    borderLeftWidth: 7,
    borderRightWidth: 7,
    borderBottomWidth: 14,
    borderLeftColor: "transparent",
    borderRightColor: "transparent",
    borderBottomColor: "#2196F3",
    transform: [{ rotate: "45deg" }],
  },
  locationArrowStem: {
    position: "absolute",
    bottom: 3,
    left: 8,
    width: 4,
    height: 4,
    borderRadius: 2,
    backgroundColor: "#2196F3",
  },
  // Layers icon (3 stacked lines)
  layersContainer: {
    width: 18,
    height: 14,
    justifyContent: "space-between",
    alignItems: "center",
  },
  layerLine1: {
    width: 18,
    height: 2.5,
    borderRadius: 1.5,
    backgroundColor: "#444",
  },
  layerLine2: {
    width: 14,
    height: 2.5,
    borderRadius: 1.5,
    backgroundColor: "#444",
  },
  layerLine3: {
    width: 10,
    height: 2.5,
    borderRadius: 1.5,
    backgroundColor: "#444",
  },
  // Map style icon
  mapStyleContainer: {
    width: 18,
    height: 18,
    alignItems: "center",
    justifyContent: "center",
  },
  mapSquare: {
    width: 16,
    height: 16,
    borderWidth: 1.5,
    borderColor: "#444",
    borderRadius: 3,
    overflow: "hidden",
    alignItems: "center",
    justifyContent: "center",
  },
  mapFold: {
    width: 16,
    height: 16,
    borderLeftWidth: 1,
    borderRightWidth: 1,
    borderColor: "#444",
    opacity: 0.4,
    transform: [{ rotate: "30deg" }],
  },
});
