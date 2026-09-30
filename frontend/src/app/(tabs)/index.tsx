/**
 * Cumulus Home screen — Redesigned for Editorial Light.
 * Warm paper background, display serif headers, Simple/Advanced layout gating.
 */
import { useCallback, useMemo, useState } from "react";
import { ScrollView, View, Text, StyleSheet, Pressable, RefreshControl } from "react-native";
import { LinearGradient } from "expo-linear-gradient";
import { ScreenBackground } from "../../components/ui/ScreenBackground";
import { SafeAreaView } from "react-native-safe-area-context";
import { useRouter } from "expo-router";
import { useQueryClient } from "@tanstack/react-query";
import { useForecast } from "../../hooks/useForecast";
import { useAlerts } from "../../hooks/useAlerts";
import { useRadarNowcast } from "../../hooks/useRadarNowcast";
import { useActiveLocation } from "../../hooks/useActiveLocation";
import { getAlertEndTime } from "../../lib/alertLifecycle";
import { runOnlineRefresh } from "../../lib/queryLifecycle";
import { useWeatherStore } from "../../stores/useWeatherStore";
import {
  CONDITION_GRADIENTS,
  getCumulusCondition,
  getIconKind,
} from "../../lib/cumulusTheme";
import { displayTemperature, formatDegrees } from "../../lib/temperature";
import {
  dailyView,
  hourlyView,
  isNightAt,
  nextHourBanner,
  precipitationNext24h,
  sunTimesFor,
  weekRange,
} from "../../lib/forecastView";
import { getForecastScreenState } from "../../lib/weatherPresentation";
import {
  ScreenState,
  SectionLabel,
  SegmentedControl,
} from "../../components/ui/WeatherClearUI";
import { useWeatherClearTheme } from "../../theme/WeatherClearThemeProvider";
import type { WeatherClearTheme } from "../../theme/weatherClearTheme";
import WeatherIcon from "../../components/weather/WeatherIcon";
import { RadarMiniMap } from "../../components/home/RadarMiniMap";
import { ConditionTiles } from "../../components/home/ConditionTiles";
import { AiBriefingCard } from "../../components/home/AiBriefingCard";
import { IncomingStormCard } from "../../components/home/IncomingStormCard";
import { useBriefing } from "../../hooks/useBriefing";
import { hourlyPrecipitation } from "../../lib/conditions";
import { useAirQualityNow } from "../../hooks/useAirQualityNow";
import { readingColumnStyle } from "../../lib/tabletLayout";

export default function HomeScreen() {
  const router = useRouter();
  const { theme } = useWeatherClearTheme();
  const styles = useMemo(() => createStyles(theme), [theme]);
  const location = useActiveLocation();
  const airQuality = useAirQualityNow().data;
  const hasCoordinates = useWeatherStore((s) => s.latitude !== null);
  const temperatureUnit = useWeatherStore((s) => s.temperatureUnit);
  const viewMode = useWeatherStore((s) => s.viewMode);
  const setViewMode = useWeatherStore((s) => s.setViewMode);

  const {
    data: forecast,
    isLoading,
    isError,
    isFetching,
    refetch,
    dataUpdatedAt,
  } = useForecast();
  const { data: alertData, alertStatus } = useAlerts();
  const briefing = useBriefing(location.name);
  const { data: radarNowcast } = useRadarNowcast();
  const firstAlert = alertData?.features[0];
  const firstAlertEndAt = firstAlert ? getAlertEndTime(firstAlert) : null;
  const queryClient = useQueryClient();
  const [refreshing, setRefreshing] = useState(false);

  const onRefresh = useCallback(async () => {
    await runOnlineRefresh(async () => {
      setRefreshing(true);
      try {
        await Promise.all([
          queryClient.refetchQueries({ queryKey: ["forecast"] }),
          queryClient.refetchQueries({ queryKey: ["alerts"] }),
        ]);
      } finally {
        setRefreshing(false);
      }
    });
  }, [queryClient]);

  const presentation = getForecastScreenState({
    data: forecast,
    isLoading,
    isError,
    isFetching,
  });

  // Device mode with no fix yet: the forecast query is simply waiting, not failing.
  if (!hasCoordinates) {
    return (
      <View style={styles.stateContainer}>
        <ScreenState
          kind="loading"
          title="Finding your location"
          message="Weather appears as soon as your position is known."
        />
      </View>
    );
  }

  if (presentation.kind === "error") {
    return (
      <View style={styles.stateContainer}>
        <ScreenState
          kind="error"
          title="Couldn’t load weather"
          message="The forecast service is unreachable right now."
          actionLabel="Try again"
          onAction={() => refetch()}
        />
      </View>
    );
  }

  if (presentation.kind === "loading" || !forecast) {
    return (
      <View style={styles.stateContainer}>
        <ScreenState
          kind="loading"
          title="Loading weather"
          message="Fetching the latest local forecast."
        />
      </View>
    );
  }

  const now = new Date();
  const forecastAgeMin = dataUpdatedAt > 0 ? Math.floor((now.getTime() - dataUpdatedAt) / 60_000) : 0;
  const { sunrise, sunset } = sunTimesFor(forecast.daily, now);
  const isNight = isNightAt(now, forecast.daily);

  const weatherCode = forecast.current.weather_code;
  const condition = getCumulusCondition(weatherCode, isNight);
  const iconKind = getIconKind(weatherCode, isNight);
  const gradient = theme.dark
    ? ([theme.colors.canvas, theme.colors.surfaceStrong] as const)
    : CONDITION_GRADIENTS[condition];

  // Every value may be missing; each renders as "—" rather than a stand-in.
  const temperature = (value: number | null | undefined) => displayTemperature(value, temperatureUnit);
  const temp = temperature(forecast.current.temperature_2m);
  const feels = temperature(forecast.current.apparent_temperature);
  const hi = temperature(forecast.daily.temperature_2m_max[0]);
  const lo = temperature(forecast.daily.temperature_2m_min[0]);

  const conditionLabel = CONDITION_LABELS[condition];
  const locationLabel = location.label;
  const locationName = location.name;

  const nowcastBanner = nextHourBanner(radarNowcast, forecast.minutely_15, now.getTime());

  const hourly = hourlyView(forecast, temperatureUnit, now.getTime());
  const precipTotal = precipitationNext24h(forecast, now.getTime());
  const precipTotalIn = precipTotal === null ? "\u2014" : `${precipTotal.toFixed(2)}"`;

  // `date` is a date-only string ("2026-07-03"); `new Date(date)` parses it as
  // UTC midnight, the previous day in US timezones, so parse it as local.
  const daily = dailyView(forecast, temperatureUnit, now).map((d) => ({
    ...d,
    day: d.isToday ? "Today" : new Date(`${d.date}T00:00:00`).toLocaleDateString([], { weekday: "short" }),
    now: d.isToday ? temp : null,
  }));
  const week = weekRange(daily);

  const precipHours = hourlyPrecipitation(forecast, now.getTime(), 24);
  const precipPeak = Math.max(0.05, ...precipHours.map((v) => v ?? 0));
  const mph = (v: number | null | undefined) => (v == null ? null : Math.round(v));
  const conditions = {
    uv: forecast.daily.uv_index_max?.[0] ?? null,
    windMph: mph(forecast.current.wind_speed_10m),
    gustMph: mph(forecast.current.wind_gusts_10m),
    windFromDeg: forecast.current.wind_direction_10m ?? null,
    humidity: mph(forecast.current.relative_humidity_2m),
    dew: temperature(forecast.current.dew_point_2m),
    dewF: forecast.current.dew_point_2m ?? null,
    pressureHPa: forecast.current.surface_pressure ?? null,
    rainTodayIn: forecast.daily.precipitation_sum?.[0] ?? null,
    rainTomorrowIn: forecast.daily.precipitation_sum?.[1] ?? null,
    rainNext: precipHours.slice(0, 12),
    feels,
    actual: temp,
    feelsF: forecast.current.apparent_temperature ?? null,
    actualF: forecast.current.temperature_2m ?? null,
    sunrise,
    sunset,
    now,
    pm25: airQuality?.pm25 ?? null,
    ozonePpb: airQuality?.ozonePpb ?? null,
  };

  const isAdv = viewMode === "advanced";

  return (
    <ScreenBackground
      accessibilityLabel="Current weather"
      colors={gradient}
      style={styles.container}
    >
      <SafeAreaView style={styles.flex} edges={["top"]}>
        <ScrollView
          contentInsetAdjustmentBehavior="automatic"
          showsVerticalScrollIndicator={false}
          contentContainerStyle={[styles.scroll, readingColumnStyle]}
          refreshControl={
            <RefreshControl
              refreshing={refreshing}
              onRefresh={onRefresh}
              tintColor={theme.colors.text}
              colors={[theme.colors.accent]}
            />
          }
        >
          {/* Top bar — Location & Toggle */}
          <View style={styles.topBar}>
            <Pressable
              accessibilityRole="button"
              accessibilityLabel={`Choose weather location. Current location: ${locationLabel}`}
              onPress={() => router.push("/settings")}
              style={styles.locationContainer}
            >
              <View style={styles.locationRow}>
                <View style={styles.locationDot} />
                <Text style={styles.locationLabelText}>
                  {location.isDevice && !location.isFallback ? "MY LOCATION" : location.isFallback ? "DEFAULT LOCATION" : "LOCATION"}
                </Text>
              </View>
              <View style={styles.locationNameRow}>
                <Text
                  numberOfLines={1}
                  style={styles.locationNameText}
                >
                  {locationName}
                </Text>
                <Text style={styles.expandChevron}>{"\u25BE"}</Text>
              </View>
            </Pressable>

            <SegmentedControl
              accessibilityLabel="Forecast detail"
              options={[
                { label: "Simple", value: "simple" },
                { label: "Adv", value: "advanced" },
              ]}
              value={viewMode}
              onChange={setViewMode}
            />
          </View>
          {location.notice ? (
            <Text
              accessibilityRole={location.isFallback ? "alert" : undefined}
              style={styles.staleNotice}
            >
              {location.notice}
            </Text>
          ) : null}
          {presentation.stale ? (
            <Text accessibilityRole="alert" style={styles.staleNotice}>
              Showing the last available forecast
            </Text>
          ) : forecastAgeMin >= 30 ? (
            // A forecast restored from disk at cold start, shown while it refreshes.
            <Text style={styles.staleNotice}>
              Updated {formatAge(forecastAgeMin)} ago{isFetching ? " · refreshing" : ""}
            </Text>
          ) : null}

          {/* Hero section */}
          <View style={styles.hero}>
            <View style={styles.heroIcon}>
              <WeatherIcon kind={iconKind} size={76} time={isNight ? "night" : "day"} />
            </View>
            <Text style={styles.heroCondition}>{conditionLabel}</Text>
            <View style={styles.heroTempRow}>
              <Text
                testID="current-temperature"
                accessibilityLabel={temp === null ? "Temperature unavailable" : `${temp} degrees ${temperatureUnit}`}
                style={styles.heroTemp}
              >
                {temp ?? "\u2014"}
              </Text>
              {temp !== null ? <Text style={styles.heroDeg}>{"\u00B0"}</Text> : null}
            </View>
            <Text style={styles.heroMeta}>
              Feels {formatDegrees(feels)}   {"\u00B7"}   H {formatDegrees(hi)}   L {formatDegrees(lo)}
            </Text>
          </View>

          <AiBriefingCard briefing={briefing} />
          <IncomingStormCard />

          {/* Nowcast banner */}
          {nowcastBanner ? (
            <Pressable
              accessibilityRole="button"
              accessibilityLabel={`${nowcastBanner.headline}. ${nowcastBanner.sub}`}
              style={styles.nowcastBanner}
              onPress={() => router.push("/nowcast")}
            >
              <View style={styles.nowcastIcon}>
                <WeatherIcon kind="rain" size={24} />
              </View>
              <View style={{ flex: 1 }}>
                <Text style={styles.nowcastHeadline}>{nowcastBanner.headline}</Text>
                <Text style={styles.nowcastSub}>{nowcastBanner.sub}</Text>
              </View>
              <Text style={styles.chevron}>{"\u203A"}</Text>
            </Pressable>
          ) : null}

          {/* Active alerts */}
          {firstAlert && firstAlertEndAt !== null ? (
            <Pressable
              accessibilityRole="button"
              accessibilityLabel={`${firstAlert.properties.event}. ${
                alertStatus.kind === "current" ? "" : `${alertStatus.accessibilityLabel} `
              }Open alert details`}
              style={styles.alertCard}
              onPress={() =>
                router.push({
                  pathname: "/alert/[id]",
                  params: { id: firstAlert.properties.id },
                })
              }
            >
              <View style={styles.alertIndicatorDot} />
              <View style={{ flex: 1 }}>
                <Text style={styles.alertTitle}>{firstAlert.properties.event}</Text>
                <Text style={styles.alertSub} numberOfLines={1}>
                  Until{" "}
                  {new Date(firstAlertEndAt).toLocaleTimeString([], {
                    hour: "numeric",
                    minute: "2-digit",
                  })}
                  {alertStatus.kind === "current" ? "" : ` · ${alertStatus.label}`}
                </Text>
              </View>
              <Text style={styles.chevron}>{"\u203A"}</Text>
            </Pressable>
          ) : alertStatus.kind !== "current" ? (
            <Pressable
              accessibilityRole="button"
              accessibilityLabel={`${alertStatus.accessibilityLabel} Open weather alerts`}
              style={styles.alertCard}
              onPress={() => router.push("/alerts")}
            >
              <View style={styles.alertIndicatorDot} />
              <View style={{ flex: 1 }}>
                <Text accessibilityRole="alert" style={styles.alertTitle}>
                  Alert status {alertStatus.kind === "offline"
                    ? "offline"
                    : alertStatus.kind === "stale"
                      ? "stale"
                      : alertStatus.kind === "checking"
                        ? "checking"
                        : "unavailable"}
                </Text>
                <Text style={styles.alertSub} numberOfLines={2}>
                  {alertStatus.accessibilityLabel}
                </Text>
              </View>
              <Text style={styles.chevron}>{"\u203A"}</Text>
            </Pressable>
          ) : null}

          {/* Hourly strip */}
          <View style={styles.sectionWrap}>
            <SectionLabel>HOURLY</SectionLabel>
          </View>
          <ScrollView
            horizontal
            showsHorizontalScrollIndicator={false}
            contentContainerStyle={styles.hourlyStrip}
          >
            {hourly.map((h, i) => (
              <View
                key={i}
                accessible
                accessibilityLabel={`${h.isNow ? "Now" : h.label}, ${
                  h.temp === null ? "temperature unavailable" : `${h.temp} degrees`
                }, ${h.chance === null ? "precipitation chance unavailable" : `${h.chance} percent chance of precipitation`}`}
                style={[styles.hourlyCell, h.isNow ? styles.hourlyCellNow : null]}
              >
                <Text style={[styles.hourlyTime, h.isNow ? styles.hourlyTimeNow : null]}>
                  {h.label}
                </Text>
                <View style={{ marginVertical: 6 }}>
                  <WeatherIcon kind={h.icon} size={22} time={h.night ? "night" : "day"} />
                </View>
                <Text style={styles.hourlyTemp}>{formatDegrees(h.temp)}</Text>
              </View>
            ))}
          </ScrollView>

          {/* 24h precip chart */}
          <View style={styles.sectionWrap}>
            <SectionLabel trailing={precipTotalIn}>PRECIPITATION · NEXT 24H</SectionLabel>
          </View>
          <View
            accessible
            accessibilityLabel={
              precipTotal === null
                ? "Precipitation over the next 24 hours unavailable."
                : `${precipTotal.toFixed(2)} inches of precipitation expected over the next 24 hours. Bars show each hour's amount.`
            }
            style={styles.card}
          >
            <View style={styles.precipChart}>
              {precipHours.map((amount, i) => {
                // Bars are each hour's amount, scaled to the wettest hour; dry or
                // unknown hours are a faint stub. (The self-hosted Open-Meteo has
                // no precipitation probability, so amounts are what we can show.)
                const barH = amount ? Math.max(3, (amount / precipPeak) * 42) : 2;
                return (
                  <View key={i} style={styles.precipBarSlot}>
                    <View style={[styles.precipBar, { height: barH, opacity: amount ? 1 : 0.25 }]} />
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
          <View style={styles.sectionWrap}>
            <SectionLabel>7-DAY FORECAST</SectionLabel>
          </View>
          <View style={[styles.card, { padding: 0, overflow: "hidden" }]}>
            {daily.map((d, i) => {
              // Days with a missing high or low get no bar rather than a fake range.
              const hasRange = week !== null && d.lo !== null && d.hi !== null;
              const range = week ? week.hi - week.lo || 1 : 1;
              const leftPct = hasRange ? (((d.lo as number) - week!.lo) / range) * 100 : 0;
              const widthPct = hasRange ? (((d.hi as number) - (d.lo as number)) / range) * 100 : 0;
              const nowPct = week && d.now != null ? ((d.now - week.lo) / range) * 100 : 0;
              return (
                <View
                  key={i}
                  style={[
                    styles.dailyRow,
                    i > 0 ? styles.dailyRowBorder : null,
                  ]}
                  accessible
                  accessibilityLabel={`${d.day}, low ${d.lo === null ? "unavailable" : `${d.lo} degrees`}, high ${
                    d.hi === null ? "unavailable" : `${d.hi} degrees`
                  }, ${d.chance === null ? "precipitation chance unavailable" : `${d.chance} percent chance of precipitation`}`}
                >
                  <Text style={[styles.dailyDay, d.day === "Today" ? styles.dailyDayToday : null]}>
                    {d.day}
                  </Text>
                  <View style={{ width: 24, alignItems: "center" }}>
                    <WeatherIcon kind={d.icon} size={21} />
                  </View>
                  <Text style={styles.dailyLo}>{formatDegrees(d.lo)}</Text>
                  <View style={styles.dailyBarTrack}>
                    {hasRange ? (
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
                    ) : null}
                    {d.now != null ? (
                      <View
                        style={[
                          styles.dailyNowDot,
                          { left: `${nowPct}%` },
                        ]}
                      />
                    ) : null}
                  </View>
                  <Text style={styles.dailyHi}>{formatDegrees(d.hi)}</Text>
                </View>
              );
            })}
          </View>

          {/* Mini radar map */}
          <RadarMiniMap />

          {/* Advanced mode: condition tiles and daylight */}
          {isAdv ? (
            <>
              <View style={styles.sectionWrap}>
                <SectionLabel>CONDITIONS</SectionLabel>
              </View>
              <ConditionTiles {...conditions} />
            </>
          ) : null}

          <View style={{ height: 100 }} />
        </ScrollView>
      </SafeAreaView>
    </ScreenBackground>
  );
}

// ───────── helpers

function formatAge(minutes: number): string {
  if (minutes < 90) return `${minutes} min`;
  const hours = Math.round(minutes / 60);
  return `${hours} hr`;
}

const CONDITION_LABELS: Record<ReturnType<typeof getCumulusCondition>, string> = {
  clearDay: "Sunny",
  clearNight: "Clear",
  cloudy: "Cloudy",
  rain: "Rain",
  storm: "Thunderstorms",
  snow: "Snow",
  fog: "Foggy",
  unknown: "Conditions unavailable",
};

function createStyles(theme: WeatherClearTheme) {
  const cumulus = {
    background: theme.colors.canvas,
    accent: theme.colors.accent,
    alert: theme.colors.destructive,
    rain: theme.colors.rain,
    ink: theme.colors.text,
    inkDim: theme.colors.textSecondary,
    inkMuted: theme.colors.textMuted,
    inkFaint: theme.colors.textFaint,
  };
  const cumulusFonts = {
    display: theme.typography.display,
    ui: theme.typography.ui,
  };

  return StyleSheet.create({
  container: { flex: 1 },
  flex: { flex: 1 },
  scroll: { paddingBottom: 32 },
  stateContainer: { flex: 1, backgroundColor: theme.colors.canvas },
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
    backgroundColor: theme.colors.surfaceMuted,
    borderWidth: 1,
    borderColor: theme.colors.divider,
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
    flex: 1,
    minWidth: 0,
    maxWidth: "68%",
    flexDirection: "column",
  },
  staleNotice: {
    paddingHorizontal: 24,
    paddingTop: 4,
    color: theme.colors.warning,
    fontFamily: theme.typography.uiSemibold,
    fontSize: 11,
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
    flexShrink: 1,
    fontFamily: cumulusFonts.display,
    fontSize: 29,
    fontWeight: "500",
    color: cumulus.ink,
    letterSpacing: -0.2,
  },
  expandChevron: {
    fontSize: 16,
    color: theme.colors.textFaint,
    marginLeft: 4,
    marginTop: 4,
  },

  toggleContainer: {
    flexDirection: "row",
    backgroundColor: theme.colors.surfaceMuted,
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
    backgroundColor: theme.colors.surface,
    borderWidth: 1,
    borderColor: theme.colors.border,
    flexDirection: "row",
    alignItems: "center",
    gap: 12,
    boxShadow: theme.dark
      ? "0 4px 12px rgba(0,0,0,0.28)"
      : "0 4px 12px rgba(60,50,40,0.08)",
  },
  nowcastIcon: {
    width: 34,
    height: 34,
    borderRadius: 10,
    backgroundColor: theme.colors.accentSoft,
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
    backgroundColor: theme.dark
      ? "rgba(228,125,125,0.16)"
      : "rgba(223,106,106,0.12)",
    borderWidth: 1,
    borderColor: theme.dark
      ? "rgba(228,125,125,0.36)"
      : "rgba(223,106,106,0.3)",
    flexDirection: "row",
    alignItems: "center",
    gap: 12,
  },
  alertIndicatorDot: { width: 8, height: 8, borderRadius: 4, backgroundColor: cumulus.alert },
  alertTitle: { color: cumulus.alert, fontSize: 14, fontWeight: "700", fontFamily: cumulusFonts.ui },
  alertSub: { color: cumulus.inkDim, fontSize: 12, marginTop: 1, fontFamily: cumulusFonts.ui },
  chevron: { color: cumulus.inkMuted, fontSize: 20, fontWeight: "400" },

  // Section Header
  sectionWrap: {
    paddingHorizontal: 24,
    paddingTop: 20,
    paddingBottom: 8,
  },
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
    backgroundColor: theme.colors.surface,
    borderRadius: 16,
    borderWidth: 1,
    borderColor: theme.colors.border,
  },
  hourlyCellNow: {
    backgroundColor: theme.colors.surfaceMuted,
    borderColor: theme.colors.divider,
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
    backgroundColor: theme.colors.surface,
    borderWidth: 1,
    borderColor: theme.colors.border,
    borderRadius: 20,
    padding: 16,
    boxShadow: theme.dark
      ? "0 3px 10px rgba(0,0,0,0.24)"
      : "0 3px 10px rgba(60,50,40,0.05)",
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
    borderTopColor: theme.colors.divider,
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
    backgroundColor: theme.colors.divider,
    position: "relative",
  },
  dailyNowDot: {
    position: "absolute",
    top: -3,
    width: 12,
    height: 12,
    borderRadius: 6,
    backgroundColor: theme.colors.surface,
    borderWidth: 3,
    borderColor: cumulus.accent,
    marginLeft: -6,
  },

  // Stats Grid
  });
}
