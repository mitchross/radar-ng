/**
 * Home-screen active-alert card — first NWS alert, links to detail page.
 * Extracted from app/(tabs)/index.tsx.
 */
import { memo } from "react";
import { View, Text, StyleSheet, TouchableOpacity } from "react-native";
import { useRouter } from "expo-router";
import { cumulus, cumulusFonts } from "../../lib/cumulusTheme";

export const AlertsCard = memo(function AlertsCard({
  id,
  event,
  expires,
}: {
  id: string;
  event: string;
  expires: string;
}) {
  const router = useRouter();
  return (
    <TouchableOpacity
      style={styles.alertCard}
      activeOpacity={0.8}
      onPress={() =>
        router.push({
          pathname: "/alert/[id]",
          params: { id },
        })
      }
    >
      <View style={styles.alertIndicatorDot} />
      <View style={{ flex: 1 }}>
        <Text style={styles.alertTitle}>{event}</Text>
        <Text style={styles.alertSub} numberOfLines={1}>
          Until{" "}
          {new Date(expires).toLocaleTimeString([], {
            hour: "numeric",
            minute: "2-digit",
          })}
        </Text>
      </View>
      <Text style={styles.chevron}>{"\u203A"}</Text>
    </TouchableOpacity>
  );
});

const styles = StyleSheet.create({
  // Alerts card
  alertCard: {
    marginHorizontal: 16,
    marginTop: 10,
    padding: 14,
    borderRadius: 20,
    backgroundColor: "rgba(223,106,106,0.12)",
    borderWidth: 1,
    borderColor: "rgba(223,106,106,0.3)",
    flexDirection: "row",
    alignItems: "center",
    gap: 12,
  },
  alertIndicatorDot: { width: 8, height: 8, borderRadius: 4, backgroundColor: cumulus.alert },
  alertTitle: { color: cumulus.alert, fontSize: 14, fontWeight: "700", fontFamily: cumulusFonts.ui },
  alertSub: { color: cumulus.inkDim, fontSize: 12, marginTop: 1, fontFamily: cumulusFonts.ui },
  chevron: { color: cumulus.inkMuted, fontSize: 20, fontWeight: "400" },
});
