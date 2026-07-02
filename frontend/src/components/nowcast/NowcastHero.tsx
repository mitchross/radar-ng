/**
 * Nowcast-screen hero verdict — extracted from screens/NowcastScreen.tsx.
 */
import { memo } from "react";
import { View, Text, StyleSheet } from "react-native";
import { cumulus, cumulusFonts } from "../../lib/cumulusTheme";

export const NowcastHero = memo(function NowcastHero({
  rainStart,
  rainEndMin,
  peakMin,
}: {
  rainStart: number;
  rainEndMin: number;
  peakMin: number;
}) {
  return (
    <View style={styles.hero}>
      {rainStart < 0 ? (
        <Text style={styles.heroDry}>
          No rain expected{"\n"}
          <Text style={styles.heroDrySub}>for the next hour</Text>
        </Text>
      ) : rainStart === 0 ? (
        <Text style={styles.heroDry}>
          Raining <Text style={{ color: cumulus.rain, fontWeight: "500" }}>now.</Text>
        </Text>
      ) : (
        <Text style={styles.heroDry}>
          Rain starts in{"\n"}
          <Text style={{ color: cumulus.rain, fontWeight: "500" }}>
            {rainStart} {rainStart === 1 ? "minute" : "minutes"}
          </Text>
        </Text>
      )}
      {rainStart >= 0 && rainEndMin > 0 && (
        <Text style={styles.heroSub}>
          Expected to last ~{rainEndMin - rainStart} min
          <Text style={styles.heroDim}>  {"·"}  peaks at </Text>
          <Text style={styles.heroStrong}>+{peakMin}m</Text>
        </Text>
      )}
    </View>
  );
});

const styles = StyleSheet.create({
  hero: { paddingHorizontal: 24, paddingTop: 20 },
  heroDry: {
    color: cumulus.ink,
    fontSize: 46,
    fontWeight: "400",
    letterSpacing: -1.2,
    lineHeight: 48,
    fontFamily: cumulusFonts.display,
  },
  heroDrySub: {
    fontSize: 22,
    color: cumulus.inkDim,
    fontStyle: "italic",
    fontFamily: cumulusFonts.display,
  },
  heroSub: {
    color: cumulus.inkMuted,
    fontSize: 13,
    marginTop: 10,
    fontFamily: cumulusFonts.ui,
    fontWeight: "500",
  },
  heroStrong: { color: cumulus.ink, fontWeight: "600" },
  heroDim: { color: cumulus.inkFaint },
});
