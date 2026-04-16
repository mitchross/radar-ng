import { ScrollView, View, Text, StyleSheet } from "react-native";
import { SafeAreaView } from "react-native-safe-area-context";
import { useForecast } from "../../hooks/useForecast";
import { useAlerts } from "../../hooks/useAlerts";
import { useLocation } from "../../hooks/useLocation";
import { getWeatherInfo } from "../../lib/weatherCodes";

export default function ForecastScreen() {
  useLocation();
  const { data: forecast, isLoading } = useForecast();
  const { data: alertData } = useAlerts();

  if (isLoading || !forecast) {
    return (
      <SafeAreaView style={styles.container}>
        <Text style={styles.loading}>Loading weather...</Text>
      </SafeAreaView>
    );
  }

  const weather = getWeatherInfo(forecast.current.weather_code);
  const alerts = alertData?.features ?? [];

  return (
    <SafeAreaView style={styles.container}>
      <ScrollView showsVerticalScrollIndicator={false} contentContainerStyle={styles.scroll}>
        {/* Location header */}
        <Text style={styles.location}>Grand Rapids, MI</Text>

        {/* Hero temperature */}
        <View style={styles.heroSection}>
          <Text style={styles.heroIcon}>{weather.icon}</Text>
          <Text style={styles.heroTemp}>
            {Math.round(forecast.current.temperature_2m)}{"\u00B0"}
          </Text>
          <Text style={styles.heroCondition}>{weather.label}</Text>
          <Text style={styles.heroFeelsLike}>
            Feels like {Math.round(forecast.current.apparent_temperature)}{"\u00B0"}
          </Text>
        </View>

        {/* Alert card */}
        {alerts.length > 0 && (
          <View style={styles.alertCard}>
            <Text style={styles.alertIcon}>{"\u26A0\uFE0F"}</Text>
            <View style={{ flex: 1 }}>
              <Text style={styles.alertTitle}>{alerts[0].properties.event}</Text>
              <Text style={styles.alertSub} numberOfLines={1}>
                Until {new Date(alerts[0].properties.expires).toLocaleTimeString([], { hour: "numeric", minute: "2-digit" })}
              </Text>
            </View>
          </View>
        )}

        {/* Stats row */}
        <View style={styles.statsCard}>
          <StatItem label="High" value={`${Math.round(forecast.daily.temperature_2m_max[0])}\u00B0`} />
          <StatItem label="Low" value={`${Math.round(forecast.daily.temperature_2m_min[0])}\u00B0`} />
          <StatItem label="Wind" value={`${Math.round(forecast.current.wind_speed_10m)}`} unit="mph" />
          <StatItem label="Humidity" value={`${forecast.current.relative_humidity_2m}`} unit="%" />
        </View>

        {/* Hourly forecast card */}
        <View style={styles.card}>
          <Text style={styles.cardTitle}>HOURLY FORECAST</Text>
          <ScrollView horizontal showsHorizontalScrollIndicator={false} contentContainerStyle={styles.hourlyScroll}>
            {forecast.hourly.time.slice(0, 24).map((time, i) => {
              const hr = new Date(time);
              const isNow = i === 0;
              const w = getWeatherInfo(forecast.hourly.weather_code[i]);
              return (
                <View key={time} style={[styles.hourlyItem, isNow && styles.hourlyItemActive]}>
                  <Text style={styles.hourlyTime}>
                    {isNow ? "Now" : hr.toLocaleTimeString([], { hour: "numeric" })}
                  </Text>
                  <Text style={styles.hourlyIcon}>{w.icon}</Text>
                  <Text style={styles.hourlyTemp}>
                    {Math.round(forecast.hourly.temperature_2m[i])}{"\u00B0"}
                  </Text>
                  <Text style={styles.hourlyPrecip}>
                    {forecast.hourly.precipitation_probability[i] > 0
                      ? `${forecast.hourly.precipitation_probability[i]}%`
                      : ""}
                  </Text>
                </View>
              );
            })}
          </ScrollView>
        </View>

        {/* Daily forecast card */}
        <View style={styles.card}>
          <Text style={styles.cardTitle}>7-DAY FORECAST</Text>
          {forecast.daily.time.map((date, i) => {
            const w = getWeatherInfo(forecast.daily.weather_code[i]);
            const dayName = i === 0 ? "Today" : new Date(date).toLocaleDateString([], { weekday: "short" });
            const low = Math.round(forecast.daily.temperature_2m_min[i]);
            const high = Math.round(forecast.daily.temperature_2m_max[i]);
            return (
              <View key={date} style={styles.dailyRow}>
                <Text style={styles.dailyDay}>{dayName}</Text>
                <Text style={styles.dailyIcon}>{w.icon}</Text>
                <Text style={styles.dailyLow}>{low}{"\u00B0"}</Text>
                <View style={styles.dailyBarTrack}>
                  <View style={[styles.dailyBarFill, {
                    left: `${Math.max(0, (low - 30) / 80 * 100)}%`,
                    right: `${Math.max(0, 100 - (high - 30) / 80 * 100)}%`,
                  }]} />
                </View>
                <Text style={styles.dailyHigh}>{high}{"\u00B0"}</Text>
              </View>
            );
          })}
        </View>

        {/* Details card */}
        <View style={styles.card}>
          <Text style={styles.cardTitle}>DETAILS</Text>
          <View style={styles.detailsGrid}>
            <DetailItem label="Sunrise" value={new Date(forecast.daily.sunrise[0]).toLocaleTimeString([], { hour: "numeric", minute: "2-digit" })} />
            <DetailItem label="Sunset" value={new Date(forecast.daily.sunset[0]).toLocaleTimeString([], { hour: "numeric", minute: "2-digit" })} />
            <DetailItem label="Wind Gusts" value={`${Math.round(forecast.current.wind_gusts_10m)} mph`} />
            <DetailItem label="Wind Dir" value={`${forecast.current.wind_direction_10m}\u00B0`} />
          </View>
        </View>

        <View style={{ height: 100 }} />
      </ScrollView>
    </SafeAreaView>
  );
}

function StatItem({ label, value, unit }: { label: string; value: string; unit?: string }) {
  return (
    <View style={styles.statItem}>
      <Text style={styles.statValue}>
        {value}<Text style={styles.statUnit}>{unit}</Text>
      </Text>
      <Text style={styles.statLabel}>{label}</Text>
    </View>
  );
}

function DetailItem({ label, value }: { label: string; value: string }) {
  return (
    <View style={styles.detailItem}>
      <Text style={styles.detailLabel}>{label}</Text>
      <Text style={styles.detailValue}>{value}</Text>
    </View>
  );
}

const styles = StyleSheet.create({
  container: {
    flex: 1,
    backgroundColor: "#0d1117",
  },
  scroll: {
    paddingHorizontal: 16,
  },
  loading: {
    color: "#8b949e",
    fontSize: 16,
    textAlign: "center",
    marginTop: 100,
  },
  location: {
    color: "#8b949e",
    fontSize: 14,
    fontWeight: "600",
    textAlign: "center",
    marginTop: 8,
    letterSpacing: 0.5,
  },
  // Hero
  heroSection: {
    alignItems: "center",
    paddingVertical: 24,
  },
  heroIcon: {
    fontSize: 48,
    marginBottom: 4,
  },
  heroTemp: {
    fontSize: 72,
    fontWeight: "200",
    color: "#fff",
    lineHeight: 80,
  },
  heroCondition: {
    fontSize: 18,
    color: "#c9d1d9",
    fontWeight: "500",
    marginTop: 4,
  },
  heroFeelsLike: {
    fontSize: 14,
    color: "#8b949e",
    marginTop: 2,
  },
  // Alert
  alertCard: {
    flexDirection: "row",
    alignItems: "center",
    backgroundColor: "rgba(244, 67, 54, 0.15)",
    borderWidth: 1,
    borderColor: "rgba(244, 67, 54, 0.3)",
    borderRadius: 12,
    padding: 12,
    gap: 10,
    marginBottom: 16,
  },
  alertIcon: {
    fontSize: 20,
  },
  alertTitle: {
    color: "#f44336",
    fontSize: 14,
    fontWeight: "700",
  },
  alertSub: {
    color: "#ef9a9a",
    fontSize: 12,
    marginTop: 2,
  },
  // Stats
  statsCard: {
    flexDirection: "row",
    backgroundColor: "#161b22",
    borderRadius: 16,
    borderWidth: 1,
    borderColor: "#21262d",
    padding: 16,
    marginBottom: 16,
    justifyContent: "space-around",
  },
  statItem: {
    alignItems: "center",
    gap: 4,
  },
  statValue: {
    color: "#fff",
    fontSize: 20,
    fontWeight: "600",
  },
  statUnit: {
    fontSize: 13,
    color: "#8b949e",
  },
  statLabel: {
    color: "#8b949e",
    fontSize: 11,
    textTransform: "uppercase",
    letterSpacing: 0.5,
  },
  // Cards
  card: {
    backgroundColor: "#161b22",
    borderRadius: 16,
    borderWidth: 1,
    borderColor: "#21262d",
    padding: 16,
    marginBottom: 16,
  },
  cardTitle: {
    color: "#8b949e",
    fontSize: 12,
    fontWeight: "700",
    letterSpacing: 1,
    marginBottom: 12,
  },
  // Hourly
  hourlyScroll: {
    gap: 4,
  },
  hourlyItem: {
    alignItems: "center",
    paddingVertical: 10,
    paddingHorizontal: 10,
    borderRadius: 16,
    minWidth: 58,
    gap: 6,
  },
  hourlyItemActive: {
    backgroundColor: "#1E88E5",
  },
  hourlyTime: {
    color: "#8b949e",
    fontSize: 12,
    fontWeight: "500",
  },
  hourlyIcon: {
    fontSize: 18,
  },
  hourlyTemp: {
    color: "#fff",
    fontSize: 16,
    fontWeight: "600",
  },
  hourlyPrecip: {
    color: "#58a6ff",
    fontSize: 10,
    minHeight: 14,
  },
  // Daily
  dailyRow: {
    flexDirection: "row",
    alignItems: "center",
    paddingVertical: 10,
    gap: 8,
  },
  dailyDay: {
    color: "#c9d1d9",
    fontSize: 15,
    fontWeight: "500",
    width: 48,
  },
  dailyIcon: {
    fontSize: 18,
    width: 28,
    textAlign: "center",
  },
  dailyLow: {
    color: "#58a6ff",
    fontSize: 14,
    fontWeight: "500",
    width: 32,
    textAlign: "right",
    fontVariant: ["tabular-nums"],
  },
  dailyBarTrack: {
    flex: 1,
    height: 4,
    backgroundColor: "#21262d",
    borderRadius: 2,
    overflow: "hidden",
  },
  dailyBarFill: {
    position: "absolute",
    top: 0,
    bottom: 0,
    backgroundColor: "#1E88E5",
    borderRadius: 2,
  },
  dailyHigh: {
    color: "#ff9800",
    fontSize: 14,
    fontWeight: "500",
    width: 32,
    fontVariant: ["tabular-nums"],
  },
  // Details
  detailsGrid: {
    flexDirection: "row",
    flexWrap: "wrap",
  },
  detailItem: {
    width: "50%",
    paddingVertical: 8,
  },
  detailLabel: {
    color: "#8b949e",
    fontSize: 12,
    marginBottom: 2,
  },
  detailValue: {
    color: "#fff",
    fontSize: 16,
    fontWeight: "500",
  },
});
