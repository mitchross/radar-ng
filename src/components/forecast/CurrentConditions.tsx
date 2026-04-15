import { View, Text, StyleSheet } from "react-native";
import type { OpenMeteoResponse } from "../../types/weather";
import { getWeatherInfo } from "../../lib/weatherCodes";

interface Props {
  forecast: OpenMeteoResponse;
}

export function CurrentConditions({ forecast }: Props) {
  const { current, daily } = forecast;
  const weather = getWeatherInfo(current.weather_code);
  const high = daily.temperature_2m_max[0];
  const low = daily.temperature_2m_min[0];

  return (
    <View style={styles.container}>
      <View style={styles.mainRow}>
        <Text style={styles.temp}>{Math.round(current.temperature_2m)}{"\u00B0"}</Text>
        <View style={styles.details}>
          <Text style={styles.condition}>
            {weather.icon} {weather.label}
          </Text>
          <Text style={styles.highLow}>
            H:{Math.round(high)}{"\u00B0"} L:{Math.round(low)}{"\u00B0"}
          </Text>
        </View>
      </View>
      <View style={styles.statsRow}>
        <Stat label="Feels Like" value={`${Math.round(current.apparent_temperature)}\u00B0`} />
        <Stat label="Wind" value={`${Math.round(current.wind_speed_10m)} mph`} />
        <Stat label="Humidity" value={`${current.relative_humidity_2m}%`} />
        <Stat label="Gusts" value={`${Math.round(current.wind_gusts_10m)} mph`} />
      </View>
    </View>
  );
}

function Stat({ label, value }: { label: string; value: string }) {
  return (
    <View style={styles.stat}>
      <Text style={styles.statLabel}>{label}</Text>
      <Text style={styles.statValue}>{value}</Text>
    </View>
  );
}

const styles = StyleSheet.create({
  container: {
    padding: 16,
    gap: 16,
  },
  mainRow: {
    flexDirection: "row",
    alignItems: "center",
    gap: 16,
  },
  temp: {
    fontSize: 48,
    fontWeight: "200",
    color: "#fff",
  },
  details: {
    gap: 4,
  },
  condition: {
    fontSize: 16,
    color: "#ccc",
  },
  highLow: {
    fontSize: 14,
    color: "#999",
  },
  statsRow: {
    flexDirection: "row",
    justifyContent: "space-between",
  },
  stat: {
    alignItems: "center",
    gap: 4,
  },
  statLabel: {
    fontSize: 11,
    color: "#888",
    textTransform: "uppercase",
  },
  statValue: {
    fontSize: 15,
    color: "#fff",
    fontWeight: "500",
  },
});
