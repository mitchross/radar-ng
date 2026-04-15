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
    right: 12,
    top: 100,
    gap: 8,
    zIndex: 50,
  },
  button: {
    width: 56,
    height: 56,
    borderRadius: 28,
    backgroundColor: "rgba(26, 26, 46, 0.85)",
    justifyContent: "center",
    alignItems: "center",
    borderWidth: 2,
    borderColor: "transparent",
  },
  buttonActive: {
    borderColor: "#4fc3f7",
    backgroundColor: "rgba(79, 195, 247, 0.15)",
  },
  icon: { fontSize: 18 },
  label: { fontSize: 9, color: "#888", marginTop: 1 },
  labelActive: { color: "#4fc3f7" },
});
