/**
 * Cumulus Nowcast screen — 60-minute precipitation outlook.
 * Redesigned for Editorial Light. Gated cards in Simple/Advanced mode.
 */
import { useCallback, useMemo, useState } from "react";
import { ScrollView, View, Text, StyleSheet, RefreshControl, Pressable } from "react-native";
import { SafeAreaView } from "react-native-safe-area-context";
import { LinearGradient } from "expo-linear-gradient";
import { useRouter } from "expo-router";
import { useQueryClient } from "@tanstack/react-query";
import { useForecast } from "../hooks/useForecast";
import { useRadarNowcast } from "../hooks/useRadarNowcast";
import { useLocation } from "../hooks/useLocation";
import { activeLocationName } from "../lib/locationLabel";
import { useWeatherStore } from "../stores/useWeatherStore";
import { CONDITION_GRADIENTS, getCumulusCondition, isNightAt } from "../lib/cumulusTheme";
import {
  describeNowcast,
  getForecastScreenState,
  getNowcastVerdict,
} from "../lib/weatherPresentation";
import {
  ScreenState,
  SectionLabel,
  SegmentedControl,
} from "../components/ui/WeatherClearUI";
import { useWeatherClearTheme } from "../theme/WeatherClearThemeProvider";
import type { WeatherClearTheme } from "../theme/weatherClearTheme";
import WeatherIcon from "../components/weather/WeatherIcon";
import { interpolateRadarNowcast } from "../lib/radarNowcast";
import type { RadarNowcastPoint } from "../types/weather";

type Minute = { i: number; intensity: number };

export default function NowcastScreen() {
  useLocation();
  const router = useRouter();
  const { theme } = useWeatherClearTheme();
  const styles = useMemo(() => createStyles(theme), [theme]);
  const locationMode = useWeatherStore((s) => s.locationMode);
  const selectedPlace = useWeatherStore((s) => s.selectedPlace);
  const devicePlace = useWeatherStore((s) => s.devicePlace);
  const viewMode = useWeatherStore((s) => s.viewMode);
  const setViewMode = useWeatherStore((s) => s.setViewMode);
  const setActiveLayer = useWeatherStore((s) => s.setActiveLayer);
  const setTimelineMode = useWeatherStore((s) => s.setTimelineMode);
  const setCurrentFrameIndex = useWeatherStore((s) => s.setCurrentFrameIndex);
  const setIsPlaying = useWeatherStore((s) => s.setIsPlaying);
  const {
    data: forecast,
    isLoading,
    isError,
    isFetching,
    refetch,
  } = useForecast();
  const radarNowcast = useRadarNowcast();
  const queryClient = useQueryClient();
  const [refreshing, setRefreshing] = useState(false);

  const onRefresh = useCallback(async () => {
    setRefreshing(true);
    try {
      await Promise.all([
        queryClient.refetchQueries({ queryKey: ["forecast"] }),
        queryClient.refetchQueries({ queryKey: ["radar-nowcast"] }),
      ]);
    } finally {
      setRefreshing(false);
    }
  }, [queryClient]);

  const openMotionRadar = useCallback(() => {
    setActiveLayer("radar");
    setTimelineMode("forecast");
    setCurrentFrameIndex(-1);
    setIsPlaying(true);
    router.push("/radar");
  }, [router, setActiveLayer, setCurrentFrameIndex, setIsPlaying, setTimelineMode]);

  const presentation = getForecastScreenState({
    data: forecast,
    isLoading,
    isError,
    isFetching,
  });

  if (presentation.kind === "error") {
    return (
      <View style={styles.stateContainer}>
        <ScreenState
          kind="error"
          title="Nowcast unavailable"
          message="The next-hour precipitation forecast could not be loaded."
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
          title="Loading nowcast"
          message="Fetching the next-hour precipitation forecast."
        />
      </View>
    );
  }

  const now = new Date();
  const sunrise = new Date(forecast.daily.sunrise[0]);
  const sunset = new Date(forecast.daily.sunset[0]);
  const isNight = isNightAt(now, sunrise, sunset);
  const condition = getCumulusCondition(forecast.current.weather_code, isNight);
  const gradient = theme.dark
    ? ([theme.colors.canvas, theme.colors.surfaceStrong] as const)
    : CONDITION_GRADIENTS[condition];

  const pointNowcast =
    radarNowcast.data &&
    (radarNowcast.data.status === "ok" || radarNowcast.data.status === "degraded") &&
    radarNowcast.data.points.length > 0
      ? radarNowcast.data
      : null;
  const usingRadarNowcast = pointNowcast !== null;
  const minutes = pointNowcast
    ? buildRadarMinutes(pointNowcast.points)
    : buildMinutes(forecast.minutely_15);
  const verdict = getNowcastVerdict(
    usingRadarNowcast || forecast.minutely_15?.precipitation?.length
      ? minutes.map((minute) => minute.intensity)
      : undefined,
  );
  const rainStart =
    verdict.kind === "starting"
      ? verdict.startMinute
      : verdict.kind === "raining"
        ? 0
        : -1;
  const peakMin =
    verdict.kind === "starting" || verdict.kind === "raining"
      ? verdict.peakMinute
      : 0;
  const rainEndMin =
    verdict.kind === "starting" || verdict.kind === "raining"
      ? verdict.endMinute
      : -1;

  // Chart intensity is inches/hour; integrate sixty one-minute samples.
  const totalIn = minutes.reduce((sum, minute) => sum + minute.intensity / 60, 0);

  const location = activeLocationName(locationMode, selectedPlace, devicePlace);
  const isAdv = viewMode === "advanced";
  const radarFrameCount = pointNowcast?.points.length ?? 0;
  const radarResolution = pointNowcast?.spatial_resolution_km;

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
              tintColor={theme.colors.text}
              colors={[theme.colors.accent]}
            />
          }
        >
          {/* Header */}
          <View
            accessibilityLabel="Next hour precipitation"
            style={styles.header}
          >
            <View style={styles.headerCopy}>
              <Text style={styles.headerKicker}>NEXT-HOUR PRECIPITATION</Text>
              <Text numberOfLines={1} style={styles.headerLocation}>{location}</Text>
            </View>
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
          {presentation.stale ? (
            <Text accessibilityRole="alert" style={styles.staleNotice}>
              Showing the last available nowcast
            </Text>
          ) : null}

          {/* Hero verdict */}
          <View style={styles.hero}>
            {verdict.kind === "unavailable" ? (
              <Text style={styles.heroDry}>
                Forecast{"\n"}
                <Text style={styles.heroDrySub}>unavailable</Text>
              </Text>
            ) : verdict.kind === "dry" ? (
              <Text style={styles.heroDry}>
                No rain expected{"\n"}
                <Text style={styles.heroDrySub}>for the next hour</Text>
              </Text>
            ) : verdict.kind === "raining" ? (
              <Text style={styles.heroDry}>
                Raining <Text style={{ color: theme.colors.rain }}>now.</Text>
              </Text>
            ) : (
              <Text style={styles.heroDry}>
                Rain starts in{"\n"}
                <Text style={{ color: theme.colors.rain }}>
                  {rainStart} {rainStart === 1 ? "minute" : "minutes"}
                </Text>
              </Text>
            )}
            {rainStart >= 0 && rainEndMin > 0 ? (
              <Text style={styles.heroSub}>
                Expected to last ~{rainEndMin - rainStart} min
                <Text style={styles.heroDim}>  {"\u00B7"}  peaks at </Text>
                <Text style={styles.heroStrong}>+{peakMin}m</Text>
              </Text>
            ) : null}
          </View>

          <Pressable
            onPress={openMotionRadar}
            accessibilityRole="button"
            accessibilityLabel="Play the next hour motion radar"
            style={({ pressed }) => [styles.motionCard, pressed ? styles.motionCardPressed : null]}
          >
            <View style={styles.motionPlay}>
              <View style={styles.motionPlayIcon} />
            </View>
            <View style={styles.motionCopy}>
              <Text style={styles.motionKicker}>PLAY MOTION RADAR</Text>
              <Text style={styles.motionTitle}>
                {usingRadarNowcast
                  ? `${radarFrameCount} MRMS frames · next ${pointNowcast?.horizon_minutes ?? 60} min`
                  : "Open the live forecast timeline"}
              </Text>
              <Text style={styles.motionSub}>
                {usingRadarNowcast
                  ? verdict.kind === "dry"
                    ? "Dry at your point; animate storms around you"
                    : "See precipitation move through your area"
                  : "Motion guidance is warming; observed radar is available"}
              </Text>
            </View>
            <Text style={styles.motionChevron}>›</Text>
          </Pressable>

          {!usingRadarNowcast ? (
            <Text accessibilityRole="alert" style={styles.guidanceNotice}>
              Showing 15-minute model guidance until MRMS point nowcast is ready.
            </Text>
          ) : null}

          {/* Intensity chart */}
          <View
            accessible
            accessibilityLabel={describeNowcast(verdict)}
            style={styles.card}
          >
            <View style={styles.chartHeader}>
              <Text style={styles.chartLabel}>INTENSITY {"\u00B7"} IN/HR</Text>
              <Text style={styles.chartLabel}>NEXT 60 MIN</Text>
            </View>
            <NowcastChart minutes={minutes} dry={verdict.kind === "dry"} />
            <View style={styles.chartAxis}>
              <Text style={styles.axisTick}>NOW</Text>
              <Text style={styles.axisTick}>+15</Text>
              <Text style={styles.axisTick}>+30</Text>
              <Text style={styles.axisTick}>+45</Text>
              <Text style={styles.axisTick}>+60</Text>
            </View>
            <View style={styles.scaleRow}>
              <Text style={styles.axisTick}>LIGHT</Text>
              <LinearGradient
                colors={["#7ae5a8", "#4d7fb8", "#3f6fd6", "#c2603a", "#df6a6a"]}
                start={{ x: 0, y: 0 }}
                end={{ x: 1, y: 0 }}
                style={styles.scaleGrad}
              />
              <Text style={styles.axisTick}>INTENSE</Text>
            </View>
          </View>

          {/* Key moments */}
          <View style={styles.sectionWrap}>
            <SectionLabel>KEY MOMENTS</SectionLabel>
          </View>
          <View style={styles.keyGrid}>
            <KeyCard
              label="STARTS"
              value={rainStart < 0 ? "\u2014" : `+${rainStart}m`}
              icon="rain"
              color={theme.colors.rain}
            />
            <KeyCard
              label="PEAK"
              value={rainStart < 0 ? "\u2014" : `+${peakMin}m`}
              sub={rainStart < 0 ? undefined : `${minutes[peakMin].intensity.toFixed(2)}"/hr`}
              icon="heavyRain"
              color={theme.colors.hot}
            />
            <KeyCard
              label="ENDS"
              value={rainEndMin < 0 ? "\u2014" : `+${rainEndMin}m`}
              icon="partlyCloudy"
              color={theme.colors.warning}
            />
            <KeyCard
              label="TOTAL"
              value={`${totalIn.toFixed(2)}"`}
              sub="next hour"
              icon="cloudy"
              color={theme.colors.accent}
            />
          </View>

          {/* Advanced Mode: honest model provenance and limitations */}
          {isAdv ? (
            <>
              <View style={styles.sectionWrap}>
                <SectionLabel>FORECAST MODEL</SectionLabel>
              </View>
              <View style={styles.card}>
                <Row
                  label="Product"
                  value={usingRadarNowcast ? "Motion radar at your point" : "Point precipitation guidance"}
                />
                <Row
                  label="Source"
                  value={usingRadarNowcast ? "NOAA MRMS + pySTEPS" : "Self-hosted Open-Meteo"}
                />
                <Row
                  label="Native interval"
                  value={usingRadarNowcast ? `${pointNowcast?.step_minutes ?? 5} minutes` : "15 minutes"}
                />
                {usingRadarNowcast && radarResolution != null ? (
                  <Row label="Spatial resolution" value={`~${radarResolution} km`} />
                ) : null}
                <Row
                  label="Last update"
                  value={`${Math.max(0, Math.round((Date.now() - new Date(
                    pointNowcast?.issued_at
                      ? pointNowcast.issued_at
                      : forecast.current.time,
                  ).getTime()) / 60000))} min ago`}
                  last
                />
              </View>

              <View style={styles.sectionWrap}>
                <SectionLabel>LIMITS</SectionLabel>
              </View>
              <View style={styles.card}>
                <Text style={styles.noteBody}>
                  {usingRadarNowcast
                    ? "This is reflectivity advected from recent MRMS observations. Rain rate uses a standard Z-R estimate and should not be read as a rain-gauge measurement."
                    : "This is model guidance for the selected point, interpolated between 15-minute values. It does not claim block-level variation or a measured probability of confidence."}
                </Text>
              </View>
            </>
          ) : null}

          {/* Product provenance */}
          <View style={[styles.card, { marginTop: 14, marginBottom: 24 }]}>
            <Text style={styles.noteTitle}>About this forecast</Text>
            <Text style={styles.noteBody}>
              {usingRadarNowcast
                ? "The bars sample your self-hosted MRMS motion nowcast at this location. Play Motion Radar to see the full 0–60 minute field evolve around you."
                : "The bars interpolate self-hosted Open-Meteo 15-minute guidance while the MRMS point product warms up. Observed and forecast radar remain available on the Radar screen."}
            </Text>
          </View>

          <View style={{ height: 100 }} />
        </ScrollView>
      </SafeAreaView>
    </LinearGradient>
  );
}

function NowcastChart({ minutes, dry }: { minutes: Minute[]; dry: boolean }) {
  const { theme } = useWeatherClearTheme();
  const styles = useMemo(() => createStyles(theme), [theme]);
  const H = 140;
  const maxI = Math.max(0.5, ...minutes.map((m) => m.intensity));
  return (
    <View style={styles.chartBox}>
      {[0.25, 0.5, 0.75].map((y) => (
        <View
          key={y}
          style={[styles.gridLine, { top: H - y * H * 0.95 }]}
        />
      ))}
      <View style={styles.barsRow}>
        {minutes.map((m, i) => {
          const h = Math.max(1, (m.intensity / maxI) * H * 0.95);
          const color = intensityColor(m.intensity / maxI, theme);
          return (
            <View
              key={i}
              style={{
                flex: 1,
                height: h,
                marginHorizontal: 0.4,
                backgroundColor: color,
                opacity: m.intensity > 0.02 ? 1 : 0.25,
                borderRadius: 1,
              }}
            />
          );
        })}
      </View>
      {dry ? (
        <View pointerEvents="none" style={styles.dryChartLabel}>
          <Text style={styles.dryChartText}>DRY AT YOUR LOCATION</Text>
        </View>
      ) : null}
      <View style={styles.baseline} />
    </View>
  );
}

function intensityColor(pct: number, theme: WeatherClearTheme): string {
  if (pct < 0.15) return "#7ae5a8";
  if (pct < 0.35) return theme.colors.rain;
  if (pct < 0.6) return theme.colors.cold;
  if (pct < 0.85) return theme.colors.accent;
  return theme.colors.hot;
}

function KeyCard({
  label,
  value,
  sub,
  icon,
  color,
}: {
  label: string;
  value: string;
  sub?: string;
  icon: Parameters<typeof WeatherIcon>[0]["kind"];
  color: string;
}) {
  const { theme } = useWeatherClearTheme();
  const styles = useMemo(() => createStyles(theme), [theme]);
  return (
    <View
      accessible
      accessibilityLabel={`${label}: ${value}${sub ? `, ${sub}` : ""}`}
      style={styles.keyCard}
    >
      <View style={[styles.keyIcon, { backgroundColor: `${color}16` }]}>
        <WeatherIcon kind={icon} size={26} />
      </View>
      <View style={{ flex: 1 }}>
        <Text style={styles.keyLabel}>{label}</Text>
        <Text style={styles.keyValue}>{value}</Text>
        {sub ? <Text style={styles.keySub}>{sub}</Text> : null}
      </View>
    </View>
  );
}

function Row({
  label,
  value,
  children,
  last,
}: {
  label: string;
  value?: string;
  children?: React.ReactNode;
  last?: boolean;
}) {
  const { theme } = useWeatherClearTheme();
  const styles = useMemo(() => createStyles(theme), [theme]);
  return (
    <View style={[styles.row, last ? null : styles.rowBorder]}>
      <Text style={styles.rowLabel}>{label}</Text>
      {children ?? <Text style={styles.rowValue}>{value}</Text>}
    </View>
  );
}

// Helper: build minute intervals
function buildMinutes(minutely: { time: string[]; precipitation: number[] } | undefined): Minute[] {
  if (!minutely || minutely.precipitation.length === 0) {
    return Array.from({ length: 60 }, (_, i) => ({ i, intensity: 0 }));
  }
  const now = Date.now();
  const startIdx = Math.max(
    0,
    minutely.time.findIndex((t) => new Date(t).getTime() >= now - 7.5 * 60_000),
  );
  const quarters = minutely.precipitation
    .slice(startIdx, startIdx + 5)
    // The API requests precipitation_unit=inch; convert each 15-minute
    // accumulation to an hourly rate for the chart.
    .map((amountInches) => amountInches * 4);
  while (quarters.length < 5) quarters.push(0);

  const out: Minute[] = [];
  for (let i = 0; i < 60; i++) {
    const q = Math.min(3, Math.floor(i / 15));
    const frac = (i % 15) / 15;
    const intensity = Math.max(
      0,
      quarters[q] * (1 - frac) + quarters[q + 1] * frac,
    );
    out.push({ i, intensity });
  }
  return out;
}

function buildRadarMinutes(points: RadarNowcastPoint[]): Minute[] {
  return interpolateRadarNowcast(points).map((intensity, i) => ({ i, intensity }));
}


function createStyles(theme: WeatherClearTheme) {
  const cumulus = {
    background: theme.colors.canvas,
    ink: theme.colors.text,
    inkDim: theme.colors.textSecondary,
    inkMuted: theme.colors.textMuted,
    inkFaint: theme.colors.textFaint,
  };
  const cumulusFonts = {
    display: theme.typography.display,
    ui: theme.typography.ui,
    mono: theme.typography.mono,
  };

  return StyleSheet.create({
  container: { flex: 1 },
  flex: { flex: 1 },
  scroll: { paddingBottom: 120 },
  stateContainer: { flex: 1, backgroundColor: theme.colors.canvas },
  loadingContainer: { flex: 1, backgroundColor: cumulus.background },
  loading: {
    color: cumulus.inkDim,
    fontSize: 16,
    textAlign: "center",
    marginTop: 120,
    fontFamily: cumulusFonts.ui,
  },

  header: {
    flexDirection: "row",
    alignItems: "center",
    justifyContent: "space-between",
    gap: 12,
    paddingHorizontal: 20,
    paddingTop: 16,
    paddingBottom: 8,
  },
  headerCopy: {
    flex: 1,
    minWidth: 0,
  },
  staleNotice: {
    paddingHorizontal: 20,
    paddingTop: 4,
    color: theme.colors.warning,
    fontFamily: theme.typography.uiSemibold,
    fontSize: 11,
  },
  backBtn: {
    width: 36,
    height: 36,
    borderRadius: 12,
    backgroundColor: theme.colors.surfaceMuted,
    borderWidth: 1,
    borderColor: theme.colors.divider,
    alignItems: "center",
    justifyContent: "center",
  },
  backChev: { color: cumulus.ink, fontSize: 22, fontWeight: "500", marginTop: -2 },
  headerKicker: {
    color: cumulus.inkMuted,
    fontSize: 11,
    fontWeight: "700",
    letterSpacing: 1.6,
    fontFamily: cumulusFonts.ui,
  },
  headerLocation: {
    color: cumulus.ink,
    fontSize: 15,
    fontWeight: "600",
    marginTop: 2,
    fontFamily: cumulusFonts.ui,
  },

  hero: { paddingHorizontal: 24, paddingTop: 20 },
  heroDry: {
    color: cumulus.ink,
    fontSize: 46,
    fontWeight: "400",
    letterSpacing: -1.2,
    lineHeight: 48,
    fontFamily: cumulusFonts.display,
  },
  heroDrySub: {
    fontSize: 22,
    color: cumulus.inkDim,
    fontStyle: "italic",
    fontFamily: cumulusFonts.display,
  },
  heroSub: {
    color: cumulus.inkMuted,
    fontSize: 13,
    marginTop: 10,
    fontFamily: cumulusFonts.ui,
    fontWeight: "500",
  },
  heroStrong: { color: cumulus.ink, fontWeight: "600" },
  heroDim: { color: cumulus.inkFaint },

  motionCard: {
    marginHorizontal: 16,
    marginTop: 20,
    minHeight: 82,
    borderRadius: 20,
    padding: 14,
    flexDirection: "row",
    alignItems: "center",
    gap: 12,
    backgroundColor: theme.colors.surfaceStrong,
    borderWidth: 1,
    borderColor: theme.colors.accentBorder,
  },
  motionCardPressed: { opacity: 0.72 },
  motionPlay: {
    width: 46,
    height: 46,
    borderRadius: 23,
    alignItems: "center",
    justifyContent: "center",
    backgroundColor: theme.colors.accent,
  },
  motionPlayIcon: {
    width: 0,
    height: 0,
    marginLeft: 3,
    borderTopWidth: 6,
    borderBottomWidth: 6,
    borderLeftWidth: 10,
    borderTopColor: "transparent",
    borderBottomColor: "transparent",
    borderLeftColor: "#FFFFFF",
  },
  motionCopy: { flex: 1, minWidth: 0 },
  motionKicker: {
    color: theme.colors.accent,
    fontSize: 9,
    fontWeight: "800",
    letterSpacing: 1.3,
    fontFamily: cumulusFonts.ui,
  },
  motionTitle: {
    color: cumulus.ink,
    fontSize: 14,
    fontWeight: "700",
    marginTop: 3,
    fontFamily: cumulusFonts.ui,
  },
  motionSub: {
    color: cumulus.inkMuted,
    fontSize: 11,
    lineHeight: 15,
    marginTop: 2,
    fontFamily: cumulusFonts.ui,
  },
  motionChevron: { color: theme.colors.accent, fontSize: 28, fontWeight: "300" },
  guidanceNotice: {
    marginHorizontal: 24,
    marginTop: 9,
    color: theme.colors.warning,
    fontSize: 11,
    lineHeight: 15,
    fontFamily: cumulusFonts.ui,
  },

  card: {
    marginHorizontal: 16,
    marginTop: 20,
    backgroundColor: theme.colors.surface,
    borderWidth: 1,
    borderColor: theme.colors.border,
    borderRadius: 20,
    padding: 16,
    boxShadow: theme.dark
      ? "0 3px 10px rgba(0,0,0,0.24)"
      : "0 3px 10px rgba(60,50,40,0.05)",
  },

  chartHeader: {
    flexDirection: "row",
    justifyContent: "space-between",
    marginBottom: 10,
  },
  chartLabel: {
    color: cumulus.inkMuted,
    fontSize: 9,
    fontWeight: "700",
    letterSpacing: 1.2,
    fontFamily: cumulusFonts.ui,
  },
  chartBox: {
    height: 140,
    backgroundColor: "transparent",
    position: "relative",
    justifyContent: "flex-end",
  },
  gridLine: {
    position: "absolute",
    left: 0,
    right: 0,
    height: StyleSheet.hairlineWidth,
    backgroundColor: theme.colors.divider,
  },
  barsRow: {
    flexDirection: "row",
    alignItems: "flex-end",
    height: 140,
  },
  dryChartLabel: {
    position: "absolute",
    left: 0,
    right: 0,
    top: 58,
    alignItems: "center",
  },
  dryChartText: {
    color: cumulus.inkFaint,
    fontSize: 9,
    fontWeight: "800",
    letterSpacing: 1.3,
    fontFamily: cumulusFonts.ui,
  },
  baseline: {
    position: "absolute",
    left: 0,
    right: 0,
    bottom: 0,
    height: 1,
    backgroundColor: theme.colors.border,
  },
  chartAxis: {
    flexDirection: "row",
    justifyContent: "space-between",
    marginTop: 8,
  },
  axisTick: {
    color: cumulus.inkMuted,
    fontSize: 10,
    fontWeight: "700",
    fontFamily: cumulusFonts.ui,
  },
  scaleRow: {
    flexDirection: "row",
    alignItems: "center",
    gap: 8,
    marginTop: 14,
  },
  scaleGrad: {
    flex: 1,
    height: 6,
    borderRadius: 3,
  },

  sectionWrap: { paddingHorizontal: 24, paddingTop: 24, paddingBottom: 10 },
  sectionHeader: { paddingHorizontal: 24, paddingTop: 24, paddingBottom: 10 },
  sectionTitle: {
    color: cumulus.inkMuted,
    fontSize: 10,
    fontWeight: "800",
    letterSpacing: 1.6,
    fontFamily: cumulusFonts.ui,
  },

  keyGrid: {
    flexDirection: "row",
    flexWrap: "wrap",
    marginHorizontal: 16,
    gap: 9,
  },
  keyCard: {
    flexBasis: "47%",
    flexGrow: 1,
    backgroundColor: theme.colors.surface,
    borderWidth: 1,
    borderColor: theme.colors.border,
    borderRadius: 16,
    padding: 12,
    flexDirection: "row",
    alignItems: "center",
    gap: 12,
  },
  keyIcon: {
    width: 38,
    height: 38,
    borderRadius: 10,
    alignItems: "center",
    justifyContent: "center",
  },
  keyLabel: {
    color: cumulus.inkMuted,
    fontSize: 9,
    fontWeight: "700",
    letterSpacing: 1.2,
    fontFamily: cumulusFonts.ui,
  },
  keyValue: {
    color: cumulus.ink,
    fontSize: 20,
    fontFamily: cumulusFonts.display,
    fontWeight: "500",
    marginTop: 2,
  },
  keySub: {
    color: cumulus.inkDim,
    fontSize: 10,
    fontFamily: cumulusFonts.ui,
    fontWeight: "500",
    marginTop: 2,
  },

  row: {
    flexDirection: "row",
    justifyContent: "space-between",
    alignItems: "center",
    paddingVertical: 12,
  },
  rowBorder: {
    borderBottomWidth: 1,
    borderBottomColor: theme.colors.divider,
  },
  rowLabel: { color: cumulus.inkDim, fontSize: 13, fontFamily: cumulusFonts.ui, fontWeight: "500" },
  rowValue: { color: cumulus.ink, fontSize: 14, fontWeight: "600", fontFamily: cumulusFonts.ui },

  noteTitle: {
    color: cumulus.ink,
    fontSize: 14,
    fontWeight: "600",
    fontFamily: cumulusFonts.ui,
    marginBottom: 6,
  },
  noteBody: {
    color: cumulus.inkDim,
    fontSize: 12,
    lineHeight: 18,
    fontFamily: cumulusFonts.ui,
  },

  });
}
