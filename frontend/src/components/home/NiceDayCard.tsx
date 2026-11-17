/**
 * Nice Day and Bonfire card under the hourly strip. Scored from the forecast
 * already loaded (shared scorer in web/src/shared/niceDay.ts), so it renders
 * in the same pass as the strip and never pops in later.
 */
import React, { useMemo } from "react";
import { StyleSheet, Text, View } from "react-native";
import { SectionLabel } from "../ui/WeatherClearUI";
import { startHourIndex } from "../../lib/forecastView";
import { wallClock } from "../../lib/forecastTime";
import { NICE_DAY_CREDIT, niceDayRows, niceDaySummary, type NiceDayWord } from "../../lib/niceDay";
import { useWeatherClearTheme } from "../../theme/WeatherClearThemeProvider";
import type { WeatherClearTheme } from "../../theme/weatherClearTheme";
import type { OpenMeteoResponse } from "../../types/weather";

const WORD_COLORS: Record<NiceDayWord, string> = {
  Great: "#2e9e5b",
  Good: "#4a8f3a",
  Fair: "#b98a17",
  Poor: "#d0703a",
  "Do Not": "#d0434a",
};

export function NiceDayCard({ forecast, nowMs }: { forecast: OpenMeteoResponse; nowMs: number }) {
  const { theme } = useWeatherClearTheme();
  const styles = useMemo(() => createStyles(theme), [theme]);
  const rows = useMemo(() => {
    const start = startHourIndex(forecast.hourly.time, nowMs, forecast.utc_offset_seconds);
    return niceDayRows(niceDaySummary(forecast.hourly, start, wallClock(nowMs, forecast.utc_offset_seconds).dateKey));
  }, [forecast, nowMs]);
  if (!rows.length) return null;
  return (
    <>
      <View style={styles.sectionWrap}>
        <SectionLabel>NICE DAY · BONFIRE</SectionLabel>
      </View>
      <View style={styles.card}>
        {rows.map((r, i) => (
          <View
            key={r.label}
            accessible
            accessibilityLabel={`${r.label}: ${r.word}. ${r.when}.`}
            style={[styles.row, i > 0 ? styles.rowBorder : null]}
          >
            <View style={styles.flex}>
              <Text style={styles.label}>{r.label}</Text>
              <Text style={styles.when}>{r.when}</Text>
            </View>
            <Text style={[styles.word, { color: WORD_COLORS[r.word] }]}>{r.word}</Text>
          </View>
        ))}
        <Text style={styles.credit}>{NICE_DAY_CREDIT}</Text>
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
      paddingVertical: 8,
    },
    flex: { flex: 1 },
    row: { flexDirection: "row", alignItems: "center", minHeight: 48, gap: 12 },
    rowBorder: { borderTopWidth: StyleSheet.hairlineWidth, borderTopColor: theme.colors.divider },
    label: { color: theme.colors.text, fontSize: 15, fontFamily: theme.typography.uiMedium },
    when: { color: theme.colors.textSecondary, fontSize: 13, marginTop: 1, fontFamily: theme.typography.ui },
    word: { fontSize: 20, fontFamily: theme.typography.display },
    credit: { color: theme.colors.textMuted, fontSize: 12, marginTop: 6, marginBottom: 4, fontFamily: theme.typography.ui },
  });
