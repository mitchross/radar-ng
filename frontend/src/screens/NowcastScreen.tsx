/**
 * Cumulus Nowcast screen — 60-minute precipitation outlook.
 * Redesigned for Editorial Light. Gated cards in Simple/Advanced mode.
 *
 * Perf structure: each visual section is a React.memo component under
 * src/components/nowcast/ receiving plain derived props; derived arrays are
 * computed once here with useMemo keyed on the forecast query data.
 */
import { useCallback, useMemo, useState } from "react";
import { ScrollView, View, Text, StyleSheet, RefreshControl } from "react-native";
import { SafeAreaView } from "react-native-safe-area-context";
import { LinearGradient } from "expo-linear-gradient";
import { useRouter } from "expo-router";
import { useQueryClient } from "@tanstack/react-query";
import { useForecast } from "../hooks/useForecast";
import { useLocation } from "../hooks/useLocation";
import { activeLocationLabel } from "../lib/locationLabel";
import { useWeatherStore } from "../stores/useWeatherStore";
import {
  cumulus,
  cumulusFonts,
  CONDITION_GRADIENTS,
  getCumulusCondition,
  isNightAt,
} from "../lib/cumulusTheme";
import { NowcastHeader } from "../components/nowcast/NowcastHeader";
import { NowcastHero } from "../components/nowcast/NowcastHero";
import { IntensityChartCard, type Minute } from "../components/nowcast/IntensityChartCard";
import { SectionHeader } from "../components/nowcast/SectionHeader";
import { KeyMomentsGrid } from "../components/nowcast/KeyMomentsGrid";
import { ForecastModelCard } from "../components/nowcast/ForecastModelCard";
import { HyperLocalVariationCard } from "../components/nowcast/HyperLocalVariationCard";
import { AboutNoteCard } from "../components/nowcast/AboutNoteCard";

export default function NowcastScreen() {
  useLocation();
  const router = useRouter();
  const locationMode = useWeatherStore((s) => s.locationMode);
  const selectedPlace = useWeatherStore((s) => s.selectedPlace);
  const devicePlace = useWeatherStore((s) => s.devicePlace);
  const viewMode = useWeatherStore((s) => s.viewMode);
  const { data: forecast, isLoading } = useForecast();
  const queryClient = useQueryClient();
  const [refreshing, setRefreshing] = useState(false);

  const onRefresh = useCallback(async () => {
    setRefreshing(true);
    try {
      await queryClient.invalidateQueries({ queryKey: ["forecast"] });
    } finally {
      setRefreshing(false);
    }
  }, [queryClient]);

  const onBack = useCallback(() => router.back(), [router]);

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
    const condition = getCumulusCondition(forecast.current.weather_code, isNight);
    const gradient = CONDITION_GRADIENTS[condition];

    const minutes = buildMinutes(forecast.minutely_15);
    const rainStart = minutes.findIndex((m) => m.intensity > 0.08);
    const peakMin = minutes.reduce((best, m, i) => (m.intensity > minutes[best].intensity ? i : best), 0);
    const reversedEnd = [...minutes].map((m) => m.intensity).reverse().findIndex((v) => v > 0.05);
    const rainEndMin = reversedEnd >= 0 ? 59 - reversedEnd : -1;

    // Total precipitation in inches
    const totalMm = minutes.reduce((s, m) => s + m.intensity, 0);
    const totalIn = totalMm / 25.4;
    const peakRate =
      rainStart < 0 ? undefined : `${(minutes[peakMin].intensity / 25.4).toFixed(2)}"/hr`;

    const confidence = estimateConfidence(forecast);
    const lastUpdateMin = Math.round(
      (Date.now() - new Date(forecast.current.time).getTime()) / 60000,
    );

    return {
      gradient,
      minutes,
      rainStart,
      peakMin,
      rainEndMin,
      totalIn,
      peakRate,
      confidence,
      lastUpdateMin,
    };
  }, [forecast]);

  if (isLoading || !forecast || !derived) {
    return (
      <View style={styles.loadingContainer}>
        <SafeAreaView style={styles.flex}>
          <Text style={styles.loading}>Loading...</Text>
        </SafeAreaView>
      </View>
    );
  }

  const {
    gradient,
    minutes,
    rainStart,
    peakMin,
    rainEndMin,
    totalIn,
    peakRate,
    confidence,
    lastUpdateMin,
  } = derived;

  const location = activeLocationLabel(locationMode, selectedPlace, devicePlace);
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
          {/* Header */}
          <NowcastHeader location={location} onBack={onBack} />

          {/* Hero verdict */}
          <NowcastHero rainStart={rainStart} rainEndMin={rainEndMin} peakMin={peakMin} />

          {/* Intensity chart */}
          <IntensityChartCard minutes={minutes} />

          {/* Key moments */}
          <SectionHeader title="KEY MOMENTS" />
          <KeyMomentsGrid
            rainStart={rainStart}
            peakMin={peakMin}
            rainEndMin={rainEndMin}
            totalIn={totalIn}
            peakRate={peakRate}
          />

          {/* Advanced Mode: Forecast model details & Hyper-local variation */}
          {isAdv && (
            <>
              <SectionHeader title="FORECAST MODEL" />
              <ForecastModelCard confidence={confidence} lastUpdateMin={lastUpdateMin} />

              <SectionHeader title="HYPER-LOCAL VARIATION" />
              <HyperLocalVariationCard totalIn={totalIn} />
            </>
          )}

          {/* Note about hyper-local */}
          <AboutNoteCard />

          <View style={{ height: 100 }} />
        </ScrollView>
      </SafeAreaView>
    </LinearGradient>
  );
}

// Helper: build minute intervals
function buildMinutes(minutely: { time: string[]; precipitation: number[] } | undefined): Minute[] {
  if (!minutely || minutely.precipitation.length === 0) {
    return Array.from({ length: 60 }, (_, i) => ({ i, intensity: 0, confLo: 0, confHi: 0 }));
  }
  const now = Date.now();
  const startIdx = Math.max(
    0,
    minutely.time.findIndex((t) => new Date(t).getTime() >= now - 7.5 * 60_000),
  );
  const quarters = minutely.precipitation.slice(startIdx, startIdx + 5);
  while (quarters.length < 5) quarters.push(0);

  const out: Minute[] = [];
  for (let i = 0; i < 60; i++) {
    const q = Math.min(3, Math.floor(i / 15));
    const frac = (i % 15) / 15;
    const intensity = Math.max(
      0,
      quarters[q] * (1 - frac) + quarters[q + 1] * frac,
    );
    const spread = 0.15 + i * 0.005;
    out.push({
      i,
      intensity,
      confLo: Math.max(0, intensity - spread),
      confHi: intensity + spread,
    });
  }
  return out;
}

function estimateConfidence(forecast: { current: { time: string } }): number {
  const ageMin = (Date.now() - new Date(forecast.current.time).getTime()) / 60_000;
  if (ageMin < 5) return 0.95;
  if (ageMin < 15) return 0.8;
  if (ageMin < 30) return 0.65;
  return 0.5;
}

const styles = StyleSheet.create({
  container: { flex: 1 },
  flex: { flex: 1 },
  scroll: { paddingBottom: 120 },
  loadingContainer: { flex: 1, backgroundColor: cumulus.background },
  loading: {
    color: cumulus.inkDim,
    fontSize: 16,
    textAlign: "center",
    marginTop: 120,
    fontFamily: cumulusFonts.ui,
  },
});
