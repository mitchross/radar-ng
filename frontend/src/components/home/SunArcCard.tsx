/**
 * Home-screen Advanced-mode sun-path arc card.
 * Extracted from app/(tabs)/index.tsx.
 */
import { memo } from "react";
import { View, StyleSheet } from "react-native";
import { SunArc } from "./StatWidgets";

export const SunArcCard = memo(function SunArcCard({
  sunriseText,
  sunsetText,
  progress,
}: {
  sunriseText: string;
  sunsetText: string;
  progress: number;
}) {
  return (
    <View style={[styles.card, styles.sunArcCard]}>
      <SunArc
        sunrise={sunriseText}
        sunset={sunsetText}
        progress={progress}
      />
    </View>
  );
});

const styles = StyleSheet.create({
  // Card
  card: {
    marginHorizontal: 16,
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

  // Sun Arc Card
  sunArcCard: {
    marginTop: 12,
    paddingVertical: 14,
  },
});
