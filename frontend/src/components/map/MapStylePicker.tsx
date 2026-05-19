/**
 * Apple-Weather-style map theme picker: 3 style tiles (light / dark /
 * satellite) with a flat-vs-globe projection toggle above. Reads/writes
 * mapStyle + mapProjection in the Zustand store.
 *
 * Note: MapLibre React Native does not expose the native globe projection
 * prop here yet. The preference persists for forward compatibility.
 */
import { View, Text, StyleSheet, Pressable, TouchableOpacity } from "react-native";
import { LinearGradient } from "expo-linear-gradient";
import { useWeatherStore } from "../../stores/useWeatherStore";
import { cumulus } from "../../lib/cumulusTheme";
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

  if (!visible) return null;

  return (
    <>
      <Pressable style={styles.scrim} onPress={onClose} />
      <View style={styles.card}>
        <Text style={styles.title}>Map Theme</Text>

        <View style={styles.projRow}>
          {PROJ_TOGGLES.map((p) => {
            const active = mapProjection === p.id;
            return (
              <TouchableOpacity
                key={p.id}
                onPress={() => setMapProjection(p.id)}
                style={[styles.projBtn, active && styles.projBtnActive]}
                activeOpacity={0.8}
              >
                <Text style={[styles.projLabel, active && styles.projLabelActive]}>
                  {p.label}
                </Text>
              </TouchableOpacity>
            );
          })}
        </View>

        <View style={styles.tileRow}>
          {STYLE_TILES.map((t) => {
            const active = mapStyle === t.id;
            return (
              <TouchableOpacity
                key={t.id}
                onPress={() => {
                  setMapStyle(t.id);
                  onClose();
                }}
                activeOpacity={0.85}
                style={styles.tileCol}
              >
                <LinearGradient
                  colors={t.gradient}
                  style={[styles.tile, active && styles.tileActive]}
                  start={{ x: 0, y: 0 }}
                  end={{ x: 1, y: 1 }}
                >
                  {active && <View style={styles.activeDot} />}
                </LinearGradient>
                <Text style={[styles.tileLabel, active && styles.tileLabelActive]}>
                  {t.label}
                </Text>
              </TouchableOpacity>
            );
          })}
        </View>

        {mapProjection === "globe" && (
          <Text style={styles.note}>
            Globe projection renders flat until MapLibre RN exposes the native toggle.
          </Text>
        )}
      </View>
    </>
  );
}

const styles = StyleSheet.create({
  scrim: {
    position: "absolute",
    left: 0,
    right: 0,
    top: 0,
    bottom: 0,
    zIndex: 30,
    backgroundColor: "rgba(0,0,0,0.15)",
  },
  card: {
    position: "absolute",
    left: 16,
    right: 16,
    bottom: 220,
    zIndex: 31,
    backgroundColor: "rgba(20,26,44,0.95)",
    borderRadius: 22,
    padding: 16,
    borderWidth: 1,
    borderColor: "rgba(255,255,255,0.1)",
    shadowColor: "#000",
    shadowOpacity: 0.3,
    shadowRadius: 16,
    shadowOffset: { width: 0, height: 8 },
    elevation: 10,
  },
  title: {
    color: cumulus.ink,
    fontSize: 15,
    fontWeight: "700",
    marginBottom: 12,
  },
  projRow: {
    flexDirection: "row",
    backgroundColor: "rgba(255,255,255,0.08)",
    borderRadius: 10,
    padding: 3,
    marginBottom: 14,
  },
  projBtn: {
    flex: 1,
    paddingVertical: 7,
    alignItems: "center",
    borderRadius: 8,
  },
  projBtnActive: {
    backgroundColor: cumulus.accent,
  },
  projLabel: { color: cumulus.inkDim, fontSize: 12, fontWeight: "600" },
  projLabelActive: { color: "#fff" },

  tileRow: { flexDirection: "row", gap: 10 },
  tileCol: { flex: 1, alignItems: "center" },
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
    borderColor: cumulus.accent,
  },
  activeDot: {
    width: 10,
    height: 10,
    borderRadius: 5,
    backgroundColor: cumulus.accent,
    borderWidth: 2,
    borderColor: "#fff",
  },
  tileLabel: {
    color: cumulus.inkDim,
    fontSize: 12,
    marginTop: 6,
    fontWeight: "500",
  },
  tileLabelActive: { color: cumulus.ink, fontWeight: "700" },

  note: {
    color: cumulus.inkMuted,
    fontSize: 10,
    lineHeight: 14,
    marginTop: 10,
    textAlign: "center",
  },
});
