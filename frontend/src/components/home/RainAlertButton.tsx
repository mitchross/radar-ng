import { useMemo } from "react";
import { Linking, Pressable, StyleSheet, Text } from "react-native";
import { useRainAlertOptIn } from "../../hooks/useRainAlertOptIn";
import { tapHaptic } from "../../lib/haptics";
import { useWeatherClearTheme } from "../../theme/WeatherClearThemeProvider";
import type { WeatherClearTheme } from "../../theme/weatherClearTheme";

/**
 * "Notify me" for rain alerts, shown wherever rain is announced. Enables the
 * existing Settings preference through its permission-checked path and
 * reflects the enabled state in place.
 */
export function RainAlertButton() {
  const { theme } = useWeatherClearTheme();
  const styles = useMemo(() => createStyles(theme), [theme]);
  const { enabled, blocked, busy, setRainAlerts } = useRainAlertOptIn();

  const label = enabled ? "Alerts on" : blocked ? "Allow notifications" : "Notify me";
  const onPress = enabled
    ? undefined
    : blocked
      ? () => void Linking.openSettings()
      : () => { tapHaptic(); void setRainAlerts(true); };
  return (
    <Pressable
      accessibilityRole="button"
      accessibilityLabel={enabled ? "Rain alerts are on" : blocked ? "Allow notifications in system settings" : "Notify me when rain starts"}
      accessibilityState={{ selected: enabled, disabled: enabled || busy }}
      disabled={enabled || busy}
      onPress={onPress}
      style={({ pressed }) => [styles.pill, enabled ? styles.pillOn : null, pressed ? styles.pressed : null]}
    >
      <Text style={[styles.text, enabled ? styles.textOn : null]}>{enabled ? "✓ " : ""}{label}</Text>
    </Pressable>
  );
}

function createStyles(theme: WeatherClearTheme) {
  return StyleSheet.create({
    pill: {
      minHeight: 36,
      paddingHorizontal: 12,
      borderRadius: 18,
      justifyContent: "center",
      backgroundColor: theme.colors.accent,
    },
    pillOn: { backgroundColor: theme.colors.surfaceMuted },
    pressed: { opacity: 0.7 },
    text: { color: "#ffffff", fontSize: 13, fontFamily: theme.typography.uiSemibold },
    textOn: { color: theme.colors.textSecondary },
  });
}
