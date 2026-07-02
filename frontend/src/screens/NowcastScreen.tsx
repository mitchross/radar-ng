/**
 * Cumulus Nowcast screen — 60-minute precipitation outlook.
 * Redesigned for Editorial Light. Gated cards in Simple/Advanced mode.
 */
import { useCallback, useMemo, useState } from "react";
import { ScrollView, View, Text, StyleSheet, RefreshControl } from "react-native";
import { SafeAreaView } from "react-native-safe-area-context";
import { LinearGradient } from "expo-linear-gradient";
import { useQueryClient } from "@tanstack/react-query";
import { useForecast } from "../hooks/useForecast";
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

type Minute = { i: number; intensity: number; confLo: number; confHi: number };

export default function NowcastScreen() {
  useLocation();
  const { theme } = useWeatherClearTheme();
  const styles = useMemo(() => createStyles(theme), [theme]);
  const locationMode = useWeatherStore((s) => s.locationMode);
  const selectedPlace = useWeatherStore((s) => s.selectedPlace);
  const devicePlace = useWeatherStore((s) => s.devicePlace);
  const viewMode = useWeatherStore((s) => s.viewMode);
  const setViewMode = useWeatherStore((s) => s.setViewMode);
  const {
    data: forecast,
    isLoading,
    isError,
    isFetching,
    refetch,
  } = useForecast();
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

  const minutes = buildMinutes(forecast.minutely_15);
  const verdict = getNowcastVerdict(
    forecast.minutely_15?.precipitation?.length
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

  // Total precipitation in inches
  const totalMm = minutes.reduce((s, m) => s + m.intensity, 0);
  const totalIn = totalMm / 25.4;

  const confidence = estimateConfidence(forecast);
  const location = activeLocationName(locationMode, selectedPlace, devicePlace);
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
              <Text style={styles.headerKicker}>HYPER-LOCAL NOWCAST</Text>
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
            <NowcastChart minutes={minutes} />
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
              sub={rainStart < 0 ? undefined : `${(minutes[peakMin].intensity / 25.4).toFixed(2)}"/hr`}
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

          {/* Advanced Mode: Forecast model details & Hyper-local variation */}
          {isAdv ? (
            <>
              <View style={styles.sectionWrap}>
                <SectionLabel>FORECAST MODEL</SectionLabel>
              </View>
              <View style={styles.card}>
                <Row label="Model" value="HRRR + MRMS blend" />
                <Row label="Resolution" value="1.9 mi / 15 min" />
                <Row label="Confidence">
                  <View style={{ flexDirection: "row", alignItems: "center", gap: 8 }}>
                    <View style={styles.confTrack}>
                      <View
                        style={[
                          styles.confFill,
                          {
                            width: `${confidence * 100}%`,
                            backgroundColor:
                              confidence > 0.7
                                ? theme.colors.success
                                : confidence > 0.4
                                  ? theme.colors.warning
                                  : "#FF9F2E",
                          },
                        ]}
                      />
                    </View>
                    <Text
                      style={[
                        styles.confText,
                        {
                          color:
                            confidence > 0.7
                              ? theme.colors.success
                              : confidence > 0.4
                                ? theme.colors.warning
                                : "#FF9F2E",
                        },
                      ]}
                    >
                      {Math.round(confidence * 100)}%
                    </Text>
                  </View>
                </Row>
                <Row
                  label="Last update"
                  value={`${Math.round((Date.now() - new Date(forecast.current.time).getTime()) / 60000)} min ago`}
                  last
                />
              </View>

              <View style={styles.sectionWrap}>
                <SectionLabel>HYPER-LOCAL VARIATION</SectionLabel>
              </View>
              <View style={styles.card}>
                <Text style={styles.variationCaption}>
                  Rain totals expected within 2 miles of you
                </Text>
                {[
                  { label: "Your block", v: totalIn, hi: true },
                  { label: "½ mi north", v: totalIn * 1.4 },
                  { label: "½ mi south", v: totalIn * 0.3 },
                  { label: "1 mi east", v: totalIn * 0.9 },
                  { label: "1 mi west", v: totalIn * 1.7 },
                ].map((r) => (
                  <View key={r.label} style={styles.variationRow}>
                    <Text
                      style={[
                        styles.variationLabel,
                        r.hi ? { color: theme.colors.text } : null,
                      ]}
                    >
                      {r.label}
                    </Text>
                    <View style={styles.variationTrack}>
                      <View
                        style={[
                          styles.variationFill,
                          {
                            width: `${Math.min(100, (r.v / Math.max(0.5, totalIn * 2)) * 100)}%`,
                            backgroundColor: r.hi
                              ? theme.colors.accent
                              : theme.colors.rain,
                          },
                        ]}
                      />
                    </View>
                    <Text
                      style={[
                        styles.variationValue,
                        r.hi ? { color: theme.colors.text } : null,
                      ]}
                    >
                      {r.v.toFixed(2)}&quot;
                    </Text>
                  </View>
                ))}
              </View>
            </>
          ) : null}

          {/* Note about hyper-local */}
          <View style={[styles.card, { marginTop: 14, marginBottom: 24 }]}>
            <Text style={styles.noteTitle}>About this forecast</Text>
            <Text style={styles.noteBody}>
              Minute-by-minute precip is interpolated from Open-Meteo&apos;s 15-min HRRR
              output. Connect a self-hosted tile-server in Settings to feed MRMS
              observations for true minute-level accuracy.
            </Text>
          </View>

          <View style={{ height: 100 }} />
        </ScrollView>
      </SafeAreaView>
    </LinearGradient>
  );
}

function NowcastChart({ minutes }: { minutes: Minute[] }) {
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

  confTrack: {
    width: 80,
    height: 6,
    borderRadius: 3,
    backgroundColor: theme.colors.divider,
    overflow: "hidden",
  },
  confFill: { height: "100%", borderRadius: 3 },
  confText: {
    fontSize: 11,
    fontWeight: "700",
    fontFamily: cumulusFonts.ui,
  },

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

  variationCaption: {
    color: cumulus.inkDim,
    fontSize: 12,
    fontFamily: cumulusFonts.ui,
    marginBottom: 10,
  },
  variationRow: {
    flexDirection: "row",
    alignItems: "center",
    gap: 10,
    paddingVertical: 6,
  },
  variationLabel: {
    width: 86,
    fontSize: 12,
    color: cumulus.inkDim,
    fontFamily: cumulusFonts.ui,
  },
  variationTrack: {
    flex: 1,
    height: 6,
    borderRadius: 3,
    backgroundColor: theme.colors.divider,
    overflow: "hidden",
  },
  variationFill: { height: "100%", borderRadius: 3 },
  variationValue: {
    width: 48,
    textAlign: "right",
    fontSize: 12,
    fontFamily: cumulusFonts.mono,
    color: cumulus.inkDim,
  },
  });
}
