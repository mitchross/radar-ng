/**
 * "Heavy storm arriving in ~20 min". Shown only when a tracked storm cell's
 * next-hour path crosses the active place (same rule as the web panel).
 */
import React, { useMemo } from "react";
import { StyleSheet, Text, View } from "react-native";
import { useIsFocused } from "expo-router";
import { useStormCells } from "../../hooks/useStormCells";
import { useWeatherStore } from "../../stores/useWeatherStore";
import { approachCopy, stormApproaches } from "../../../../web/src/shared/stormCone";
import { describeIncoming, incomingStorm, parseStormCells } from "../../lib/stormTracks";
import { useWeatherClearTheme } from "../../theme/WeatherClearThemeProvider";
import type { WeatherClearTheme } from "../../theme/weatherClearTheme";

export function IncomingStormCard() {
  const { theme } = useWeatherClearTheme();
  const styles = useMemo(() => createStyles(theme), [theme]);
  const focused = useIsFocused();
  const latitude = useWeatherStore((s) => s.latitude);
  const longitude = useWeatherStore((s) => s.longitude);
  const { data } = useStormCells(focused && latitude != null && longitude != null);
  const threat = useMemo(
    () => (data && latitude != null && longitude != null ? incomingStorm(parseStormCells(data), [longitude, latitude]) : null),
    [data, latitude, longitude],
  );
  const stormNoticeEnabled = useWeatherStore((s) => s.stormNoticeEnabled);
  const approach = useMemo(
    () => (stormNoticeEnabled && data && latitude != null && longitude != null ? stormApproaches(parseStormCells(data), [longitude, latitude])[0] : null),
    [stormNoticeEnabled, data, latitude, longitude],
  );
  if (!threat) return null;
  const { headline, sub, severe } = describeIncoming(threat);
  const notice = approach ? approachCopy(approach) : null;
  return (
    <View style={[styles.card, severe ? styles.severe : null]} accessibilityRole="alert" accessibilityLabel={`${headline}. ${sub}${notice ? `. ${notice.body}` : ""}`}>
      <Text style={[styles.headline, severe ? styles.severeText : null]}>{headline}</Text>
      <Text style={styles.sub}>{sub}</Text>
      {notice ? <Text style={styles.sub}>{notice.body}</Text> : null}
    </View>
  );
}

const createStyles = (theme: WeatherClearTheme) =>
  StyleSheet.create({
    card: {
      marginHorizontal: 16,
      marginTop: 12,
      padding: 14,
      borderRadius: 20,
      backgroundColor: "rgba(52,120,246,0.10)",
      borderWidth: 1,
      borderColor: "rgba(52,120,246,0.30)",
    },
    severe: { backgroundColor: "rgba(229,72,77,0.12)", borderColor: "rgba(229,72,77,0.40)" },
    headline: { color: "#2f6fe0", fontSize: 15, fontFamily: theme.typography.uiSemibold },
    severeText: { color: "#d0434a" },
    sub: { color: theme.colors.textSecondary, fontSize: 13, marginTop: 2, fontFamily: theme.typography.ui },
  });
