/**
 * Cumulus Home screen — Redesigned for Editorial Light.
 * Warm paper background, display serif headers, Simple/Advanced layout gating.
 */
import { useCallback, useState } from "react";
import { ScrollView, View, Text, StyleSheet, TouchableOpacity, Pressable, RefreshControl } from "react-native";
import { SafeAreaView } from "react-native-safe-area-context";
import { LinearGradient } from "expo-linear-gradient";
import { useRouter } from "expo-router";
import { useQueryClient } from "@tanstack/react-query";
import { useForecast } from "../../hooks/useForecast";
import { useAlerts } from "../../hooks/useAlerts";
import { useLocation } from "../../hooks/useLocation";
import { activeLocationLabel } from "../../lib/locationLabel";
import { useWeatherStore } from "../../stores/useWeatherStore";
import {
  cumulus,
  cumulusFonts,
  CONDITION_GRADIENTS,
  getCumulusCondition,
  getIconKind,
  getUVInfo,
  getWindDirection,
  isNightAt,
} from "../../lib/cumulusTheme";
import WeatherIcon from "../../components/weather/WeatherIcon";
import { RadarMiniMap } from "../../components/home/RadarMiniMap";
import {
  UVBar,
  WindDial,
  FillRing,
  VisBars,
  PressureGauge,
  SunArc,
} from "../../components/home/StatWidgets";

export default function HomeScreen() {
  useLocation();
  const router = useRouter();
  const locationMode = useWeatherStore((s) => s.locationMode);
  const selectedPlace = useWeatherStore((s) => s.selectedPlace);
  const devicePlace = useWeatherStore((s) => s.devicePlace);
  const viewMode = useWeatherStore((s) => s.viewMode);
  const setViewMode = useWeatherStore((s) => s.setViewMode);

  const { data: forecast, isLoading, isError, refetch } = useForecast();
  const { data: alertData } = useAlerts();
  const queryClient = useQueryClient();
  const [refreshing, setRefreshing] = useState(false);

  const onRefresh = useCallback(async () => {
    setRefreshing(true);
    try {
      await Promise.all([
        queryClient.refetchQueries({ queryKey: ["forecast"] }),
        queryClient.refetchQueries({ queryKey: ["alerts"] }),
      ]);
    } finally {
      setRefreshing(false);
    }
  }, [queryClient]);

  if (isError && !forecast) {
    return (
      <View style={styles.errorContainer}>
        <SafeAreaView style={[styles.flex, styles.center]}>
          <Text style={styles.errorTitle}>Couldn&apos;t load weather</Text>
          <Text style={styles.errorBody}>
            The forecast service is unreachable right now.
          </Text>
          <TouchableOpacity style={styles.retryBtn} onPress={() => refetch()}>
            <Text style={styles.retryText}>Try again</Text>
          </TouchableOpacity>
        </SafeAreaView>
      </View>
    );
  }

  if (isLoading || !forecast) {
    return (
      <View style={styles.loadingContainer}>
        <SafeAreaView style={styles.flex}>
          <Text style={styles.loading}>Loading weather...</Text>
        </SafeAreaView>
      </View>
    );
  }

  const now = new Date();
  const sunrise = new Date(forecast.daily.sunrise[0]);
  const sunset = new Date(forecast.daily.sunset[0]);
  const isNight = isNightAt(now, sunrise, sunset);

  const weatherCode = forecast.current.weather_code;
  const condition = getCumulusCondition(weatherCode, isNight);
  const iconKind = getIconKind(weatherCode, isNight);
  const gradient = CONDITION_GRADIENTS[condition];

  const temp = Math.round(forecast.current.temperature_2m ?? 0);
  const feels = Math.round(forecast.current.apparent_temperature ?? temp);
  const hi = Math.round(forecast.daily.temperature_2m_max[0] ?? temp);
  const lo = Math.round(forecast.daily.temperature_2m_min[0] ?? temp);

  const conditionLabel = CONDITION_LABELS[condition];
  const locationLabel = activeLocationLabel(locationMode, selectedPlace, devicePlace);

  // Nowcast banner logic
  const nowcastHeadline = buildNowcastHeadline(forecast.minutely_15);

  // 24h hourly strip
  const hourlyStart = findStartHourIndex(forecast.hourly.time);
  const hourly = forecast.hourly.time.slice(hourlyStart, hourlyStart + 24).map((t, i) => {
    const idx = hourlyStart + i;
    const hr = new Date(t);
    const hrIsNight = hr < sunrise || hr > sunset;
    return {
      time: formatHour(hr, i),
      temp: Math.round(forecast.hourly.temperature_2m[idx]),
      icon: getIconKind(forecast.hourly.weather_code[idx], hrIsNight),
      precip: forecast.hourly.precipitation_probability?.[idx] ?? 0,
      isNow: i === 0,
    };
  });
  const precipTotalIn = forecast.daily.precipitation_sum[0]?.toFixed(2) ?? "0.00";

  // 7-day forecast
  const todayLocal = (() => {
    const d = now;
    const y = d.getFullYear();
    const m = String(d.getMonth() + 1).padStart(2, "0");
    const day = String(d.getDate()).padStart(2, "0");
    return `${y}-${m}-${day}`;
  })();
  const daily = forecast.daily.time.map((t, i) => {
    const isToday = t === todayLocal;
    return {
      day: isToday ? "Today" : new Date(t).toLocaleDateString([], { weekday: "short" }),
      icon: getIconKind(forecast.daily.weather_code[i], false),
      hi: Math.round(forecast.daily.temperature_2m_max[i]),
      lo: Math.round(forecast.daily.temperature_2m_min[i]),
      precip: Math.round(forecast.daily.precipitation_probability_max?.[i] ?? 0),
      now: isToday ? temp : undefined,
    };
  });
  const weekHi = Math.max(...daily.map((d) => d.hi));
  const weekLo = Math.min(...daily.map((d) => d.lo));

  // Stats
  const uv = forecast.daily.uv_index_max?.[0] ?? 0;
  const uvInfo = getUVInfo(uv);
  const windMph = Math.round(forecast.current.wind_speed_10m ?? 0);
  const windDeg = forecast.current.wind_direction_10m ?? 0;
  const windCompass = getWindDirection(windDeg);
  const humidity = Math.round(forecast.current.relative_humidity_2m ?? 0);
  const dew = Math.round(forecast.current.dew_point_2m ?? 0);
  const visM = forecast.hourly.visibility?.[hourlyStart];
  const visibility = visM != null ? Math.min(10, visM / 1609) : 10;
  const pressure = Math.round(forecast.current.surface_pressure ?? 1013);
  const dayMs = sunset.getTime() - sunrise.getTime();
  const dayProgress = Math.max(0, Math.min(1, (now.getTime() - sunrise.getTime()) / dayMs));

  const isAdv = viewMode === "advanced";

  return (
    <LinearGradient colors={gradient} style={styles.container}>
      <SafeAreaView style={styles.flex} edges={["top"]}>
        <ScrollView
          showsVerticalScrollIndicator={false}
          contentContainerStyle={styles.scroll}
          refreshControl={
            <RefreshControl
              refreshing={refreshing}
              onRefresh={onRefresh}
              tintColor={cumulus.ink}
              colors={[cumulus.accent]}
            />
          }
        >
          {/* Top bar — Location & Toggle */}
          <View style={styles.topBar}>
            <TouchableOpacity
              onPress={() => router.push("/(tabs)/settings" as any)}
              style={styles.locationContainer}
              activeOpacity={0.7}
            >
              <View style={styles.locationRow}>
                <View style={styles.locationDot} />
                <Text style={styles.locationLabelText}>MY LOCATION</Text>
              </View>
              <View style={styles.locationNameRow}>
                <Text style={styles.locationNameText}>{locationLabel}</Text>
                <Text style={styles.expandChevron}>{"\u25BE"}</Text>
              </View>
            </TouchableOpacity>

            <View style={styles.toggleContainer}>
              <TouchableOpacity
                onPress={() => setViewMode("simple")}
                style={[styles.toggleBtn, !isAdv && styles.toggleBtnActive]}
                activeOpacity={0.7}
              >
                <Text style={[styles.toggleBtnText, !isAdv && styles.toggleBtnTextActive]}>Simple</Text>
              </TouchableOpacity>
              <TouchableOpacity
                onPress={() => setViewMode("advanced")}
                style={[styles.toggleBtn, isAdv && styles.toggleBtnActive]}
                activeOpacity={0.7}
              >
                <Text style={[styles.toggleBtnText, isAdv && styles.toggleBtnTextActive]}>Adv</Text>
              </TouchableOpacity>
            </View>
          </View>

          {/* Hero section */}
          <View style={styles.hero}>
            <View style={styles.heroIcon}>
              <WeatherIcon kind={iconKind} size={130} time={isNight ? "night" : "day"} />
            </View>
            <Text style={styles.heroCondition}>{conditionLabel}</Text>
            <View style={styles.heroTempRow}>
              <Text style={styles.heroTemp}>{temp}</Text>
              <Text style={styles.heroDeg}>{"\u00B0"}</Text>
            </View>
            <Text style={styles.heroMeta}>
              Feels {feels}{"\u00B0"}   {"\u00B7"}   H {hi}{"\u00B0"}   L {lo}{"\u00B0"}
            </Text>
          </View>

          {/* Nowcast banner */}
          {nowcastHeadline && (
            <Pressable
              style={styles.nowcastBanner}
              onPress={() => router.push("/nowcast" as never)}
            >
              <View style={styles.nowcastIcon}>
                <WeatherIcon kind="rain" size={24} />
              </View>
              <View style={{ flex: 1 }}>
                <Text style={styles.nowcastHeadline}>{nowcastHeadline.headline}</Text>
                <Text style={styles.nowcastSub}>{nowcastHeadline.sub}</Text>
              </View>
              <Text style={styles.chevron}>{"\u203A"}</Text>
            </Pressable>
          )}

          {/* Active alerts */}
          {alertData && alertData.features.length > 0 && (
            <TouchableOpacity
              style={styles.alertCard}
              activeOpacity={0.8}
              onPress={() =>
                router.push({
                  pathname: "/alert/[id]",
                  params: { id: alertData.features[0].properties.id },
                })
              }
            >
              <View style={styles.alertIndicatorDot} />
              <View style={{ flex: 1 }}>
                <Text style={styles.alertTitle}>{alertData.features[0].properties.event}</Text>
                <Text style={styles.alertSub} numberOfLines={1}>
                  Until{" "}
                  {new Date(alertData.features[0].properties.expires).toLocaleTimeString([], {
                    hour: "numeric",
                    minute: "2-digit",
                  })}
                </Text>
              </View>
              <Text style={styles.chevron}>{"\u203A"}</Text>
            </TouchableOpacity>
          )}

          {/* Hourly strip */}
          <SectionHeader title="HOURLY" />
          <ScrollView
            horizontal
            showsHorizontalScrollIndicator={false}
            contentContainerStyle={styles.hourlyStrip}
          >
            {hourly.map((h, i) => (
              <View
                key={i}
                style={[styles.hourlyCell, h.isNow && styles.hourlyCellNow]}
              >
                <Text style={[styles.hourlyTime, h.isNow && styles.hourlyTimeNow]}>
                  {h.isNow ? "NOW" : h.time}
                </Text>
                <View style={{ marginVertical: 6 }}>
                  <WeatherIcon kind={h.icon} size={22} time={isNight ? "night" : "day"} />
                </View>
                <Text style={styles.hourlyTemp}>{h.temp}{"\u00B0"}</Text>
              </View>
            ))}
          </ScrollView>

          {/* 24h precip chart */}
          <SectionHeader title="PRECIPITATION · 24H" right={`${precipTotalIn}"`} />
          <View style={styles.card}>
            <View style={styles.precipChart}>
              {hourly.map((h, i) => {
                const pct = h.precip / 100;
                const barH = Math.max(2, pct * 42);
                return (
                  <View key={i} style={styles.precipBarSlot}>
                    <View
                      style={[
                        styles.precipBar,
                        { height: barH, opacity: pct > 0.05 ? 1 : 0.25 },
                      ]}
                    />
                  </View>
                );
              })}
            </View>
            <View style={styles.precipAxis}>
              <Text style={styles.axisLabel}>NOW</Text>
              <Text style={styles.axisLabel}>+12H</Text>
              <Text style={styles.axisLabel}>+24H</Text>
            </View>
          </View>

          {/* 7-day forecast */}
          <SectionHeader title="7-DAY FORECAST" />
          <View style={[styles.card, { padding: 0, overflow: "hidden" }]}>
            {daily.map((d, i) => {
              const range = weekHi - weekLo || 1;
              const leftPct = ((d.lo - weekLo) / range) * 100;
              const widthPct = ((d.hi - d.lo) / range) * 100;
              const nowPct = d.now != null ? ((d.now - weekLo) / range) * 100 : 0;
              return (
                <View
                  key={i}
                  style={[
                    styles.dailyRow,
                    i > 0 && styles.dailyRowBorder,
                  ]}
                >
                  <Text style={[styles.dailyDay, d.day === "Today" && styles.dailyDayToday]}>
                    {d.day}
                  </Text>
                  <View style={{ width: 24, alignItems: "center" }}>
                    <WeatherIcon kind={d.icon} size={21} />
                  </View>
                  <Text style={styles.dailyLo}>{d.lo}{"\u00B0"}</Text>
                  <View style={styles.dailyBarTrack}>
                    <LinearGradient
                      colors={["#6db4d8", "#f0c34e", "#df6a3c"]}
                      start={{ x: 0, y: 0 }}
                      end={{ x: 1, y: 0 }}
                      style={{
                        position: "absolute",
                        left: `${leftPct}%`,
                        width: `${widthPct}%`,
                        height: "100%",
                        borderRadius: 3,
                      }}
                    />
                    {d.now != null && (
                      <View
                        style={[
                          styles.dailyNowDot,
                          { left: `${nowPct}%` },
                        ]}
                      />
                    )}
                  </View>
                  <Text style={styles.dailyHi}>{d.hi}{"\u00B0"}</Text>
                </View>
              );
            })}
          </View>

          {/* Mini radar map */}
          <RadarMiniMap
            headline={nowcastHeadline ? "Precip developing nearby" : "Clear skies overhead"}
          />

          {/* Advanced Mode: Stats grid & Twilight sun path */}
          {isAdv && (
            <>
              <SectionHeader title="CONDITIONS" />
              <View style={styles.statGrid}>
                {/* 1. UV Index */}
                <View style={styles.statCard}>
                  <Text style={styles.statLabel}>UV INDEX</Text>
                  <Text style={styles.statValue}>{Math.round(uv)}</Text>
                  <Text style={[styles.statSubText, { color: uvInfo.color }]}>
                    {uvInfo.label}
                  </Text>
                  <View style={styles.widgetWrapper}>
                    <UVBar value={uv} />
                  </View>
                </View>

                {/* 2. Wind compass */}
                <View style={styles.statCard}>
                  <Text style={styles.statLabel}>WIND</Text>
                  <View style={styles.statValueRow}>
                    <Text style={styles.statValue}>{windMph}</Text>
                    <Text style={styles.statUnit}>mph</Text>
                  </View>
                  <Text style={styles.statSubText}>{windCompass}</Text>
                  <View style={styles.widgetWrapper}>
                    <WindDial dir={windDeg} />
                  </View>
                </View>

                {/* 3. Humidity */}
                <View style={styles.statCard}>
                  <Text style={styles.statLabel}>HUMIDITY</Text>
                  <View style={styles.statValueRow}>
                    <Text style={styles.statValue}>{humidity}</Text>
                    <Text style={styles.statUnit}>%</Text>
                  </View>
                  <Text style={styles.statSubText}>Dew pt {dew}°</Text>
                  <View style={styles.widgetWrapper}>
                    <FillRing value={humidity / 100} color={cumulus.rain} />
                  </View>
                </View>

                {/* 4. Visibility */}
                <View style={styles.statCard}>
                  <Text style={styles.statLabel}>VISIBILITY</Text>
                  <View style={styles.statValueRow}>
                    <Text style={styles.statValue}>{Math.round(visibility)}</Text>
                    <Text style={styles.statUnit}>mi</Text>
                  </View>
                  <Text style={styles.statSubText}>
                    {visibility >= 9 ? "Clear view" : "Hazy"}
                  </Text>
                  <View style={styles.widgetWrapper}>
                    <VisBars value={visibility} />
                  </View>
                </View>

                {/* 5. Pressure */}
                <View style={styles.statCard}>
                  <Text style={styles.statLabel}>PRESSURE</Text>
                  <View style={styles.statValueRow}>
                    <Text style={styles.statValue}>{pressure}</Text>
                    <Text style={styles.statUnit}>hPa</Text>
                  </View>
                  <Text style={styles.statSubText}>
                    {pressure < 1010 ? "Low press." : "Normal"}
                  </Text>
                  <View style={styles.widgetWrapper}>
                    <PressureGauge value={pressure} />
                  </View>
                </View>

                {/* 6. Dew point */}
                <View style={styles.statCard}>
                  <Text style={styles.statLabel}>DEW POINT</Text>
                  <View style={styles.statValueRow}>
                    <Text style={styles.statValue}>{dew}</Text>
                    <Text style={styles.statUnit}>°</Text>
                  </View>
                  <Text style={styles.statSubText}>
                    {dew > 60 ? "Humid air" : "Comfortable"}
                  </Text>
                  <View style={styles.widgetWrapper}>
                    <FillRing value={Math.max(0, Math.min(1, (dew - 20) / 60))} color="#df6a3c" />
                  </View>
                </View>
              </View>

              {/* Sunrise/Sunset widgets grid row */}
              <View style={styles.sunriseSunsetGrid}>
                <View style={[styles.statCard, styles.rowLayoutCard]}>
                  <Text style={styles.widgetIconText}>🌅</Text>
                  <View>
                    <Text style={styles.rowLayoutLabel}>SUNRISE</Text>
                    <Text style={styles.rowLayoutVal}>
                      {sunrise.toLocaleTimeString([], { hour: "numeric", minute: "2-digit" }).toLowerCase()}
                    </Text>
                  </View>
                </View>
                <View style={[styles.statCard, styles.rowLayoutCard]}>
                  <Text style={styles.widgetIconText}>🌇</Text>
                  <View>
                    <Text style={styles.rowLayoutLabel}>SUNSET</Text>
                    <Text style={styles.rowLayoutVal}>
                      {sunset.toLocaleTimeString([], { hour: "numeric", minute: "2-digit" }).toLowerCase()}
                    </Text>
                  </View>
                </View>
              </View>

              {/* Sun Arc */}
              <View style={[styles.card, styles.sunArcCard]}>
                <SunArc
                  sunrise={sunrise.toLocaleTimeString([], { hour: "numeric", minute: "2-digit" }).toLowerCase()}
                  sunset={sunset.toLocaleTimeString([], { hour: "numeric", minute: "2-digit" }).toLowerCase()}
                  progress={dayProgress}
                />
              </View>
            </>
          )}

          <View style={{ height: 100 }} />
        </ScrollView>
      </SafeAreaView>
    </LinearGradient>
  );
}

// ───────── sub-components

function SectionHeader({ title, right }: { title: string; right?: string }) {
  return (
    <View style={styles.sectionHeader}>
      <Text style={styles.sectionTitle}>{title}</Text>
      {right ? <Text style={styles.sectionRight}>{right}</Text> : null}
    </View>
  );
}

// ───────── helpers

const CONDITION_LABELS: Record<ReturnType<typeof getCumulusCondition>, string> = {
  clearDay: "Sunny",
  clearNight: "Clear",
  cloudy: "Cloudy",
  rain: "Rain",
  storm: "Thunderstorms",
  snow: "Snow",
  fog: "Foggy",
};

function findStartHourIndex(hours: string[]): number {
  const now = Date.now();
  for (let i = 0; i < hours.length; i++) {
    if (new Date(hours[i]).getTime() >= now - 30 * 60_000) return i;
  }
  return 0;
}

function formatHour(d: Date, i: number): string {
  if (i === 0) return "NOW";
  const h = d.getHours();
  if (h === 0) return "12a";
  if (h === 12) return "12p";
  return h < 12 ? `${h}a` : `${h - 12}p`;
}

function buildNowcastHeadline(
  minutely: OpenMeteoMinutely | undefined
): { headline: string; sub: string } | null {
  if (!minutely || !minutely.precipitation || minutely.precipitation.length === 0) return null;
  const now = Date.now();
  const startIdx = minutely.time.findIndex((t) => new Date(t).getTime() >= now - 7.5 * 60_000);
  if (startIdx < 0) return null;
  const slice = minutely.precipitation.slice(startIdx, startIdx + 8);
  const firstWet = slice.findIndex((p) => p > 0.01);
  if (firstWet < 0) return null;
  const minutes = firstWet * 15;
  const continuedWet = slice.slice(firstWet).findIndex((p) => p < 0.005);
  const lastsMin = (continuedWet < 0 ? slice.length - firstWet : continuedWet) * 15;
  const total = slice.slice(firstWet).reduce((s, p) => s + Math.max(0, p), 0);
  const heavy = slice.slice(firstWet).some((p) => p > 0.3);
  const kind = heavy ? "Heavy rain" : "Rain";
  return {
    headline: minutes === 0 ? `${kind} now` : `${kind} starts in ${minutes} min`,
    sub: `Lasts ~${lastsMin} min \u00B7 ${total.toFixed(2)}" total`,
  };
}

type OpenMeteoMinutely = NonNullable<
  ReturnType<typeof useForecast> extends { data?: infer T }
    ? T extends { minutely_15?: infer M }
      ? M
      : never
    : never
>;

const styles = StyleSheet.create({
  container: { flex: 1 },
  flex: { flex: 1 },
  scroll: { paddingBottom: 120 },
  errorContainer: { flex: 1, backgroundColor: cumulus.background },
  loadingContainer: { flex: 1, backgroundColor: cumulus.background },
  loading: {
    color: cumulus.inkDim,
    fontSize: 16,
    textAlign: "center",
    marginTop: 120,
    fontFamily: cumulusFonts.ui,
  },
  center: { alignItems: "center", justifyContent: "center", paddingHorizontal: 32 },
  errorTitle: {
    color: cumulus.ink,
    fontSize: 20,
    fontWeight: "700",
    textAlign: "center",
    marginBottom: 8,
    fontFamily: cumulusFonts.ui,
  },
  errorBody: {
    color: cumulus.inkDim,
    fontSize: 14,
    textAlign: "center",
    marginBottom: 20,
    fontFamily: cumulusFonts.ui,
  },
  retryBtn: {
    backgroundColor: "#eae4d8",
    borderWidth: 1,
    borderColor: "#e3dccf",
    paddingVertical: 12,
    paddingHorizontal: 28,
    borderRadius: 14,
  },
  retryText: { color: cumulus.ink, fontSize: 15, fontWeight: "600", fontFamily: cumulusFonts.ui },

  // Top Bar Location + Toggle
  topBar: {
    paddingHorizontal: 24,
    paddingTop: 16,
    paddingBottom: 8,
    flexDirection: "row",
    alignItems: "flex-start",
    justifyContent: "space-between",
  },
  locationContainer: {
    flexDirection: "column",
  },
  locationRow: {
    flexDirection: "row",
    alignItems: "center",
  },
  locationDot: {
    width: 8,
    height: 8,
    borderRadius: 4,
    backgroundColor: cumulus.accent,
    marginRight: 8,
  },
  locationLabelText: {
    fontFamily: cumulusFonts.ui,
    fontSize: 11,
    fontWeight: "700",
    color: cumulus.inkMuted,
    letterSpacing: 1.6,
  },
  locationNameRow: {
    flexDirection: "row",
    alignItems: "center",
    marginTop: 4,
  },
  locationNameText: {
    fontFamily: cumulusFonts.display,
    fontSize: 29,
    fontWeight: "500",
    color: cumulus.ink,
    letterSpacing: -0.2,
  },
  expandChevron: {
    fontSize: 16,
    color: "#bcb3a3",
    marginLeft: 4,
    marginTop: 4,
  },

  toggleContainer: {
    flexDirection: "row",
    backgroundColor: "#eae4d8",
    borderRadius: 11,
    padding: 3,
    alignItems: "center",
  },
  toggleBtn: {
    paddingVertical: 5,
    paddingHorizontal: 9,
    borderRadius: 8,
  },
  toggleBtnActive: {
    backgroundColor: cumulus.accent,
  },
  toggleBtnText: {
    fontFamily: cumulusFonts.ui,
    fontSize: 10,
    fontWeight: "700",
    color: cumulus.inkMuted,
  },
  toggleBtnTextActive: {
    color: "#ffffff",
  },

  // Hero
  hero: {
    paddingHorizontal: 24,
    paddingTop: 12,
    paddingBottom: 16,
    position: "relative",
    minHeight: 168,
  },
  heroIcon: { position: "absolute", right: 24, top: 4, opacity: 0.95 },
  heroCondition: {
    color: cumulus.inkDim,
    fontSize: 19,
    fontFamily: cumulusFonts.display,
    fontStyle: "italic",
  },
  heroTempRow: { flexDirection: "row", alignItems: "flex-start", marginTop: 8 },
  heroTemp: {
    color: cumulus.ink,
    fontSize: 104,
    lineHeight: 104,
    fontWeight: "300",
    fontFamily: cumulusFonts.display,
    letterSpacing: -3,
  },
  heroDeg: {
    color: cumulus.ink,
    fontSize: 48,
    fontWeight: "300",
    fontFamily: cumulusFonts.display,
    marginTop: 4,
    opacity: 0.85,
  },
  heroMeta: {
    color: cumulus.inkMuted,
    fontSize: 12,
    fontWeight: "600",
    marginTop: 14,
    fontFamily: cumulusFonts.ui,
  },

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

  // Section Header
  sectionHeader: {
    flexDirection: "row",
    justifyContent: "space-between",
    alignItems: "baseline",
    paddingHorizontal: 24,
    paddingTop: 24,
    paddingBottom: 10,
  },
  sectionTitle: {
    color: cumulus.inkMuted,
    fontSize: 10,
    fontWeight: "800",
    letterSpacing: 1.6,
    fontFamily: cumulusFonts.ui,
  },
  sectionRight: {
    color: cumulus.inkDim,
    fontSize: 11,
    fontWeight: "700",
    fontFamily: cumulusFonts.display,
  },

  // Hourly strip
  hourlyStrip: { paddingHorizontal: 16, gap: 8 },
  hourlyCell: {
    minWidth: 54,
    paddingVertical: 12,
    alignItems: "center",
    backgroundColor: "#ffffff",
    borderRadius: 16,
    borderWidth: 1,
    borderColor: "#eee6d8",
  },
  hourlyCellNow: {
    backgroundColor: "#eae4d8",
    borderColor: "#e3dccf",
  },
  hourlyTime: {
    color: cumulus.inkMuted,
    fontSize: 11,
    fontWeight: "700",
    fontFamily: cumulusFonts.ui,
  },
  hourlyTimeNow: { color: cumulus.accent },
  hourlyTemp: {
    color: cumulus.ink,
    fontSize: 17,
    fontWeight: "500",
    fontFamily: cumulusFonts.display,
  },

  // Card
  card: {
    marginHorizontal: 16,
    backgroundColor: "#ffffff",
    borderWidth: 1,
    borderColor: "#eee6d8",
    borderRadius: 20,
    padding: 16,
    shadowColor: "rgba(60,50,40,0.04)",
    shadowOffset: { width: 0, height: 4 },
    shadowOpacity: 0.3,
    shadowRadius: 6,
    elevation: 1,
  },

  // Precip chart
  precipChart: {
    flexDirection: "row",
    alignItems: "flex-end",
    height: 44,
    gap: 3,
  },
  precipBarSlot: { flex: 1, justifyContent: "flex-end" },
  precipBar: {
    width: "100%",
    borderRadius: 3,
    backgroundColor: cumulus.rain,
  },
  precipAxis: {
    flexDirection: "row",
    justifyContent: "space-between",
    marginTop: 8,
  },
  axisLabel: {
    color: cumulus.inkFaint,
    fontSize: 9,
    fontWeight: "700",
    fontFamily: cumulusFonts.ui,
  },

  // Daily row
  dailyRow: {
    flexDirection: "row",
    alignItems: "center",
    paddingVertical: 10,
    paddingHorizontal: 16,
    gap: 11,
  },
  dailyRowBorder: {
    borderTopWidth: 1,
    borderTopColor: "#e7e0d3",
  },
  dailyDay: {
    color: cumulus.ink,
    fontSize: 14,
    fontWeight: "600",
    fontFamily: cumulusFonts.ui,
    width: 44,
  },
  dailyDayToday: { fontWeight: "700" },
  dailyLo: {
    color: cumulus.inkMuted,
    fontSize: 13,
    width: 28,
    textAlign: "right",
    fontFamily: cumulusFonts.ui,
  },
  dailyHi: {
    color: cumulus.ink,
    fontSize: 13,
    fontWeight: "600",
    width: 28,
    textAlign: "right",
    fontFamily: cumulusFonts.ui,
  },
  dailyBarTrack: {
    flex: 1,
    height: 6,
    borderRadius: 3,
    backgroundColor: "#e7e0d3",
    position: "relative",
  },
  dailyNowDot: {
    position: "absolute",
    top: -3,
    width: 12,
    height: 12,
    borderRadius: 6,
    backgroundColor: "#fff",
    borderWidth: 3,
    borderColor: cumulus.accent,
    marginLeft: -6,
  },

  // Stats Grid
  statGrid: {
    flexDirection: "row",
    flexWrap: "wrap",
    marginHorizontal: 16,
    gap: 9,
  },
  statCard: {
    flexBasis: "47%",
    flexGrow: 1,
    backgroundColor: "#ffffff",
    borderRadius: 14,
    borderWidth: 1,
    borderColor: "#eee6d8",
    padding: 12,
    minHeight: 110,
    position: "relative",
  },
  statLabel: {
    color: cumulus.inkMuted,
    fontSize: 9,
    fontWeight: "700",
    letterSpacing: 0.6,
    fontFamily: cumulusFonts.ui,
  },
  statValue: {
    color: cumulus.ink,
    fontSize: 22,
    fontFamily: cumulusFonts.display,
    fontWeight: "500",
    marginTop: 4,
  },
  statValueRow: {
    flexDirection: "row",
    alignItems: "baseline",
    marginTop: 4,
    gap: 2,
  },
  statUnit: {
    color: cumulus.inkMuted,
    fontSize: 10,
    fontFamily: cumulusFonts.ui,
    fontWeight: "500",
  },
  statSubText: {
    color: cumulus.inkMuted,
    fontSize: 10,
    fontWeight: "500",
    fontFamily: cumulusFonts.ui,
    marginTop: 1,
  },
  widgetWrapper: {
    position: "absolute",
    right: 12,
    bottom: 12,
  },

  // Sunrise sunset cells
  sunriseSunsetGrid: {
    flexDirection: "row",
    marginHorizontal: 16,
    marginTop: 9,
    gap: 9,
  },
  rowLayoutCard: {
    flexDirection: "row",
    alignItems: "center",
    gap: 10,
    minHeight: 52,
    paddingVertical: 10,
  },
  widgetIconText: {
    fontSize: 24,
  },
  rowLayoutLabel: {
    fontSize: 9,
    fontWeight: "700",
    color: cumulus.inkMuted,
    letterSpacing: 0.6,
    fontFamily: cumulusFonts.ui,
  },
  rowLayoutVal: {
    fontSize: 18,
    fontFamily: cumulusFonts.display,
    fontWeight: "500",
    color: cumulus.ink,
    marginTop: 2,
  },

  // Sun Arc Card
  sunArcCard: {
    marginTop: 12,
    paddingVertical: 14,
  },
});
