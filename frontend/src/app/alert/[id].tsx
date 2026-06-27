/**
 * Alert Detail — Redesigned for Editorial Light.
 * Warm paper layout, high-contrast text, clear map coordinates projection.
 */
import { ScrollView, View, Text, StyleSheet, TouchableOpacity } from "react-native";
import { SafeAreaView } from "react-native-safe-area-context";
import { LinearGradient } from "expo-linear-gradient";
import { Canvas, Path as SkPath, Rect as SkRect, Circle as SkCircle, Skia } from "@shopify/react-native-skia";
import { useLocalSearchParams, useRouter } from "expo-router";
import { useAlerts } from "../../hooks/useAlerts";
import { useWeatherStore } from "../../stores/useWeatherStore";
import { cumulus, cumulusFonts } from "../../lib/cumulusTheme";
import type { NWSAlert } from "../../types/weather";

type Severity = NWSAlert["properties"]["severity"];

const SEVERITY: Record<Severity, { color: string; label: string; glow: string }> = {
  Extreme: { color: cumulus.alert, label: "EXTREME", glow: "rgba(223,106,106,0.2)" },
  Severe: { color: cumulus.accent, label: "SEVERE", glow: "rgba(194,96,58,0.15)" },
  Moderate: { color: "#f0c34e", label: "MODERATE", glow: "rgba(240,195,78,0.15)" },
  Minor: { color: cumulus.rain, label: "MINOR", glow: "rgba(77,127,180,0.15)" },
  Unknown: { color: "rgba(33, 31, 27, 0.4)", label: "UNKNOWN", glow: "rgba(33,31,27,0.05)" },
};

const STORM_BG: readonly [string, string, string] = ["#f6f2ea", "#f6f2ea", "#f6f2ea"];

export default function AlertDetailScreen() {
  const { id } = useLocalSearchParams<{ id: string }>();
  const router = useRouter();
  const alertData = useAlerts();
  const userLat = useWeatherStore((s) => s.latitude);
  const userLon = useWeatherStore((s) => s.longitude);

  const alert = alertData.data?.features.find((f) => f.properties.id === id || f.id === id);

  if (!alert) {
    return (
      <View style={styles.errorContainer}>
        <SafeAreaView style={styles.flex} edges={["top"]}>
          <Text style={styles.notFound}>Alert not found.</Text>
        </SafeAreaView>
      </View>
    );
  }

  const sev = SEVERITY[alert.properties.severity];
  const onset = formatStamp(alert.properties.onset);
  const effective = formatStamp(alert.properties.effective);
  const expires = formatStamp(alert.properties.expires);

  return (
    <LinearGradient colors={STORM_BG} style={styles.container}>
      <SafeAreaView style={styles.flex} edges={["top"]}>
        <ScrollView
          contentContainerStyle={styles.scroll}
          showsVerticalScrollIndicator={false}
        >
          {/* Severity header band */}
          <LinearGradient
            colors={[`${sev.color}16`, `${sev.color}05`]}
            style={[styles.heroBand, { borderBottomColor: `${sev.color}22` }]}
          >
            <TouchableOpacity
              onPress={() => router.back()}
              style={styles.backBtn}
              activeOpacity={0.7}
            >
              <Text style={styles.backChev}>{"‹"}</Text>
            </TouchableOpacity>

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
              <TouchableOpacity
                style={styles.openInRadar}
                onPress={() => router.push("/(tabs)/radar" as never)}
                activeOpacity={0.85}
              >
                <Text style={styles.openInRadarText}>Open in Radar →</Text>
              </TouchableOpacity>
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
            <Text style={styles.bodyText}>{alert.properties.description}</Text>
          </Section>

          {/* What to do */}
          {alert.properties.instruction ? (
            <Section label="WHAT TO DO" tint={sev.color}>
              <Text style={styles.instructionText}>{alert.properties.instruction}</Text>
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
  const W = 360;
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
    <Canvas style={{ width: "100%", height: H, backgroundColor: "#fbf9f5" }}>
      <SkRect x={0} y={0} width={W} height={H} color="#fbf9f5" />
      <SkPath
        path={gridPath}
        style="stroke"
        strokeWidth={0.5}
        color="rgba(33, 31, 27, 0.08)"
      />
      <SkPath path={polyPath} color={`${color}22`} />
      <SkPath path={polyPath} style="stroke" strokeWidth={1.5} color={color} />
      <SkCircle cx={userX} cy={userY} r={14} color={`${color}33`} />
      <SkCircle cx={userX} cy={userY} r={5} color={cumulus.accent} />
      <SkCircle
        cx={userX}
        cy={userY}
        r={5}
        color="#fbf9f5"
        style="stroke"
        strokeWidth={2}
      />
    </Canvas>
  );
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

const styles = StyleSheet.create({
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
    width: 36,
    height: 36,
    borderRadius: 12,
    backgroundColor: "#eae4d8",
    borderWidth: 1,
    borderColor: "#e3dccf",
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
    backgroundColor: "#fbf9f5",
    borderWidth: 1,
    borderColor: "#eee6d8",
    position: "relative",
    shadowColor: "rgba(60,50,40,0.06)",
    shadowOpacity: 0.35,
    shadowRadius: 10,
    shadowOffset: { width: 0, height: 4 },
    elevation: 2,
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
    paddingVertical: 5,
    borderRadius: 8,
    backgroundColor: "#eae4d8",
    borderWidth: 1,
    borderColor: "#e3dccf",
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
    backgroundColor: "#ffffff",
    borderWidth: 1,
    borderColor: "#eee6d8",
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
    backgroundColor: "#ffffff",
    borderWidth: 1,
    borderColor: "#eee6d8",
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
    borderTopColor: "#e7e0d3",
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
    backgroundColor: "#ffffff",
    borderWidth: 1,
    borderColor: "#eee6d8",
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
