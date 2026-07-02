/**
 * Home-screen top bar — location label + Simple/Adv toggle.
 * Extracted from app/(tabs)/index.tsx.
 */
import { memo } from "react";
import { View, Text, StyleSheet, TouchableOpacity } from "react-native";
import { useRouter } from "expo-router";
import { cumulus, cumulusFonts } from "../../lib/cumulusTheme";

export const HomeTopBar = memo(function HomeTopBar({
  locationLabel,
  isAdv,
  onSetViewMode,
}: {
  locationLabel: string;
  isAdv: boolean;
  onSetViewMode: (mode: "simple" | "advanced") => void;
}) {
  const router = useRouter();
  return (
    <View style={styles.topBar}>
      <TouchableOpacity
        onPress={() => router.push("/(tabs)/settings" as any)}
        style={styles.locationContainer}
        activeOpacity={0.7}
      >
        <View style={styles.locationRow}>
          <View style={styles.locationDot} />
          <Text style={styles.locationLabelText}>MY LOCATION</Text>
        </View>
        <View style={styles.locationNameRow}>
          <Text style={styles.locationNameText}>{locationLabel}</Text>
          <Text style={styles.expandChevron}>{"\u25BE"}</Text>
        </View>
      </TouchableOpacity>

      <View style={styles.toggleContainer}>
        <TouchableOpacity
          onPress={() => onSetViewMode("simple")}
          style={[styles.toggleBtn, !isAdv && styles.toggleBtnActive]}
          activeOpacity={0.7}
        >
          <Text style={[styles.toggleBtnText, !isAdv && styles.toggleBtnTextActive]}>Simple</Text>
        </TouchableOpacity>
        <TouchableOpacity
          onPress={() => onSetViewMode("advanced")}
          style={[styles.toggleBtn, isAdv && styles.toggleBtnActive]}
          activeOpacity={0.7}
        >
          <Text style={[styles.toggleBtnText, isAdv && styles.toggleBtnTextActive]}>Adv</Text>
        </TouchableOpacity>
      </View>
    </View>
  );
});

const styles = StyleSheet.create({
  // Top Bar Location + Toggle
  topBar: {
    paddingHorizontal: 24,
    paddingTop: 16,
    paddingBottom: 8,
    flexDirection: "row",
    alignItems: "flex-start",
    justifyContent: "space-between",
  },
  locationContainer: {
    flexDirection: "column",
  },
  locationRow: {
    flexDirection: "row",
    alignItems: "center",
  },
  locationDot: {
    width: 8,
    height: 8,
    borderRadius: 4,
    backgroundColor: cumulus.accent,
    marginRight: 8,
  },
  locationLabelText: {
    fontFamily: cumulusFonts.ui,
    fontSize: 11,
    fontWeight: "700",
    color: cumulus.inkMuted,
    letterSpacing: 1.6,
  },
  locationNameRow: {
    flexDirection: "row",
    alignItems: "center",
    marginTop: 4,
  },
  locationNameText: {
    fontFamily: cumulusFonts.display,
    fontSize: 29,
    fontWeight: "500",
    color: cumulus.ink,
    letterSpacing: -0.2,
  },
  expandChevron: {
    fontSize: 16,
    color: "#bcb3a3",
    marginLeft: 4,
    marginTop: 4,
  },

  toggleContainer: {
    flexDirection: "row",
    backgroundColor: "#eae4d8",
    borderRadius: 11,
    padding: 3,
    alignItems: "center",
  },
  toggleBtn: {
    paddingVertical: 5,
    paddingHorizontal: 9,
    borderRadius: 8,
  },
  toggleBtnActive: {
    backgroundColor: cumulus.accent,
  },
  toggleBtnText: {
    fontFamily: cumulusFonts.ui,
    fontSize: 10,
    fontWeight: "700",
    color: cumulus.inkMuted,
  },
  toggleBtnTextActive: {
    color: "#ffffff",
  },
});
