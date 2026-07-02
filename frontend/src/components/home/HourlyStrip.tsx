/**
 * Home-screen 24h hourly strip — horizontal scroll of hour cells.
 * Extracted from app/(tabs)/index.tsx.
 */
import { memo } from "react";
import { ScrollView, View, Text, StyleSheet } from "react-native";
import { cumulus, cumulusFonts, type IconKind } from "../../lib/cumulusTheme";
import WeatherIcon from "../weather/WeatherIcon";

export type HourlyEntry = {
  time: string;
  temp: number;
  icon: IconKind;
  precip: number;
  isNow: boolean;
};

export const HourlyStrip = memo(function HourlyStrip({
  hourly,
  isNight,
}: {
  hourly: HourlyEntry[];
  isNight: boolean;
}) {
  return (
    <ScrollView
      horizontal
      showsHorizontalScrollIndicator={false}
      contentContainerStyle={styles.hourlyStrip}
    >
      {hourly.map((h, i) => (
        <View
          key={i}
          style={[styles.hourlyCell, h.isNow && styles.hourlyCellNow]}
        >
          <Text style={[styles.hourlyTime, h.isNow && styles.hourlyTimeNow]}>
            {h.isNow ? "NOW" : h.time}
          </Text>
          <View style={{ marginVertical: 6 }}>
            <WeatherIcon kind={h.icon} size={22} time={isNight ? "night" : "day"} />
          </View>
          <Text style={styles.hourlyTemp}>{h.temp}{"\u00B0"}</Text>
        </View>
      ))}
    </ScrollView>
  );
});

const styles = StyleSheet.create({
  // Hourly strip
  hourlyStrip: { paddingHorizontal: 16, gap: 8 },
  hourlyCell: {
    minWidth: 54,
    paddingVertical: 12,
    alignItems: "center",
    backgroundColor: "#ffffff",
    borderRadius: 16,
    borderWidth: 1,
    borderColor: "#eee6d8",
  },
  hourlyCellNow: {
    backgroundColor: "#eae4d8",
    borderColor: "#e3dccf",
  },
  hourlyTime: {
    color: cumulus.inkMuted,
    fontSize: 11,
    fontWeight: "700",
    fontFamily: cumulusFonts.ui,
  },
  hourlyTimeNow: { color: cumulus.accent },
  hourlyTemp: {
    color: cumulus.ink,
    fontSize: 17,
    fontWeight: "500",
    fontFamily: cumulusFonts.display,
  },
});
