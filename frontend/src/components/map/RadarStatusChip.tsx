import { StyleSheet, Text, View, type StyleProp, type ViewStyle } from "react-native";
import type { RadarStatus } from "../../lib/radarStatus";
import { useWeatherClearTheme } from "../../theme/WeatherClearThemeProvider";

/** The LIVE / UPDATED 12M AGO / OFFLINE freshness badge, shared by every radar surface. */
export function RadarStatusChip({ status, style }: { status: RadarStatus; style?: StyleProp<ViewStyle> }) {
  const { theme } = useWeatherClearTheme();
  const dotColor = {
    live: theme.colors.success,
    stale: theme.colors.warning,
    offline: theme.colors.destructive,
    unavailable: theme.colors.textFaint,
  }[status.tone];
  return (
    <View
      accessible
      accessibilityRole={status.tone === "live" ? undefined : "alert"}
      accessibilityLabel={status.accessibilityLabel}
      testID="radar-status-chip"
      style={[styles.badge, style]}
    >
      <View style={[styles.dot, { backgroundColor: dotColor }]} />
      <Text style={[styles.text, { fontFamily: theme.typography.uiBold }]}>{status.label}</Text>
    </View>
  );
}

const styles = StyleSheet.create({
  badge: {
    flexDirection: "row",
    alignItems: "center",
    gap: 5,
    // A near-opaque scrim keeps the white status readable over any map tile.
    backgroundColor: "rgba(0,0,0,0.88)",
    paddingHorizontal: 8,
    paddingVertical: 3,
    borderRadius: 8,
  },
  dot: { width: 6, height: 6, borderRadius: 3 },
  text: { color: "#ffffff", fontSize: 10, letterSpacing: 1.4 },
});
