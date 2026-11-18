/**
 * "Look up tonight": ISS passes from our server, planets and moon worked out
 * on the phone, and a clear-or-cloudy call. Full card from two hours before
 * sunset until sunrise; a "Tonight" link the rest of the day.
 */
import React, { useMemo, useState } from "react";
import { Pressable, StyleSheet, Text, View } from "react-native";
import { SectionLabel } from "../ui/WeatherClearUI";
import { wallClock } from "../../lib/forecastTime";
import { inSkyWindow, skyTonightCard, tonight } from "../../lib/skyTonight";
import { useIssAlertSync, useSkyTonight } from "../../hooks/useSkyTonight";
import { useWeatherStore } from "../../stores/useWeatherStore";
import { useWeatherClearTheme } from "../../theme/WeatherClearThemeProvider";
import type { WeatherClearTheme } from "../../theme/weatherClearTheme";
import type { OpenMeteoResponse } from "../../types/weather";

export function SkyTonightCard({ forecast, nowMs }: { forecast: OpenMeteoResponse; nowMs: number }) {
  const { theme } = useWeatherClearTheme();
  const styles = useMemo(() => createStyles(theme), [theme]);
  const [expanded, setExpanded] = useState(false);
  const serverUrl = useWeatherStore((s) => s.serverUrl);
  const { latitude: lat, longitude: lon } = forecast;
  const { data } = useSkyTonight(serverUrl, lat, lon);
  const clock = { timeZone: forecast.timezone, utcOffsetSeconds: forecast.utc_offset_seconds };
  const hourly = forecast.hourly.cloud_cover ? forecast.hourly : null;
  useIssAlertSync(data, hourly, clock);
  const night = tonight(nowMs, wallClock(nowMs, forecast.utc_offset_seconds).dateKey, lat, lon);
  if (!night) return null;

  if (!expanded && !inSkyWindow(nowMs, night)) {
    return (
      <View style={[styles.sectionWrap, styles.linkRow]}>
        <SectionLabel>LOOK UP TONIGHT</SectionLabel>
        <Pressable accessibilityRole="button" accessibilityLabel="Show tonight's sky" onPress={() => setExpanded(true)} hitSlop={12}>
          <Text style={styles.link}>Tonight ›</Text>
        </Pressable>
      </View>
    );
  }

  const card = skyTonightCard({ nowMs, night, lat, lon, report: data ?? null, hourly, clock });
  return (
    <>
      <View style={styles.sectionWrap}>
        <SectionLabel>LOOK UP TONIGHT</SectionLabel>
      </View>
      <View style={styles.card} accessible accessibilityLabel={describe(card)}>
        {card.verdictText ? <Text style={styles.verdict}>{card.verdictText}</Text> : null}
        <Text style={styles.head}>ISS</Text>
        {card.passes.map((p) => (
          <Text key={p} style={styles.line}>{p}</Text>
        ))}
        {card.issNote ? <Text style={styles.note}>{card.issNote}</Text> : null}
        <Text style={styles.head}>PLANETS</Text>
        {card.planets.length ? (
          card.planets.map((p) => <Text key={p} style={styles.line}>{p}</Text>)
        ) : (
          <Text style={styles.note}>No bright planets up after dark.</Text>
        )}
        <Text style={styles.head}>MOON</Text>
        <Text style={styles.line}>{card.moon}</Text>
        <Text style={styles.credit}>{card.credit} · passes 10° up or higher after dusk</Text>
      </View>
    </>
  );
}

function describe(card: ReturnType<typeof skyTonightCard>): string {
  return [card.verdictText, ...card.passes.map((p) => `ISS ${p}`), card.issNote, ...card.planets, `Moon ${card.moon}`]
    .filter(Boolean)
    .join(". ");
}

const createStyles = (theme: WeatherClearTheme) =>
  StyleSheet.create({
    sectionWrap: { paddingHorizontal: 24, paddingTop: 20, paddingBottom: 8 },
    linkRow: { flexDirection: "row", alignItems: "center", justifyContent: "space-between" },
    link: { color: theme.colors.accent, fontSize: 14, fontFamily: theme.typography.uiMedium },
    card: {
      marginHorizontal: 16,
      backgroundColor: theme.colors.surface,
      borderWidth: 1,
      borderColor: theme.colors.border,
      borderRadius: 20,
      paddingHorizontal: 16,
      paddingVertical: 12,
    },
    verdict: { color: theme.colors.text, fontSize: 20, fontFamily: theme.typography.display, marginBottom: 4 },
    head: { color: theme.colors.textSecondary, fontSize: 12, letterSpacing: 0.6, marginTop: 10, fontFamily: theme.typography.uiMedium },
    line: { color: theme.colors.text, fontSize: 15, marginTop: 3, fontFamily: theme.typography.ui },
    note: { color: theme.colors.textSecondary, fontSize: 14, marginTop: 3, fontFamily: theme.typography.ui },
    credit: { color: theme.colors.textSecondary, fontSize: 12, marginTop: 12, fontFamily: theme.typography.ui },
  });
