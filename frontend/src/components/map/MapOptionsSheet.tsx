/**
 * One sheet for everything that changes what the map shows: the weather
 * layer, the base map style and the storm/lightning overlays. Replaces the
 * layer popover and the separate style sheet, so the map keeps two buttons.
 */
import { BottomSheet, RNHostView } from "@expo/ui";
import { useMemo } from "react";
import { View, Text, StyleSheet, Pressable, Switch, useWindowDimensions } from "react-native";
import { SymbolView } from "expo-symbols";
import { useWeatherStore } from "../../stores/useWeatherStore";
import { useWeatherClearTheme } from "../../theme/WeatherClearThemeProvider";
import type { WeatherClearTheme } from "../../theme/weatherClearTheme";
import type { MapStyle } from "../../types/weather";
import { LAYER_OPTIONS, LAYER_SYMBOLS } from "./radarLayers";

const MAP_STYLES: { id: MapStyle; label: string; color: string }[] = [
  { id: "light", label: "Light", color: "#e8ece5" },
  { id: "dark", label: "Dark", color: "#263342" },
  { id: "satellite", label: "Satellite", color: "#385346" },
];

export function MapOptionsSheet({ visible, onClose }: { visible: boolean; onClose: () => void }) {
  const { width } = useWindowDimensions();
  const { theme } = useWeatherClearTheme();
  const styles = useMemo(() => createStyles(theme), [theme]);
  const activeLayer = useWeatherStore((s) => s.activeLayer);
  const setActiveLayer = useWeatherStore((s) => s.setActiveLayer);
  const mapStyle = useWeatherStore((s) => s.mapStyle);
  const setMapStyle = useWeatherStore((s) => s.setMapStyle);
  const extrasVisible = useWeatherStore((s) => s.extrasVisible);
  const toggleExtras = useWeatherStore((s) => s.toggleExtras);
  // Match phone sheet insets and keep the iPad form sheet compact.
  const contentWidth = Math.min(500, width - 32);
  const tileWidth = (contentWidth - 2 * 10) / 3;

  return (
    <BottomSheet isPresented={visible} onDismiss={onClose} containerColor={theme.colors.surface}>
      <RNHostView matchContents>
        <View style={[styles.content, { width: contentWidth }]}>
          <View style={styles.header}>
            <Text accessibilityRole="header" style={styles.title}>Map</Text>
            <Pressable accessibilityRole="button" accessibilityLabel="Close map options" onPress={onClose} style={styles.done}>
              <Text style={styles.doneText}>Done</Text>
            </Pressable>
          </View>

          <Text style={styles.section}>WEATHER LAYER</Text>
          <View style={styles.grid}>
            {LAYER_OPTIONS.map((opt) => {
              const active = activeLayer === opt.id;
              return (
                <Pressable
                  key={opt.id}
                  accessibilityRole="radio"
                  accessibilityLabel={`${opt.name} radar layer`}
                  accessibilityState={{ checked: active }}
                  onPress={() => { setActiveLayer(opt.id); onClose(); }}
                  style={({ pressed }) => [
                    styles.layerTile,
                    { width: tileWidth },
                    active ? styles.layerTileActive : null,
                    pressed ? styles.pressed : null,
                  ]}
                >
                  <SymbolView
                    name={LAYER_SYMBOLS[opt.icon]}
                    size={24}
                    tintColor={active ? theme.colors.accent : theme.colors.text}
                    type="hierarchical"
                  />
                  <Text style={[styles.layerName, active ? { color: theme.colors.accent } : null]} numberOfLines={1}>
                    {opt.name}
                  </Text>
                </Pressable>
              );
            })}
          </View>

          <Text style={styles.section}>MAP STYLE</Text>
          <View style={styles.grid}>
            {MAP_STYLES.map((opt) => {
              const active = mapStyle === opt.id;
              return (
                <Pressable
                  key={opt.id}
                  accessibilityRole="radio"
                  accessibilityLabel={`${opt.label} map style`}
                  accessibilityState={{ checked: active }}
                  onPress={() => setMapStyle(opt.id)}
                  style={({ pressed }) => [styles.styleTile, { width: tileWidth }, pressed ? styles.pressed : null]}
                >
                  <View style={[styles.swatch, { backgroundColor: opt.color }, active ? styles.swatchActive : null]} />
                  <Text style={[styles.layerName, active ? { color: theme.colors.accent } : null]}>{opt.label}</Text>
                </Pressable>
              );
            })}
          </View>

          <View style={styles.switchRow}>
            <SymbolView name={{ ios: "cloud.bolt.fill", android: "thunderstorm" }} size={22} tintColor={theme.colors.text} type="hierarchical" />
            <View style={{ flex: 1 }}>
              <Text style={styles.switchTitle}>Storms & lightning</Text>
              <Text style={styles.switchDetail}>Warnings, storm cells and strikes</Text>
            </View>
            <Switch
              value={extrasVisible}
              onValueChange={toggleExtras}
              trackColor={{ true: theme.colors.accent }}
              style={styles.switch}
              accessibilityLabel="Toggle storm and lightning overlays"
            />
          </View>

          <Text style={styles.tip}>Tip: press and hold the map to read the value at a point.</Text>
        </View>
      </RNHostView>
    </BottomSheet>
  );
}

function createStyles(theme: WeatherClearTheme) {
  return StyleSheet.create({
    content: { paddingBottom: theme.spacing.xl },
    header: { flexDirection: "row", alignItems: "center", justifyContent: "space-between" },
    title: { color: theme.colors.text, fontFamily: theme.typography.uiSemibold, fontSize: 22 },
    done: { minHeight: 44, minWidth: 60, alignItems: "center", justifyContent: "center" },
    doneText: { color: theme.colors.accent, fontFamily: theme.typography.uiSemibold, fontSize: 17 },
    section: {
      color: theme.colors.textMuted,
      fontFamily: theme.typography.uiSemibold,
      fontSize: 11,
      letterSpacing: 0.8,
      marginTop: theme.spacing.md,
      marginBottom: theme.spacing.sm,
    },
    grid: { flexDirection: "row", flexWrap: "wrap", gap: 10 },
    layerTile: {
      minHeight: 72,
      borderRadius: 14,
      borderCurve: "continuous",
      backgroundColor: theme.colors.surfaceStrong,
      borderWidth: 1.5,
      borderColor: theme.colors.border,
      alignItems: "center",
      justifyContent: "center",
      gap: 6,
      paddingHorizontal: 4,
    },
    layerTileActive: { borderColor: theme.colors.accent, backgroundColor: theme.colors.accentSoft },
    layerName: { color: theme.colors.text, fontFamily: theme.typography.uiMedium, fontSize: 13 },
    styleTile: { alignItems: "center", gap: 6, minHeight: 44 },
    swatch: {
      width: "100%",
      height: 48,
      borderRadius: 12,
      borderCurve: "continuous",
      borderWidth: 1.5,
      borderColor: theme.colors.border,
    },
    swatchActive: { borderColor: theme.colors.accent, borderWidth: 2.5 },
    pressed: { opacity: 0.7 },
    switchRow: {
      flexDirection: "row",
      alignItems: "center",
      gap: 12,
      minHeight: 60,
      marginTop: theme.spacing.lg,
      paddingHorizontal: 12,
      borderRadius: 14,
      backgroundColor: theme.colors.surfaceStrong,
    },
    switch: { alignSelf: "center" },
    switchTitle: { color: theme.colors.text, fontFamily: theme.typography.uiMedium, fontSize: 16 },
    switchDetail: { color: theme.colors.textSecondary, fontFamily: theme.typography.ui, fontSize: 12, marginTop: 2 },
    tip: {
      color: theme.colors.textMuted,
      fontFamily: theme.typography.ui,
      fontSize: 12,
      textAlign: "center",
      marginTop: theme.spacing.md,
    },
  });
}
