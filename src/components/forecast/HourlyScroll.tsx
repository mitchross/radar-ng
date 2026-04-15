import { ScrollView, View, Text, StyleSheet } from "react-native";
import type { OpenMeteoResponse } from "../../types/weather";
import { getWeatherInfo } from "../../lib/weatherCodes";

interface Props {
  forecast: OpenMeteoResponse;
}

export function HourlyScroll({ forecast }: Props) {
  const { hourly } = forecast;
  const now = new Date();
  const currentHourIndex = hourly.time.findIndex((t) => new Date(t) >= now);
  const startIndex = Math.max(0, currentHourIndex);
  const hours = hourly.time.slice(startIndex, startIndex + 24);

  return (
    <ScrollView
      horizontal
      showsHorizontalScrollIndicator={false}
      contentContainerStyle={styles.scroll}
    >
      {hours.map((time, i) => {
        const idx = startIndex + i;
        const weather = getWeatherInfo(hourly.weather_code[idx]);
        const isNow = i === 0;
        return (
          <View key={time} style={styles.card}>
            <Text style={styles.hour}>
              {isNow ? "Now" : new Date(time).toLocaleTimeString([], { hour: "numeric" })}
            </Text>
            <Text style={styles.icon}>{weather.icon}</Text>
            <Text style={styles.cardTemp}>
              {Math.round(hourly.temperature_2m[idx])}{"\u00B0"}
            </Text>
            <Text style={styles.precip}>
              {hourly.precipitation_probability[idx]}%
            </Text>
            <Text style={styles.wind}>
              {Math.round(hourly.wind_speed_10m[idx])}
            </Text>
          </View>
        );
      })}
    </ScrollView>
  );
}

const styles = StyleSheet.create({
  scroll: {
    paddingHorizontal: 16,
    gap: 4,
  },
  card: {
    width: 64,
    alignItems: "center",
    paddingVertical: 12,
    paddingHorizontal: 4,
    borderRadius: 12,
    backgroundColor: "rgba(255, 255, 255, 0.05)",
    gap: 6,
  },
  hour: {
    fontSize: 12,
    color: "#aaa",
    fontWeight: "500",
  },
  icon: {
    fontSize: 20,
  },
  cardTemp: {
    fontSize: 16,
    color: "#fff",
    fontWeight: "600",
  },
  precip: {
    fontSize: 11,
    color: "#4fc3f7",
  },
  wind: {
    fontSize: 11,
    color: "#888",
  },
});
