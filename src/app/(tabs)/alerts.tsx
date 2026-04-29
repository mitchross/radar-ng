/**
 * Cumulus Alerts tab — NWS active alerts list, severity-colored cards.
 * Empty state when none active. Tap → /alert/[id] detail modal.
 */
import { ScrollView, View, Text, StyleSheet, TouchableOpacity, RefreshControl } from "react-native";
import { SafeAreaView } from "react-native-safe-area-context";
import { LinearGradient } from "expo-linear-gradient";
import { useRouter } from "expo-router";
import { useAlerts } from "../../hooks/useAlerts";
import { useLocation } from "../../hooks/useLocation";
import { cumulus, CONDITION_GRADIENTS } from "../../lib/cumulusTheme";
import type { NWSAlert } from "../../types/weather";

const SEVERITY_COLOR: Record<NWSAlert["properties"]["severity"], string> = {
  Extreme: "#FF3B4A",
  Severe: "#FF6E3A",
  Moderate: "#FFC14D",
  Minor: "#4FB8FF",
  Unknown: "rgba(255,255,255,0.4)",
};

export default function AlertsScreen() {
  useLocation();
  const router = useRouter();
  const { data, isLoading, refetch, isFetching } = useAlerts();
  const alerts = data?.features ?? [];

  return (
    <LinearGradient colors={CONDITION_GRADIENTS.storm} style={styles.container}>
      <SafeAreaView style={styles.flex} edges={["top"]}>
        <View style={styles.header}>
          <View>
            <Text style={styles.kicker}>ACTIVE ALERTS</Text>
            <Text style={styles.title}>{alerts.length > 0 ? `${alerts.length} in your area` : "All clear"}</Text>
          </View>
          <TouchableOpacity onPress={() => refetch()} style={styles.refresh} activeOpacity={0.7}>
            <Text style={styles.refreshText}>{isFetching ? "↻" : "↻"}</Text>
          </TouchableOpacity>
        </View>

        <ScrollView
          contentContainerStyle={styles.scroll}
          showsVerticalScrollIndicator={false}
          refreshControl={
            <RefreshControl
              refreshing={isFetching}
              onRefresh={refetch}
              tintColor={cumulus.ink}
              colors={[cumulus.accent]}
            />
          }
        >
          {isLoading && <Text style={styles.muted}>Loading…</Text>}
          {!isLoading && alerts.length === 0 && <EmptyState />}
          {alerts.map((alert) => (
            <AlertCard
              key={alert.id}
              alert={alert}
              onPress={() => router.push(`/alert/${encodeURIComponent(alert.id)}` as any)}
            />
          ))}
          <View style={{ height: 120 }} />
        </ScrollView>
      </SafeAreaView>
    </LinearGradient>
  );
}

function AlertCard({ alert, onPress }: { alert: NWSAlert; onPress: () => void }) {
  const color = SEVERITY_COLOR[alert.properties.severity];
  const expires = new Date(alert.properties.expires);
  const expiresLabel = expires.toLocaleString([], {
    month: "short", day: "numeric", hour: "numeric", minute: "2-digit",
  });
  return (
    <TouchableOpacity onPress={onPress} activeOpacity={0.85} style={styles.card}>
      <View style={[styles.stripe, { backgroundColor: color }]} />
      <View style={styles.cardBody}>
        <View style={styles.cardHeaderRow}>
          <View style={[styles.severityPill, { borderColor: color, backgroundColor: `${color}22` }]}>
            <Text style={[styles.severityText, { color }]}>{alert.properties.severity.toUpperCase()}</Text>
          </View>
          <Text style={styles.urgency}>{alert.properties.urgency.toUpperCase()}</Text>
        </View>
        <Text style={styles.event} numberOfLines={2}>{alert.properties.event}</Text>
        {alert.properties.headline && (
          <Text style={styles.headline} numberOfLines={3}>{alert.properties.headline}</Text>
        )}
        <View style={styles.metaRow}>
          <Text style={styles.area} numberOfLines={1}>{alert.properties.areaDesc}</Text>
          <Text style={styles.expires}>•  Until {expiresLabel}</Text>
        </View>
      </View>
    </TouchableOpacity>
  );
}

function EmptyState() {
  return (
    <View style={styles.empty}>
      <View style={styles.emptyDotOuter}>
        <View style={styles.emptyDot} />
      </View>
      <Text style={styles.emptyTitle}>No active alerts</Text>
      <Text style={styles.emptyMeta}>
        National Weather Service reports no warnings, watches, or advisories for your location.
      </Text>
    </View>
  );
}

const styles = StyleSheet.create({
  container: { flex: 1 },
  flex: { flex: 1 },
  header: {
    flexDirection: "row",
    alignItems: "flex-end",
    justifyContent: "space-between",
    paddingHorizontal: 20,
    paddingTop: 10,
    paddingBottom: 12,
  },
  kicker: {
    fontSize: 11,
    fontFamily: "SF Mono",
    fontWeight: "700",
    color: cumulus.inkMuted,
    letterSpacing: 1.6,
  },
  title: { fontSize: 28, color: cumulus.ink, fontWeight: "700", letterSpacing: -0.4, marginTop: 2 },
  refresh: {
    width: 36,
    height: 36,
    borderRadius: 12,
    backgroundColor: cumulus.cardStrong,
    borderWidth: 1,
    borderColor: cumulus.inkLine,
    alignItems: "center",
    justifyContent: "center",
  },
  refreshText: { color: cumulus.ink, fontSize: 18, fontWeight: "700" },

  scroll: { paddingHorizontal: 16, paddingTop: 4, paddingBottom: 140 },
  muted: { color: cumulus.inkMuted, textAlign: "center", paddingVertical: 20 },

  card: {
    flexDirection: "row",
    backgroundColor: cumulus.card,
    borderWidth: 1,
    borderColor: cumulus.cardLine,
    borderRadius: 18,
    overflow: "hidden",
    marginBottom: 10,
  },
  stripe: { width: 5 },
  cardBody: { flex: 1, padding: 14 },
  cardHeaderRow: { flexDirection: "row", alignItems: "center", gap: 10, marginBottom: 6 },
  severityPill: {
    paddingHorizontal: 8,
    paddingVertical: 3,
    borderRadius: 6,
    borderWidth: 1,
  },
  severityText: { fontSize: 10, fontWeight: "800", letterSpacing: 0.8, fontFamily: "SF Mono" },
  urgency: { fontSize: 10, color: cumulus.inkMuted, fontFamily: "SF Mono", letterSpacing: 0.8 },
  event: { color: cumulus.ink, fontSize: 17, fontWeight: "700", letterSpacing: -0.2 },
  headline: { color: cumulus.inkDim, fontSize: 13, marginTop: 5, lineHeight: 18 },
  metaRow: { flexDirection: "row", alignItems: "center", marginTop: 8, gap: 6 },
  area: { flex: 1, color: cumulus.inkMuted, fontSize: 11, fontFamily: "SF Mono" },
  expires: { color: cumulus.inkMuted, fontSize: 11, fontFamily: "SF Mono" },

  empty: {
    alignItems: "center",
    paddingTop: 80,
    paddingHorizontal: 40,
  },
  emptyDotOuter: {
    width: 72,
    height: 72,
    borderRadius: 36,
    backgroundColor: "rgba(74,222,128,0.12)",
    borderWidth: 1,
    borderColor: "rgba(74,222,128,0.35)",
    alignItems: "center",
    justifyContent: "center",
    marginBottom: 18,
  },
  emptyDot: {
    width: 28,
    height: 28,
    borderRadius: 14,
    backgroundColor: cumulus.ok,
  },
  emptyTitle: { color: cumulus.ink, fontSize: 22, fontWeight: "700", letterSpacing: -0.3 },
  emptyMeta: {
    color: cumulus.inkDim,
    fontSize: 13,
    lineHeight: 19,
    textAlign: "center",
    marginTop: 8,
  },
});
