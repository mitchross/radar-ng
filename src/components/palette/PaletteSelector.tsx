/**
 * Three-swatch palette picker — Classic (NWS), Vivid (CARROT-style),
 * Muted (viridis, colorblind-safe). Lives in Settings.
 */
import { View, Text, StyleSheet, TouchableOpacity } from "react-native";
import { LinearGradient } from "expo-linear-gradient";
import { useWeatherStore } from "../../stores/useWeatherStore";
import { cumulus } from "../../lib/cumulusTheme";
import type { Palette } from "../../types/weather";

const PALETTES: {
  id: Palette;
  label: string;
  description: string;
  swatch: [string, string, string, string];
}[] = [
  {
    id: "classic",
    label: "Classic",
    description: "NWS standard",
    swatch: ["#00c800", "#ffc800", "#ff3200", "#ff00ff"],
  },
  {
    id: "vivid",
    label: "Vivid",
    description: "High-contrast",
    swatch: ["#50c8ff", "#7aff50", "#ff64aa", "#8c28ff"],
  },
  {
    id: "muted",
    label: "Muted",
    description: "Colorblind-safe",
    swatch: ["#440154", "#3b528b", "#21918c", "#fde724"],
  },
];

export function PaletteSelector() {
  const activePalette = useWeatherStore((s) => s.activePalette);
  const setActivePalette = useWeatherStore((s) => s.setActivePalette);

  return (
    <View style={styles.row}>
      {PALETTES.map((p) => {
        const active = activePalette === p.id;
        return (
          <TouchableOpacity
            key={p.id}
            onPress={() => setActivePalette(p.id)}
            style={[styles.tile, active && styles.tileActive]}
            activeOpacity={0.85}
          >
            <LinearGradient
              colors={p.swatch}
              start={{ x: 0, y: 0 }}
              end={{ x: 1, y: 0 }}
              style={styles.swatch}
            />
            <Text style={[styles.label, active && styles.labelActive]}>{p.label}</Text>
            <Text style={styles.description}>{p.description}</Text>
          </TouchableOpacity>
        );
      })}
    </View>
  );
}

const styles = StyleSheet.create({
  row: { flexDirection: "row", gap: 10 },
  tile: {
    flex: 1,
    padding: 10,
    borderRadius: 14,
    backgroundColor: "rgba(255,255,255,0.06)",
    borderWidth: 2,
    borderColor: "transparent",
    alignItems: "center",
  },
  tileActive: {
    borderColor: cumulus.accent,
    backgroundColor: "rgba(139,124,255,0.14)",
  },
  swatch: {
    width: "100%",
    height: 14,
    borderRadius: 7,
    marginBottom: 8,
  },
  label: { color: cumulus.ink, fontSize: 13, fontWeight: "600" },
  labelActive: { color: cumulus.accentBright },
  description: { color: cumulus.inkMuted, fontSize: 10, marginTop: 2 },
});
