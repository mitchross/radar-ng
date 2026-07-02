/**
 * Home-screen 7-day forecast list card.
 * Extracted from app/(tabs)/index.tsx.
 */
import { memo } from "react";
import { View, Text, StyleSheet } from "react-native";
import { LinearGradient } from "expo-linear-gradient";
import { cumulus, cumulusFonts, type IconKind } from "../../lib/cumulusTheme";
import WeatherIcon from "../weather/WeatherIcon";

export type DailyEntry = {
  day: string;
  icon: IconKind;
  hi: number;
  lo: number;
  precip: number;
  now?: number;
};

export const DailyList = memo(function DailyList({
  daily,
  weekHi,
  weekLo,
}: {
  daily: DailyEntry[];
  weekHi: number;
  weekLo: number;
}) {
  return (
    <View style={[styles.card, { padding: 0, overflow: "hidden" }]}>
      {daily.map((d, i) => {
        const range = weekHi - weekLo || 1;
        const leftPct = ((d.lo - weekLo) / range) * 100;
        const widthPct = ((d.hi - d.lo) / range) * 100;
        const nowPct = d.now != null ? ((d.now - weekLo) / range) * 100 : 0;
        return (
          <View
            key={i}
            style={[
              styles.dailyRow,
              i > 0 && styles.dailyRowBorder,
            ]}
          >
            <Text style={[styles.dailyDay, d.day === "Today" && styles.dailyDayToday]}>
              {d.day}
            </Text>
            <View style={{ width: 24, alignItems: "center" }}>
              <WeatherIcon kind={d.icon} size={21} />
            </View>
            <Text style={styles.dailyLo}>{d.lo}{"\u00B0"}</Text>
            <View style={styles.dailyBarTrack}>
              <LinearGradient
                colors={["#6db4d8", "#f0c34e", "#df6a3c"]}
                start={{ x: 0, y: 0 }}
                end={{ x: 1, y: 0 }}
                style={{
                  position: "absolute",
                  left: `${leftPct}%`,
                  width: `${widthPct}%`,
                  height: "100%",
                  borderRadius: 3,
                }}
              />
              {d.now != null && (
                <View
                  style={[
                    styles.dailyNowDot,
                    { left: `${nowPct}%` },
                  ]}
                />
              )}
            </View>
            <Text style={styles.dailyHi}>{d.hi}{"\u00B0"}</Text>
          </View>
        );
      })}
    </View>
  );
});

const styles = StyleSheet.create({
  // Card
  card: {
    marginHorizontal: 16,
    backgroundColor: "#ffffff",
    borderWidth: 1,
    borderColor: "#eee6d8",
    borderRadius: 20,
    padding: 16,
    shadowColor: "rgba(60,50,40,0.04)",
    shadowOffset: { width: 0, height: 4 },
    shadowOpacity: 0.3,
    shadowRadius: 6,
    elevation: 1,
  },

  // Daily row
  dailyRow: {
    flexDirection: "row",
    alignItems: "center",
    paddingVertical: 10,
    paddingHorizontal: 16,
    gap: 11,
  },
  dailyRowBorder: {
    borderTopWidth: 1,
    borderTopColor: "#e7e0d3",
  },
  dailyDay: {
    color: cumulus.ink,
    fontSize: 14,
    fontWeight: "600",
    fontFamily: cumulusFonts.ui,
    width: 44,
  },
  dailyDayToday: { fontWeight: "700" },
  dailyLo: {
    color: cumulus.inkMuted,
    fontSize: 13,
    width: 28,
    textAlign: "right",
    fontFamily: cumulusFonts.ui,
  },
  dailyHi: {
    color: cumulus.ink,
    fontSize: 13,
    fontWeight: "600",
    width: 28,
    textAlign: "right",
    fontFamily: cumulusFonts.ui,
  },
  dailyBarTrack: {
    flex: 1,
    height: 6,
    borderRadius: 3,
    backgroundColor: "#e7e0d3",
    position: "relative",
  },
  dailyNowDot: {
    position: "absolute",
    top: -3,
    width: 12,
    height: 12,
    borderRadius: 6,
    backgroundColor: "#fff",
    borderWidth: 3,
    borderColor: cumulus.accent,
    marginLeft: -6,
  },
});
