/**
 * Nowcast-screen minute intensity chart card — extracted from screens/NowcastScreen.tsx.
 */
import { memo } from "react";
import { View, Text, StyleSheet } from "react-native";
import { LinearGradient } from "expo-linear-gradient";
import { cumulus, cumulusFonts } from "../../lib/cumulusTheme";

export type Minute = { i: number; intensity: number; confLo: number; confHi: number };

export const IntensityChartCard = memo(function IntensityChartCard({
  minutes,
}: {
  minutes: Minute[];
}) {
  return (
    <View style={styles.card}>
      <View style={styles.chartHeader}>
        <Text style={styles.chartLabel}>INTENSITY {"·"} IN/HR</Text>
        <Text style={styles.chartLabel}>NEXT 60 MIN</Text>
      </View>
      <NowcastChart minutes={minutes} />
      <View style={styles.chartAxis}>
        <Text style={styles.axisTick}>NOW</Text>
        <Text style={styles.axisTick}>+15</Text>
        <Text style={styles.axisTick}>+30</Text>
        <Text style={styles.axisTick}>+45</Text>
        <Text style={styles.axisTick}>+60</Text>
      </View>
      <View style={styles.scaleRow}>
        <Text style={styles.axisTick}>LIGHT</Text>
        <LinearGradient
          colors={["#7ae5a8", "#4d7fb8", "#3f6fd6", "#c2603a", "#df6a6a"]}
          start={{ x: 0, y: 0 }}
          end={{ x: 1, y: 0 }}
          style={styles.scaleGrad}
        />
        <Text style={styles.axisTick}>INTENSE</Text>
      </View>
    </View>
  );
});

function NowcastChart({ minutes }: { minutes: Minute[] }) {
  const H = 140;
  const maxI = Math.max(0.5, ...minutes.map((m) => m.intensity));
  return (
    <View style={styles.chartBox}>
      {[0.25, 0.5, 0.75].map((y) => (
        <View
          key={y}
          style={[styles.gridLine, { top: H - y * H * 0.95 }]}
        />
      ))}
      <View style={styles.barsRow}>
        {minutes.map((m, i) => {
          const h = Math.max(1, (m.intensity / maxI) * H * 0.95);
          const color = intensityColor(m.intensity / maxI);
          return (
            <View
              key={i}
              style={{
                flex: 1,
                height: h,
                marginHorizontal: 0.4,
                backgroundColor: color,
                opacity: m.intensity > 0.02 ? 1 : 0.25,
                borderRadius: 1,
              }}
            />
          );
        })}
      </View>
      <View style={styles.baseline} />
    </View>
  );
}

function intensityColor(pct: number): string {
  if (pct < 0.15) return "#7ae5a8";
  if (pct < 0.35) return cumulus.rain;
  if (pct < 0.6) return cumulus.rainHeavy;
  if (pct < 0.85) return cumulus.accent;
  return cumulus.hot;
}

const styles = StyleSheet.create({
  card: {
    marginHorizontal: 16,
    marginTop: 20,
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

  chartHeader: {
    flexDirection: "row",
    justifyContent: "space-between",
    marginBottom: 10,
  },
  chartLabel: {
    color: cumulus.inkMuted,
    fontSize: 9,
    fontWeight: "700",
    letterSpacing: 1.2,
    fontFamily: cumulusFonts.ui,
  },
  chartBox: {
    height: 140,
    backgroundColor: "transparent",
    position: "relative",
    justifyContent: "flex-end",
  },
  gridLine: {
    position: "absolute",
    left: 0,
    right: 0,
    height: StyleSheet.hairlineWidth,
    backgroundColor: "rgba(33, 31, 27, 0.08)",
  },
  barsRow: {
    flexDirection: "row",
    alignItems: "flex-end",
    height: 140,
  },
  baseline: {
    position: "absolute",
    left: 0,
    right: 0,
    bottom: 0,
    height: 1,
    backgroundColor: "rgba(33, 31, 27, 0.16)",
  },
  chartAxis: {
    flexDirection: "row",
    justifyContent: "space-between",
    marginTop: 8,
  },
  axisTick: {
    color: cumulus.inkMuted,
    fontSize: 10,
    fontWeight: "700",
    fontFamily: cumulusFonts.ui,
  },
  scaleRow: {
    flexDirection: "row",
    alignItems: "center",
    gap: 8,
    marginTop: 14,
  },
  scaleGrad: {
    flex: 1,
    height: 6,
    borderRadius: 3,
  },
});
