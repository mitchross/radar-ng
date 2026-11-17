/**
 * Holiday countdown card near the 7-day list: the next holidays, days to go
 * and the sunset on each, in the location's zone. Pure client maths
 * (web/src/shared/holidays.ts + astro.ts), so it needs no request.
 */
import React, { useMemo } from "react";
import { StyleSheet, Text, View } from "react-native";
import { SectionLabel } from "../ui/WeatherClearUI";
import { wallClock } from "../../lib/forecastTime";
import { countdownRows } from "../../lib/holidays";
import { useWeatherClearTheme } from "../../theme/WeatherClearThemeProvider";
import type { WeatherClearTheme } from "../../theme/weatherClearTheme";
import type { OpenMeteoResponse } from "../../types/weather";

export function CountdownCard({ forecast, nowMs }: { forecast: OpenMeteoResponse; nowMs: number }) {
  const { theme } = useWeatherClearTheme();
  const styles = useMemo(() => createStyles(theme), [theme]);
  const todayKey = wallClock(nowMs, forecast.utc_offset_seconds).dateKey;
  const rows = useMemo(
    () =>
      countdownRows(todayKey, {
        lat: forecast.latitude,
        lon: forecast.longitude,
        timeZone: forecast.timezone,
        utcOffsetSeconds: forecast.utc_offset_seconds,
      }),
    [todayKey, forecast.latitude, forecast.longitude, forecast.timezone, forecast.utc_offset_seconds],
  );
  if (!rows.length) return null;
  return (
    <>
      <View style={styles.sectionWrap}>
        <SectionLabel>COMING UP</SectionLabel>
      </View>
      <View style={styles.card}>
        {rows.map((r, i) => (
          <View
            key={r.id}
            accessible
            accessibilityLabel={`${r.name}, ${r.dateLabel}, ${r.inLabel}.${r.sunsetLabel ? ` ${r.sunsetLabel}.` : ""}`}
            style={[styles.row, i > 0 ? styles.rowBorder : null]}
          >
            <View style={styles.flex}>
              <Text style={styles.name}>{r.name}</Text>
              <Text style={styles.detail}>
                {r.dateLabel}
                {r.sunsetLabel ? ` · ${r.sunsetLabel}` : ""}
              </Text>
            </View>
            <Text style={styles.days}>{r.inLabel}</Text>
          </View>
        ))}
      </View>
    </>
  );
}

const createStyles = (theme: WeatherClearTheme) =>
  StyleSheet.create({
    sectionWrap: { paddingHorizontal: 24, paddingTop: 20, paddingBottom: 8 },
    card: {
      marginHorizontal: 16,
      backgroundColor: theme.colors.surface,
      borderWidth: 1,
      borderColor: theme.colors.border,
      borderRadius: 20,
      paddingHorizontal: 16,
      paddingVertical: 4,
    },
    flex: { flex: 1 },
    row: { flexDirection: "row", alignItems: "center", minHeight: 52, gap: 12 },
    rowBorder: { borderTopWidth: StyleSheet.hairlineWidth, borderTopColor: theme.colors.divider },
    name: { color: theme.colors.text, fontSize: 15, fontFamily: theme.typography.uiMedium },
    detail: { color: theme.colors.textSecondary, fontSize: 13, marginTop: 1, fontFamily: theme.typography.ui },
    days: { color: theme.colors.text, fontSize: 17, fontFamily: theme.typography.display },
  });
