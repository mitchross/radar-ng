import { useMemo } from "react";
import { Linking, Pressable, StyleSheet, Text, View } from "react-native";
import { useWeatherClearTheme } from "../../theme/WeatherClearThemeProvider";
import type { WeatherClearTheme } from "../../theme/weatherClearTheme";

export type LocationPermissionKind = "undetermined" | "denied";

interface Props {
  kind: LocationPermissionKind;
  /** The city shown meanwhile, so the card says whose weather this is. */
  fallbackName: string;
  onAllow: () => void;
  onChooseCity: () => void;
  onOpenSettings?: () => void;
}

const COPY: Record<LocationPermissionKind, { title: string; body: (city: string) => string; primary: string }> = {
  undetermined: {
    title: "Weather for where you are?",
    body: (city) => `Allow location and the forecast, radar and rain alerts follow you. Until then this is ${city}.`,
    primary: "Allow location",
  },
  denied: {
    title: "Location is off",
    body: (city) => `Showing ${city}. Turn location on in Settings, or pick a city to follow instead.`,
    primary: "Open Settings",
  },
};

/** Pre-prompt before the OS location dialog, and the recovery card after a denial. */
export function LocationPermissionCard({ kind, fallbackName, onAllow, onChooseCity, onOpenSettings }: Props) {
  const { theme } = useWeatherClearTheme();
  const styles = useMemo(() => createStyles(theme), [theme]);
  const copy = COPY[kind];
  const onPrimary = kind === "denied" ? (onOpenSettings ?? (() => void Linking.openSettings())) : onAllow;
  return (
    <View accessible={false} style={styles.card} testID="location-permission-card">
      <Text accessibilityRole="header" style={styles.title}>{copy.title}</Text>
      <Text style={styles.body}>{copy.body(fallbackName)}</Text>
      <View style={styles.actions}>
        <Pressable
          accessibilityRole="button"
          accessibilityLabel={copy.primary}
          onPress={onPrimary}
          style={({ pressed }) => [styles.button, styles.primary, pressed ? styles.pressed : null]}
        >
          <Text style={styles.primaryText}>{copy.primary}</Text>
        </Pressable>
        <Pressable
          accessibilityRole="button"
          accessibilityLabel="Choose a city"
          onPress={onChooseCity}
          style={({ pressed }) => [styles.button, styles.secondary, pressed ? styles.pressed : null]}
        >
          <Text style={styles.secondaryText}>Choose a city</Text>
        </Pressable>
      </View>
    </View>
  );
}

function createStyles(theme: WeatherClearTheme) {
  return StyleSheet.create({
    card: {
      marginHorizontal: 16,
      marginTop: 12,
      padding: 16,
      borderRadius: 20,
      borderCurve: "continuous",
      borderWidth: 1,
      borderColor: theme.colors.border,
      backgroundColor: theme.colors.surface,
      gap: 6,
    },
    title: { color: theme.colors.text, fontSize: 16, fontFamily: theme.typography.uiSemibold },
    body: { color: theme.colors.textSecondary, fontSize: 13, lineHeight: 18, fontFamily: theme.typography.ui },
    actions: { flexDirection: "row", gap: 10, marginTop: 8, flexWrap: "wrap" },
    button: { minHeight: 44, paddingHorizontal: 16, borderRadius: 22, justifyContent: "center" },
    primary: { backgroundColor: theme.colors.accent },
    secondary: { backgroundColor: theme.colors.surfaceMuted },
    pressed: { opacity: 0.7 },
    primaryText: { color: "#ffffff", fontSize: 14, fontFamily: theme.typography.uiSemibold },
    secondaryText: { color: theme.colors.text, fontSize: 14, fontFamily: theme.typography.uiSemibold },
  });
}
