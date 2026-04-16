import { ScrollView, View, Text, StyleSheet, Dimensions } from "react-native";
import { SafeAreaView } from "react-native-safe-area-context";
import { LinearGradient } from "expo-linear-gradient";
import { useForecast } from "../../hooks/useForecast";
import { useAlerts } from "../../hooks/useAlerts";
import { useLocation } from "../../hooks/useLocation";
import { getWeatherInfo } from "../../lib/weatherCodes";
import {
  getWeatherTheme,
  getTempFontWeight,
  getTempColor,
  getWeatherQuip,
} from "../../lib/weatherTheme";

const SCREEN_WIDTH = Dimensions.get("window").width;

export default function ForecastScreen() {
  useLocation();
  const { data: forecast, isLoading } = useForecast();
  const { data: alertData } = useAlerts();

  if (isLoading || !forecast) {
    return (
      <LinearGradient colors={["#1A237E", "#283593", "#3949AB"]} style={styles.container}>
        <SafeAreaView style={styles.flex}>
          <Text style={styles.loading}>Loading weather...</Text>
        </SafeAreaView>
      </LinearGradient>
    );
  }

  const weather = getWeatherInfo(forecast.current.weather_code);
  const alerts = alertData?.features ?? [];
  const temp = Math.round(forecast.current.temperature_2m);
  const feelsLike = Math.round(forecast.current.apparent_temperature);

  // Determine day/night from sunrise/sunset
  const now = new Date();
  const sunrise = new Date(forecast.daily.sunrise[0]);
  const sunset = new Date(forecast.daily.sunset[0]);
  const isNight = now < sunrise || now > sunset;

  // Weather-adaptive theme
  const theme = getWeatherTheme(forecast.current.weather_code, isNight);
  const quip = getWeatherQuip(forecast.current.weather_code, isNight);
  const tempWeight = getTempFontWeight(temp);

  // Hourly temps for chart line
  const hourlyTemps = forecast.hourly.temperature_2m.slice(0, 24);
  const minHourly = Math.min(...hourlyTemps);
  const maxHourly = Math.max(...hourlyTemps);
  const hourlyRange = maxHourly - minHourly || 1;

  return (
    <LinearGradient colors={theme.gradient} style={styles.container}>
      <SafeAreaView style={styles.flex}>
        <ScrollView showsVerticalScrollIndicator={false} contentContainerStyle={styles.scroll}>
          {/* Location header */}
          <Text style={[styles.location, { color: theme.textSecondary }]}>
            Grand Rapids, MI
          </Text>

          {/* Hero temperature — CARROT style with adaptive weight */}
          <View style={styles.heroSection}>
            <Text style={styles.heroIcon}>{weather.icon}</Text>
            <Text
              style={[
                styles.heroTemp,
                { fontWeight: tempWeight, color: theme.textPrimary },
              ]}
            >
              {temp}{"\u00B0"}
            </Text>
            <Text style={[styles.heroCondition, { color: theme.textPrimary }]}>
              {weather.label}
            </Text>
            <Text style={[styles.heroFeelsLike, { color: theme.textSecondary }]}>
              Feels like {feelsLike}{"\u00B0"}
            </Text>
          </View>

          {/* Snarky personality quote */}
          <View style={[styles.quipCard, { backgroundColor: theme.cardBg, borderColor: theme.cardBorder }]}>
            <Text style={[styles.quipText, { color: theme.textPrimary }]}>
              {quip}
            </Text>
          </View>

          {/* Alert card */}
          {alerts.length > 0 && (
            <View style={styles.alertCard}>
              <Text style={styles.alertIcon}>{"\u26A0\uFE0F"}</Text>
              <View style={styles.alertContent}>
                <Text style={styles.alertTitle}>{alerts[0].properties.event}</Text>
                <Text style={styles.alertSub} numberOfLines={1}>
                  Until{" "}
                  {new Date(alerts[0].properties.expires).toLocaleTimeString(
                    [],
                    { hour: "numeric", minute: "2-digit" }
                  )}
                </Text>
              </View>
            </View>
          )}

          {/* Stats row — glassmorphism card */}
          <View style={[styles.glassCard, { backgroundColor: theme.cardBg, borderColor: theme.cardBorder }]}>
            <View style={styles.statsRow}>
              <StatItem
                label="High"
                value={`${Math.round(forecast.daily.temperature_2m_max[0])}\u00B0`}
                color={getTempColor(forecast.daily.temperature_2m_max[0])}
              />
              <View style={[styles.statDivider, { backgroundColor: theme.cardBorder }]} />
              <StatItem
                label="Low"
                value={`${Math.round(forecast.daily.temperature_2m_min[0])}\u00B0`}
                color={getTempColor(forecast.daily.temperature_2m_min[0])}
              />
              <View style={[styles.statDivider, { backgroundColor: theme.cardBorder }]} />
              <StatItem
                label="Wind"
                value={`${Math.round(forecast.current.wind_speed_10m)}`}
                unit="mph"
                color={theme.textPrimary}
              />
              <View style={[styles.statDivider, { backgroundColor: theme.cardBorder }]} />
              <StatItem
                label="Humidity"
                value={`${forecast.current.relative_humidity_2m}`}
                unit="%"
                color={theme.textPrimary}
              />
            </View>
          </View>

          {/* Hourly forecast — chart style with temp line */}
          <View style={[styles.glassCard, { backgroundColor: theme.cardBg, borderColor: theme.cardBorder }]}>
            <Text style={[styles.cardTitle, { color: theme.textSecondary }]}>
              HOURLY FORECAST
            </Text>
            <ScrollView
              horizontal
              showsHorizontalScrollIndicator={false}
              contentContainerStyle={styles.hourlyScroll}
            >
              {forecast.hourly.time.slice(0, 24).map((time, i) => {
                const hr = new Date(time);
                const isNow = i === 0;
                const w = getWeatherInfo(forecast.hourly.weather_code[i]);
                const t = Math.round(hourlyTemps[i]);
                const pct = (hourlyTemps[i] - minHourly) / hourlyRange;
                // Temperature dot position (higher temp = higher on chart)
                const dotBottom = 4 + pct * 36;
                const precip = forecast.hourly.precipitation_probability[i];

                return (
                  <View key={time} style={styles.hourlyItem}>
                    <Text
                      style={[
                        styles.hourlyTime,
                        { color: isNow ? theme.accent : theme.textSecondary },
                      ]}
                    >
                      {isNow ? "Now" : hr.toLocaleTimeString([], { hour: "numeric" })}
                    </Text>
                    <Text style={styles.hourlyWeatherIcon}>{w.icon}</Text>
                    {/* Temperature dot chart */}
                    <View style={styles.hourlyChartArea}>
                      <View
                        style={[
                          styles.hourlyDot,
                          {
                            bottom: dotBottom,
                            backgroundColor: getTempColor(t),
                          },
                        ]}
                      />
                      {/* Line to next dot */}
                      {i < 23 && (
                        <View
                          style={[
                            styles.hourlyLine,
                            {
                              bottom: dotBottom + 3,
                              transform: [
                                {
                                  rotate: `${Math.atan2(
                                    -((forecast.hourly.temperature_2m[i + 1] - minHourly) / hourlyRange * 36 -
                                      pct * 36),
                                    52
                                  ) * (180 / Math.PI)}deg`,
                                },
                              ],
                              backgroundColor: getTempColor(t),
                            },
                          ]}
                        />
                      )}
                    </View>
                    <Text style={[styles.hourlyTemp, { color: getTempColor(t) }]}>
                      {t}{"\u00B0"}
                    </Text>
                    <Text
                      style={[
                        styles.hourlyPrecip,
                        { color: precip > 0 ? "#42A5F5" : "transparent" },
                      ]}
                    >
                      {precip > 0 ? `${precip}%` : "-"}
                    </Text>
                  </View>
                );
              })}
            </ScrollView>
          </View>

          {/* 7-Day forecast */}
          <View style={[styles.glassCard, { backgroundColor: theme.cardBg, borderColor: theme.cardBorder }]}>
            <Text style={[styles.cardTitle, { color: theme.textSecondary }]}>
              7-DAY FORECAST
            </Text>
            {forecast.daily.time.map((date, i) => {
              const w = getWeatherInfo(forecast.daily.weather_code[i]);
              const dayName =
                i === 0
                  ? "Today"
                  : new Date(date).toLocaleDateString([], {
                      weekday: "short",
                    });
              const low = Math.round(forecast.daily.temperature_2m_min[i]);
              const high = Math.round(forecast.daily.temperature_2m_max[i]);
              // Calculate bar positions relative to the weekly range
              const weekMin = Math.min(...forecast.daily.temperature_2m_min);
              const weekMax = Math.max(...forecast.daily.temperature_2m_max);
              const weekRange = weekMax - weekMin || 1;
              const barLeft = ((low - weekMin) / weekRange) * 100;
              const barRight = 100 - ((high - weekMin) / weekRange) * 100;

              return (
                <View key={date} style={styles.dailyRow}>
                  <Text style={[styles.dailyDay, { color: theme.textPrimary }]}>
                    {dayName}
                  </Text>
                  <Text style={styles.dailyIcon}>{w.icon}</Text>
                  <Text style={[styles.dailyLow, { color: getTempColor(low) }]}>
                    {low}{"\u00B0"}
                  </Text>
                  <View style={[styles.dailyBarTrack, { backgroundColor: "rgba(255,255,255,0.1)" }]}>
                    <LinearGradient
                      colors={[getTempColor(low), getTempColor(high)]}
                      start={{ x: 0, y: 0 }}
                      end={{ x: 1, y: 0 }}
                      style={[
                        styles.dailyBarFill,
                        {
                          left: `${Math.max(0, barLeft)}%`,
                          right: `${Math.max(0, barRight)}%`,
                        },
                      ]}
                    />
                  </View>
                  <Text style={[styles.dailyHigh, { color: getTempColor(high) }]}>
                    {high}{"\u00B0"}
                  </Text>
                </View>
              );
            })}
          </View>

          {/* Details card */}
          <View style={[styles.glassCard, { backgroundColor: theme.cardBg, borderColor: theme.cardBorder }]}>
            <Text style={[styles.cardTitle, { color: theme.textSecondary }]}>
              DETAILS
            </Text>
            <View style={styles.detailsGrid}>
              <DetailItem
                icon={"\u2600\uFE0F"}
                label="Sunrise"
                value={sunrise.toLocaleTimeString([], {
                  hour: "numeric",
                  minute: "2-digit",
                })}
                theme={theme}
              />
              <DetailItem
                icon={"\uD83C\uDF05"}
                label="Sunset"
                value={sunset.toLocaleTimeString([], {
                  hour: "numeric",
                  minute: "2-digit",
                })}
                theme={theme}
              />
              <DetailItem
                icon={"\uD83D\uDCA8"}
                label="Wind Gusts"
                value={`${Math.round(forecast.current.wind_gusts_10m)} mph`}
                theme={theme}
              />
              <DetailItem
                icon={"\uD83E\uDDED"}
                label="Wind Dir"
                value={`${forecast.current.wind_direction_10m}\u00B0`}
                theme={theme}
              />
            </View>
          </View>

          <View style={{ height: 100 }} />
        </ScrollView>
      </SafeAreaView>
    </LinearGradient>
  );
}

function StatItem({
  label,
  value,
  unit,
  color,
}: {
  label: string;
  value: string;
  unit?: string;
  color: string;
}) {
  return (
    <View style={styles.statItem}>
      <Text style={[styles.statValue, { color }]}>
        {value}
        {unit && <Text style={styles.statUnit}>{unit}</Text>}
      </Text>
      <Text style={styles.statLabel}>{label}</Text>
    </View>
  );
}

function DetailItem({
  icon,
  label,
  value,
  theme,
}: {
  icon: string;
  label: string;
  value: string;
  theme: ReturnType<typeof getWeatherTheme>;
}) {
  return (
    <View style={styles.detailItem}>
      <Text style={styles.detailIcon}>{icon}</Text>
      <Text style={[styles.detailLabel, { color: theme.textSecondary }]}>
        {label}
      </Text>
      <Text style={[styles.detailValue, { color: theme.textPrimary }]}>
        {value}
      </Text>
    </View>
  );
}

const styles = StyleSheet.create({
  container: {
    flex: 1,
  },
  flex: {
    flex: 1,
  },
  scroll: {
    paddingHorizontal: 20,
  },
  loading: {
    color: "rgba(255,255,255,0.6)",
    fontSize: 16,
    textAlign: "center",
    marginTop: 100,
  },
  // Location
  location: {
    fontSize: 16,
    fontWeight: "600",
    textAlign: "center",
    marginTop: 8,
    letterSpacing: 0.5,
  },
  // Hero
  heroSection: {
    alignItems: "center",
    paddingVertical: 20,
  },
  heroIcon: {
    fontSize: 56,
    marginBottom: 4,
  },
  heroTemp: {
    fontSize: 96,
    lineHeight: 104,
    letterSpacing: -4,
  },
  heroCondition: {
    fontSize: 20,
    fontWeight: "500",
    marginTop: 2,
  },
  heroFeelsLike: {
    fontSize: 15,
    marginTop: 4,
  },
  // Personality quote
  quipCard: {
    borderRadius: 16,
    borderWidth: 1,
    paddingVertical: 12,
    paddingHorizontal: 16,
    marginBottom: 16,
    alignItems: "center",
  },
  quipText: {
    fontSize: 14,
    fontStyle: "italic",
    textAlign: "center",
    lineHeight: 20,
  },
  // Alert
  alertCard: {
    flexDirection: "row",
    alignItems: "center",
    backgroundColor: "rgba(244, 67, 54, 0.2)",
    borderWidth: 1,
    borderColor: "rgba(244, 67, 54, 0.4)",
    borderRadius: 16,
    padding: 14,
    gap: 12,
    marginBottom: 16,
  },
  alertIcon: {
    fontSize: 22,
  },
  alertContent: {
    flex: 1,
  },
  alertTitle: {
    color: "#FF8A80",
    fontSize: 15,
    fontWeight: "700",
  },
  alertSub: {
    color: "#EF9A9A",
    fontSize: 13,
    marginTop: 2,
  },
  // Glass cards
  glassCard: {
    borderRadius: 20,
    borderWidth: 1,
    padding: 18,
    marginBottom: 16,
  },
  cardTitle: {
    fontSize: 13,
    fontWeight: "700",
    letterSpacing: 1.2,
    marginBottom: 14,
  },
  // Stats
  statsRow: {
    flexDirection: "row",
    justifyContent: "space-around",
    alignItems: "center",
  },
  statItem: {
    alignItems: "center",
    flex: 1,
    gap: 4,
  },
  statDivider: {
    width: 1,
    height: 32,
  },
  statValue: {
    fontSize: 22,
    fontWeight: "700",
  },
  statUnit: {
    fontSize: 14,
    fontWeight: "400",
    opacity: 0.7,
  },
  statLabel: {
    color: "rgba(255,255,255,0.5)",
    fontSize: 11,
    textTransform: "uppercase",
    letterSpacing: 0.5,
    fontWeight: "600",
  },
  // Hourly
  hourlyScroll: {
    gap: 0,
  },
  hourlyItem: {
    alignItems: "center",
    width: 56,
    gap: 4,
  },
  hourlyTime: {
    fontSize: 12,
    fontWeight: "600",
  },
  hourlyWeatherIcon: {
    fontSize: 16,
  },
  hourlyChartArea: {
    height: 48,
    width: 56,
    position: "relative",
  },
  hourlyDot: {
    position: "absolute",
    left: 24,
    width: 8,
    height: 8,
    borderRadius: 4,
  },
  hourlyLine: {
    position: "absolute",
    left: 28,
    width: 52,
    height: 2,
    borderRadius: 1,
    opacity: 0.4,
    transformOrigin: "left center",
  },
  hourlyTemp: {
    fontSize: 15,
    fontWeight: "700",
  },
  hourlyPrecip: {
    fontSize: 11,
    fontWeight: "500",
    minHeight: 14,
  },
  // Daily
  dailyRow: {
    flexDirection: "row",
    alignItems: "center",
    paddingVertical: 11,
    gap: 10,
  },
  dailyDay: {
    fontSize: 15,
    fontWeight: "500",
    width: 52,
  },
  dailyIcon: {
    fontSize: 18,
    width: 28,
    textAlign: "center",
  },
  dailyLow: {
    fontSize: 15,
    fontWeight: "600",
    width: 36,
    textAlign: "right",
    fontVariant: ["tabular-nums"],
  },
  dailyBarTrack: {
    flex: 1,
    height: 6,
    borderRadius: 3,
    overflow: "hidden",
  },
  dailyBarFill: {
    position: "absolute",
    top: 0,
    bottom: 0,
    borderRadius: 3,
  },
  dailyHigh: {
    fontSize: 15,
    fontWeight: "600",
    width: 36,
    fontVariant: ["tabular-nums"],
  },
  // Details
  detailsGrid: {
    flexDirection: "row",
    flexWrap: "wrap",
  },
  detailItem: {
    width: "50%",
    paddingVertical: 10,
    paddingHorizontal: 4,
  },
  detailIcon: {
    fontSize: 18,
    marginBottom: 4,
  },
  detailLabel: {
    fontSize: 12,
    marginBottom: 2,
  },
  detailValue: {
    fontSize: 18,
    fontWeight: "600",
  },
});
