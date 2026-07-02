/**
 * Home-screen nowcast banner — tappable rain headline linking to /nowcast.
 * Extracted from app/(tabs)/index.tsx.
 */
import { memo } from "react";
import { View, Text, StyleSheet, Pressable } from "react-native";
import { useRouter } from "expo-router";
import { cumulus, cumulusFonts } from "../../lib/cumulusTheme";
import WeatherIcon from "../weather/WeatherIcon";

export const NowcastBanner = memo(function NowcastBanner({
  headline,
  sub,
}: {
  headline: string;
  sub: string;
}) {
  const router = useRouter();
  return (
    <Pressable
      style={styles.nowcastBanner}
      onPress={() => router.push("/nowcast" as never)}
    >
      <View style={styles.nowcastIcon}>
        <WeatherIcon kind="rain" size={24} />
      </View>
      <View style={{ flex: 1 }}>
        <Text style={styles.nowcastHeadline}>{headline}</Text>
        <Text style={styles.nowcastSub}>{sub}</Text>
      </View>
      <Text style={styles.chevron}>{"\u203A"}</Text>
    </Pressable>
  );
});

const styles = StyleSheet.create({
  // Nowcast
  nowcastBanner: {
    marginHorizontal: 16,
    marginTop: 12,
    padding: 14,
    borderRadius: 20,
    backgroundColor: "#ffffff",
    borderWidth: 1,
    borderColor: "#eee6d8",
    flexDirection: "row",
    alignItems: "center",
    gap: 12,
    shadowColor: "rgba(60,50,40,0.06)",
    shadowOffset: { width: 0, height: 4 },
    shadowOpacity: 0.4,
    shadowRadius: 8,
    elevation: 2,
  },
  nowcastIcon: {
    width: 34,
    height: 34,
    borderRadius: 10,
    backgroundColor: "rgba(77,127,184,0.12)",
    alignItems: "center",
    justifyContent: "center",
  },
  nowcastHeadline: { color: cumulus.ink, fontSize: 14, fontWeight: "600", fontFamily: cumulusFonts.ui },
  nowcastSub: { color: cumulus.inkDim, fontSize: 12, marginTop: 1, fontFamily: cumulusFonts.ui },
  chevron: { color: cumulus.inkMuted, fontSize: 20, fontWeight: "400" },
});
