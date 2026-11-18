/**
 * "Snow at your spot": headline, 48 hourly bars and short lines from our
 * server. Shows only when snow is due in 72 h or on the ground. Fixed height,
 * with a skeleton of the same size while a snowy forecast loads.
 */
import React, { useMemo } from "react";
import { StyleSheet, Text, View } from "react-native";
import { SectionLabel } from "../ui/WeatherClearUI";
import { snowCardShows, snowLikely, snowSpotLabel, snowSpotView } from "../../lib/snowSpot";
import { useSnowCardVisibility, useSnowSpot } from "../../hooks/useSnowSpot";
import { useWeatherStore } from "../../stores/useWeatherStore";
import { useWeatherClearTheme } from "../../theme/WeatherClearThemeProvider";
import type { WeatherClearTheme } from "../../theme/weatherClearTheme";
import type { OpenMeteoResponse } from "../../types/weather";

const CARD_HEIGHT = 196;
const BARS_HEIGHT = 44;

export function SnowSpotCard({ forecast, nowMs }: { forecast: OpenMeteoResponse; nowMs: number }) {
  const { theme } = useWeatherClearTheme();
  const styles = useMemo(() => createStyles(theme), [theme]);
  const serverUrl = useWeatherStore((s) => s.serverUrl);
  const { data, isPending } = useSnowSpot(serverUrl, forecast.latitude, forecast.longitude);
  const shows = snowCardShows(data ?? null, nowMs);
  useSnowCardVisibility(shows);

  if (isPending) {
    if (!snowLikely(forecast.hourly, nowMs, forecast.utc_offset_seconds)) return null;
    return (
      <>
        <View style={styles.sectionWrap}>
          <SectionLabel>SNOW AT YOUR SPOT</SectionLabel>
        </View>
        <View style={styles.card} accessibilityLabel="Loading snow at your spot" accessibilityState={{ busy: true }}>
          <View style={[styles.skel, { width: "60%", height: 20 }]} />
          <View style={styles.skel} />
          <View style={[styles.skel, { height: BARS_HEIGHT }]} />
          <View style={styles.skel} />
        </View>
      </>
    );
  }
  if (!data || !shows) return null;

  const v = snowSpotView(data, nowMs, { timeZone: forecast.timezone, utcOffsetSeconds: forecast.utc_offset_seconds });
  return (
    <>
      <View style={styles.sectionWrap}>
        <SectionLabel>SNOW AT YOUR SPOT</SectionLabel>
      </View>
      <View style={styles.card} accessible accessibilityLabel={snowSpotLabel(v)}>
        <Text style={styles.head} numberOfLines={1}>{v.headline}</Text>
        <Text style={styles.window} numberOfLines={1}>{v.window ?? " "}</Text>
        <View style={styles.bars}>
          {v.bars.map((b) => (
            <View
              key={b.at}
              style={[
                styles.bar,
                { height: Math.max(b.inches > 0 ? 3 : 1, Math.round(b.height * BARS_HEIGHT)) },
                b.inches > 0 ? styles.barOn : null,
              ]}
            />
          ))}
        </View>
        <View style={styles.lines}>
          {v.lines.map((l) => (
            <Text key={l} style={styles.line} numberOfLines={1}>{l}</Text>
          ))}
        </View>
        <Text style={styles.credit} numberOfLines={1}>{v.credit || " "}</Text>
      </View>
    </>
  );
}

const createStyles = (theme: WeatherClearTheme) =>
  StyleSheet.create({
    sectionWrap: { paddingHorizontal: 24, paddingTop: 20, paddingBottom: 8 },
    card: {
      marginHorizontal: 16,
      height: CARD_HEIGHT,
      overflow: "hidden",
      backgroundColor: theme.colors.surface,
      borderWidth: 1,
      borderColor: theme.colors.border,
      borderRadius: 20,
      paddingHorizontal: 16,
      paddingVertical: 12,
    },
    head: { color: theme.colors.text, fontSize: 20, lineHeight: 26, fontFamily: theme.typography.display },
    window: { color: theme.colors.textSecondary, fontSize: 13, lineHeight: 18, fontFamily: theme.typography.ui },
    bars: { flexDirection: "row", alignItems: "flex-end", gap: 1, height: BARS_HEIGHT, marginVertical: 10 },
    bar: { flex: 1, borderRadius: 1, backgroundColor: theme.colors.border },
    barOn: { backgroundColor: theme.colors.accent },
    lines: { flexDirection: "row", flexWrap: "wrap", height: 40, columnGap: 8 },
    line: { width: "48%", color: theme.colors.text, fontSize: 13, lineHeight: 20, fontFamily: theme.typography.ui },
    credit: { color: theme.colors.textSecondary, fontSize: 12, lineHeight: 16, marginTop: 6, fontFamily: theme.typography.ui },
    skel: { height: 14, borderRadius: 6, marginBottom: 12, backgroundColor: theme.colors.border, opacity: 0.6 },
  });
