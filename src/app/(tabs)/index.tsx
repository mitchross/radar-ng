/**
 * Cumulus Home screen — ported from UI-Handoff home.jsx to Expo/RN.
 * Location row + gear (→ settings modal), hero temp + WeatherIcon, nowcast
 * banner, 24h hourly strip, 24h precip chart, 7-day forecast, radar mini
 * tease, stat grid (UV/wind/humidity/visibility/pressure/AQI), sun arc.
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
import {
  cumulus,
  CONDITION_GRADIENTS,
  getCumulusCondition,
  getIconKind,
  getUVInfo,
  getWindInfo,
  getWindDirection,
  isNightAt,
} from "../../lib/cumulusTheme";
import WeatherIcon from "../../components/weather/WeatherIcon";
import { RadarMiniMap } from "../../components/home/RadarMiniMap";

export default function HomeScreen() {
  useLocation();
  const router = useRouter();
  const { data: forecast, isLoading } = useForecast();
  const { data: alertData } = useAlerts();
  const queryClient = useQueryClient();
  const [refreshing, setRefreshing] = useState(false);
  const onRefresh = useCallback(async () => {
    setRefreshing(true);
    try {
      await Promise.all([
        queryClient.invalidateQueries({ queryKey: ["forecast"] }),
        queryClient.invalidateQueries({ queryKey: ["alerts"] }),
      ]);
    } finally {
      setRefreshing(false);
    }
  }, [queryClient]);

  if (isLoading || !forecast) {
    return (
      <LinearGradient colors={CONDITION_GRADIENTS.clearNight} style={styles.container}>
        <SafeAreaView style={styles.flex}>
          <Text style={styles.loading}>Loading weather...</Text>
        </SafeAreaView>
      </LinearGradient>
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

  // Nowcast banner logic — use minutely_15 to detect precip start
  const nowcastHeadline = buildNowcastHeadline(forecast.minutely_15);

  // 24h hourly (next 24 starting at current hour)
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

  // 7-day
  const daily = forecast.daily.time.map((t, i) => ({
    day: i === 0 ? "Today" : new Date(t).toLocaleDateString([], { weekday: "short" }),
    icon: getIconKind(forecast.daily.weather_code[i], false),
    hi: Math.round(forecast.daily.temperature_2m_max[i]),
    lo: Math.round(forecast.daily.temperature_2m_min[i]),
    precip: Math.round(forecast.daily.precipitation_probability_max?.[i] ?? 0),
    now: i === 0 ? temp : undefined,
  }));
  const weekHi = Math.max(...daily.map((d) => d.hi));
  const weekLo = Math.min(...daily.map((d) => d.lo));

  // Stats — everything nullish-guarded since Open-Meteo can return null
  // for missing fields (e.g. humidity, pressure in some regions/times).
  const uv = forecast.daily.uv_index_max?.[0] ?? 0;
  const uvInfo = getUVInfo(uv);
  const windMph = Math.round(forecast.current.wind_speed_10m ?? 0);
  const windInfo = getWindInfo(windMph);
  const windDeg = forecast.current.wind_direction_10m ?? 0;
  const windCompass = getWindDirection(windDeg);
  const humidity = Math.round(forecast.current.relative_humidity_2m ?? 0);
  const dew = Math.round(forecast.current.dew_point_2m ?? 0);
  // Open-Meteo visibility is in meters → miles
  const visM = forecast.hourly.visibility?.[hourlyStart];
  const visibility = visM != null ? Math.min(10, visM / 1609) : 10;
  const pressure = Math.round(forecast.current.surface_pressure ?? 1013);
  const dayMs = sunset.getTime() - sunrise.getTime();
  const dayProgress = Math.max(0, Math.min(1, (now.getTime() - sunrise.getTime()) / dayMs));

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
          {/* Top bar — location + gear (→ settings) */}
          <View style={styles.topBar}>
            <View>
              <View style={styles.locationRow}>
                <View style={styles.locationDotOuter}>
                  <View style={styles.locationDotInner} />
                </View>
                <Text style={styles.locationText}>Grand Rapids, MI</Text>
              </View>
              <Text style={styles.updated}>
                {"UPDATED " + now.toLocaleTimeString([], { hour: "numeric", minute: "2-digit" }).toUpperCase()}
              </Text>
            </View>
            <TouchableOpacity
              style={styles.iconBtn}
              onPress={() => router.push("/(tabs)/settings" as any)}
              activeOpacity={0.7}
            >
              <View style={styles.gearRing} />
              <View style={styles.gearDot} />
            </TouchableOpacity>
          </View>

          {/* Hero — temp + weather icon */}
          <View style={styles.hero}>
            <View style={styles.heroIcon}>
              <WeatherIcon kind={iconKind} size={140} time={isNight ? "night" : "day"} />
            </View>
            <Text style={styles.heroCondition}>{conditionLabel}</Text>
            <View style={styles.heroTempRow}>
              <Text style={styles.heroTemp}>{temp}</Text>
              <Text style={styles.heroDeg}>{"\u00B0"}</Text>
            </View>
            <Text style={styles.heroMeta}>
              Feels like <Text style={styles.heroMetaStrong}>{feels}{"\u00B0"}</Text>
              <Text style={styles.heroMetaDim}>  {"\u00B7"}  </Text>
              H <Text style={styles.heroMetaStrong}>{hi}{"\u00B0"}</Text>
              <Text style={styles.heroMetaDim}>  L  </Text>
              <Text style={styles.heroMetaStrong}>{lo}{"\u00B0"}</Text>
            </Text>
          </View>

          {/* Nowcast banner */}
          {nowcastHeadline && (
            <Pressable
              style={styles.nowcastBanner}
              onPress={() => router.push("/nowcast" as never)}
            >
              <View style={styles.nowcastIcon}>
                <WeatherIcon kind="rain" size={28} />
              </View>
              <View style={{ flex: 1 }}>
                <Text style={styles.nowcastHeadline}>{nowcastHeadline.headline}</Text>
                <Text style={styles.nowcastSub}>{nowcastHeadline.sub}</Text>
              </View>
              <Text style={styles.chevron}>{"\u203A"}</Text>
            </Pressable>
          )}

          {/* Active alert */}
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
              <View style={styles.alertDot} />
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

          {/* 24h hourly strip */}
          <SectionHeader title="HOURLY · NEXT 24" right="48H" />
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
                <View style={{ marginVertical: 3 }}>
                  <WeatherIcon kind={h.icon} size={30} time={isNight ? "night" : "day"} />
                </View>
                <Text style={styles.hourlyTemp}>{h.temp}{"\u00B0"}</Text>
                {h.precip > 0 ? (
                  <Text style={styles.hourlyPrecip}>{h.precip}%</Text>
                ) : (
                  <View style={{ height: 11 }} />
                )}
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
              <Text style={styles.axisLabel}>+6h</Text>
              <Text style={styles.axisLabel}>+12h</Text>
              <Text style={styles.axisLabel}>+18h</Text>
              <Text style={styles.axisLabel}>+24h</Text>
            </View>
          </View>

          {/* 7 day */}
          <SectionHeader title="7 DAY FORECAST" />
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
                  <Text style={[styles.dailyDay, i === 0 && styles.dailyDayToday]}>
                    {d.day}
                  </Text>
                  <View style={{ width: 30 }}>
                    <WeatherIcon kind={d.icon} size={28} />
                  </View>
                  <Text style={styles.dailyPrecip}>
                    {d.precip > 0 ? `${d.precip}%` : ""}
                  </Text>
                  <View style={styles.dailyBarTrack}>
                    <LinearGradient
                      colors={[cumulus.cold, cumulus.sun, cumulus.hot]}
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
                    {i === 0 && d.now != null && (
                      <View
                        style={[
                          styles.dailyNowDot,
                          { left: `${nowPct}%` },
                        ]}
                      />
                    )}
                  </View>
                  <Text style={styles.dailyLo}>{d.lo}{"\u00B0"}</Text>
                  <Text style={styles.dailyHi}>{d.hi}{"\u00B0"}</Text>
                </View>
              );
            })}
          </View>

          {/* Real radar mini-map — see components/home/RadarMiniMap.tsx */}
          <RadarMiniMap
            headline={nowcastHeadline ? "Precip developing nearby" : "Clear skies overhead"}
          />

          {/* Stat grid */}
          <View style={styles.statGrid}>
            <StatCard
              label="UV INDEX"
              value={Math.round(uv).toString()}
              sub={uvInfo.label}
              accent={uvInfo.color}
              barPct={Math.min(1, uv / 11)}
            />
            <StatCard
              label="WIND"
              value={windMph.toString()}
              unit="mph"
              sub={`${windCompass} \u00B7 ${windInfo.label}`}
              accent={windInfo.color}
              barPct={Math.min(1, windMph / 40)}
            />
            <StatCard
              label="HUMIDITY"
              value={humidity.toString()}
              unit="%"
              sub={`Dew ${dew}\u00B0`}
              accent={cumulus.rain}
              barPct={humidity / 100}
            />
            <StatCard
              label="VISIBILITY"
              value={visibility.toFixed(1)}
              unit="mi"
              sub={visibility >= 9 ? "Clear" : visibility >= 3 ? "Hazy" : "Low"}
              accent={cumulus.rain}
              barPct={Math.min(1, visibility / 10)}
            />
            <StatCard
              label="PRESSURE"
              value={pressure.toString()}
              unit="hPa"
              sub={pressure < 1010 ? "Falling" : pressure > 1020 ? "High" : "Steady"}
              accent={cumulus.accent}
              barPct={Math.max(0, Math.min(1, (pressure - 980) / 60))}
            />
            <StatCard
              label="DEWPOINT"
              value={dew.toString()}
              unit={"\u00B0"}
              sub={dew > 65 ? "Oppressive" : dew > 55 ? "Sticky" : "Pleasant"}
              accent={cumulus.accent}
              barPct={Math.max(0, Math.min(1, (dew - 20) / 60))}
            />
          </View>

          {/* Sun + daylight */}
          <SectionHeader title="SUN & DAYLIGHT" />
          <View style={styles.card}>
            <View style={styles.sunRow}>
              <View>
                <Text style={styles.sunLabel}>SUNRISE</Text>
                <Text style={styles.sunValue}>
                  {sunrise.toLocaleTimeString([], { hour: "numeric", minute: "2-digit" })}
                </Text>
              </View>
              <View style={{ alignItems: "center" }}>
                <Text style={styles.sunLabel}>DAYLIGHT</Text>
                <Text style={styles.sunValue}>
                  {Math.floor(dayMs / 3600000)}h {Math.round((dayMs % 3600000) / 60000)}m
                </Text>
              </View>
              <View>
                <Text style={[styles.sunLabel, { textAlign: "right" }]}>SUNSET</Text>
                <Text style={[styles.sunValue, { textAlign: "right" }]}>
                  {sunset.toLocaleTimeString([], { hour: "numeric", minute: "2-digit" })}
                </Text>
              </View>
            </View>
            <View style={styles.daylightTrack}>
              <View style={[styles.daylightFill, { width: `${dayProgress * 100}%` }]} />
              {!isNight && (
                <View
                  style={[
                    styles.daylightSun,
                    { left: `${dayProgress * 100}%` },
                  ]}
                />
              )}
            </View>
          </View>

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

function StatCard({
  label,
  value,
  unit,
  sub,
  accent,
  barPct,
}: {
  label: string;
  value: string;
  unit?: string;
  sub: string;
  accent: string;
  barPct: number;
}) {
  return (
    <View style={styles.statCard}>
      <Text style={styles.statLabel}>{label}</Text>
      <View style={styles.statValueRow}>
        <Text style={styles.statValue}>{value}</Text>
        {unit ? <Text style={styles.statUnit}>{unit}</Text> : null}
      </View>
      <Text style={styles.statSub}>{sub}</Text>
      <View style={styles.statBarTrack}>
        <View style={[styles.statBarFill, { width: `${Math.max(2, barPct * 100)}%`, backgroundColor: accent }]} />
      </View>
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
  const slice = minutely.precipitation.slice(startIdx, startIdx + 8); // next 2h at 15m
  const firstWet = slice.findIndex((p) => p > 0.01);
  if (firstWet < 0) return null;
  const minutes = firstWet * 15;
  // Find end of precip
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
  scroll: { paddingBottom: 140 },
  loading: { color: "rgba(255,255,255,0.6)", fontSize: 16, textAlign: "center", marginTop: 120 },

  // Top bar
  topBar: {
    paddingHorizontal: 20,
    paddingTop: 4,
    paddingBottom: 2,
    flexDirection: "row",
    alignItems: "center",
    justifyContent: "space-between",
  },
  locationRow: { flexDirection: "row", alignItems: "center", gap: 6 },
  locationDotOuter: {
    width: 14,
    height: 14,
    borderRadius: 7,
    backgroundColor: "rgba(139,124,255,0.25)",
    alignItems: "center",
    justifyContent: "center",
  },
  locationDotInner: {
    width: 6,
    height: 6,
    borderRadius: 3,
    backgroundColor: cumulus.accent,
  },
  locationText: { color: cumulus.ink, fontSize: 15, fontWeight: "600" },
  updated: {
    color: cumulus.inkMuted,
    fontSize: 10,
    letterSpacing: 0.8,
    marginTop: 2,
    fontVariant: ["tabular-nums"],
  },
  iconBtn: {
    width: 36,
    height: 36,
    borderRadius: 12,
    backgroundColor: "rgba(255,255,255,0.08)",
    borderWidth: 1,
    borderColor: cumulus.inkLine,
    alignItems: "center",
    justifyContent: "center",
  },
  gearRing: {
    position: "absolute",
    width: 14,
    height: 14,
    borderRadius: 7,
    borderWidth: 1.5,
    borderColor: cumulus.ink,
  },
  gearDot: {
    position: "absolute",
    width: 3,
    height: 3,
    borderRadius: 2,
    backgroundColor: cumulus.ink,
  },

  // Hero
  hero: {
    paddingHorizontal: 20,
    paddingTop: 4,
    paddingBottom: 4,
    position: "relative",
    minHeight: 180,
  },
  heroIcon: { position: "absolute", right: 10, top: -10, opacity: 0.95 },
  heroCondition: { color: cumulus.inkDim, fontSize: 13, fontWeight: "500" },
  heroTempRow: { flexDirection: "row", alignItems: "flex-start", marginTop: 2 },
  heroTemp: {
    color: cumulus.ink,
    fontSize: 112,
    lineHeight: 112,
    fontWeight: "200",
    letterSpacing: -5,
  },
  heroDeg: {
    color: cumulus.ink,
    fontSize: 56,
    fontWeight: "300",
    marginTop: 10,
    letterSpacing: -2,
    opacity: 0.55,
  },
  heroMeta: { color: cumulus.inkDim, fontSize: 14, marginTop: -2 },
  heroMetaStrong: { color: cumulus.ink, fontWeight: "600" },
  heroMetaDim: { color: cumulus.inkFaint },

  // Nowcast banner
  nowcastBanner: {
    marginHorizontal: 16,
    marginTop: 14,
    padding: 12,
    borderRadius: 16,
    backgroundColor: "rgba(79,184,255,0.18)",
    borderWidth: 1,
    borderColor: "rgba(79,184,255,0.45)",
    flexDirection: "row",
    alignItems: "center",
    gap: 12,
  },
  nowcastIcon: {
    width: 38,
    height: 38,
    borderRadius: 12,
    backgroundColor: "rgba(79,184,255,0.3)",
    alignItems: "center",
    justifyContent: "center",
  },
  nowcastHeadline: { color: cumulus.ink, fontSize: 14, fontWeight: "600" },
  nowcastSub: { color: cumulus.inkDim, fontSize: 12, marginTop: 1 },

  // Alert
  alertCard: {
    marginHorizontal: 16,
    marginTop: 10,
    padding: 12,
    borderRadius: 16,
    backgroundColor: "rgba(255,59,74,0.15)",
    borderWidth: 1,
    borderColor: "rgba(255,59,74,0.45)",
    flexDirection: "row",
    alignItems: "center",
    gap: 12,
  },
  alertDot: { width: 8, height: 8, borderRadius: 4, backgroundColor: cumulus.alert },
  alertTitle: { color: "#FF8A80", fontSize: 14, fontWeight: "700" },
  alertSub: { color: "#EF9A9A", fontSize: 12, marginTop: 1 },

  chevron: { color: cumulus.ink, fontSize: 22, fontWeight: "400" },

  // Section header
  sectionHeader: {
    flexDirection: "row",
    justifyContent: "space-between",
    alignItems: "baseline",
    paddingHorizontal: 20,
    paddingTop: 22,
    paddingBottom: 10,
  },
  sectionTitle: {
    color: cumulus.inkMuted,
    fontSize: 11,
    fontWeight: "700",
    letterSpacing: 1.6,
  },
  sectionRight: {
    color: cumulus.inkDim,
    fontSize: 11,
    letterSpacing: 0.8,
    fontVariant: ["tabular-nums"],
  },

  // Hourly strip
  hourlyStrip: { paddingHorizontal: 16, gap: 8 },
  hourlyCell: {
    minWidth: 54,
    padding: 8,
    alignItems: "center",
    backgroundColor: cumulus.card,
    borderRadius: 14,
    borderWidth: 1,
    borderColor: cumulus.cardLine,
  },
  hourlyCellNow: {
    backgroundColor: cumulus.accentSoft,
    borderColor: cumulus.accentBorder,
  },
  hourlyTime: {
    color: cumulus.inkMuted,
    fontSize: 10,
    fontWeight: "700",
    letterSpacing: 0.3,
  },
  hourlyTimeNow: { color: "#C7BDFF" },
  hourlyTemp: { color: cumulus.ink, fontSize: 15, fontWeight: "700", marginTop: 2 },
  hourlyPrecip: {
    color: cumulus.rain,
    fontSize: 9,
    fontWeight: "700",
    marginTop: 1,
    fontVariant: ["tabular-nums"],
  },

  // Card
  card: {
    marginHorizontal: 16,
    backgroundColor: cumulus.card,
    borderWidth: 1,
    borderColor: cumulus.cardLine,
    borderRadius: 18,
    padding: 14,
  },

  // Precip chart
  precipChart: {
    flexDirection: "row",
    alignItems: "flex-end",
    height: 48,
    gap: 2,
  },
  precipBarSlot: { flex: 1, justifyContent: "flex-end" },
  precipBar: {
    width: "100%",
    borderRadius: 2,
    backgroundColor: cumulus.rain,
  },
  precipAxis: {
    flexDirection: "row",
    justifyContent: "space-between",
    marginTop: 6,
  },
  axisLabel: {
    color: cumulus.inkMuted,
    fontSize: 10,
    fontVariant: ["tabular-nums"],
  },

  // Daily
  dailyRow: {
    flexDirection: "row",
    alignItems: "center",
    paddingVertical: 11,
    paddingHorizontal: 14,
    gap: 10,
  },
  dailyRowBorder: {
    borderTopWidth: StyleSheet.hairlineWidth,
    borderTopColor: cumulus.cardLine,
  },
  dailyDay: { color: cumulus.inkDim, fontSize: 14, fontWeight: "500", width: 52 },
  dailyDayToday: { color: cumulus.ink, fontWeight: "700" },
  dailyPrecip: {
    color: cumulus.rain,
    fontSize: 11,
    fontWeight: "700",
    width: 32,
    textAlign: "right",
    fontVariant: ["tabular-nums"],
  },
  dailyBarTrack: {
    flex: 1,
    height: 6,
    borderRadius: 3,
    backgroundColor: "rgba(255,255,255,0.08)",
    position: "relative",
  },
  dailyNowDot: {
    position: "absolute",
    top: -2,
    width: 10,
    height: 10,
    borderRadius: 5,
    backgroundColor: "#fff",
    borderWidth: 2,
    borderColor: "#000",
    marginLeft: -5,
  },
  dailyLo: {
    color: cumulus.inkDim,
    fontSize: 13,
    width: 26,
    textAlign: "right",
    fontVariant: ["tabular-nums"],
  },
  dailyHi: {
    color: cumulus.ink,
    fontSize: 13,
    fontWeight: "500",
    width: 26,
    textAlign: "right",
    fontVariant: ["tabular-nums"],
  },

  // Radar tease
  radarTease: {
    marginHorizontal: 16,
    marginTop: 18,
    height: 140,
    borderRadius: 18,
    borderWidth: 1,
    borderColor: cumulus.cardLine,
    overflow: "hidden",
  },
  radarBlob: { position: "absolute", borderRadius: 40 },
  radarSweepWrap: {
    position: "absolute",
    right: 18,
    top: 18,
    bottom: 18,
    width: 104,
    alignItems: "center",
    justifyContent: "center",
  },
  radarRing: {
    position: "absolute",
    borderRadius: 999,
    borderWidth: 1,
    borderColor: "rgba(139,124,255,0.35)",
  },
  radarRing1: { width: 100, height: 100, opacity: 0.28 },
  radarRing2: { width: 70, height: 70, opacity: 0.5 },
  radarRing3: { width: 42, height: 42, opacity: 0.75 },
  radarCenter: {
    position: "absolute",
    width: 7,
    height: 7,
    borderRadius: 3.5,
    backgroundColor: cumulus.accent,
  },
  radarArm: {
    position: "absolute",
    width: 2,
    height: 50,
    top: 6,
    borderRadius: 1,
    backgroundColor: cumulus.accent,
    opacity: 0.85,
    transform: [{ rotate: "38deg" }, { translateY: 18 }],
  },
  radarLive: {
    position: "absolute",
    top: 10,
    left: 12,
    flexDirection: "row",
    alignItems: "center",
    gap: 5,
  },
  radarLiveDot: {
    width: 6,
    height: 6,
    borderRadius: 3,
    backgroundColor: cumulus.ok,
  },
  radarLiveText: {
    color: cumulus.ok,
    fontSize: 10,
    fontWeight: "700",
    letterSpacing: 1.4,
  },
  radarBottom: {
    position: "absolute",
    left: 14,
    right: 14,
    bottom: 12,
    flexDirection: "row",
    alignItems: "center",
    justifyContent: "space-between",
  },
  radarLabel: {
    color: "rgba(255,255,255,0.65)",
    fontSize: 10,
    letterSpacing: 1.6,
    fontVariant: ["tabular-nums"],
  },
  radarTitle: { color: cumulus.ink, fontSize: 15, fontWeight: "600", marginTop: 2 },
  radarChevronBox: {
    width: 30,
    height: 30,
    borderRadius: 10,
    backgroundColor: "rgba(255,255,255,0.15)",
    alignItems: "center",
    justifyContent: "center",
  },

  // Stat grid
  statGrid: {
    flexDirection: "row",
    flexWrap: "wrap",
    marginHorizontal: 11,
    marginTop: 12,
    gap: 10,
  },
  statCard: {
    flexBasis: "47%",
    flexGrow: 1,
    backgroundColor: cumulus.card,
    borderRadius: 16,
    borderWidth: 1,
    borderColor: cumulus.cardLine,
    padding: 12,
    minHeight: 92,
  },
  statLabel: {
    color: cumulus.inkMuted,
    fontSize: 10,
    fontWeight: "700",
    letterSpacing: 1.2,
  },
  statValueRow: {
    flexDirection: "row",
    alignItems: "baseline",
    marginTop: 4,
    gap: 4,
  },
  statValue: {
    color: cumulus.ink,
    fontSize: 24,
    fontWeight: "500",
    letterSpacing: -0.5,
  },
  statUnit: { color: cumulus.inkDim, fontSize: 12 },
  statSub: { color: cumulus.inkDim, fontSize: 11, marginTop: 2 },
  statBarTrack: {
    marginTop: 10,
    height: 3,
    borderRadius: 2,
    backgroundColor: "rgba(255,255,255,0.08)",
    overflow: "hidden",
  },
  statBarFill: { height: "100%", borderRadius: 2 },

  // Sun
  sunRow: {
    flexDirection: "row",
    justifyContent: "space-between",
    marginBottom: 12,
  },
  sunLabel: {
    color: cumulus.inkMuted,
    fontSize: 10,
    fontWeight: "700",
    letterSpacing: 1.2,
  },
  sunValue: { color: cumulus.ink, fontSize: 15, fontWeight: "700", marginTop: 2 },
  daylightTrack: {
    height: 4,
    borderRadius: 2,
    backgroundColor: "rgba(255,255,255,0.08)",
    position: "relative",
  },
  daylightFill: {
    position: "absolute",
    left: 0,
    top: 0,
    bottom: 0,
    borderRadius: 2,
    backgroundColor: "rgba(255,193,77,0.5)",
  },
  daylightSun: {
    position: "absolute",
    top: -3,
    width: 10,
    height: 10,
    borderRadius: 5,
    marginLeft: -5,
    backgroundColor: cumulus.sun,
    borderWidth: 2,
    borderColor: "rgba(0,0,0,0.3)",
  },
});
