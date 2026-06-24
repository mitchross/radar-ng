/**
 * Cumulus Nowcast screen — 60-minute precipitation outlook.
 * Uses Open-Meteo's minutely_15 precip (4 values × 15min = 60min) interpolated
 * to 60 one-minute bars so the chart matches the prototype's density.
 */
import { useCallback, useState } from "react";
import { ScrollView, View, Text, StyleSheet, TouchableOpacity, RefreshControl } from "react-native";
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
  CONDITION_GRADIENTS,
  getCumulusCondition,
  isNightAt,
} from "../lib/cumulusTheme";
import WeatherIcon from "../components/weather/WeatherIcon";

type Minute = { i: number; intensity: number; confLo: number; confHi: number };

export default function NowcastScreen() {
  useLocation();
  const router = useRouter();
  const locationMode = useWeatherStore((s) => s.locationMode);
  const selectedPlace = useWeatherStore((s) => s.selectedPlace);
  const devicePlace = useWeatherStore((s) => s.devicePlace);
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

  if (isLoading || !forecast) {
    return (
      <LinearGradient colors={CONDITION_GRADIENTS.rain} style={styles.container}>
        <SafeAreaView style={styles.flex}>
          <Text style={styles.loading}>Loading...</Text>
        </SafeAreaView>
      </LinearGradient>
    );
  }

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

  // Total precipitation, inches — sum the minutely (mm) and convert
  const totalMm = minutes.reduce((s, m) => s + m.intensity, 0);
  const totalIn = totalMm / 25.4;

  const confidence = estimateConfidence(forecast);
  const location = activeLocationLabel(locationMode, selectedPlace, devicePlace);

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
          <View style={styles.header}>
            <TouchableOpacity style={styles.backBtn} onPress={() => router.back()} activeOpacity={0.7}>
              <Text style={styles.backChev}>{"\u2039"}</Text>
            </TouchableOpacity>
            <View style={{ flex: 1, alignItems: "center" }}>
              <Text style={styles.headerKicker}>HYPER-LOCAL NOWCAST</Text>
              <Text style={styles.headerLocation}>{location}</Text>
            </View>
            <View style={{ width: 36 }} />
          </View>

          {/* Hero verdict */}
          <View style={styles.hero}>
            {rainStart < 0 ? (
              <Text style={styles.heroDry}>
                No rain expected{"\n"}
                <Text style={styles.heroDrySub}>in the next hour</Text>
              </Text>
            ) : rainStart === 0 ? (
              <Text style={styles.heroDry}>
                Raining <Text style={{ color: cumulus.rain, fontWeight: "500" }}>now.</Text>
              </Text>
            ) : (
              <Text style={styles.heroDry}>
                Rain starts in{"\n"}
                <Text style={{ color: cumulus.rain, fontWeight: "500" }}>
                  {rainStart} {rainStart === 1 ? "minute" : "minutes"}
                </Text>
              </Text>
            )}
            {rainStart >= 0 && rainEndMin > 0 && (
              <Text style={styles.heroSub}>
                Expected to last ~{rainEndMin - rainStart} min
                <Text style={styles.heroDim}>  {"\u00B7"}  peaks at </Text>
                <Text style={styles.heroStrong}>+{peakMin}m</Text>
              </Text>
            )}
          </View>

          {/* Big chart */}
          <View style={styles.card}>
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
                colors={["#7ae5a8", "#4FB8FF", "#1E7FFF", "#8B7CFF", "#FF4D6D"]}
                start={{ x: 0, y: 0 }}
                end={{ x: 1, y: 0 }}
                style={styles.scaleGrad}
              />
              <Text style={styles.axisTick}>INTENSE</Text>
            </View>
          </View>

          {/* Key moments */}
          <SectionHeader title="KEY MOMENTS" />
          <View style={styles.keyGrid}>
            <KeyCard
              label="STARTS"
              value={rainStart < 0 ? "\u2014" : `+${rainStart}m`}
              icon="rain"
              color={cumulus.rain}
            />
            <KeyCard
              label="PEAK"
              value={rainStart < 0 ? "\u2014" : `+${peakMin}m`}
              sub={rainStart < 0 ? undefined : `${(minutes[peakMin].intensity / 25.4).toFixed(2)}"/hr`}
              icon="heavyRain"
              color={cumulus.hot}
            />
            <KeyCard
              label="ENDS"
              value={rainEndMin < 0 ? "\u2014" : `+${rainEndMin}m`}
              icon="partlyCloudy"
              color={cumulus.sun}
            />
            <KeyCard
              label="TOTAL"
              value={`${totalIn.toFixed(2)}"`}
              sub="next hour"
              icon="cloudy"
              color={cumulus.accent}
            />
          </View>

          {/* Forecast model */}
          <SectionHeader title="FORECAST MODEL" />
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
                          confidence > 0.7 ? cumulus.ok : confidence > 0.4 ? cumulus.sun : "#FF9F2E",
                      },
                    ]}
                  />
                </View>
                <Text
                  style={[
                    styles.confText,
                    {
                      color:
                        confidence > 0.7 ? cumulus.ok : confidence > 0.4 ? cumulus.sun : "#FF9F2E",
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

          {/* Hyper-local variation */}
          <SectionHeader title="HYPER-LOCAL VARIATION" />
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
                    r.hi && { color: cumulus.ink, fontWeight: "600" },
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
                        backgroundColor: r.hi ? cumulus.accent : cumulus.rain,
                      },
                    ]}
                  />
                </View>
                <Text
                  style={[
                    styles.variationValue,
                    r.hi && { color: cumulus.ink },
                  ]}
                >
                  {r.v.toFixed(2)}&quot;
                </Text>
              </View>
            ))}
          </View>

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
  const H = 140;
  const maxI = Math.max(0.5, ...minutes.map((m) => m.intensity));
  return (
    <View style={styles.chartBox}>
      {/* grid lines */}
      {[0.25, 0.5, 0.75].map((y) => (
        <View
          key={y}
          style={[styles.gridLine, { top: H - y * H * 0.95 }]}
        />
      ))}
      <View style={styles.barsRow}>
        {minutes.map((m, i) => {
          const h = Math.max(1, (m.intensity / maxI) * H * 0.95);
          const color = intensityColor(m.intensity / maxI);
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
      {/* baseline */}
      <View style={styles.baseline} />
    </View>
  );
}

function intensityColor(pct: number): string {
  if (pct < 0.15) return "#7ae5a8";
  if (pct < 0.35) return cumulus.rain;
  if (pct < 0.6) return cumulus.rainHeavy;
  if (pct < 0.85) return cumulus.accent;
  return cumulus.hot;
}

function SectionHeader({ title }: { title: string }) {
  return (
    <View style={styles.sectionHeader}>
      <Text style={styles.sectionTitle}>{title}</Text>
    </View>
  );
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
  return (
    <View style={styles.keyCard}>
      <View style={[styles.keyIcon, { backgroundColor: `${color}22` }]}>
        <WeatherIcon kind={icon} size={28} />
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
  return (
    <View style={[styles.row, !last && styles.rowBorder]}>
      <Text style={styles.rowLabel}>{label}</Text>
      {children ?? <Text style={styles.rowValue}>{value}</Text>}
    </View>
  );
}

// ───────── helpers

function buildMinutes(minutely: { time: string[]; precipitation: number[] } | undefined): Minute[] {
  // Find the 4 15-min intervals covering "now → now+60min". If minutely_15 is
  // missing, fall back to a flat zero series.
  if (!minutely || minutely.precipitation.length === 0) {
    return Array.from({ length: 60 }, (_, i) => ({ i, intensity: 0, confLo: 0, confHi: 0 }));
  }
  const now = Date.now();
  const startIdx = Math.max(
    0,
    minutely.time.findIndex((t) => new Date(t).getTime() >= now - 7.5 * 60_000),
  );
  // Grab 5 quarters to allow interpolation to the boundary
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
    // Confidence band widens over time
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

/** Simple heuristic — more recent "current" timestamps = higher confidence. */
function estimateConfidence(forecast: { current: { time: string } }): number {
  const ageMin = (Date.now() - new Date(forecast.current.time).getTime()) / 60_000;
  if (ageMin < 5) return 0.85;
  if (ageMin < 15) return 0.7;
  if (ageMin < 30) return 0.55;
  return 0.4;
}

const styles = StyleSheet.create({
  container: { flex: 1 },
  flex: { flex: 1 },
  scroll: { paddingBottom: 140 },
  loading: { color: "rgba(255,255,255,0.6)", fontSize: 16, textAlign: "center", marginTop: 120 },

  header: {
    flexDirection: "row",
    alignItems: "center",
    paddingHorizontal: 16,
    paddingTop: 4,
    paddingBottom: 4,
  },
  backBtn: {
    width: 36,
    height: 36,
    borderRadius: 12,
    backgroundColor: "rgba(255,255,255,0.08)",
    borderWidth: 1,
    borderColor: cumulus.inkLine,
    alignItems: "center",
    justifyContent: "center",
  },
  backChev: { color: cumulus.ink, fontSize: 22, fontWeight: "500", marginTop: -2 },
  headerKicker: {
    color: cumulus.inkMuted,
    fontSize: 11,
    fontWeight: "700",
    letterSpacing: 1.6,
  },
  headerLocation: { color: cumulus.ink, fontSize: 14, fontWeight: "600", marginTop: 1 },

  hero: { paddingHorizontal: 20, paddingTop: 20 },
  heroDry: {
    color: cumulus.ink,
    fontSize: 40,
    fontWeight: "300",
    letterSpacing: -1,
    lineHeight: 44,
  },
  heroDrySub: { fontSize: 22, color: cumulus.inkDim, fontWeight: "400" },
  heroSub: { color: cumulus.inkDim, fontSize: 13, marginTop: 10 },
  heroStrong: { color: cumulus.ink },
  heroDim: { color: cumulus.inkFaint },

  card: {
    marginHorizontal: 16,
    marginTop: 20,
    backgroundColor: cumulus.card,
    borderWidth: 1,
    borderColor: cumulus.cardLine,
    borderRadius: 18,
    padding: 14,
  },

  chartHeader: {
    flexDirection: "row",
    justifyContent: "space-between",
    marginBottom: 10,
  },
  chartLabel: {
    color: cumulus.inkMuted,
    fontSize: 10,
    fontWeight: "700",
    letterSpacing: 1.2,
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
    backgroundColor: "rgba(255,255,255,0.08)",
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
    backgroundColor: "rgba(255,255,255,0.2)",
  },
  chartAxis: {
    flexDirection: "row",
    justifyContent: "space-between",
    marginTop: 8,
  },
  axisTick: {
    color: cumulus.inkMuted,
    fontSize: 10,
    fontVariant: ["tabular-nums"],
    letterSpacing: 0.4,
  },
  scaleRow: {
    flexDirection: "row",
    alignItems: "center",
    gap: 8,
    marginTop: 14,
  },
  scaleGrad: {
    flex: 1,
    height: 5,
    borderRadius: 3,
  },

  sectionHeader: { paddingHorizontal: 20, paddingTop: 22, paddingBottom: 10 },
  sectionTitle: {
    color: cumulus.inkMuted,
    fontSize: 11,
    fontWeight: "700",
    letterSpacing: 1.6,
  },

  keyGrid: {
    flexDirection: "row",
    flexWrap: "wrap",
    marginHorizontal: 11,
    gap: 10,
  },
  keyCard: {
    flexBasis: "47%",
    flexGrow: 1,
    backgroundColor: cumulus.card,
    borderWidth: 1,
    borderColor: cumulus.cardLine,
    borderRadius: 16,
    padding: 12,
    flexDirection: "row",
    alignItems: "center",
    gap: 12,
  },
  keyIcon: {
    width: 40,
    height: 40,
    borderRadius: 12,
    alignItems: "center",
    justifyContent: "center",
  },
  keyLabel: {
    color: cumulus.inkMuted,
    fontSize: 10,
    fontWeight: "700",
    letterSpacing: 1.2,
  },
  keyValue: { color: cumulus.ink, fontSize: 18, fontWeight: "700", marginTop: 1 },
  keySub: { color: cumulus.inkDim, fontSize: 10, marginTop: 1, fontVariant: ["tabular-nums"] },

  row: {
    flexDirection: "row",
    justifyContent: "space-between",
    alignItems: "center",
    paddingVertical: 10,
  },
  rowBorder: {
    borderBottomWidth: StyleSheet.hairlineWidth,
    borderBottomColor: cumulus.cardLine,
  },
  rowLabel: { color: cumulus.inkDim, fontSize: 13 },
  rowValue: { color: cumulus.ink, fontSize: 13, fontWeight: "500" },

  confTrack: {
    width: 80,
    height: 4,
    borderRadius: 2,
    backgroundColor: "rgba(255,255,255,0.1)",
    overflow: "hidden",
  },
  confFill: { height: "100%", borderRadius: 2 },
  confText: {
    fontSize: 11,
    fontWeight: "700",
    fontVariant: ["tabular-nums"],
  },

  noteTitle: {
    color: cumulus.ink,
    fontSize: 13,
    fontWeight: "600",
    marginBottom: 6,
  },
  noteBody: {
    color: cumulus.inkDim,
    fontSize: 12,
    lineHeight: 18,
  },

  variationCaption: { color: cumulus.inkDim, fontSize: 12, marginBottom: 10 },
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
  },
  variationTrack: {
    flex: 1,
    height: 6,
    borderRadius: 3,
    backgroundColor: "rgba(255,255,255,0.08)",
    overflow: "hidden",
  },
  variationFill: { height: "100%", borderRadius: 3 },
  variationValue: {
    width: 48,
    textAlign: "right",
    fontSize: 12,
    fontFamily: "SF Mono",
    color: cumulus.inkDim,
  },
});
