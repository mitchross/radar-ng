/**
 * Optional local-LLM briefing. Renders nothing unless a briefing arrived, so an
 * absent or failing model leaves the home screen exactly as it was.
 */
import React, { useMemo } from "react";
import { StyleSheet, Text, View } from "react-native";
import { useWeatherClearTheme } from "../../theme/WeatherClearThemeProvider";
import type { WeatherClearTheme } from "../../theme/weatherClearTheme";
import type { Briefing } from "../../lib/api";

export function AiBriefingCard({ briefing }: { briefing: Briefing | null }) {
  const { theme } = useWeatherClearTheme();
  const styles = useMemo(() => createStyles(theme), [theme]);
  if (!briefing) return null;
  return (
    <View style={styles.card} accessibilityLabel={`AI briefing. ${briefing.headline}. ${briefing.body}`}>
      <View style={styles.head}>
        <Text style={styles.spark}>{"\u2726"}</Text>
        <Text style={styles.headline} numberOfLines={1}>{briefing.headline || "Right now"}</Text>
      </View>
      <Text style={styles.body}>{briefing.body}</Text>
      <Text style={styles.foot}>AI summary{briefing.model ? ` \u00B7 ${briefing.model}` : ""}</Text>
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
      backgroundColor: theme.dark ? "rgba(124,92,255,0.16)" : "rgba(124,92,255,0.08)",
      borderWidth: 1,
      borderColor: "rgba(124,92,255,0.28)",
    },
    head: { flexDirection: "row", alignItems: "center", gap: 8 },
    spark: { color: "#7c5cff", fontSize: 14 },
    headline: { flex: 1, color: theme.colors.text, fontSize: 15, fontFamily: theme.typography.uiSemibold },
    body: { color: theme.colors.text, fontSize: 14, lineHeight: 20, marginTop: 6, fontFamily: theme.typography.ui },
    foot: { color: theme.colors.textMuted, fontSize: 11, marginTop: 6, fontFamily: theme.typography.ui },
  });
