/**
 * Home-screen section header — extracted from app/(tabs)/index.tsx.
 */
import { memo } from "react";
import { View, Text, StyleSheet } from "react-native";
import { cumulus, cumulusFonts } from "../../lib/cumulusTheme";

export const SectionHeader = memo(function SectionHeader({
  title,
  right,
}: {
  title: string;
  right?: string;
}) {
  return (
    <View style={styles.sectionHeader}>
      <Text style={styles.sectionTitle}>{title}</Text>
      {right ? <Text style={styles.sectionRight}>{right}</Text> : null}
    </View>
  );
});

const styles = StyleSheet.create({
  // Section Header
  sectionHeader: {
    flexDirection: "row",
    justifyContent: "space-between",
    alignItems: "baseline",
    paddingHorizontal: 24,
    paddingTop: 24,
    paddingBottom: 10,
  },
  sectionTitle: {
    color: cumulus.inkMuted,
    fontSize: 10,
    fontWeight: "800",
    letterSpacing: 1.6,
    fontFamily: cumulusFonts.ui,
  },
  sectionRight: {
    color: cumulus.inkDim,
    fontSize: 11,
    fontWeight: "700",
    fontFamily: cumulusFonts.display,
  },
});
