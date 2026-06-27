/**
 * Three-swatch palette picker — Classic (NWS), Vivid (CARROT-style),
 * Muted (viridis, colorblind-safe). Lives in Settings.
 */
import { useMemo } from "react";
import { View, Text, StyleSheet, Pressable } from "react-native";
import { LinearGradient } from "expo-linear-gradient";
import { useWeatherStore } from "../../stores/useWeatherStore";
import { useWeatherClearTheme } from "../../theme/WeatherClearThemeProvider";
import type { WeatherClearTheme } from "../../theme/weatherClearTheme";
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
  const { theme } = useWeatherClearTheme();
  const styles = useMemo(() => createStyles(theme), [theme]);
  const activePalette = useWeatherStore((s) => s.activePalette);
  const setActivePalette = useWeatherStore((s) => s.setActivePalette);

  return (
    <View style={styles.row}>
      {PALETTES.map((p) => {
        const active = activePalette === p.id;
        return (
          <Pressable
            key={p.id}
            accessibilityRole="radio"
            accessibilityLabel={`${p.label} radar palette, ${p.description}`}
            accessibilityState={{ selected: active }}
            onPress={() => setActivePalette(p.id)}
            style={[styles.tile, active ? styles.tileActive : null]}
          >
            <LinearGradient
              colors={p.swatch}
              start={{ x: 0, y: 0 }}
              end={{ x: 1, y: 0 }}
              style={styles.swatch}
            />
            <Text style={[styles.label, active ? styles.labelActive : null]}>{p.label}</Text>
            <Text style={styles.description}>{p.description}</Text>
          </Pressable>
        );
      })}
    </View>
  );
}

function createStyles(theme: WeatherClearTheme) {
  return StyleSheet.create({
  row: { flexDirection: "row", gap: 10 },
  tile: {
    flex: 1,
    minHeight: 64,
    padding: 10,
    borderRadius: 14,
    backgroundColor: theme.colors.surfaceStrong,
    borderWidth: 2,
    borderColor: "transparent",
    alignItems: "center",
  },
  tileActive: {
    borderColor: theme.colors.accent,
    backgroundColor: theme.colors.accentSoft,
  },
  swatch: {
    width: "100%",
    height: 14,
    borderRadius: 7,
    marginBottom: 8,
  },
  label: {
    color: theme.colors.text,
    fontSize: 13,
    fontFamily: theme.typography.uiSemibold,
  },
  labelActive: { color: theme.colors.accent },
  description: {
    color: theme.colors.textMuted,
    fontSize: 10,
    fontFamily: theme.typography.ui,
    marginTop: 2,
  },
  });
}
