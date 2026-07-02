/**
 * Nowcast-screen "About this forecast" note card —
 * extracted from screens/NowcastScreen.tsx.
 */
import { memo } from "react";
import { View, Text, StyleSheet } from "react-native";
import { cumulus, cumulusFonts } from "../../lib/cumulusTheme";

export const AboutNoteCard = memo(function AboutNoteCard() {
  return (
    <View style={styles.card}>
      <Text style={styles.noteTitle}>About this forecast</Text>
      <Text style={styles.noteBody}>
        Minute-by-minute precip is interpolated from Open-Meteo&apos;s 15-min HRRR
        output. Connect a self-hosted tile-server in Settings to feed MRMS
        observations for true minute-level accuracy.
      </Text>
    </View>
  );
});

const styles = StyleSheet.create({
  card: {
    marginHorizontal: 16,
    marginTop: 14,
    marginBottom: 24,
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

  noteTitle: {
    color: cumulus.ink,
    fontSize: 14,
    fontWeight: "600",
    fontFamily: cumulusFonts.ui,
    marginBottom: 6,
  },
  noteBody: {
    color: cumulus.inkDim,
    fontSize: 12,
    lineHeight: 18,
    fontFamily: cumulusFonts.ui,
  },
});
