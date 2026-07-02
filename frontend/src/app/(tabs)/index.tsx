/**
 * Cumulus Home screen — Redesigned for Editorial Light.
 * Warm paper background, display serif headers, Simple/Advanced layout gating.
 *
 * Perf structure: each visual section is a React.memo component under
 * src/components/home/ receiving plain derived props; derived arrays are
 * computed once here with useMemo keyed on the forecast query data.
 */
import { useCallback, useMemo, useState } from "react";
import { ScrollView, View, Text, StyleSheet, TouchableOpacity, RefreshControl } from "react-native";
import { SafeAreaView } from "react-native-safe-area-context";
import { LinearGradient } from "expo-linear-gradient";
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
  getWindDirection,
  isNightAt,
} from "../../lib/cumulusTheme";
import {
  CONDITION_LABELS,
  buildNowcastHeadline,
  findStartHourIndex,
  formatHour,
} from "../../lib/forecastDisplay";
import { RadarMiniMap } from "../../components/home/RadarMiniMap";
import { SectionHeader } from "../../components/home/SectionHeader";
import { HomeTopBar } from "../../components/home/HomeTopBar";
import { HeroSection } from "../../components/home/HeroSection";
import { NowcastBanner } from "../../components/home/NowcastBanner";
import { AlertsCard } from "../../components/home/AlertsCard";
import { HourlyStrip, type HourlyEntry } from "../../components/home/HourlyStrip";
import { PrecipChart } from "../../components/home/PrecipChart";
import { DailyList, type DailyEntry } from "../../components/home/DailyList";
import { StatsGrid } from "../../components/home/StatsGrid";
import { SunriseSunsetRow } from "../../components/home/SunriseSunsetRow";
import { SunArcCard } from "../../components/home/SunArcCard";

export default function HomeScreen() {
  useLocation();
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

  // False positive: Open-Meteo's `forecast.current` field trips the React
  // Compiler's ref-`.current` heuristic; deps [forecast] are correct
  // (react-query replaces the object identity on refetch). The compiler is
  // disabled in app.json, so this diagnostic is lint-only.
  // eslint-disable-next-line react-hooks/preserve-manual-memoization
  const derived = useMemo(() => {
    if (!forecast) return null;

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

    // Nowcast banner logic
    const nowcastHeadline = buildNowcastHeadline(forecast.minutely_15);

    // 24h hourly strip
    const hourlyStart = findStartHourIndex(forecast.hourly.time);
    const hourly: HourlyEntry[] = forecast.hourly.time.slice(hourlyStart, hourlyStart + 24).map((t, i) => {
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
    const daily: DailyEntry[] = forecast.daily.time.map((t, i) => {
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
    const sunriseText = sunrise.toLocaleTimeString([], { hour: "numeric", minute: "2-digit" }).toLowerCase();
    const sunsetText = sunset.toLocaleTimeString([], { hour: "numeric", minute: "2-digit" }).toLowerCase();

    return {
      isNight,
      iconKind,
      gradient,
      conditionLabel,
      temp,
      feels,
      hi,
      lo,
      nowcastHeadline,
      hourly,
      precipTotalIn,
      daily,
      weekHi,
      weekLo,
      uv,
      windMph,
      windDeg,
      windCompass,
      humidity,
      dew,
      visibility,
      pressure,
      dayProgress,
      sunriseText,
      sunsetText,
    };
  }, [forecast]);

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

  if (isLoading || !forecast || !derived) {
    return (
      <View style={styles.loadingContainer}>
        <SafeAreaView style={styles.flex}>
          <Text style={styles.loading}>Loading weather...</Text>
        </SafeAreaView>
      </View>
    );
  }

  const {
    isNight,
    iconKind,
    gradient,
    conditionLabel,
    temp,
    feels,
    hi,
    lo,
    nowcastHeadline,
    hourly,
    precipTotalIn,
    daily,
    weekHi,
    weekLo,
    uv,
    windMph,
    windDeg,
    windCompass,
    humidity,
    dew,
    visibility,
    pressure,
    dayProgress,
    sunriseText,
    sunsetText,
  } = derived;

  const locationLabel = activeLocationLabel(locationMode, selectedPlace, devicePlace);
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
          <HomeTopBar
            locationLabel={locationLabel}
            isAdv={isAdv}
            onSetViewMode={setViewMode}
          />

          {/* Hero section */}
          <HeroSection
            iconKind={iconKind}
            isNight={isNight}
            conditionLabel={conditionLabel}
            temp={temp}
            feels={feels}
            hi={hi}
            lo={lo}
          />

          {/* Nowcast banner */}
          {nowcastHeadline && (
            <NowcastBanner
              headline={nowcastHeadline.headline}
              sub={nowcastHeadline.sub}
            />
          )}

          {/* Active alerts */}
          {alertData && alertData.features.length > 0 && (
            <AlertsCard
              id={alertData.features[0].properties.id}
              event={alertData.features[0].properties.event}
              expires={alertData.features[0].properties.expires}
            />
          )}

          {/* Hourly strip */}
          <SectionHeader title="HOURLY" />
          <HourlyStrip hourly={hourly} isNight={isNight} />

          {/* 24h precip chart */}
          <SectionHeader title="PRECIPITATION · 24H" right={`${precipTotalIn}"`} />
          <PrecipChart hourly={hourly} />

          {/* 7-day forecast */}
          <SectionHeader title="7-DAY FORECAST" />
          <DailyList daily={daily} weekHi={weekHi} weekLo={weekLo} />

          {/* Mini radar map */}
          <RadarMiniMap
            headline={nowcastHeadline ? "Precip developing nearby" : "Clear skies overhead"}
          />

          {/* Advanced Mode: Stats grid & Twilight sun path */}
          {isAdv && (
            <>
              <SectionHeader title="CONDITIONS" />
              <StatsGrid
                uv={uv}
                windMph={windMph}
                windDeg={windDeg}
                windCompass={windCompass}
                humidity={humidity}
                dew={dew}
                visibility={visibility}
                pressure={pressure}
              />

              {/* Sunrise/Sunset widgets grid row */}
              <SunriseSunsetRow sunriseText={sunriseText} sunsetText={sunsetText} />

              {/* Sun Arc */}
              <SunArcCard
                sunriseText={sunriseText}
                sunsetText={sunsetText}
                progress={dayProgress}
              />
            </>
          )}

          <View style={{ height: 100 }} />
        </ScrollView>
      </SafeAreaView>
    </LinearGradient>
  );
}

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
});
