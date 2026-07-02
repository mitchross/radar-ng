/**
 * Home-screen 24h precipitation chart card.
 * Extracted from app/(tabs)/index.tsx.
 */
import { memo } from "react";
import { View, Text, StyleSheet } from "react-native";
import { cumulus, cumulusFonts } from "../../lib/cumulusTheme";
import type { HourlyEntry } from "./HourlyStrip";

export const PrecipChart = memo(function PrecipChart({
  hourly,
}: {
  hourly: HourlyEntry[];
}) {
  return (
    <View style={styles.card}>
      <View style={styles.precipChart}>
        {hourly.map((h, i) => {
          const pct = h.precip / 100;
          const barH = Math.max(2, pct * 42);
          return (
            <View key={i} style={styles.precipBarSlot}>
              <View
                style={[
                  styles.precipBar,
                  { height: barH, opacity: pct > 0.05 ? 1 : 0.25 },
                ]}
              />
            </View>
          );
        })}
      </View>
      <View style={styles.precipAxis}>
        <Text style={styles.axisLabel}>NOW</Text>
        <Text style={styles.axisLabel}>+12H</Text>
        <Text style={styles.axisLabel}>+24H</Text>
      </View>
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

  // Precip chart
  precipChart: {
    flexDirection: "row",
    alignItems: "flex-end",
    height: 44,
    gap: 3,
  },
  precipBarSlot: { flex: 1, justifyContent: "flex-end" },
  precipBar: {
    width: "100%",
    borderRadius: 3,
    backgroundColor: cumulus.rain,
  },
  precipAxis: {
    flexDirection: "row",
    justifyContent: "space-between",
    marginTop: 8,
  },
  axisLabel: {
    color: cumulus.inkFaint,
    fontSize: 9,
    fontWeight: "700",
    fontFamily: cumulusFonts.ui,
  },
});
