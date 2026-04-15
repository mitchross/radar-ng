import { ScrollView, Text, StyleSheet } from "react-native";
import { useLocalSearchParams } from "expo-router";
import { useAlerts } from "../../hooks/useAlerts";

export default function AlertDetailScreen() {
  const { id } = useLocalSearchParams<{ id: string }>();
  const { data: alertData } = useAlerts();

  const alert = alertData?.features.find((f) => f.properties.id === id);

  if (!alert) {
    return (
      <ScrollView style={styles.container}>
        <Text style={styles.body}>Alert not found.</Text>
      </ScrollView>
    );
  }

  return (
    <ScrollView style={styles.container}>
      <Text style={styles.event}>{alert.properties.event}</Text>
      <Text style={styles.area}>{alert.properties.areaDesc}</Text>
      <Text style={styles.expires}>
        Expires: {new Date(alert.properties.expires).toLocaleString()}
      </Text>
      <Text style={styles.body}>{alert.properties.description}</Text>
    </ScrollView>
  );
}

const styles = StyleSheet.create({
  container: {
    flex: 1,
    backgroundColor: "#1a1a2e",
    padding: 16,
  },
  event: {
    fontSize: 22,
    fontWeight: "700",
    color: "#fff",
    marginBottom: 8,
  },
  area: {
    fontSize: 14,
    color: "#aaa",
    marginBottom: 4,
  },
  expires: {
    fontSize: 13,
    color: "#f44336",
    marginBottom: 16,
  },
  body: {
    fontSize: 15,
    color: "#ddd",
    lineHeight: 22,
  },
});
