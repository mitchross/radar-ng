import { BottomSheet, RNHostView } from "@expo/ui";
import { useMemo } from "react";
import { View, Text, StyleSheet, Pressable, useWindowDimensions } from "react-native";
import { useWeatherStore } from "../../stores/useWeatherStore";
import { useWeatherClearTheme } from "../../theme/WeatherClearThemeProvider";
import type { WeatherClearTheme } from "../../theme/weatherClearTheme";
import type { MapStyle } from "../../types/weather";

const STYLES: { id: MapStyle; label: string; detail: string; color: string }[] = [
  { id: "light", label: "Light", detail: "Roads and place names", color: "#e8ece5" },
  { id: "dark", label: "Dark", detail: "Low-light viewing", color: "#263342" },
  { id: "satellite", label: "Satellite", detail: "Aerial imagery", color: "#385346" },
];

export function MapStylePicker({ visible, onClose }: { visible: boolean; onClose: () => void }) {
  const { width } = useWindowDimensions();
  const mapStyle = useWeatherStore((s) => s.mapStyle);
  const setMapStyle = useWeatherStore((s) => s.setMapStyle);
  const { theme } = useWeatherClearTheme();
  const styles = useMemo(() => createStyles(theme), [theme]);

  return (
      <BottomSheet isPresented={visible} onDismiss={onClose} containerColor={theme.colors.surface}>
        <RNHostView matchContents>
        <View style={[styles.content, { width: width - 64 }]}>
          <View style={styles.header}>
            <Text accessibilityRole="header" style={styles.title}>Map style</Text>
            <Pressable accessibilityRole="button" accessibilityLabel="Close map style picker" onPress={onClose} style={styles.done}>
              <Text style={styles.doneText}>Done</Text>
            </Pressable>
          </View>
          {STYLES.map((option) => (
            <Pressable
              key={option.id}
              accessibilityRole="radio"
              accessibilityLabel={`${option.label} map style`}
              accessibilityState={{ checked: mapStyle === option.id }}
              onPress={() => { setMapStyle(option.id); onClose(); }}
              style={({ pressed }) => [styles.row, pressed && { backgroundColor: theme.colors.surfaceMuted }]}
            >
              <View style={[styles.swatch, { backgroundColor: option.color }]} />
              <View style={styles.copy}>
                <Text style={styles.label}>{option.label}</Text>
                <Text style={styles.detail}>{option.detail}</Text>
              </View>
              {mapStyle === option.id ? <Text style={styles.check}>✓</Text> : null}
            </Pressable>
          ))}
        </View>
        </RNHostView>
      </BottomSheet>
  );
}

function createStyles(theme: WeatherClearTheme) {
  return StyleSheet.create({
    content: { paddingBottom: theme.spacing.xl },
    header: { flexDirection: "row", alignItems: "center", justifyContent: "space-between", marginBottom: theme.spacing.sm },
    title: { color: theme.colors.text, fontFamily: theme.typography.uiSemibold, fontSize: 22 },
    done: { minHeight: 44, minWidth: 60, alignItems: "center", justifyContent: "center" },
    doneText: { color: theme.colors.accent, fontFamily: theme.typography.uiSemibold, fontSize: 17 },
    row: { flexDirection: "row", alignItems: "center", gap: theme.spacing.lg, minHeight: 72, borderRadius: theme.radii.md, paddingHorizontal: theme.spacing.md },
    swatch: { width: 42, height: 42, borderRadius: theme.radii.md, borderCurve: "continuous", borderWidth: StyleSheet.hairlineWidth, borderColor: theme.colors.border },
    copy: { flex: 1, gap: 4 },
    label: { color: theme.colors.text, fontFamily: theme.typography.uiMedium, fontSize: 17 },
    detail: { color: theme.colors.textSecondary, fontFamily: theme.typography.ui, fontSize: 13 },
    check: { color: theme.colors.accent, fontSize: 22, fontWeight: "600" },
  });
}
