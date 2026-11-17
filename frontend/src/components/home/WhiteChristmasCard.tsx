/**
 * Seasonal White Christmas card (Nov 15 to Dec 25): the share of 1991-2020
 * years with 1 inch of snow on the ground on Dec 25 at the nearest NCEI
 * station, from our server. Tapping it opens the radar map with the stations.
 */
import React, { useMemo } from "react";
import { Pressable, StyleSheet, Text, View } from "react-native";
import { useRouter } from "expo-router";
import { SectionLabel } from "../ui/WeatherClearUI";
import { wallClock } from "../../lib/forecastTime";
import { inWhiteChristmasSeason, WHITE_CHRISTMAS_CREDIT, whiteChristmasCard } from "../../lib/whiteChristmas";
import { useWhiteChristmasNearest } from "../../hooks/useWhiteChristmas";
import { useWeatherStore } from "../../stores/useWeatherStore";
import { useWeatherClearTheme } from "../../theme/WeatherClearThemeProvider";
import type { WeatherClearTheme } from "../../theme/weatherClearTheme";
import type { OpenMeteoResponse } from "../../types/weather";

export function WhiteChristmasCard({ forecast, nowMs }: { forecast: OpenMeteoResponse; nowMs: number }) {
  const { theme } = useWeatherClearTheme();
  const styles = useMemo(() => createStyles(theme), [theme]);
  const router = useRouter();
  const serverUrl = useWeatherStore((s) => s.serverUrl);
  const todayKey = wallClock(nowMs, forecast.utc_offset_seconds).dateKey;
  const inSeason = inWhiteChristmasSeason(todayKey);
  const { data } = useWhiteChristmasNearest(serverUrl, forecast.latitude, forecast.longitude, inSeason);
  const card = whiteChristmasCard(todayKey, data ?? null);
  if (!card) return null;
  return (
    <>
      <View style={styles.sectionWrap}>
        <SectionLabel>WHITE CHRISTMAS</SectionLabel>
      </View>
      <Pressable
        accessibilityRole="button"
        accessibilityLabel={`${card.sentence}. Show stations on the radar map`}
        onPress={() => router.push({ pathname: "/radar", params: { wxmas: "1" } })}
        style={({ pressed }) => [styles.card, pressed ? { opacity: 0.7 } : null]}
      >
        <Text style={styles.pct}>{card.pctLabel}</Text>
        <View style={styles.flex}>
          <Text style={styles.headline}>
            {card.headline} {card.detail}
          </Text>
          <Text style={styles.credit}>{WHITE_CHRISTMAS_CREDIT} · tap for the map</Text>
        </View>
      </Pressable>
    </>
  );
}

const createStyles = (theme: WeatherClearTheme) =>
  StyleSheet.create({
    sectionWrap: { paddingHorizontal: 24, paddingTop: 20, paddingBottom: 8 },
    card: {
      marginHorizontal: 16,
      flexDirection: "row",
      alignItems: "center",
      gap: 14,
      minHeight: 72,
      backgroundColor: theme.colors.surface,
      borderWidth: 1,
      borderColor: theme.colors.border,
      borderRadius: 20,
      paddingHorizontal: 16,
      paddingVertical: 12,
    },
    flex: { flex: 1 },
    pct: { color: theme.colors.text, fontSize: 30, minWidth: 64, fontFamily: theme.typography.display },
    headline: { color: theme.colors.text, fontSize: 15, fontFamily: theme.typography.uiMedium },
    credit: { color: theme.colors.textSecondary, fontSize: 12, marginTop: 3, fontFamily: theme.typography.ui },
  });
