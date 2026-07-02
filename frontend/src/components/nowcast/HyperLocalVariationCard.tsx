/**
 * Nowcast-screen hyper-local variation card (Advanced mode) —
 * extracted from screens/NowcastScreen.tsx.
 */
import { memo } from "react";
import { View, Text, StyleSheet } from "react-native";
import { cumulus, cumulusFonts } from "../../lib/cumulusTheme";

export const HyperLocalVariationCard = memo(function HyperLocalVariationCard({
  totalIn,
}: {
  totalIn: number;
}) {
  return (
    <View style={styles.card}>
      <Text style={styles.variationCaption}>
        Rain totals expected within 2 miles of you
      </Text>
      {[
        { label: "Your block", v: totalIn, hi: true },
        { label: "½ mi north", v: totalIn * 1.4 },
        { label: "½ mi south", v: totalIn * 0.3 },
        { label: "1 mi east", v: totalIn * 0.9 },
        { label: "1 mi west", v: totalIn * 1.7 },
      ].map((r) => (
        <View key={r.label} style={styles.variationRow}>
          <Text
            style={[
              styles.variationLabel,
              r.hi && { color: cumulus.ink, fontWeight: "600" },
            ]}
          >
            {r.label}
          </Text>
          <View style={styles.variationTrack}>
            <View
              style={[
                styles.variationFill,
                {
                  width: `${Math.min(100, (r.v / Math.max(0.5, totalIn * 2)) * 100)}%`,
                  backgroundColor: r.hi ? cumulus.accent : cumulus.rain,
                },
              ]}
            />
          </View>
          <Text
            style={[
              styles.variationValue,
              r.hi && { color: cumulus.ink },
            ]}
          >
            {r.v.toFixed(2)}&quot;
          </Text>
        </View>
      ))}
    </View>
  );
});

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

  variationCaption: {
    color: cumulus.inkDim,
    fontSize: 12,
    fontFamily: cumulusFonts.ui,
    marginBottom: 10,
  },
  variationRow: {
    flexDirection: "row",
    alignItems: "center",
    gap: 10,
    paddingVertical: 6,
  },
  variationLabel: {
    width: 86,
    fontSize: 12,
    color: cumulus.inkDim,
    fontFamily: cumulusFonts.ui,
  },
  variationTrack: {
    flex: 1,
    height: 6,
    borderRadius: 3,
    backgroundColor: "#e7e0d3",
    overflow: "hidden",
  },
  variationFill: { height: "100%", borderRadius: 3 },
  variationValue: {
    width: 48,
    textAlign: "right",
    fontSize: 12,
    fontFamily: cumulusFonts.mono,
    color: cumulus.inkDim,
  },
});
