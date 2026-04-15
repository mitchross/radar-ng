import { View, Text, StyleSheet } from "react-native";
import type { OpenMeteoResponse } from "../../types/weather";
import { getWeatherInfo } from "../../lib/weatherCodes";

interface Props {
  forecast: OpenMeteoResponse;
}

export function DailyForecast({ forecast }: Props) {
  const { daily } = forecast;

  return (
    <View style={styles.container}>
      <Text style={styles.title}>7-Day Forecast</Text>
      {daily.time.map((date, i) => {
        const weather = getWeatherInfo(daily.weather_code[i]);
        const dayName = i === 0 ? "Today" : new Date(date).toLocaleDateString([], { weekday: "short" });
        return (
          <View key={date} style={styles.row}>
            <Text style={styles.day}>{dayName}</Text>
            <Text style={styles.icon}>{weather.icon}</Text>
            <View style={styles.tempBar}>
              <Text style={styles.low}>{Math.round(daily.temperature_2m_min[i])}{"\u00B0"}</Text>
              <View style={styles.bar} />
              <Text style={styles.high}>{Math.round(daily.temperature_2m_max[i])}{"\u00B0"}</Text>
            </View>
            <Text style={styles.precip}>
              {daily.precipitation_sum[i] > 0 ? `${daily.precipitation_sum[i].toFixed(1)}"` : ""}
            </Text>
          </View>
        );
      })}
    </View>
  );
}

const styles = StyleSheet.create({
  container: { padding: 16, gap: 4 },
  title: { fontSize: 14, fontWeight: "600", color: "#888", textTransform: "uppercase", marginBottom: 8 },
  row: { flexDirection: "row", alignItems: "center", paddingVertical: 10, borderBottomWidth: StyleSheet.hairlineWidth, borderBottomColor: "#333" },
  day: { width: 50, fontSize: 15, color: "#ddd", fontWeight: "500" },
  icon: { width: 30, fontSize: 18, textAlign: "center" },
  tempBar: { flex: 1, flexDirection: "row", alignItems: "center", gap: 8, marginHorizontal: 12 },
  bar: { flex: 1, height: 4, borderRadius: 2, backgroundColor: "rgba(79, 195, 247, 0.3)" },
  low: { fontSize: 14, color: "#4fc3f7", fontWeight: "500", fontVariant: ["tabular-nums"], width: 35, textAlign: "right" },
  high: { fontSize: 14, color: "#ff9800", fontWeight: "500", fontVariant: ["tabular-nums"], width: 35 },
  precip: { width: 40, fontSize: 12, color: "#4fc3f7", textAlign: "right" },
});
