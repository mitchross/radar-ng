/**
 * Alert Detail — severity-tinted hero, polygon mini-map, time block,
 * description, what-to-do, and source. Matches design_handoff_radar_ng
 * AlertDetailScreen.
 */
import { ScrollView, View, Text, StyleSheet, TouchableOpacity } from "react-native";
import { SafeAreaView } from "react-native-safe-area-context";
import { LinearGradient } from "expo-linear-gradient";
import { Canvas, Path as SkPath, Rect as SkRect, Circle as SkCircle, Skia } from "@shopify/react-native-skia";
import { useLocalSearchParams, useRouter } from "expo-router";
import { useAlerts } from "../../hooks/useAlerts";
import { useWeatherStore } from "../../stores/useWeatherStore";
import { cumulus } from "../../lib/cumulusTheme";
import type { NWSAlert } from "../../types/weather";

type Severity = NWSAlert["properties"]["severity"];

const SEVERITY: Record<Severity, { color: string; label: string; glow: string }> = {
  Extreme: { color: "#FF3B4A", label: "EXTREME", glow: "rgba(255,59,74,0.35)" },
  Severe: { color: "#FF8A3A", label: "SEVERE", glow: "rgba(255,138,58,0.30)" },
  Moderate: { color: "#F5D042", label: "MODERATE", glow: "rgba(245,208,66,0.25)" },
  Minor: { color: "#4FB8FF", label: "MINOR", glow: "rgba(79,184,255,0.22)" },
  Unknown: { color: "rgba(255,255,255,0.4)", label: "UNKNOWN", glow: "rgba(255,255,255,0.15)" },
};

const STORM_BG: readonly [string, string, string] = ["#1a0d2e", "#0f1424", "#0a0e1a"];

export default function AlertDetailScreen() {
  const { id } = useLocalSearchParams<{ id: string }>();
  const router = useRouter();
  const { data: alertData } = useAlerts();
  const userLat = useWeatherStore((s) => s.latitude);
  const userLon = useWeatherStore((s) => s.longitude);

  const alert = alertData?.features.find((f) => f.properties.id === id || f.id === id);

  if (!alert) {
    return (
      <LinearGradient colors={STORM_BG} style={styles.container}>
        <SafeAreaView style={styles.flex} edges={["top"]}>
          <Text style={styles.notFound}>Alert not found.</Text>
        </SafeAreaView>
      </LinearGradient>
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
          {/* Severity-tinted hero band */}
          <LinearGradient
            colors={[`${sev.color}55`, `${sev.color}11`]}
            style={[styles.heroBand, { borderBottomColor: `${sev.color}33` }]}
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

          {/* Polygon mini-map (only when we have real geometry; otherwise the
              county list below is more useful than a fake placeholder shape). */}
          {(alert.geometry?.coordinates?.[0]?.length ?? 0) >= 3 ? (
            <View style={[styles.polyWrap, { shadowColor: sev.color }]}>
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
            ? { backgroundColor: `${tint}15`, borderColor: `${tint}33` }
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

  // Project polygon → canvas; fall back to a stylized placeholder shape if
  // the alert lacks a polygon.
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

  // Backdrop grid as a single compound path
  const gridPath = Skia.Path.Make();
  for (let i = 1; i < 8; i++) {
    gridPath.moveTo(i * 45, 0);
    gridPath.lineTo(i * 45, H);
  }
  for (let i = 1; i < 6; i++) {
    gridPath.moveTo(0, i * 30);
    gridPath.lineTo(W, i * 30);
  }

  // Skia's reconciler does not handle React Fragments — list each element
  // directly. Always render the user pin (off-canvas if no point known).
  const userX = userPt?.x ?? -100;
  const userY = userPt?.y ?? -100;

  return (
    <Canvas style={{ width: "100%", height: H, backgroundColor: "#0e1726" }}>
      <SkRect x={0} y={0} width={W} height={H} color="#0e1726" />
      <SkPath
        path={gridPath}
        style="stroke"
        strokeWidth={0.5}
        color="rgba(255,255,255,0.08)"
      />
      <SkPath path={polyPath} color={`${color}52`} />
      <SkPath path={polyPath} style="stroke" strokeWidth={1.5} color={color} />
      <SkCircle cx={userX} cy={userY} r={14} color={`${color}33`} />
      <SkCircle cx={userX} cy={userY} r={5} color="#ffffff" />
      <SkCircle
        cx={userX}
        cy={userY}
        r={5}
        color="#0e1726"
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
  notFound: { color: cumulus.inkDim, fontSize: 16, textAlign: "center", marginTop: 120 },

  heroBand: {
    paddingHorizontal: 16,
    paddingTop: 14,
    paddingBottom: 20,
    borderBottomWidth: 1,
  },
  backBtn: {
    width: 36,
    height: 36,
    borderRadius: 18,
    backgroundColor: "rgba(0,0,0,0.3)",
    alignItems: "center",
    justifyContent: "center",
    marginBottom: 12,
  },
  backChev: { color: "#fff", fontSize: 22, fontWeight: "500", marginTop: -2 },

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
    backgroundColor: "#fff",
    shadowColor: "#fff",
    shadowOpacity: 0.7,
    shadowRadius: 3,
    shadowOffset: { width: 0, height: 0 },
  },
  severityText: {
    color: "#fff",
    fontSize: 10,
    fontFamily: "SF Mono",
    fontWeight: "800",
    letterSpacing: 1.4,
  },
  urgency: {
    fontSize: 11,
    fontFamily: "SF Mono",
    color: "rgba(255,255,255,0.7)",
    letterSpacing: 0.6,
  },
  event: {
    color: cumulus.ink,
    fontSize: 26,
    fontWeight: "700",
    letterSpacing: -0.4,
    lineHeight: 30,
  },
  area: {
    fontSize: 13,
    color: cumulus.inkDim,
    marginTop: 6,
  },

  polyWrap: {
    marginHorizontal: 16,
    marginTop: 14,
    borderRadius: 16,
    overflow: "hidden",
    backgroundColor: "#0e1726",
    borderWidth: 1,
    borderColor: cumulus.cardLine,
    position: "relative",
    shadowOpacity: 0.4,
    shadowRadius: 20,
    shadowOffset: { width: 0, height: 8 },
  },
  polyKicker: {
    position: "absolute",
    top: 10,
    left: 12,
    fontSize: 10,
    fontFamily: "SF Mono",
    color: "rgba(255,255,255,0.6)",
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
    backgroundColor: "rgba(255,255,255,0.92)",
  },
  openInRadarText: {
    color: "#0a0e1a",
    fontSize: 11,
    fontWeight: "700",
  },

  areaCard: {
    marginHorizontal: 16,
    marginTop: 14,
    padding: 14,
    borderRadius: 14,
    backgroundColor: "rgba(255,255,255,0.05)",
    borderWidth: 1,
    borderColor: cumulus.cardLine,
  },
  areaKicker: {
    fontSize: 10,
    fontFamily: "SF Mono",
    color: cumulus.inkMuted,
    letterSpacing: 1.4,
    fontWeight: "700",
    marginBottom: 8,
  },
  areaList: {
    color: cumulus.ink,
    fontSize: 14,
    lineHeight: 20,
  },
  areaNote: {
    color: cumulus.inkMuted,
    fontSize: 11,
    fontFamily: "SF Mono",
    marginTop: 8,
  },

  timeCard: {
    marginHorizontal: 16,
    marginTop: 14,
    padding: 14,
    borderRadius: 14,
    backgroundColor: "rgba(255,255,255,0.05)",
    borderWidth: 1,
    borderColor: cumulus.cardLine,
  },
  timeRow: { flexDirection: "row", gap: 12 },
  timeCell: { flex: 1 },
  timeLabel: {
    fontSize: 9,
    fontFamily: "SF Mono",
    color: cumulus.inkMuted,
    letterSpacing: 1.4,
    fontWeight: "700",
    marginBottom: 4,
  },
  timeValue: {
    fontSize: 13,
    fontFamily: "SF Mono",
    fontWeight: "600",
    color: cumulus.ink,
  },
  timeFooter: {
    marginTop: 12,
    paddingTop: 10,
    borderTopWidth: StyleSheet.hairlineWidth,
    borderTopColor: cumulus.cardLine,
  },
  timeFooterText: {
    fontSize: 11,
    fontFamily: "SF Mono",
    color: cumulus.inkMuted,
  },

  section: { marginHorizontal: 16, marginTop: 14 },
  sectionLabel: {
    fontSize: 10,
    fontFamily: "SF Mono",
    color: cumulus.inkMuted,
    letterSpacing: 1.6,
    fontWeight: "700",
    paddingHorizontal: 4,
    paddingTop: 6,
    paddingBottom: 8,
  },
  sectionBody: {
    padding: 14,
    borderRadius: 14,
    backgroundColor: "rgba(255,255,255,0.05)",
    borderWidth: 1,
    borderColor: cumulus.cardLine,
  },

  bodyText: {
    fontSize: 14,
    lineHeight: 22,
    color: "rgba(255,255,255,0.82)",
  },
  instructionText: {
    fontSize: 14,
    lineHeight: 22,
    color: cumulus.ink,
    fontWeight: "500",
  },
  sourceRow: {
    flexDirection: "row",
    justifyContent: "space-between",
  },
  sourceText: {
    fontSize: 12,
    fontFamily: "SF Mono",
    color: cumulus.inkMuted,
  },
});
