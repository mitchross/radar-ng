/**
 * Alert Detail — Redesigned for Editorial Light.
 * Warm paper layout, high-contrast text, clear map coordinates projection.
 */
import { useMemo } from "react";
import { ScrollView, View, Text, StyleSheet, Pressable, useWindowDimensions } from "react-native";
import { SafeAreaView } from "react-native-safe-area-context";
import { LinearGradient } from "expo-linear-gradient";
import { Canvas, Path as SkPath, Rect as SkRect, Circle as SkCircle, Skia } from "@shopify/react-native-skia";
import { useLocalSearchParams, useRouter } from "expo-router";
import { useAlerts } from "../../hooks/useAlerts";
import { useWeatherStore } from "../../stores/useWeatherStore";
import { ScreenState } from "../../components/ui/WeatherClearUI";
import { useWeatherClearTheme } from "../../theme/WeatherClearThemeProvider";
import type { WeatherClearTheme } from "../../theme/weatherClearTheme";
import type { NWSAlert } from "../../types/weather";

type Severity = NWSAlert["properties"]["severity"];

export default function AlertDetailScreen() {
  const { id } = useLocalSearchParams<{ id: string }>();
  const router = useRouter();
  const { theme } = useWeatherClearTheme();
  const styles = useMemo(() => createStyles(theme), [theme]);
  const alertData = useAlerts();
  const userLat = useWeatherStore((s) => s.latitude);
  const userLon = useWeatherStore((s) => s.longitude);

  const alert = alertData.data?.features.find((f) => f.properties.id === id || f.id === id);

  if (!alert) {
    return (
      <View style={styles.errorContainer}>
        <ScreenState
          kind="empty"
          title="Alert unavailable"
          message="This alert is no longer active or has not been loaded."
          actionLabel="Go back"
          onAction={() => router.back()}
        />
      </View>
    );
  }

  const sev = severityStyle(alert.properties.severity, theme);
  const onset = formatStamp(alert.properties.onset);
  const effective = formatStamp(alert.properties.effective);
  const expires = formatStamp(alert.properties.expires);

  return (
    <LinearGradient
      accessibilityLabel="Weather alert detail"
      colors={[theme.colors.canvas, theme.colors.canvas]}
      style={styles.container}
    >
      <SafeAreaView style={styles.flex} edges={["top"]}>
        <ScrollView
          contentInsetAdjustmentBehavior="automatic"
          contentContainerStyle={styles.scroll}
          showsVerticalScrollIndicator={false}
        >
          {/* Severity header band */}
          <LinearGradient
            colors={[`${sev.color}16`, `${sev.color}05`]}
            style={[styles.heroBand, { borderBottomColor: `${sev.color}22` }]}
          >
            <Pressable
              accessibilityRole="button"
              accessibilityLabel="Close alert detail"
              onPress={() => router.back()}
              style={styles.backBtn}
            >
              <Text style={styles.backChev}>{"‹"}</Text>
            </Pressable>

            <View style={styles.severityRow}>
              <View style={[styles.severityPill, { backgroundColor: sev.color }]}>
                <View style={styles.severityDot} />
                <Text style={styles.severityText}>{sev.label}</Text>
              </View>
              <Text style={styles.urgency}>
                {alert.properties.urgency.toUpperCase()}
              </Text>
            </View>

            <Text style={styles.event}>{alert.properties.event}</Text>
            <Text style={styles.area}>{alert.properties.areaDesc}</Text>
          </LinearGradient>

          {/* Polygon mini-map */}
          {(alert.geometry?.coordinates?.[0]?.length ?? 0) >= 3 ? (
            <View style={styles.polyWrap}>
              <PolygonMap alert={alert} color={sev.color} userLat={userLat} userLon={userLon} />
              <Text style={styles.polyKicker}>WARNING POLYGON</Text>
              <Pressable
                accessibilityRole="button"
                accessibilityLabel="Open alert polygon in Radar"
                style={styles.openInRadar}
                onPress={() => router.push("/(tabs)/radar" as never)}
              >
                <Text style={styles.openInRadarText}>Open in Radar →</Text>
              </Pressable>
            </View>
          ) : (
            <View style={styles.areaCard}>
              <Text style={styles.areaKicker}>AFFECTED AREAS</Text>
              <Text style={styles.areaList}>{alert.properties.areaDesc}</Text>
              <Text style={styles.areaNote}>
                County-level advisory · no polygon issued
              </Text>
            </View>
          )}

          {/* Time block */}
          <View style={styles.timeCard}>
            <View style={styles.timeRow}>
              <TimeCell label="ONSET" value={onset} />
              <TimeCell label="EFFECTIVE" value={effective} />
              <TimeCell label="EXPIRES" value={expires} accent={sev.color} />
            </View>
            <View style={styles.timeFooter}>
              <Text style={styles.timeFooterText}>
                Issued by {alert.properties.senderName ?? "National Weather Service"}
              </Text>
            </View>
          </View>

          {/* Description */}
          <Section label="DESCRIPTION">
            <Text selectable style={styles.bodyText}>{alert.properties.description}</Text>
          </Section>

          {/* What to do */}
          {alert.properties.instruction ? (
            <Section label="WHAT TO DO" tint={sev.color}>
              <Text selectable style={styles.instructionText}>{alert.properties.instruction}</Text>
            </Section>
          ) : null}

          {/* Source */}
          <Section label="SOURCE">
            <View style={styles.sourceRow}>
              <Text style={styles.sourceText}>
                {alert.properties.senderName ?? "NWS"}
              </Text>
              <Text style={styles.sourceText}>NWS CAP · v1.2</Text>
            </View>
          </Section>

          <View style={{ height: 60 }} />
        </ScrollView>
      </SafeAreaView>
    </LinearGradient>
  );
}

function TimeCell({ label, value, accent }: { label: string; value: string; accent?: string }) {
  const { theme } = useWeatherClearTheme();
  const styles = useMemo(() => createStyles(theme), [theme]);
  return (
    <View style={styles.timeCell}>
      <Text style={styles.timeLabel}>{label}</Text>
      <Text style={[styles.timeValue, accent ? { color: accent } : null]}>
        {value}
      </Text>
    </View>
  );
}

function Section({
  label,
  tint,
  children,
}: {
  label: string;
  tint?: string;
  children: React.ReactNode;
}) {
  const { theme } = useWeatherClearTheme();
  const styles = useMemo(() => createStyles(theme), [theme]);
  return (
    <View style={styles.section}>
      <Text style={styles.sectionLabel}>{label}</Text>
      <View
        style={[
          styles.sectionBody,
          tint
            ? { backgroundColor: `${tint}12`, borderColor: `${tint}22` }
            : null,
        ]}
      >
        {children}
      </View>
    </View>
  );
}

function PolygonMap({
  alert,
  color,
  userLat,
  userLon,
}: {
  alert: NWSAlert;
  color: string;
  userLat: number | null;
  userLon: number | null;
}) {
  const { theme } = useWeatherClearTheme();
  const { width } = useWindowDimensions();
  const W = Math.min(360, width - 32);
  const H = 160;
  const rawCoords = alert.geometry?.coordinates?.[0] ?? [];

  let polyPts: { x: number; y: number }[];
  let userPt: { x: number; y: number } | null = null;

  if (rawCoords.length >= 3) {
    let minLon = Infinity;
    let maxLon = -Infinity;
    let minLat = Infinity;
    let maxLat = -Infinity;
    for (const [lon, lat] of rawCoords) {
      if (lon < minLon) minLon = lon;
      if (lon > maxLon) maxLon = lon;
      if (lat < minLat) minLat = lat;
      if (lat > maxLat) maxLat = lat;
    }
    if (userLat != null && userLon != null) {
      if (userLon < minLon) minLon = userLon;
      if (userLon > maxLon) maxLon = userLon;
      if (userLat < minLat) minLat = userLat;
      if (userLat > maxLat) maxLat = userLat;
    }
    const padLon = Math.max((maxLon - minLon) * 0.15, 0.05);
    const padLat = Math.max((maxLat - minLat) * 0.15, 0.05);
    minLon -= padLon;
    maxLon += padLon;
    minLat -= padLat;
    maxLat += padLat;
    const lonSpan = maxLon - minLon || 1;
    const latSpan = maxLat - minLat || 1;
    const project = (lon: number, lat: number) => ({
      x: ((lon - minLon) / lonSpan) * W,
      y: H - ((lat - minLat) / latSpan) * H,
    });
    polyPts = rawCoords.map(([lon, lat]) => project(lon, lat));
    if (userLat != null && userLon != null) userPt = project(userLon, userLat);
  } else {
    polyPts = [
      { x: 130, y: 55 },
      { x: 220, y: 50 },
      { x: 270, y: 80 },
      { x: 240, y: 120 },
      { x: 160, y: 115 },
    ];
    userPt = { x: 200, y: 85 };
  }

  const polyPath = Skia.Path.Make();
  polyPts.forEach((p, i) => {
    if (i === 0) polyPath.moveTo(p.x, p.y);
    else polyPath.lineTo(p.x, p.y);
  });
  polyPath.close();

  const gridPath = Skia.Path.Make();
  for (let i = 1; i < 8; i++) {
    gridPath.moveTo(i * 45, 0);
    gridPath.lineTo(i * 45, H);
  }
  for (let i = 1; i < 6; i++) {
    gridPath.moveTo(0, i * 30);
    gridPath.lineTo(W, i * 30);
  }

  const userX = userPt?.x ?? -100;
  const userY = userPt?.y ?? -100;

  return (
    <Canvas
      accessibilityLabel={`Warning polygon for ${alert.properties.areaDesc}`}
      style={{ width: "100%", height: H, backgroundColor: theme.colors.surfaceStrong }}
    >
      <SkRect x={0} y={0} width={W} height={H} color={theme.colors.surfaceStrong} />
      <SkPath
        path={gridPath}
        style="stroke"
        strokeWidth={0.5}
        color={theme.colors.divider}
      />
      <SkPath path={polyPath} color={`${color}22`} />
      <SkPath path={polyPath} style="stroke" strokeWidth={1.5} color={color} />
      <SkCircle cx={userX} cy={userY} r={14} color={`${color}33`} />
      <SkCircle cx={userX} cy={userY} r={5} color={theme.colors.accent} />
      <SkCircle
        cx={userX}
        cy={userY}
        r={5}
        color={theme.colors.surfaceStrong}
        style="stroke"
        strokeWidth={2}
      />
    </Canvas>
  );
}

function severityStyle(
  severity: Severity,
  theme: WeatherClearTheme,
): { color: string; label: string } {
  return {
    Extreme: { color: theme.colors.destructive, label: "EXTREME" },
    Severe: { color: theme.colors.accent, label: "SEVERE" },
    Moderate: { color: theme.colors.warning, label: "MODERATE" },
    Minor: { color: theme.colors.rain, label: "MINOR" },
    Unknown: { color: theme.colors.textMuted, label: "UNKNOWN" },
  }[severity];
}

function formatStamp(s: string | null | undefined): string {
  if (!s) return "—";
  const d = new Date(s);
  if (Number.isNaN(d.getTime())) return "—";
  return d.toLocaleString([], {
    month: "short",
    day: "numeric",
    hour: "numeric",
    minute: "2-digit",
  });
}

function createStyles(theme: WeatherClearTheme) {
  const cumulus = {
    background: theme.colors.canvas,
    ink: theme.colors.text,
    inkDim: theme.colors.textSecondary,
    inkMuted: theme.colors.textMuted,
  };
  const cumulusFonts = {
    display: theme.typography.display,
    ui: theme.typography.ui,
    mono: theme.typography.mono,
  };

  return StyleSheet.create({
  container: { flex: 1 },
  flex: { flex: 1 },
  scroll: { paddingBottom: 60 },
  errorContainer: { flex: 1, backgroundColor: cumulus.background },
  notFound: {
    color: cumulus.inkDim,
    fontSize: 16,
    textAlign: "center",
    marginTop: 120,
    fontFamily: cumulusFonts.ui,
  },

  heroBand: {
    paddingHorizontal: 24,
    paddingTop: 16,
    paddingBottom: 20,
    borderBottomWidth: 1,
  },
  backBtn: {
    width: 44,
    height: 44,
    borderRadius: 12,
    backgroundColor: theme.colors.surfaceMuted,
    borderWidth: 1,
    borderColor: theme.colors.divider,
    alignItems: "center",
    justifyContent: "center",
    marginBottom: 12,
  },
  backChev: { color: cumulus.ink, fontSize: 22, fontWeight: "500", marginTop: -2 },

  severityRow: { flexDirection: "row", alignItems: "center", gap: 8, marginBottom: 10 },
  severityPill: {
    flexDirection: "row",
    alignItems: "center",
    gap: 6,
    paddingHorizontal: 9,
    paddingVertical: 4,
    borderRadius: 6,
  },
  severityDot: {
    width: 6,
    height: 6,
    borderRadius: 3,
    backgroundColor: "#ffffff",
  },
  severityText: {
    color: "#ffffff",
    fontSize: 10,
    fontFamily: cumulusFonts.mono,
    fontWeight: "800",
    letterSpacing: 1.4,
  },
  urgency: {
    fontSize: 11,
    fontFamily: cumulusFonts.mono,
    color: cumulus.inkMuted,
    letterSpacing: 0.6,
  },
  event: {
    color: cumulus.ink,
    fontSize: 26,
    fontWeight: "600",
    letterSpacing: -0.4,
    lineHeight: 30,
    fontFamily: cumulusFonts.display,
  },
  area: {
    fontSize: 13,
    color: cumulus.inkDim,
    marginTop: 6,
    fontFamily: cumulusFonts.ui,
  },

  polyWrap: {
    marginHorizontal: 16,
    marginTop: 14,
    borderRadius: 20,
    overflow: "hidden",
    backgroundColor: theme.colors.surfaceStrong,
    borderWidth: 1,
    borderColor: theme.colors.border,
    position: "relative",
    boxShadow: theme.dark
      ? "0 4px 12px rgba(0,0,0,0.28)"
      : "0 4px 12px rgba(60,50,40,0.08)",
  },
  polyKicker: {
    position: "absolute",
    top: 10,
    left: 12,
    fontSize: 9,
    fontFamily: cumulusFonts.mono,
    color: cumulus.inkMuted,
    letterSpacing: 1.4,
    fontWeight: "700",
  },
  openInRadar: {
    position: "absolute",
    bottom: 10,
    right: 12,
    paddingHorizontal: 10,
    minHeight: 44,
    paddingVertical: 8,
    borderRadius: 8,
    backgroundColor: theme.colors.surfaceMuted,
    borderWidth: 1,
    borderColor: theme.colors.divider,
  },
  openInRadarText: {
    color: cumulus.ink,
    fontSize: 11,
    fontWeight: "700",
    fontFamily: cumulusFonts.ui,
  },

  areaCard: {
    marginHorizontal: 16,
    marginTop: 14,
    padding: 14,
    borderRadius: 16,
    backgroundColor: theme.colors.surface,
    borderWidth: 1,
    borderColor: theme.colors.border,
  },
  areaKicker: {
    fontSize: 9,
    fontFamily: cumulusFonts.mono,
    color: cumulus.inkMuted,
    letterSpacing: 1.4,
    fontWeight: "700",
    marginBottom: 8,
  },
  areaList: {
    color: cumulus.ink,
    fontSize: 14,
    lineHeight: 20,
    fontFamily: cumulusFonts.ui,
  },
  areaNote: {
    color: cumulus.inkMuted,
    fontSize: 11,
    fontFamily: cumulusFonts.mono,
    marginTop: 8,
  },

  timeCard: {
    marginHorizontal: 16,
    marginTop: 14,
    padding: 14,
    borderRadius: 16,
    backgroundColor: theme.colors.surface,
    borderWidth: 1,
    borderColor: theme.colors.border,
  },
  timeRow: { flexDirection: "row", gap: 12 },
  timeCell: { flex: 1 },
  timeLabel: {
    fontSize: 9,
    fontFamily: cumulusFonts.mono,
    color: cumulus.inkMuted,
    letterSpacing: 1.4,
    fontWeight: "700",
    marginBottom: 4,
  },
  timeValue: {
    fontSize: 13,
    fontFamily: cumulusFonts.mono,
    fontWeight: "600",
    color: cumulus.ink,
  },
  timeFooter: {
    marginTop: 12,
    paddingTop: 10,
    borderTopWidth: 1,
    borderTopColor: theme.colors.divider,
  },
  timeFooterText: {
    fontSize: 11,
    fontFamily: cumulusFonts.mono,
    color: cumulus.inkMuted,
  },

  section: { marginHorizontal: 16, marginTop: 14 },
  sectionLabel: {
    fontSize: 9,
    fontFamily: cumulusFonts.mono,
    color: cumulus.inkMuted,
    letterSpacing: 1.6,
    fontWeight: "700",
    paddingHorizontal: 4,
    paddingTop: 6,
    paddingBottom: 8,
  },
  sectionBody: {
    padding: 14,
    borderRadius: 16,
    backgroundColor: theme.colors.surface,
    borderWidth: 1,
    borderColor: theme.colors.border,
  },

  bodyText: {
    fontSize: 14,
    lineHeight: 22,
    color: cumulus.inkDim,
    fontFamily: cumulusFonts.ui,
  },
  instructionText: {
    fontSize: 14,
    lineHeight: 22,
    color: cumulus.ink,
    fontWeight: "500",
    fontFamily: cumulusFonts.ui,
  },
  sourceRow: {
    flexDirection: "row",
    justifyContent: "space-between",
  },
  sourceText: {
    fontSize: 12,
    fontFamily: cumulusFonts.mono,
    color: cumulus.inkMuted,
  },
  });
}
