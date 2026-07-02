/**
 * Home-screen Advanced-mode conditions grid — six stat cards.
 * Extracted from app/(tabs)/index.tsx.
 */
import { memo } from "react";
import { View, Text, StyleSheet } from "react-native";
import { cumulus, cumulusFonts, getUVInfo } from "../../lib/cumulusTheme";
import {
  UVBar,
  WindDial,
  FillRing,
  VisBars,
  PressureGauge,
} from "./StatWidgets";

export const StatsGrid = memo(function StatsGrid({
  uv,
  windMph,
  windDeg,
  windCompass,
  humidity,
  dew,
  visibility,
  pressure,
}: {
  uv: number;
  windMph: number;
  windDeg: number;
  windCompass: string;
  humidity: number;
  dew: number;
  visibility: number;
  pressure: number;
}) {
  const uvInfo = getUVInfo(uv);
  return (
    <View style={styles.statGrid}>
      {/* 1. UV Index */}
      <View style={styles.statCard}>
        <Text style={styles.statLabel}>UV INDEX</Text>
        <Text style={styles.statValue}>{Math.round(uv)}</Text>
        <Text style={[styles.statSubText, { color: uvInfo.color }]}>
          {uvInfo.label}
        </Text>
        <View style={styles.widgetWrapper}>
          <UVBar value={uv} />
        </View>
      </View>

      {/* 2. Wind compass */}
      <View style={styles.statCard}>
        <Text style={styles.statLabel}>WIND</Text>
        <View style={styles.statValueRow}>
          <Text style={styles.statValue}>{windMph}</Text>
          <Text style={styles.statUnit}>mph</Text>
        </View>
        <Text style={styles.statSubText}>{windCompass}</Text>
        <View style={styles.widgetWrapper}>
          <WindDial dir={windDeg} />
        </View>
      </View>

      {/* 3. Humidity */}
      <View style={styles.statCard}>
        <Text style={styles.statLabel}>HUMIDITY</Text>
        <View style={styles.statValueRow}>
          <Text style={styles.statValue}>{humidity}</Text>
          <Text style={styles.statUnit}>%</Text>
        </View>
        <Text style={styles.statSubText}>Dew pt {dew}°</Text>
        <View style={styles.widgetWrapper}>
          <FillRing value={humidity / 100} color={cumulus.rain} />
        </View>
      </View>

      {/* 4. Visibility */}
      <View style={styles.statCard}>
        <Text style={styles.statLabel}>VISIBILITY</Text>
        <View style={styles.statValueRow}>
          <Text style={styles.statValue}>{Math.round(visibility)}</Text>
          <Text style={styles.statUnit}>mi</Text>
        </View>
        <Text style={styles.statSubText}>
          {visibility >= 9 ? "Clear view" : "Hazy"}
        </Text>
        <View style={styles.widgetWrapper}>
          <VisBars value={visibility} />
        </View>
      </View>

      {/* 5. Pressure */}
      <View style={styles.statCard}>
        <Text style={styles.statLabel}>PRESSURE</Text>
        <View style={styles.statValueRow}>
          <Text style={styles.statValue}>{pressure}</Text>
          <Text style={styles.statUnit}>hPa</Text>
        </View>
        <Text style={styles.statSubText}>
          {pressure < 1010 ? "Low press." : "Normal"}
        </Text>
        <View style={styles.widgetWrapper}>
          <PressureGauge value={pressure} />
        </View>
      </View>

      {/* 6. Dew point */}
      <View style={styles.statCard}>
        <Text style={styles.statLabel}>DEW POINT</Text>
        <View style={styles.statValueRow}>
          <Text style={styles.statValue}>{dew}</Text>
          <Text style={styles.statUnit}>°</Text>
        </View>
        <Text style={styles.statSubText}>
          {dew > 60 ? "Humid air" : "Comfortable"}
        </Text>
        <View style={styles.widgetWrapper}>
          <FillRing value={Math.max(0, Math.min(1, (dew - 20) / 60))} color="#df6a3c" />
        </View>
      </View>
    </View>
  );
});

const styles = StyleSheet.create({
  // Stats Grid
  statGrid: {
    flexDirection: "row",
    flexWrap: "wrap",
    marginHorizontal: 16,
    gap: 9,
  },
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
  statLabel: {
    color: cumulus.inkMuted,
    fontSize: 9,
    fontWeight: "700",
    letterSpacing: 0.6,
    fontFamily: cumulusFonts.ui,
  },
  statValue: {
    color: cumulus.ink,
    fontSize: 22,
    fontFamily: cumulusFonts.display,
    fontWeight: "500",
    marginTop: 4,
  },
  statValueRow: {
    flexDirection: "row",
    alignItems: "baseline",
    marginTop: 4,
    gap: 2,
  },
  statUnit: {
    color: cumulus.inkMuted,
    fontSize: 10,
    fontFamily: cumulusFonts.ui,
    fontWeight: "500",
  },
  statSubText: {
    color: cumulus.inkMuted,
    fontSize: 10,
    fontWeight: "500",
    fontFamily: cumulusFonts.ui,
    marginTop: 1,
  },
  widgetWrapper: {
    position: "absolute",
    right: 12,
    bottom: 12,
  },
});
