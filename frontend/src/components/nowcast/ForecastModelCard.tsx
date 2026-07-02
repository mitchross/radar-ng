/**
 * Nowcast-screen forecast-model details card (Advanced mode) —
 * extracted from screens/NowcastScreen.tsx.
 */
import { memo } from "react";
import { View, Text, StyleSheet } from "react-native";
import { cumulus, cumulusFonts } from "../../lib/cumulusTheme";

export const ForecastModelCard = memo(function ForecastModelCard({
  confidence,
  lastUpdateMin,
}: {
  confidence: number;
  lastUpdateMin: number;
}) {
  return (
    <View style={styles.card}>
      <Row label="Model" value="HRRR + MRMS blend" />
      <Row label="Resolution" value="1.9 mi / 15 min" />
      <Row label="Confidence">
        <View style={{ flexDirection: "row", alignItems: "center", gap: 8 }}>
          <View style={styles.confTrack}>
            <View
              style={[
                styles.confFill,
                {
                  width: `${confidence * 100}%`,
                  backgroundColor:
                    confidence > 0.7 ? cumulus.ok : confidence > 0.4 ? cumulus.sun : "#FF9F2E",
                },
              ]}
            />
          </View>
          <Text
            style={[
              styles.confText,
              {
                color:
                  confidence > 0.7 ? cumulus.ok : confidence > 0.4 ? cumulus.sun : "#FF9F2E",
              },
            ]}
          >
            {Math.round(confidence * 100)}%
          </Text>
        </View>
      </Row>
      <Row label="Last update" value={`${lastUpdateMin} min ago`} last />
    </View>
  );
});

function Row({
  label,
  value,
  children,
  last,
}: {
  label: string;
  value?: string;
  children?: React.ReactNode;
  last?: boolean;
}) {
  return (
    <View style={[styles.row, !last && styles.rowBorder]}>
      <Text style={styles.rowLabel}>{label}</Text>
      {children ?? <Text style={styles.rowValue}>{value}</Text>}
    </View>
  );
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

  row: {
    flexDirection: "row",
    justifyContent: "space-between",
    alignItems: "center",
    paddingVertical: 12,
  },
  rowBorder: {
    borderBottomWidth: 1,
    borderBottomColor: "#e7e0d3",
  },
  rowLabel: { color: cumulus.inkDim, fontSize: 13, fontFamily: cumulusFonts.ui, fontWeight: "500" },
  rowValue: { color: cumulus.ink, fontSize: 14, fontWeight: "600", fontFamily: cumulusFonts.ui },

  confTrack: {
    width: 80,
    height: 6,
    borderRadius: 3,
    backgroundColor: "#e7e0d3",
    overflow: "hidden",
  },
  confFill: { height: "100%", borderRadius: 3 },
  confText: {
    fontSize: 11,
    fontWeight: "700",
    fontFamily: cumulusFonts.ui,
  },
});
