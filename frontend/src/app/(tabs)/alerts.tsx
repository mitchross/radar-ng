/**
 * Cumulus Alerts tab — Redesigned for Editorial Light.
 * NWS active alerts list with Simple/Advanced gating.
 */
import { ScrollView, View, Text, StyleSheet, TouchableOpacity, RefreshControl } from "react-native";
import { SafeAreaView } from "react-native-safe-area-context";
import { LinearGradient } from "expo-linear-gradient";
import { useRouter } from "expo-router";
import { useAlerts } from "../../hooks/useAlerts";
import { useLocation } from "../../hooks/useLocation";
import { useWeatherStore } from "../../stores/useWeatherStore";
import { cumulus, cumulusFonts, CONDITION_GRADIENTS } from "../../lib/cumulusTheme";
import type { NWSAlert } from "../../types/weather";

const SEVERITY_COLOR: Record<NWSAlert["properties"]["severity"], string> = {
  Extreme: cumulus.alert,
  Severe: cumulus.accent,
  Moderate: "#f0c34e",
  Minor: cumulus.rain,
  Unknown: "rgba(33, 31, 27, 0.4)",
};

const SEVERITY_GLOW: Record<NWSAlert["properties"]["severity"], string> = {
  Extreme: "rgba(223, 106, 106, 0.2)",
  Severe: "rgba(194, 96, 58, 0.15)",
  Moderate: "rgba(240, 195, 78, 0.15)",
  Minor: "rgba(77, 127, 180, 0.15)",
  Unknown: "rgba(33, 31, 27, 0.05)",
};

export default function AlertsScreen() {
  useLocation();
  const router = useRouter();
  const viewMode = useWeatherStore((s) => s.viewMode);
  const { data, isLoading, refetch, isFetching } = useAlerts();
  const alerts = data?.features ?? [];
  const isAdv = viewMode === "advanced";

  return (
    <LinearGradient colors={CONDITION_GRADIENTS.storm} style={styles.container}>
      <SafeAreaView style={styles.flex} edges={["top"]}>
        <View style={styles.header}>
          <View>
            <Text style={styles.kicker}>ACTIVE ALERTS</Text>
            <Text style={styles.title}>
              {alerts.length > 0 ? `${alerts.length} active` : "All clear"}
            </Text>
          </View>
          <TouchableOpacity onPress={() => refetch()} style={styles.refresh} activeOpacity={0.7}>
            <Text style={styles.refreshText}>↻</Text>
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
          {!isLoading && alerts.length === 0 && <EmptyState isAdv={isAdv} />}
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
  const glow = SEVERITY_GLOW[alert.properties.severity];
  const expires = new Date(alert.properties.expires);
  const expiresLabel = expires.toLocaleString([], {
    month: "short",
    day: "numeric",
    hour: "numeric",
    minute: "2-digit",
  });
  return (
    <TouchableOpacity
      onPress={onPress}
      activeOpacity={0.85}
      style={[
        styles.card,
        {
          shadowColor: glow,
          shadowOpacity: 0.4,
          shadowRadius: 10,
          shadowOffset: { width: 0, height: 4 },
        },
      ]}
    >
      <View style={[styles.stripe, { backgroundColor: color }]} />
      <View style={styles.cardBody}>
        <View style={styles.cardHeaderRow}>
          <View style={[styles.severityPill, { borderColor: `${color}33`, backgroundColor: `${color}12` }]}>
            <View style={[styles.severityDot, { backgroundColor: color }]} />
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

function EmptyState({ isAdv }: { isAdv: boolean }) {
  return (
    <View style={styles.emptyContainer}>
      <View style={styles.empty}>
        {/* Pulsing check circle indicator */}
        <View style={styles.pulsingCircle}>
          <View style={styles.innerCheckCircle}>
            <Text style={styles.checkMarkText}>✓</Text>
          </View>
        </View>
        <Text style={styles.emptyTitle}>No active alerts</Text>
        <Text style={styles.emptyMeta}>
          The National Weather Service reports no warnings, watches, or advisories for your location.
        </Text>
      </View>

      {isAdv && (
        <View style={styles.metadataCard}>
          <View style={styles.metadataRow}>
            <Text style={styles.metadataLabel}>Source</Text>
            <Text style={styles.metadataValue}>NWS CAP</Text>
          </View>
          <View style={[styles.metadataRow, styles.rowBorder]}>
            <Text style={styles.metadataLabel}>Monitored zone</Text>
            <Text style={styles.metadataValue}>MIZ064 · Kent</Text>
          </View>
          <View style={[styles.metadataRow, styles.rowBorder]}>
            <Text style={styles.metadataLabel}>Last polled</Text>
            <Text style={styles.metadataValue}>2 min ago</Text>
          </View>
        </View>
      )}
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
    paddingHorizontal: 24,
    paddingTop: 16,
    paddingBottom: 8,
  },
  kicker: {
    fontSize: 10,
    fontFamily: cumulusFonts.ui,
    fontWeight: "800",
    color: cumulus.inkMuted,
    letterSpacing: 1.6,
  },
  title: {
    fontSize: 34,
    color: cumulus.ink,
    fontFamily: cumulusFonts.display,
    fontWeight: "500",
    letterSpacing: -0.4,
    marginTop: 4,
  },
  refresh: {
    width: 36,
    height: 36,
    borderRadius: 12,
    backgroundColor: "#eae4d8",
    borderWidth: 1,
    borderColor: "#e3dccf",
    alignItems: "center",
    justifyContent: "center",
  },
  refreshText: { color: cumulus.ink, fontSize: 18, fontWeight: "700" },

  scroll: { paddingHorizontal: 16, paddingTop: 4, paddingBottom: 140 },
  muted: { color: cumulus.inkMuted, textAlign: "center", paddingVertical: 20, fontFamily: cumulusFonts.ui },

  card: {
    flexDirection: "row",
    backgroundColor: "#ffffff",
    borderWidth: 1,
    borderColor: "#eee6d8",
    borderRadius: 20,
    overflow: "hidden",
    marginBottom: 10,
  },
  stripe: { width: 5 },
  cardBody: { flex: 1, padding: 14 },
  cardHeaderRow: { flexDirection: "row", alignItems: "center", gap: 10, marginBottom: 6 },
  severityPill: {
    flexDirection: "row",
    alignItems: "center",
    gap: 6,
    paddingHorizontal: 8,
    paddingVertical: 3,
    borderRadius: 6,
    borderWidth: 1,
  },
  severityDot: { width: 6, height: 6, borderRadius: 3 },
  severityText: { fontSize: 10, fontWeight: "800", letterSpacing: 0.8, fontFamily: cumulusFonts.mono },
  urgency: { fontSize: 10, color: cumulus.inkMuted, fontFamily: cumulusFonts.mono, letterSpacing: 0.8 },
  event: { color: cumulus.ink, fontSize: 17, fontWeight: "700", letterSpacing: -0.2, fontFamily: cumulusFonts.ui },
  headline: { color: cumulus.inkDim, fontSize: 13, marginTop: 5, lineHeight: 18, fontFamily: cumulusFonts.ui },
  metaRow: { flexDirection: "row", alignItems: "center", marginTop: 8, gap: 6 },
  area: { flex: 1, color: cumulus.inkMuted, fontSize: 11, fontFamily: cumulusFonts.mono },
  expires: { color: cumulus.inkMuted, fontSize: 11, fontFamily: cumulusFonts.mono },

  emptyContainer: {
    width: "100%",
  },
  empty: {
    alignItems: "center",
    paddingTop: 64,
    paddingHorizontal: 40,
  },
  pulsingCircle: {
    width: 130,
    height: 130,
    borderRadius: 65,
    backgroundColor: "rgba(46, 158, 99, 0.08)",
    borderWidth: 1,
    borderColor: "rgba(46, 158, 99, 0.18)",
    alignItems: "center",
    justifyContent: "center",
    marginBottom: 26,
  },
  innerCheckCircle: {
    width: 62,
    height: 62,
    borderRadius: 31,
    backgroundColor: "#2e9e63",
    alignItems: "center",
    justifyContent: "center",
    shadowColor: "#2e9e63",
    shadowOffset: { width: 0, height: 4 },
    shadowOpacity: 0.3,
    shadowRadius: 6,
    elevation: 3,
  },
  checkMarkText: {
    color: "#ffffff",
    fontSize: 26,
    fontWeight: "700",
  },
  emptyTitle: {
    color: cumulus.ink,
    fontSize: 25,
    fontFamily: cumulusFonts.display,
    fontWeight: "500",
  },
  emptyMeta: {
    color: cumulus.inkDim,
    fontSize: 14,
    lineHeight: 20,
    textAlign: "center",
    marginTop: 8,
    fontFamily: cumulusFonts.ui,
  },

  // Metadata card
  metadataCard: {
    marginTop: 34,
    marginHorizontal: 16,
    backgroundColor: "#ffffff",
    borderWidth: 1,
    borderColor: "#eee6d8",
    borderRadius: 20,
    paddingVertical: 4,
    paddingHorizontal: 16,
    shadowColor: "rgba(60,50,40,0.04)",
    shadowOffset: { width: 0, height: 4 },
    shadowOpacity: 0.3,
    shadowRadius: 6,
    elevation: 1,
  },
  metadataRow: {
    flexDirection: "row",
    justifyContent: "space-between",
    alignItems: "center",
    paddingVertical: 13,
  },
  metadataLabel: {
    fontSize: 13,
    fontFamily: cumulusFonts.ui,
    fontWeight: "600",
    color: cumulus.inkDim,
  },
  metadataValue: {
    fontSize: 14,
    fontFamily: cumulusFonts.display,
    fontWeight: "500",
    color: cumulus.ink,
  },
  rowBorder: {
    borderTopWidth: 1,
    borderTopColor: "#f1ebdd",
  },
});
