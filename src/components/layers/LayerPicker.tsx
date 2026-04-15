import { View, TouchableOpacity, Text, StyleSheet } from "react-native";
import { useWeatherStore } from "../../stores/useWeatherStore";
import { LAYERS } from "../../lib/constants";
import type { LayerConfig } from "../../types/weather";

export function LayerPicker() {
  const activeLayer = useWeatherStore((s) => s.activeLayer);
  const setActiveLayer = useWeatherStore((s) => s.setActiveLayer);
  const visibleOverlays = useWeatherStore((s) => s.visibleOverlays);
  const toggleOverlay = useWeatherStore((s) => s.toggleOverlay);
  const dataSource = useWeatherStore((s) => s.dataSource);

  const handlePress = (layer: LayerConfig) => {
    if (layer.isFillLayer) {
      setActiveLayer(layer.id);
    } else {
      toggleOverlay(layer.id);
    }
  };

  const isActive = (layer: LayerConfig) => {
    if (layer.isFillLayer) return activeLayer === layer.id;
    return visibleOverlays.has(layer.id);
  };

  const availableLayers =
    dataSource === "rainviewer"
      ? LAYERS.filter((l) => l.id === "radar")
      : LAYERS;

  return (
    <View style={styles.container}>
      {availableLayers.map((layer) => (
        <TouchableOpacity
          key={layer.id}
          style={[styles.button, isActive(layer) && styles.buttonActive]}
          onPress={() => handlePress(layer)}
          activeOpacity={0.7}
        >
          <Text style={styles.icon}>{layer.icon}</Text>
          <Text style={[styles.label, isActive(layer) && styles.labelActive]}>
            {layer.label}
          </Text>
        </TouchableOpacity>
      ))}
    </View>
  );
}

const styles = StyleSheet.create({
  container: {
    position: "absolute",
    right: 10,
    top: 90,
    gap: 6,
    zIndex: 50,
  },
  button: {
    width: 48,
    height: 48,
    borderRadius: 24,
    backgroundColor: "rgba(10, 10, 20, 0.75)",
    justifyContent: "center",
    alignItems: "center",
    borderWidth: 1.5,
    borderColor: "rgba(255,255,255,0.1)",
  },
  buttonActive: {
    borderColor: "#4fc3f7",
    backgroundColor: "rgba(79, 195, 247, 0.2)",
  },
  icon: {
    fontSize: 16,
  },
  label: {
    fontSize: 8,
    color: "#666",
    marginTop: 1,
  },
  labelActive: {
    color: "#4fc3f7",
  },
});
