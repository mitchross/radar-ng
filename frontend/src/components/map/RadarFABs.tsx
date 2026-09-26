/**
 * Radar right-rail controls: layer picker, storms, inspector, locate, map
 * style, refresh. Liquid Glass buttons with SF Symbols (Material Symbols on
 * Android); an Apple-Weather-style layer popover with a layer-tinted background.
 */
import { useEffect, useState } from "react";
import { Animated, Easing, View, StyleSheet, Text, Pressable } from "react-native";
import { useQueryClient } from "@tanstack/react-query";
import { useWeatherStore } from "../../stores/useWeatherStore";
import { pickNowFrameIndex } from "../../hooks/useManifest";
import { cumulus } from "../../lib/cumulusTheme";
import { runOnlineRefresh } from "../../lib/queryLifecycle";
import { refreshDeviceLocation } from "../../hooks/useLocation";
import type { LayerType } from "../../types/weather";
import { MAP_CHROME_MAX_FONT_SCALE } from "../../lib/constants";
import { useMapChromeInsets } from "../../hooks/useMapChromeInsets";
import { MapChromeSurface } from "../ui/MapChromeSurface";
import { SymbolView, type SymbolViewProps } from "expo-symbols";

type IconKind = "umbrella" | "thermo" | "dust" | "wind" | "bolt" | "layers" | "drop" | "cloud" | "ozone";

// Five layers, plain English, no acronyms. HRRR/Composite/CAPE/Precip-Type
// are still ingested in the backend (and feed the merged radar timeline) —
// they're just hidden from the picker because nobody asks "is the CAPE
// high today?" in normal life.
const LAYER_OPTIONS: {
  id: LayerType;
  name: string;
  icon: IconKind;
  tint: string;
  selfHostedOnly?: boolean;
}[] = [
  { id: "radar", name: "Radar", icon: "umbrella", tint: "#E8EFFA" },
  { id: "temperature", name: "Temperature", icon: "thermo", tint: "#D6F1FB", selfHostedOnly: true },
  { id: "wind", name: "Wind", icon: "wind", tint: "#DDE8F5", selfHostedOnly: true },
  { id: "precip-accum", name: "Rain Total", icon: "drop", tint: "#DEEAFA", selfHostedOnly: true },
  { id: "cloud", name: "Clouds", icon: "cloud", tint: "#ECECEF", selfHostedOnly: true },
  { id: "air-quality", name: "Air Quality", icon: "dust", tint: "#E9F5E4", selfHostedOnly: true },
  { id: "ozone", name: "Ozone", icon: "ozone", tint: "#EDF2FB", selfHostedOnly: true },
];

export function RadarFABs({
  inspectorActive,
  onToggleInspector,
  onOpenStylePicker,
}: {
  inspectorActive: boolean;
  onToggleInspector: () => void;
  onOpenStylePicker?: () => void;
}) {
  const chrome = useMapChromeInsets();
  const activeLayer = useWeatherStore((s) => s.activeLayer);
  const setActiveLayer = useWeatherStore((s) => s.setActiveLayer);
  const extrasVisible = useWeatherStore((s) => s.extrasVisible);
  const toggleExtras = useWeatherStore((s) => s.toggleExtras);
  const setCurrentFrameIndex = useWeatherStore((s) => s.setCurrentFrameIndex);
  const setIsPlaying = useWeatherStore((s) => s.setIsPlaying);
  const requestRecenter = useWeatherStore((s) => s.requestRecenter);
  const queryClient = useQueryClient();
  const [layerOpen, setLayerOpen] = useState(false);

  const [refreshing, setRefreshing] = useState(false);

  // Pause + immediately snap to "Now" using the current frame list, then
  // refetch the manifest. Use refetchQueries (not invalidateQueries) so the
  // request actually fires even when React Query still considers the data
  // fresh under refetchInterval — invalidate was a near-no-op for fast
  // taps. Also surface a spin state so the user gets visual confirmation.
  const onRefresh = async () => {
    if (refreshing) return;
    await runOnlineRefresh(async () => {
      setRefreshing(true);
      setIsPlaying(false);
      const frames = useWeatherStore.getState().frames;
      setCurrentFrameIndex(pickNowFrameIndex(frames));
      try {
        await queryClient.refetchQueries({ queryKey: ["manifest"] });
      } finally {
        setRefreshing(false);
      }
    });
  };

  const options = LAYER_OPTIONS;
  const activeOpt = options.find((o) => o.id === activeLayer) ?? options[0];
  const popoverBg = activeOpt?.tint ?? "#E8EFFA";

  return (
    <>
      <View style={[styles.rail, { top: chrome.top, right: chrome.right }]}>
        <GlassBtn
          active={layerOpen}
          onPress={() => setLayerOpen((v) => !v)}
          accessibilityLabel="Choose radar layer"
          // Shows the active layer, so the rail says what's on the map.
          symbol={LAYER_SYMBOLS[activeOpt.icon]}
        />
        <GlassBtn
          active={extrasVisible}
          onPress={toggleExtras}
          accessibilityLabel="Toggle storm and lightning overlays"
          symbol={{ ios: extrasVisible ? "cloud.bolt.fill" : "cloud.bolt", android: "thunderstorm" }}
        />
        <GlassBtn
          active={inspectorActive}
          onPress={onToggleInspector}
          accessibilityLabel={
            inspectorActive ? "Clear pinned inspection" : "Inspect a point on the map"
          }
          symbol={{ ios: inspectorActive ? "mappin.slash" : "hand.point.up.left", android: inspectorActive ? "location_off" : "touch_app" }}
        />
        <GlassBtn
          onPress={() => {
            requestRecenter();
            refreshDeviceLocation();
          }}
          accessibilityLabel="Center map on your location"
          symbol={{ ios: "location.fill", android: "near_me" }}
        />
        <GlassBtn
          onPress={onOpenStylePicker}
          accessibilityLabel="Choose map style"
          symbol={{ ios: "map.fill", android: "map" }}
        />
        <GlassBtn
          onPress={onRefresh}
          accessibilityLabel={refreshing ? "Refreshing radar" : "Refresh radar"}
          disabled={refreshing}
          symbol={{ ios: "arrow.clockwise", android: "refresh" }}
          spinning={refreshing}
        />
      </View>

      {layerOpen ? (
        <>
          <Pressable
            style={styles.scrim}
            onPress={() => setLayerOpen(false)}
            accessibilityRole="button"
            accessibilityLabel="Close radar layer picker"
          />
          <View style={[styles.panel, { top: chrome.top - 8, right: chrome.right + 56, backgroundColor: popoverBg }]}>
            {options.map((opt) => {
              const isActive = activeLayer === opt.id;
              return (
                <Pressable
                  key={opt.id}
                  onPress={() => {
                    setActiveLayer(opt.id);
                    setLayerOpen(false);
                  }}
                  style={({ pressed }) => [
                    styles.panelRow,
                    pressed ? styles.panelRowPressed : null,
                  ]}
                  accessibilityRole="radio"
                  accessibilityState={{ checked: isActive }}
                  accessibilityLabel={`${opt.name} radar layer`}
                >
                  <View style={styles.checkCol}>
                    {isActive ? <SymbolView name={{ ios: "checkmark", android: "check" }} size={14} tintColor={ICON_COLOR} weight="bold" /> : null}
                  </View>
                  <View style={styles.iconCol}>
                    <SymbolView name={LAYER_SYMBOLS[opt.icon]} size={20} tintColor={ICON_COLOR} type="hierarchical" />
                  </View>
                  <Text maxFontSizeMultiplier={MAP_CHROME_MAX_FONT_SCALE} style={[styles.panelRowTitle, isActive && styles.panelRowTitleActive]}>
                    {opt.name}
                  </Text>
                </Pressable>
              );
            })}
          </View>
        </>
      ) : null}
    </>
  );
}

function GlassBtn({
  symbol,
  spinning = false,
  onPress,
  active,
  accessibilityLabel,
  disabled = false,
}: {
  symbol: SymbolName;
  spinning?: boolean;
  onPress?: () => void;
  active?: boolean;
  accessibilityLabel: string;
  disabled?: boolean;
}) {
  return (
    <Pressable
      onPress={onPress}
      disabled={disabled}
      style={({ pressed }) => [
        styles.btnTarget,
        pressed ? styles.btnPressed : null,
        disabled ? styles.btnDisabled : null,
      ]}
      accessibilityRole="button"
      accessibilityLabel={accessibilityLabel}
      accessibilityState={{ selected: Boolean(active), disabled }}
    >
      <MapChromeSurface
        style={styles.btn}
        fallbackStyle={[styles.btnFill, active ? styles.btnActive : null]}
        colorScheme="light"
        tintColor={active ? cumulus.accent : undefined}
        interactive
        pointerEvents="none"
      >
        <SpinningSymbol name={symbol} color={active ? "#ffffff" : ICON_COLOR} spinning={spinning} />
      </MapChromeSurface>
    </Pressable>
  );
}

/** ─── Symbols: SF Symbols on iOS, Material Symbols on Android ───────── */

type SymbolName = SymbolViewProps["name"];
const ICON_COLOR = "#1a2030";

const LAYER_SYMBOLS: Record<IconKind, SymbolName> = {
  umbrella: { ios: "cloud.rain.fill", android: "rainy" },
  thermo: { ios: "thermometer.medium", android: "thermostat" },
  dust: { ios: "aqi.medium", android: "masks" },
  wind: { ios: "wind", android: "air" },
  bolt: { ios: "cloud.bolt.fill", android: "thunderstorm" },
  layers: { ios: "square.3.layers.3d", android: "layers" },
  drop: { ios: "drop.fill", android: "water_drop" },
  cloud: { ios: "cloud.fill", android: "cloud" },
  ozone: { ios: "sun.haze.fill", android: "hexagon" },
};

/** Spins while a refresh is in flight so the tap reads as "doing something". */
function SpinningSymbol({ name, color, spinning }: { name: SymbolName; color: string; spinning: boolean }) {
  const [rotation] = useState(() => new Animated.Value(0));
  useEffect(() => {
    if (!spinning) {
      rotation.stopAnimation();
      rotation.setValue(0);
      return;
    }
    const loop = Animated.loop(
      Animated.timing(rotation, { toValue: 1, duration: 800, easing: Easing.linear, useNativeDriver: true }),
    );
    loop.start();
    return () => loop.stop();
  }, [spinning, rotation]);
  const rotate = rotation.interpolate({ inputRange: [0, 1], outputRange: ["0deg", "360deg"] });
  return (
    <Animated.View style={{ transform: [{ rotate }] }}>
      <SymbolView name={name} size={19} tintColor={color} weight="semibold" type="hierarchical" />
    </Animated.View>
  );
}

/** ─── Styles ───────────────────────────────────────────────────────── */

const styles = StyleSheet.create({
  rail: {
    position: "absolute",
    right: 12,
    top: 112,
    zIndex: 20,
    gap: 10,
  },
  btnTarget: { width: 44, height: 44, minWidth: 44, minHeight: 44 },
  btn: {
    width: 44,
    height: 44,
    borderRadius: 22,
    alignItems: "center",
    justifyContent: "center",
  },
  btnFill: {
    backgroundColor: "rgba(255,255,255,0.9)",
    shadowColor: "#000",
    shadowOpacity: 0.12,
    shadowRadius: 6,
    shadowOffset: { width: 0, height: 2 },
    elevation: 3,
    borderWidth: 1,
    borderColor: "rgba(10,20,40,0.06)",
  },
  btnActive: {
    backgroundColor: cumulus.accent,
    borderColor: cumulus.accent,
  },
  btnPressed: { transform: [{ scale: 0.96 }], opacity: 0.82 },
  btnDisabled: { opacity: 0.62 },

  scrim: {
    position: "absolute",
    left: 0,
    right: 0,
    top: 0,
    bottom: 0,
    zIndex: 24,
  },
  panel: {
    position: "absolute",
    right: 68,
    top: 104,
    width: 230,
    borderRadius: 18,
    paddingVertical: 10,
    paddingHorizontal: 6,
    zIndex: 25,
    shadowColor: "#000",
    shadowOpacity: 0.18,
    shadowRadius: 14,
    shadowOffset: { width: 0, height: 6 },
    elevation: 8,
    borderWidth: 1,
    borderColor: "rgba(10,20,40,0.06)",
  },
  panelRow: {
    flexDirection: "row",
    alignItems: "center",
    paddingVertical: 10,
    paddingHorizontal: 6,
    gap: 10,
    minHeight: 44,
  },
  panelRowPressed: { backgroundColor: "rgba(255,255,255,0.28)", borderRadius: 12 },
  checkCol: { width: 16, alignItems: "center" },
  iconCol: { width: 22, alignItems: "center" },
  panelRowTitle: { color: "#1a2030", fontSize: 16, fontWeight: "400", flex: 1 },
  panelRowTitleActive: { fontWeight: "600" },
  panelHint: {
    color: "rgba(10,20,40,0.55)",
    fontSize: 11,
    paddingHorizontal: 12,
    paddingTop: 4,
    paddingBottom: 6,
  },
});
