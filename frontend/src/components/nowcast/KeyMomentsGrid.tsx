/**
 * Nowcast-screen key-moments card grid — extracted from screens/NowcastScreen.tsx.
 */
import { memo } from "react";
import { View, Text, StyleSheet } from "react-native";
import { cumulus, cumulusFonts } from "../../lib/cumulusTheme";
import WeatherIcon from "../weather/WeatherIcon";

export const KeyMomentsGrid = memo(function KeyMomentsGrid({
  rainStart,
  peakMin,
  rainEndMin,
  totalIn,
  peakRate,
}: {
  rainStart: number;
  peakMin: number;
  rainEndMin: number;
  totalIn: number;
  peakRate?: string;
}) {
  return (
    <View style={styles.keyGrid}>
      <KeyCard
        label="STARTS"
        value={rainStart < 0 ? "—" : `+${rainStart}m`}
        icon="rain"
        color={cumulus.rain}
      />
      <KeyCard
        label="PEAK"
        value={rainStart < 0 ? "—" : `+${peakMin}m`}
        sub={peakRate}
        icon="heavyRain"
        color={cumulus.hot}
      />
      <KeyCard
        label="ENDS"
        value={rainEndMin < 0 ? "—" : `+${rainEndMin}m`}
        icon="partlyCloudy"
        color={cumulus.sun}
      />
      <KeyCard
        label="TOTAL"
        value={`${totalIn.toFixed(2)}"`}
        sub="next hour"
        icon="cloudy"
        color={cumulus.accent}
      />
    </View>
  );
});

function KeyCard({
  label,
  value,
  sub,
  icon,
  color,
}: {
  label: string;
  value: string;
  sub?: string;
  icon: Parameters<typeof WeatherIcon>[0]["kind"];
  color: string;
}) {
  return (
    <View style={styles.keyCard}>
      <View style={[styles.keyIcon, { backgroundColor: `${color}16` }]}>
        <WeatherIcon kind={icon} size={26} />
      </View>
      <View style={{ flex: 1 }}>
        <Text style={styles.keyLabel}>{label}</Text>
        <Text style={styles.keyValue}>{value}</Text>
        {sub ? <Text style={styles.keySub}>{sub}</Text> : null}
      </View>
    </View>
  );
}

const styles = StyleSheet.create({
  keyGrid: {
    flexDirection: "row",
    flexWrap: "wrap",
    marginHorizontal: 16,
    gap: 9,
  },
  keyCard: {
    flexBasis: "47%",
    flexGrow: 1,
    backgroundColor: "#ffffff",
    borderWidth: 1,
    borderColor: "#eee6d8",
    borderRadius: 16,
    padding: 12,
    flexDirection: "row",
    alignItems: "center",
    gap: 12,
  },
  keyIcon: {
    width: 38,
    height: 38,
    borderRadius: 10,
    alignItems: "center",
    justifyContent: "center",
  },
  keyLabel: {
    color: cumulus.inkMuted,
    fontSize: 9,
    fontWeight: "700",
    letterSpacing: 1.2,
    fontFamily: cumulusFonts.ui,
  },
  keyValue: {
    color: cumulus.ink,
    fontSize: 20,
    fontFamily: cumulusFonts.display,
    fontWeight: "500",
    marginTop: 2,
  },
  keySub: {
    color: cumulus.inkDim,
    fontSize: 10,
    fontFamily: cumulusFonts.ui,
    fontWeight: "500",
    marginTop: 2,
  },
});
