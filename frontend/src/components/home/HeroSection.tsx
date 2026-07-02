/**
 * Home-screen hero — big temperature readout + condition icon.
 * Extracted from app/(tabs)/index.tsx.
 */
import { memo } from "react";
import { View, Text, StyleSheet } from "react-native";
import { cumulus, cumulusFonts, type IconKind } from "../../lib/cumulusTheme";
import WeatherIcon from "../weather/WeatherIcon";

export const HeroSection = memo(function HeroSection({
  iconKind,
  isNight,
  conditionLabel,
  temp,
  feels,
  hi,
  lo,
}: {
  iconKind: IconKind;
  isNight: boolean;
  conditionLabel: string;
  temp: number;
  feels: number;
  hi: number;
  lo: number;
}) {
  return (
    <View style={styles.hero}>
      <View style={styles.heroIcon}>
        <WeatherIcon kind={iconKind} size={130} time={isNight ? "night" : "day"} />
      </View>
      <Text style={styles.heroCondition}>{conditionLabel}</Text>
      <View style={styles.heroTempRow}>
        <Text style={styles.heroTemp}>{temp}</Text>
        <Text style={styles.heroDeg}>{"\u00B0"}</Text>
      </View>
      <Text style={styles.heroMeta}>
        Feels {feels}{"\u00B0"}   {"\u00B7"}   H {hi}{"\u00B0"}   L {lo}{"\u00B0"}
      </Text>
    </View>
  );
});

const styles = StyleSheet.create({
  // Hero
  hero: {
    paddingHorizontal: 24,
    paddingTop: 12,
    paddingBottom: 16,
    position: "relative",
    minHeight: 168,
  },
  heroIcon: { position: "absolute", right: 24, top: 4, opacity: 0.95 },
  heroCondition: {
    color: cumulus.inkDim,
    fontSize: 19,
    fontFamily: cumulusFonts.display,
    fontStyle: "italic",
  },
  heroTempRow: { flexDirection: "row", alignItems: "flex-start", marginTop: 8 },
  heroTemp: {
    color: cumulus.ink,
    fontSize: 104,
    lineHeight: 104,
    fontWeight: "300",
    fontFamily: cumulusFonts.display,
    letterSpacing: -3,
  },
  heroDeg: {
    color: cumulus.ink,
    fontSize: 48,
    fontWeight: "300",
    fontFamily: cumulusFonts.display,
    marginTop: 4,
    opacity: 0.85,
  },
  heroMeta: {
    color: cumulus.inkMuted,
    fontSize: 12,
    fontWeight: "600",
    marginTop: 14,
    fontFamily: cumulusFonts.ui,
  },
});
