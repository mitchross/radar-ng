/**
 * Nowcast-screen section header — extracted from screens/NowcastScreen.tsx.
 */
import { memo } from "react";
import { View, Text, StyleSheet } from "react-native";
import { cumulus, cumulusFonts } from "../../lib/cumulusTheme";

export const SectionHeader = memo(function SectionHeader({ title }: { title: string }) {
  return (
    <View style={styles.sectionHeader}>
      <Text style={styles.sectionTitle}>{title}</Text>
    </View>
  );
});

const styles = StyleSheet.create({
  sectionHeader: { paddingHorizontal: 24, paddingTop: 24, paddingBottom: 10 },
  sectionTitle: {
    color: cumulus.inkMuted,
    fontSize: 10,
    fontWeight: "800",
    letterSpacing: 1.6,
    fontFamily: cumulusFonts.ui,
  },
});
