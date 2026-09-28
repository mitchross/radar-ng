/**
 * Radar map controls: one Liquid Glass capsule holding the map options
 * (layer, style, storms; its symbol is the active layer) and locate. Refresh
 * is automatic and inspecting is a long-press, so neither needs a button.
 */
import { View, StyleSheet, Pressable } from "react-native";
import { SymbolView, type SymbolViewProps } from "expo-symbols";
import { useWeatherStore } from "../../stores/useWeatherStore";
import { refreshDeviceLocation } from "../../hooks/useLocation";
import { useMapChromeInsets } from "../../hooks/useMapChromeInsets";
import { MapChromeSurface } from "../ui/MapChromeSurface";
import { LAYER_SYMBOLS, layerOption } from "./radarLayers";

const ICON_COLOR = "#1a2030";

export function RadarFABs({ onOpenMapOptions }: { onOpenMapOptions: () => void }) {
  const chrome = useMapChromeInsets();
  const activeLayer = useWeatherStore((s) => s.activeLayer);
  const requestRecenter = useWeatherStore((s) => s.requestRecenter);
  const layer = layerOption(activeLayer);

  return (
    <MapChromeSurface
      style={[styles.capsule, { top: chrome.top, right: chrome.right }]}
      fallbackStyle={styles.capsuleFill}
      colorScheme="light"
      pointerEvents="box-none"
    >
      <CapsuleButton
        symbol={LAYER_SYMBOLS[layer.icon]}
        onPress={onOpenMapOptions}
        accessibilityLabel="Choose radar layer"
        accessibilityHint={`${layer.name} is showing. Opens layers, map style and storm overlays.`}
      />
      <View style={styles.divider} />
      <CapsuleButton
        symbol={{ ios: "location.fill", android: "near_me" }}
        onPress={() => {
          requestRecenter();
          refreshDeviceLocation();
        }}
        accessibilityLabel="Center map on your location"
      />
    </MapChromeSurface>
  );
}

function CapsuleButton({ symbol, onPress, accessibilityLabel, accessibilityHint }: {
  symbol: SymbolViewProps["name"];
  onPress: () => void;
  accessibilityLabel: string;
  accessibilityHint?: string;
}) {
  return (
    <Pressable
      onPress={onPress}
      style={({ pressed }) => [styles.button, pressed ? styles.pressed : null]}
      accessibilityRole="button"
      accessibilityLabel={accessibilityLabel}
      accessibilityHint={accessibilityHint}
    >
      <SymbolView name={symbol} size={20} tintColor={ICON_COLOR} weight="semibold" type="hierarchical" />
    </Pressable>
  );
}

const styles = StyleSheet.create({
  capsule: {
    position: "absolute",
    zIndex: 20,
    width: 48,
    borderRadius: 24,
    overflow: "hidden",
  },
  capsuleFill: {
    backgroundColor: "rgba(255,255,255,0.92)",
    borderWidth: StyleSheet.hairlineWidth,
    borderColor: "rgba(10,20,40,0.12)",
    shadowColor: "#000",
    shadowOpacity: 0.14,
    shadowRadius: 8,
    shadowOffset: { width: 0, height: 2 },
    elevation: 4,
  },
  button: { width: 48, height: 48, alignItems: "center", justifyContent: "center" },
  pressed: { opacity: 0.55 },
  divider: { height: StyleSheet.hairlineWidth, marginHorizontal: 10, backgroundColor: "rgba(10,20,40,0.18)" },
});
