/**
 * Home-screen Advanced-mode sunrise/sunset widget row.
 * Extracted from app/(tabs)/index.tsx.
 */
import { memo } from "react";
import { View, Text, StyleSheet } from "react-native";
import { cumulus, cumulusFonts } from "../../lib/cumulusTheme";

export const SunriseSunsetRow = memo(function SunriseSunsetRow({
  sunriseText,
  sunsetText,
}: {
  sunriseText: string;
  sunsetText: string;
}) {
  return (
    <View style={styles.sunriseSunsetGrid}>
      <View style={[styles.statCard, styles.rowLayoutCard]}>
        <Text style={styles.widgetIconText}>🌅</Text>
        <View>
          <Text style={styles.rowLayoutLabel}>SUNRISE</Text>
          <Text style={styles.rowLayoutVal}>
            {sunriseText}
          </Text>
        </View>
      </View>
      <View style={[styles.statCard, styles.rowLayoutCard]}>
        <Text style={styles.widgetIconText}>🌇</Text>
        <View>
          <Text style={styles.rowLayoutLabel}>SUNSET</Text>
          <Text style={styles.rowLayoutVal}>
            {sunsetText}
          </Text>
        </View>
      </View>
    </View>
  );
});

const styles = StyleSheet.create({
  statCard: {
    flexBasis: "47%",
    flexGrow: 1,
    backgroundColor: "#ffffff",
    borderRadius: 14,
    borderWidth: 1,
    borderColor: "#eee6d8",
    padding: 12,
    minHeight: 110,
    position: "relative",
  },

  // Sunrise sunset cells
  sunriseSunsetGrid: {
    flexDirection: "row",
    marginHorizontal: 16,
    marginTop: 9,
    gap: 9,
  },
  rowLayoutCard: {
    flexDirection: "row",
    alignItems: "center",
    gap: 10,
    minHeight: 52,
    paddingVertical: 10,
  },
  widgetIconText: {
    fontSize: 24,
  },
  rowLayoutLabel: {
    fontSize: 9,
    fontWeight: "700",
    color: cumulus.inkMuted,
    letterSpacing: 0.6,
    fontFamily: cumulusFonts.ui,
  },
  rowLayoutVal: {
    fontSize: 18,
    fontFamily: cumulusFonts.display,
    fontWeight: "500",
    color: cumulus.ink,
    marginTop: 2,
  },
});
