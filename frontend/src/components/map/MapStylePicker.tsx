/**
 * Apple-Weather-style map theme picker: 3 style tiles (light / dark /
 * satellite) with a flat-vs-globe projection toggle above. Reads/writes
 * mapStyle + mapProjection in the Zustand store.
 *
 * Note: MapLibre React Native does not expose the native globe projection
 * prop here yet. The preference persists for forward compatibility.
 */
import { useMemo } from "react";
import { View, Text, StyleSheet, Pressable } from "react-native";
import { LinearGradient } from "expo-linear-gradient";
import { useWeatherStore } from "../../stores/useWeatherStore";
import { useWeatherClearTheme } from "../../theme/WeatherClearThemeProvider";
import type { WeatherClearTheme } from "../../theme/weatherClearTheme";
import type { MapStyle, MapProjection } from "../../types/weather";

interface Props {
  visible: boolean;
  onClose: () => void;
}

const STYLE_TILES: { id: MapStyle; label: string; gradient: [string, string] }[] = [
  { id: "light", label: "Light", gradient: ["#F3F6FB", "#D4DEEC"] },
  { id: "dark", label: "Dark", gradient: ["#2A3142", "#0E1320"] },
  { id: "satellite", label: "Satellite", gradient: ["#1d3b5c", "#25603f"] },
];

const PROJ_TOGGLES: { id: MapProjection; label: string }[] = [
  { id: "flat", label: "Flat" },
  { id: "globe", label: "Globe" },
];

export function MapStylePicker({ visible, onClose }: Props) {
  const mapStyle = useWeatherStore((s) => s.mapStyle);
  const setMapStyle = useWeatherStore((s) => s.setMapStyle);
  const mapProjection = useWeatherStore((s) => s.mapProjection);
  const setMapProjection = useWeatherStore((s) => s.setMapProjection);
  const { theme } = useWeatherClearTheme();
  const styles = useMemo(() => createStyles(theme), [theme]);

  if (!visible) return null;

  return (
    <>
      <Pressable
        style={styles.scrim}
        onPress={onClose}
        accessibilityRole="button"
        accessibilityLabel="Close map style picker"
      />
      <View style={styles.card} accessibilityRole="summary">
        <Text style={styles.title}>Map style</Text>

        <View style={styles.projRow}>
          {PROJ_TOGGLES.map((p) => {
            const active = mapProjection === p.id;
            return (
              <Pressable
                key={p.id}
                onPress={() => setMapProjection(p.id)}
                style={({ pressed }) => [
                  styles.projBtn,
                  active ? styles.projBtnActive : null,
                  pressed ? styles.controlPressed : null,
                ]}
                accessibilityRole="radio"
                accessibilityLabel={`${p.label} map projection`}
                accessibilityState={{ checked: active }}
              >
                <Text style={[styles.projLabel, active ? styles.projLabelActive : null]}>
                  {p.label}
                </Text>
              </Pressable>
            );
          })}
        </View>

        <View style={styles.tileRow}>
          {STYLE_TILES.map((t) => {
            const active = mapStyle === t.id;
            return (
              <Pressable
                key={t.id}
                onPress={() => {
                  setMapStyle(t.id);
                  onClose();
                }}
                style={({ pressed }) => [
                  styles.tileCol,
                  pressed ? styles.controlPressed : null,
                ]}
                accessibilityRole="radio"
                accessibilityLabel={`${t.label} map style`}
                accessibilityState={{ checked: active }}
              >
                <LinearGradient
                  colors={t.gradient}
                  style={[styles.tile, active ? styles.tileActive : null]}
                  start={{ x: 0, y: 0 }}
                  end={{ x: 1, y: 1 }}
                >
                  {active ? <View style={styles.activeDot} /> : null}
                </LinearGradient>
                <Text style={[styles.tileLabel, active ? styles.tileLabelActive : null]}>
                  {t.label}
                </Text>
              </Pressable>
            );
          })}
        </View>

        {mapProjection === "globe" ? (
          <Text style={styles.note}>
            Globe projection renders flat until MapLibre RN exposes the native toggle.
          </Text>
        ) : null}
      </View>
    </>
  );
}

function createStyles(theme: WeatherClearTheme) {
  return StyleSheet.create({
  scrim: {
    position: "absolute",
    left: 0,
    right: 0,
    top: 0,
    bottom: 0,
    zIndex: 30,
    backgroundColor: theme.colors.scrim,
  },
  card: {
    position: "absolute",
    left: 16,
    right: 16,
    bottom: 220,
    zIndex: 31,
    backgroundColor: theme.colors.surface,
    borderRadius: 22,
    padding: 16,
    borderWidth: 1,
    borderColor: theme.colors.border,
    shadowColor: "#000",
    shadowOpacity: 0.3,
    shadowRadius: 16,
    shadowOffset: { width: 0, height: 8 },
    elevation: 10,
  },
  title: {
    color: theme.colors.text,
    fontFamily: theme.typography.uiBold,
    fontSize: 15,
    fontWeight: "700",
    marginBottom: 12,
  },
  projRow: {
    flexDirection: "row",
    backgroundColor: theme.colors.surfaceMuted,
    borderRadius: 10,
    padding: 3,
    marginBottom: 14,
  },
  projBtn: {
    flex: 1,
    paddingVertical: 7,
    alignItems: "center",
    justifyContent: "center",
    borderRadius: 8,
    minHeight: 44,
  },
  projBtnActive: {
    backgroundColor: theme.colors.accent,
  },
  projLabel: {
    color: theme.colors.textSecondary,
    fontSize: 12,
    fontFamily: theme.typography.uiSemibold,
  },
  projLabelActive: { color: "#fff" },

  tileRow: { flexDirection: "row", gap: 10 },
  tileCol: { flex: 1, minHeight: 44, alignItems: "center" },
  tile: {
    width: "100%",
    aspectRatio: 1,
    borderRadius: 14,
    borderWidth: 2,
    borderColor: "transparent",
    alignItems: "center",
    justifyContent: "center",
  },
  tileActive: {
    borderColor: theme.colors.accent,
  },
  activeDot: {
    width: 10,
    height: 10,
    borderRadius: 5,
    backgroundColor: theme.colors.accent,
    borderWidth: 2,
    borderColor: "#fff",
  },
  tileLabel: {
    color: theme.colors.textSecondary,
    fontSize: 12,
    marginTop: 6,
    fontWeight: "500",
    fontFamily: theme.typography.uiMedium,
  },
  tileLabelActive: { color: theme.colors.text, fontWeight: "700" },

  note: {
    color: theme.colors.textMuted,
    fontSize: 10,
    lineHeight: 14,
    marginTop: 10,
    textAlign: "center",
  },
  controlPressed: { opacity: 0.7 },
  });
}
