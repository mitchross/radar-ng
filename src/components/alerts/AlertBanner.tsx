import { TouchableOpacity, Text, StyleSheet } from "react-native";
import { useRouter } from "expo-router";
import { useAlerts } from "../../hooks/useAlerts";
import type { NWSAlert } from "../../types/weather";

const SEVERITY_COLORS: Record<string, string> = {
  Extreme: "#d32f2f",
  Severe: "#f44336",
  Moderate: "#ff9800",
  Minor: "#ffc107",
  Unknown: "#9e9e9e",
};

export function AlertBanner() {
  const { data: alertData } = useAlerts();
  const router = useRouter();

  if (!alertData || alertData.features.length === 0) return null;

  const alert = alertData.features.reduce<NWSAlert>((worst, current) => {
    const severityOrder = ["Extreme", "Severe", "Moderate", "Minor", "Unknown"];
    const worstIdx = severityOrder.indexOf(worst.properties.severity);
    const currentIdx = severityOrder.indexOf(current.properties.severity);
    return currentIdx < worstIdx ? current : worst;
  }, alertData.features[0]);

  const bgColor = SEVERITY_COLORS[alert.properties.severity] ?? "#9e9e9e";

  return (
    <TouchableOpacity
      style={[styles.banner, { backgroundColor: bgColor }]}
      onPress={() =>
        router.push({
          pathname: "/alert/[id]",
          params: { id: alert.properties.id },
        })
      }
      activeOpacity={0.8}
    >
      <Text style={styles.text} numberOfLines={1}>
        {"\u26A0\uFE0F"} {alert.properties.event}
        {alert.properties.headline ? ` \u2014 ${alert.properties.headline}` : ""}
      </Text>
    </TouchableOpacity>
  );
}

const styles = StyleSheet.create({
  banner: {
    position: "absolute",
    top: 0,
    left: 0,
    right: 0,
    paddingTop: 36,
    paddingBottom: 8,
    paddingHorizontal: 12,
    zIndex: 100,
  },
  text: {
    color: "#fff",
    fontSize: 12,
    fontWeight: "600",
  },
});
