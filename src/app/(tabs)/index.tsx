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
  getWindInfo,
  getUVInfo,
  getWindDirection,
} from "../../lib/weatherTheme";
import WeatherScene from "../../components/weather/WeatherScene";

const SCREEN_WIDTH = Dimensions.get("window").width;

export default function ForecastScreen() {
  useLocation();
  const { data: forecast, isLoading } = useForecast();
  const { data: alertData } = useAlerts();

  if (isLoading || !forecast) {
    return (
      <LinearGradient colors={["#0A0E2A", "#151B4A", "#1F2768", "#2A3480", "#354196"] as const} style={styles.container}>
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

  // Hourly data
  const hourlyTemps = forecast.hourly.temperature_2m.slice(0, 24);
  const minHourly = Math.min(...hourlyTemps);
  const maxHourly = Math.max(...hourlyTemps);
  const hourlyRange = maxHourly - minHourly || 1;

  // Wind info
  const windSpeed = Math.round(forecast.current.wind_speed_10m);
  const windDir = forecast.current.wind_direction_10m;
  const windGust = Math.round(forecast.current.wind_gusts_10m);
  const windInfo = getWindInfo(windSpeed);
  const windCompass = getWindDirection(windDir);

  // UV (from daily since current doesn't have it)
  const uvIndex = forecast.daily.uv_index_max?.[0] ?? 0;
  const uvInfo = getUVInfo(uvIndex);

  // Daily data for weekly range
  const weekMin = Math.min(...forecast.daily.temperature_2m_min);
  const weekMax = Math.max(...forecast.daily.temperature_2m_max);
  const weekRange = weekMax - weekMin || 1;

  return (
    <LinearGradient colors={theme.gradient} style={styles.container}>
      <SafeAreaView style={styles.flex} edges={["top"]}>
        <ScrollView showsVerticalScrollIndicator={false} contentContainerStyle={styles.scroll}>
          {/* Location header */}
          <Text style={[styles.location, { color: theme.textSecondary }]}>
            Grand Rapids, MI
          </Text>

          {/* Weather scene illustration */}
          <WeatherScene theme={theme} />

          {/* Hero temperature */}
          <View style={styles.heroSection}>
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
              Feels like {feelsLike}{"\u00B0"} {"\u00B7"} H:{Math.round(forecast.daily.temperature_2m_max[0])}{"\u00B0"} L:{Math.round(forecast.daily.temperature_2m_min[0])}{"\u00B0"}
            </Text>
          </View>

          {/* Snarky personality quote */}
          <View style={[styles.quipCard, { backgroundColor: theme.cardBg, borderColor: theme.cardBorder }]}>
            <Text style={[styles.quipText, { color: theme.textPrimary }]}>
              "{quip}"
            </Text>
          </View>

          {/* Alert card */}
          {alerts.length > 0 && (
            <View style={styles.alertCard}>
              <View style={styles.alertDot} />
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
              <Text style={styles.alertChevron}>{"\u203A"}</Text>
            </View>
          )}

          {/* Hourly forecast — chart style */}
          <View style={[styles.glassCard, { backgroundColor: theme.cardBg, borderColor: theme.cardBorder }]}>
            <Text style={[styles.sectionTitle, { color: theme.textSecondary }]}>
              HOURLY
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
                const dotBottom = 6 + pct * 32;
                const precip = forecast.hourly.precipitation_probability[i];

                return (
                  <View key={time} style={styles.hourlyItem}>
                    <Text
                      style={[
                        styles.hourlyTime,
                        { color: isNow ? theme.accent : theme.textSecondary },
                        isNow && styles.hourlyTimeNow,
                      ]}
                    >
                      {isNow ? "Now" : hr.toLocaleTimeString([], { hour: "numeric" })}
                    </Text>
                    {precip > 0 && (
                      <Text style={styles.hourlyPrecip}>
                        {precip}%
                      </Text>
                    )}
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
                      {i < 23 && (
                        <View
                          style={[
                            styles.hourlyLine,
                            {
                              bottom: dotBottom + 3,
                              transform: [
                                {
                                  rotate: `${Math.atan2(
                                    -((forecast.hourly.temperature_2m[i + 1] - minHourly) / hourlyRange * 32 -
                                      pct * 32),
                                    50
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
                  </View>
                );
              })}
            </ScrollView>
          </View>

          {/* 7-Day forecast */}
          <View style={[styles.glassCard, { backgroundColor: theme.cardBg, borderColor: theme.cardBorder }]}>
            <Text style={[styles.sectionTitle, { color: theme.textSecondary }]}>
              7-DAY FORECAST
            </Text>
            {forecast.daily.time.map((date, i) => {
              const w = getWeatherInfo(forecast.daily.weather_code[i]);
              const dayName =
                i === 0
                  ? "Today"
                  : new Date(date).toLocaleDateString([], { weekday: "short" });
              const low = Math.round(forecast.daily.temperature_2m_min[i]);
              const high = Math.round(forecast.daily.temperature_2m_max[i]);
              const barLeft = ((low - weekMin) / weekRange) * 100;
              const barRight = 100 - ((high - weekMin) / weekRange) * 100;
              const dailyPrecip = forecast.daily.precipitation_probability_max?.[i];

              return (
                <View key={date} style={styles.dailyRow}>
                  <Text style={[styles.dailyDay, { color: theme.textPrimary }]}>
                    {dayName}
                  </Text>
                  <Text style={styles.dailyIcon}>{w.icon}</Text>
                  {dailyPrecip != null && dailyPrecip > 0 ? (
                    <Text style={styles.dailyPrecip}>{dailyPrecip}%</Text>
                  ) : (
                    <Text style={[styles.dailyPrecip, { color: "transparent" }]}>0%</Text>
                  )}
                  <Text style={[styles.dailyLow, { color: getTempColor(low) }]}>
                    {low}{"\u00B0"}
                  </Text>
                  <View style={styles.dailyBarTrack}>
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

          {/* Wind & Pressure card */}
          <View style={[styles.glassCard, { backgroundColor: theme.cardBg, borderColor: theme.cardBorder }]}>
            <Text style={[styles.sectionTitle, { color: theme.textSecondary }]}>
              WIND & PRESSURE
            </Text>
            <View style={styles.windRow}>
              {/* Wind compass */}
              <View style={styles.compassContainer}>
                <View style={[styles.compassRing, { borderColor: theme.cardBorder }]}>
                  {/* Direction labels */}
                  <Text style={[styles.compassLabel, styles.compassN, { color: theme.textSecondary }]}>N</Text>
                  <Text style={[styles.compassLabel, styles.compassE, { color: theme.textSecondary }]}>E</Text>
                  <Text style={[styles.compassLabel, styles.compassS, { color: theme.textSecondary }]}>S</Text>
                  <Text style={[styles.compassLabel, styles.compassW, { color: theme.textSecondary }]}>W</Text>
                  {/* Arrow */}
                  <View style={[styles.compassArrowContainer, { transform: [{ rotate: `${windDir}deg` }] }]}>
                    <View style={[styles.compassArrow, { backgroundColor: windInfo.color }]} />
                    <View style={[styles.compassArrowHead, { borderBottomColor: windInfo.color }]} />
                  </View>
                  <View style={[styles.compassCenter, { backgroundColor: windInfo.color }]} />
                </View>
              </View>
              {/* Wind stats */}
              <View style={styles.windStats}>
                <View style={styles.windStatRow}>
                  <Text style={[styles.windStatLabel, { color: theme.textSecondary }]}>Speed</Text>
                  <Text style={[styles.windStatValue, { color: theme.textPrimary }]}>
                    {windSpeed} <Text style={styles.windStatUnit}>mph</Text>
                  </Text>
                </View>
                <View style={[styles.windDivider, { backgroundColor: theme.cardBorder }]} />
                <View style={styles.windStatRow}>
                  <Text style={[styles.windStatLabel, { color: theme.textSecondary }]}>Gusts</Text>
                  <Text style={[styles.windStatValue, { color: theme.textPrimary }]}>
                    {windGust} <Text style={styles.windStatUnit}>mph</Text>
                  </Text>
                </View>
                <View style={[styles.windDivider, { backgroundColor: theme.cardBorder }]} />
                <View style={styles.windStatRow}>
                  <Text style={[styles.windStatLabel, { color: theme.textSecondary }]}>Direction</Text>
                  <Text style={[styles.windStatValue, { color: theme.textPrimary }]}>
                    {windCompass} <Text style={styles.windStatUnit}>{windDir}{"\u00B0"}</Text>
                  </Text>
                </View>
                <View style={[styles.windDivider, { backgroundColor: theme.cardBorder }]} />
                <View style={styles.windStatRow}>
                  <Text style={[styles.windStatLabel, { color: theme.textSecondary }]}>Status</Text>
                  <Text style={[styles.windStatValue, { color: windInfo.color }]}>
                    {windInfo.label}
                  </Text>
                </View>
              </View>
            </View>
          </View>

          {/* Sun & Moon card */}
          <View style={[styles.glassCard, { backgroundColor: theme.cardBg, borderColor: theme.cardBorder }]}>
            <Text style={[styles.sectionTitle, { color: theme.textSecondary }]}>
              SUN & MOON
            </Text>
            <View style={styles.sunMoonRow}>
              <View style={styles.sunMoonItem}>
                <View style={[styles.sunMoonIcon, { backgroundColor: "rgba(255,213,79,0.15)" }]}>
                  <View style={styles.miniSun} />
                </View>
                <Text style={[styles.sunMoonLabel, { color: theme.textSecondary }]}>Sunrise</Text>
                <Text style={[styles.sunMoonValue, { color: theme.textPrimary }]}>
                  {sunrise.toLocaleTimeString([], { hour: "numeric", minute: "2-digit" })}
                </Text>
              </View>
              <View style={styles.sunMoonItem}>
                <View style={[styles.sunMoonIcon, { backgroundColor: "rgba(255,152,0,0.15)" }]}>
                  <View style={styles.miniSunset} />
                </View>
                <Text style={[styles.sunMoonLabel, { color: theme.textSecondary }]}>Sunset</Text>
                <Text style={[styles.sunMoonValue, { color: theme.textPrimary }]}>
                  {sunset.toLocaleTimeString([], { hour: "numeric", minute: "2-digit" })}
                </Text>
              </View>
              <View style={styles.sunMoonItem}>
                <View style={[styles.sunMoonIcon, { backgroundColor: "rgba(66,165,245,0.15)" }]}>
                  <Text style={styles.sunMoonEmoji}>{"\u2600\uFE0F"}</Text>
                </View>
                <Text style={[styles.sunMoonLabel, { color: theme.textSecondary }]}>Daylight</Text>
                <Text style={[styles.sunMoonValue, { color: theme.textPrimary }]}>
                  {Math.floor((sunset.getTime() - sunrise.getTime()) / 3600000)}h{" "}
                  {Math.round(((sunset.getTime() - sunrise.getTime()) % 3600000) / 60000)}m
                </Text>
              </View>
            </View>
            {/* Daylight progress arc */}
            <View style={[styles.daylightBar, { backgroundColor: "rgba(255,255,255,0.06)" }]}>
              <View
                style={[
                  styles.daylightProgress,
                  {
                    width: `${Math.max(0, Math.min(100,
                      ((now.getTime() - sunrise.getTime()) / (sunset.getTime() - sunrise.getTime())) * 100
                    ))}%`,
                    backgroundColor: isNight ? "rgba(144,202,249,0.3)" : "rgba(255,213,79,0.5)",
                  },
                ]}
              />
              {!isNight && (
                <View
                  style={[
                    styles.daylightDot,
                    {
                      left: `${Math.max(0, Math.min(100,
                        ((now.getTime() - sunrise.getTime()) / (sunset.getTime() - sunrise.getTime())) * 100
                      ))}%`,
                      backgroundColor: "#FFD54F",
                    },
                  ]}
                />
              )}
            </View>
          </View>

          {/* Atmosphere card — UV, Humidity, Dewpoint, Pressure */}
          <View style={[styles.glassCard, { backgroundColor: theme.cardBg, borderColor: theme.cardBorder }]}>
            <Text style={[styles.sectionTitle, { color: theme.textSecondary }]}>
              ATMOSPHERE
            </Text>
            <View style={styles.atmosGrid}>
              {/* UV Index with gauge */}
              <View style={styles.atmosItem}>
                <View style={styles.gaugeContainer}>
                  <View style={[styles.gaugeTrack, { backgroundColor: "rgba(255,255,255,0.06)" }]}>
                    <LinearGradient
                      colors={["#81C784", "#FFD54F", "#FFB74D", "#FF8A65", "#EF5350"]}
                      start={{ x: 0, y: 0 }}
                      end={{ x: 1, y: 0 }}
                      style={[styles.gaugeFill, { width: `${Math.min(100, (uvIndex / 11) * 100)}%` }]}
                    />
                  </View>
                  <Text style={[styles.gaugeValue, { color: uvInfo.color }]}>
                    {Math.round(uvIndex)}
                  </Text>
                </View>
                <Text style={[styles.atmosLabel, { color: theme.textSecondary }]}>UV Index</Text>
                <Text style={[styles.atmosSublabel, { color: uvInfo.color }]}>{uvInfo.label}</Text>
              </View>

              {/* Humidity */}
              <View style={styles.atmosItem}>
                <View style={styles.gaugeContainer}>
                  <View style={[styles.gaugeTrack, { backgroundColor: "rgba(255,255,255,0.06)" }]}>
                    <View
                      style={[
                        styles.gaugeFill,
                        {
                          width: `${forecast.current.relative_humidity_2m}%`,
                          backgroundColor: "#4FC3F7",
                        },
                      ]}
                    />
                  </View>
                  <Text style={[styles.gaugeValue, { color: "#4FC3F7" }]}>
                    {forecast.current.relative_humidity_2m}%
                  </Text>
                </View>
                <Text style={[styles.atmosLabel, { color: theme.textSecondary }]}>Humidity</Text>
                <Text style={[styles.atmosSublabel, { color: theme.textSecondary }]}>
                  {forecast.current.relative_humidity_2m > 70
                    ? "Muggy"
                    : forecast.current.relative_humidity_2m > 40
                    ? "Comfortable"
                    : "Dry"}
                </Text>
              </View>

              {/* Dewpoint */}
              <View style={styles.atmosItem}>
                <Text style={[styles.atmosBigValue, { color: theme.textPrimary }]}>
                  {Math.round(forecast.current.dew_point_2m ?? 0)}{"\u00B0"}
                </Text>
                <Text style={[styles.atmosLabel, { color: theme.textSecondary }]}>Dewpoint</Text>
                <Text style={[styles.atmosSublabel, { color: theme.textSecondary }]}>
                  {(forecast.current.dew_point_2m ?? 0) > 65
                    ? "Oppressive"
                    : (forecast.current.dew_point_2m ?? 0) > 55
                    ? "Sticky"
                    : "Pleasant"}
                </Text>
              </View>

              {/* Pressure */}
              <View style={styles.atmosItem}>
                <Text style={[styles.atmosBigValue, { color: theme.textPrimary }]}>
                  {(forecast.current.surface_pressure ?? 1013).toFixed(0)}
                </Text>
                <Text style={[styles.atmosLabel, { color: theme.textSecondary }]}>Pressure</Text>
                <Text style={[styles.atmosSublabel, { color: theme.textSecondary }]}>hPa</Text>
              </View>
            </View>
          </View>

          {/* Precipitation probability timeline */}
          <View style={[styles.glassCard, { backgroundColor: theme.cardBg, borderColor: theme.cardBorder }]}>
            <Text style={[styles.sectionTitle, { color: theme.textSecondary }]}>
              PRECIPITATION CHANCE
            </Text>
            <View style={styles.precipTimeline}>
              {forecast.hourly.time.slice(0, 12).map((time, i) => {
                const hr = new Date(time);
                const pct = forecast.hourly.precipitation_probability[i] ?? 0;
                const isNow = i === 0;
                return (
                  <View key={time} style={styles.precipBar}>
                    <View style={styles.precipBarTrack}>
                      <LinearGradient
                        colors={
                          pct > 60
                            ? ["#1565C0", "#42A5F5"]
                            : pct > 30
                            ? ["#1976D2", "#64B5F6"]
                            : ["#1E88E5", "#90CAF9"]
                        }
                        style={[styles.precipBarFill, { height: `${Math.max(2, pct)}%` }]}
                      />
                    </View>
                    <Text style={[styles.precipPct, { color: pct > 0 ? "#64B5F6" : theme.textSecondary }]}>
                      {pct > 0 ? `${pct}` : "-"}
                    </Text>
                    <Text
                      style={[
                        styles.precipTime,
                        { color: isNow ? theme.accent : theme.textSecondary },
                      ]}
                    >
                      {isNow ? "Now" : hr.toLocaleTimeString([], { hour: "numeric" })}
                    </Text>
                  </View>
                );
              })}
            </View>
          </View>

          <View style={{ height: 80 }} />
        </ScrollView>
      </SafeAreaView>
    </LinearGradient>
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
    fontSize: 17,
    fontWeight: "600",
    textAlign: "center",
    marginTop: 8,
    letterSpacing: 0.8,
    textTransform: "uppercase",
  },
  // Hero
  heroSection: {
    alignItems: "center",
    marginTop: -8,
    marginBottom: 12,
  },
  heroTemp: {
    fontSize: 108,
    lineHeight: 116,
    letterSpacing: -5,
  },
  heroCondition: {
    fontSize: 22,
    fontWeight: "500",
    marginTop: 0,
  },
  heroFeelsLike: {
    fontSize: 15,
    marginTop: 6,
    letterSpacing: 0.3,
  },
  // Personality quote
  quipCard: {
    borderRadius: 16,
    borderWidth: 1,
    paddingVertical: 14,
    paddingHorizontal: 20,
    marginBottom: 16,
    alignItems: "center",
  },
  quipText: {
    fontSize: 14,
    fontStyle: "italic",
    textAlign: "center",
    lineHeight: 21,
    letterSpacing: 0.2,
    opacity: 0.85,
  },
  // Alert
  alertCard: {
    flexDirection: "row",
    alignItems: "center",
    backgroundColor: "rgba(244, 67, 54, 0.15)",
    borderWidth: 1,
    borderColor: "rgba(244, 67, 54, 0.35)",
    borderRadius: 16,
    paddingVertical: 14,
    paddingHorizontal: 16,
    gap: 12,
    marginBottom: 16,
  },
  alertDot: {
    width: 8,
    height: 8,
    borderRadius: 4,
    backgroundColor: "#EF5350",
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
  alertChevron: {
    color: "#FF8A80",
    fontSize: 22,
    fontWeight: "300",
  },
  // Glass cards
  glassCard: {
    borderRadius: 20,
    borderWidth: 1,
    padding: 18,
    marginBottom: 14,
  },
  sectionTitle: {
    fontSize: 12,
    fontWeight: "700",
    letterSpacing: 1.5,
    marginBottom: 14,
  },
  // Hourly
  hourlyScroll: {
    gap: 0,
  },
  hourlyItem: {
    alignItems: "center",
    width: 54,
    gap: 3,
  },
  hourlyTime: {
    fontSize: 12,
    fontWeight: "600",
  },
  hourlyTimeNow: {
    fontWeight: "800",
  },
  hourlyPrecip: {
    fontSize: 10,
    fontWeight: "600",
    color: "#64B5F6",
    minHeight: 13,
  },
  hourlyWeatherIcon: {
    fontSize: 18,
  },
  hourlyChartArea: {
    height: 44,
    width: 54,
    position: "relative",
  },
  hourlyDot: {
    position: "absolute",
    left: 23,
    width: 8,
    height: 8,
    borderRadius: 4,
  },
  hourlyLine: {
    position: "absolute",
    left: 27,
    width: 50,
    height: 2,
    borderRadius: 1,
    opacity: 0.35,
    transformOrigin: "left center",
  },
  hourlyTemp: {
    fontSize: 15,
    fontWeight: "700",
  },
  // Daily
  dailyRow: {
    flexDirection: "row",
    alignItems: "center",
    paddingVertical: 10,
    gap: 8,
  },
  dailyDay: {
    fontSize: 15,
    fontWeight: "500",
    width: 48,
  },
  dailyIcon: {
    fontSize: 18,
    width: 26,
    textAlign: "center",
  },
  dailyPrecip: {
    fontSize: 12,
    fontWeight: "600",
    color: "#64B5F6",
    width: 30,
    textAlign: "right",
  },
  dailyLow: {
    fontSize: 15,
    fontWeight: "600",
    width: 34,
    textAlign: "right",
    fontVariant: ["tabular-nums"],
  },
  dailyBarTrack: {
    flex: 1,
    height: 5,
    borderRadius: 3,
    overflow: "hidden",
    backgroundColor: "rgba(255,255,255,0.08)",
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
    width: 34,
    fontVariant: ["tabular-nums"],
  },
  // Wind & Pressure
  windRow: {
    flexDirection: "row",
    gap: 20,
    alignItems: "center",
  },
  compassContainer: {
    width: 110,
    height: 110,
    alignItems: "center",
    justifyContent: "center",
  },
  compassRing: {
    width: 100,
    height: 100,
    borderRadius: 50,
    borderWidth: 1.5,
    alignItems: "center",
    justifyContent: "center",
  },
  compassLabel: {
    position: "absolute",
    fontSize: 10,
    fontWeight: "700",
    letterSpacing: 0.5,
  },
  compassN: { top: 6 },
  compassE: { right: 8 },
  compassS: { bottom: 6 },
  compassW: { left: 8 },
  compassArrowContainer: {
    position: "absolute",
    width: 4,
    height: 60,
    alignItems: "center",
  },
  compassArrow: {
    width: 3,
    height: 30,
    borderRadius: 1.5,
    opacity: 0.8,
  },
  compassArrowHead: {
    width: 0,
    height: 0,
    borderLeftWidth: 5,
    borderRightWidth: 5,
    borderBottomWidth: 8,
    borderLeftColor: "transparent",
    borderRightColor: "transparent",
    marginTop: -2,
  },
  compassCenter: {
    position: "absolute",
    width: 8,
    height: 8,
    borderRadius: 4,
    opacity: 0.9,
  },
  windStats: {
    flex: 1,
    gap: 0,
  },
  windStatRow: {
    flexDirection: "row",
    justifyContent: "space-between",
    alignItems: "center",
    paddingVertical: 9,
  },
  windStatLabel: {
    fontSize: 14,
    fontWeight: "500",
  },
  windStatValue: {
    fontSize: 17,
    fontWeight: "700",
  },
  windStatUnit: {
    fontSize: 13,
    fontWeight: "400",
    opacity: 0.6,
  },
  windDivider: {
    height: StyleSheet.hairlineWidth,
    opacity: 0.5,
  },
  // Sun & Moon
  sunMoonRow: {
    flexDirection: "row",
    justifyContent: "space-around",
    marginBottom: 16,
  },
  sunMoonItem: {
    alignItems: "center",
    gap: 6,
  },
  sunMoonIcon: {
    width: 44,
    height: 44,
    borderRadius: 22,
    alignItems: "center",
    justifyContent: "center",
  },
  miniSun: {
    width: 18,
    height: 18,
    borderRadius: 9,
    backgroundColor: "#FFD54F",
    opacity: 0.9,
  },
  miniSunset: {
    width: 18,
    height: 18,
    borderRadius: 9,
    backgroundColor: "#FF9800",
    opacity: 0.9,
  },
  sunMoonEmoji: {
    fontSize: 18,
  },
  sunMoonLabel: {
    fontSize: 12,
    fontWeight: "600",
    letterSpacing: 0.3,
  },
  sunMoonValue: {
    fontSize: 16,
    fontWeight: "700",
  },
  daylightBar: {
    height: 4,
    borderRadius: 2,
    overflow: "hidden",
  },
  daylightProgress: {
    position: "absolute",
    top: 0,
    left: 0,
    bottom: 0,
    borderRadius: 2,
  },
  daylightDot: {
    position: "absolute",
    top: -3,
    width: 10,
    height: 10,
    borderRadius: 5,
    marginLeft: -5,
    borderWidth: 2,
    borderColor: "rgba(0,0,0,0.3)",
  },
  // Atmosphere
  atmosGrid: {
    flexDirection: "row",
    flexWrap: "wrap",
  },
  atmosItem: {
    width: "50%",
    paddingVertical: 10,
    paddingHorizontal: 6,
    gap: 4,
  },
  gaugeContainer: {
    flexDirection: "row",
    alignItems: "center",
    gap: 10,
  },
  gaugeTrack: {
    flex: 1,
    height: 5,
    borderRadius: 3,
    overflow: "hidden",
  },
  gaugeFill: {
    height: "100%",
    borderRadius: 3,
  },
  gaugeValue: {
    fontSize: 18,
    fontWeight: "700",
    minWidth: 36,
    textAlign: "right",
  },
  atmosLabel: {
    fontSize: 12,
    fontWeight: "600",
    letterSpacing: 0.3,
  },
  atmosSublabel: {
    fontSize: 11,
    fontWeight: "500",
    opacity: 0.8,
  },
  atmosBigValue: {
    fontSize: 28,
    fontWeight: "700",
    lineHeight: 32,
  },
  // Precipitation timeline
  precipTimeline: {
    flexDirection: "row",
    justifyContent: "space-between",
    alignItems: "flex-end",
    height: 100,
  },
  precipBar: {
    flex: 1,
    alignItems: "center",
    justifyContent: "flex-end",
    gap: 4,
  },
  precipBarTrack: {
    width: "60%",
    height: 50,
    borderRadius: 3,
    backgroundColor: "rgba(255,255,255,0.04)",
    overflow: "hidden",
    justifyContent: "flex-end",
  },
  precipBarFill: {
    width: "100%",
    borderRadius: 3,
    opacity: 0.7,
  },
  precipPct: {
    fontSize: 10,
    fontWeight: "700",
  },
  precipTime: {
    fontSize: 9,
    fontWeight: "600",
  },
});
