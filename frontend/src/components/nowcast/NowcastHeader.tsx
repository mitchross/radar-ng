/**
 * Nowcast-screen header (back button + location) — extracted from screens/NowcastScreen.tsx.
 */
import { memo } from "react";
import { View, Text, StyleSheet, TouchableOpacity } from "react-native";
import { cumulus, cumulusFonts } from "../../lib/cumulusTheme";

export const NowcastHeader = memo(function NowcastHeader({
  location,
  onBack,
}: {
  location: string;
  onBack: () => void;
}) {
  return (
    <View style={styles.header}>
      <TouchableOpacity style={styles.backBtn} onPress={onBack} activeOpacity={0.7}>
        <Text style={styles.backChev}>{"‹"}</Text>
      </TouchableOpacity>
      <View style={{ flex: 1, alignItems: "center" }}>
        <Text style={styles.headerKicker}>HYPER-LOCAL NOWCAST</Text>
        <Text style={styles.headerLocation}>{location}</Text>
      </View>
      <View style={{ width: 36 }} />
    </View>
  );
});

const styles = StyleSheet.create({
  header: {
    flexDirection: "row",
    alignItems: "center",
    paddingHorizontal: 20,
    paddingTop: 16,
    paddingBottom: 8,
  },
  backBtn: {
    width: 36,
    height: 36,
    borderRadius: 12,
    backgroundColor: "#eae4d8",
    borderWidth: 1,
    borderColor: "#e3dccf",
    alignItems: "center",
    justifyContent: "center",
  },
  backChev: { color: cumulus.ink, fontSize: 22, fontWeight: "500", marginTop: -2 },
  headerKicker: {
    color: cumulus.inkMuted,
    fontSize: 11,
    fontWeight: "700",
    letterSpacing: 1.6,
    fontFamily: cumulusFonts.ui,
  },
  headerLocation: {
    color: cumulus.ink,
    fontSize: 15,
    fontWeight: "600",
    marginTop: 2,
    fontFamily: cumulusFonts.ui,
  },
});
