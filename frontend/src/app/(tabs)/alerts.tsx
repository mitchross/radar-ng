/**
 * Cumulus Alerts tab — Redesigned for Editorial Light.
 * NWS active alerts list with Simple/Advanced gating.
 */
import { useMemo } from "react";
import { ScrollView, View, Text, StyleSheet, Pressable, RefreshControl } from "react-native";
import { SafeAreaView } from "react-native-safe-area-context";
import { LinearGradient } from "expo-linear-gradient";
import { useRouter } from "expo-router";
import { useAlerts } from "../../hooks/useAlerts";
import { useLocation } from "../../hooks/useLocation";
import { useWeatherStore } from "../../stores/useWeatherStore";
import { CONDITION_GRADIENTS } from "../../lib/cumulusTheme";
import { getAlertsScreenState } from "../../lib/weatherPresentation";
import { ScreenState } from "../../components/ui/WeatherClearUI";
import { useWeatherClearTheme } from "../../theme/WeatherClearThemeProvider";
import type { WeatherClearTheme } from "../../theme/weatherClearTheme";
import type { NWSAlert } from "../../types/weather";

export default function AlertsScreen() {
  useLocation();
  const router = useRouter();
  const { theme } = useWeatherClearTheme();
  const styles = useMemo(() => createStyles(theme), [theme]);
  const viewMode = useWeatherStore((s) => s.viewMode);
  const { data, isLoading, isError, refetch, isFetching } = useAlerts();
  const alerts = data?.features ?? [];
  const isAdv = viewMode === "advanced";
  const presentation = getAlertsScreenState({ data, isLoading, isError });
  const gradient = theme.dark
    ? ([theme.colors.canvas, theme.colors.surfaceStrong] as const)
    : CONDITION_GRADIENTS.storm;

  return (
    <LinearGradient
      accessibilityLabel="Weather alerts"
      colors={gradient}
      style={styles.container}
    >
      <SafeAreaView style={styles.flex} edges={["top"]}>
        <View style={styles.header}>
          <View>
            <Text style={styles.kicker}>ACTIVE ALERTS</Text>
            <Text style={styles.title}>
              {presentation.kind === "error"
                ? "Unavailable"
                : alerts.length > 0
                  ? `${alerts.length} active`
                  : "All clear"}
            </Text>
          </View>
          <Pressable
            accessibilityRole="button"
            accessibilityLabel="Refresh weather alerts"
            onPress={() => refetch()}
            style={styles.refresh}
          >
            <Text style={styles.refreshText}>↻</Text>
          </Pressable>
        </View>

        <ScrollView
          contentInsetAdjustmentBehavior="automatic"
          contentContainerStyle={styles.scroll}
          showsVerticalScrollIndicator={false}
          refreshControl={
            <RefreshControl
              refreshing={isFetching}
              onRefresh={refetch}
              tintColor={theme.colors.text}
              colors={[theme.colors.accent]}
            />
          }
        >
          {presentation.kind === "loading" ? (
            <ScreenState
              kind="loading"
              title="Loading alerts"
              message="Checking the National Weather Service."
            />
          ) : null}
          {presentation.kind === "error" ? (
            <ScreenState
              kind="error"
              title="Alerts unavailable"
              message="The National Weather Service could not be reached."
              actionLabel="Try again"
              onAction={() => refetch()}
            />
          ) : null}
          {presentation.kind === "empty" ? <EmptyState isAdv={isAdv} /> : null}
          {presentation.kind === "content" ? alerts.map((alert) => (
            <AlertCard
              key={alert.id}
              alert={alert}
              onPress={() => router.push(`/alert/${encodeURIComponent(alert.id)}` as any)}
            />
          )) : null}
          <View style={{ height: 120 }} />
        </ScrollView>
      </SafeAreaView>
    </LinearGradient>
  );
}

function AlertCard({ alert, onPress }: { alert: NWSAlert; onPress: () => void }) {
  const { theme } = useWeatherClearTheme();
  const styles = useMemo(() => createStyles(theme), [theme]);
  const color = severityColor(alert.properties.severity, theme);
  const expires = new Date(alert.properties.expires);
  const expiresLabel = expires.toLocaleString([], {
    month: "short",
    day: "numeric",
    hour: "numeric",
    minute: "2-digit",
  });
  return (
    <Pressable
      accessibilityRole="button"
      accessibilityLabel={`${alert.properties.severity} ${alert.properties.event}. ${alert.properties.areaDesc}. Until ${expiresLabel}`}
      onPress={onPress}
      style={[
        styles.card,
        {
          boxShadow: `0 4px 12px ${color}22`,
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
        {alert.properties.headline ? (
          <Text style={styles.headline} numberOfLines={3}>{alert.properties.headline}</Text>
        ) : null}
        <View style={styles.metaRow}>
          <Text style={styles.area} numberOfLines={1}>{alert.properties.areaDesc}</Text>
          <Text style={styles.expires}>•  Until {expiresLabel}</Text>
        </View>
      </View>
    </Pressable>
  );
}

function EmptyState({ isAdv }: { isAdv: boolean }) {
  const { theme } = useWeatherClearTheme();
  const styles = useMemo(() => createStyles(theme), [theme]);
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

      {isAdv ? (
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
      ) : null}
    </View>
  );
}

function severityColor(
  severity: NWSAlert["properties"]["severity"],
  theme: WeatherClearTheme,
): string {
  return {
    Extreme: theme.colors.destructive,
    Severe: theme.colors.accent,
    Moderate: theme.colors.warning,
    Minor: theme.colors.rain,
    Unknown: theme.colors.textMuted,
  }[severity];
}

function createStyles(theme: WeatherClearTheme) {
  const cumulus = {
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
    width: 44,
    height: 44,
    borderRadius: 12,
    backgroundColor: theme.colors.surfaceMuted,
    borderWidth: 1,
    borderColor: theme.colors.divider,
    alignItems: "center",
    justifyContent: "center",
  },
  refreshText: { color: cumulus.ink, fontSize: 18, fontWeight: "700" },

  scroll: { paddingHorizontal: 16, paddingTop: 4, paddingBottom: 140 },
  muted: { color: cumulus.inkMuted, textAlign: "center", paddingVertical: 20, fontFamily: cumulusFonts.ui },

  card: {
    flexDirection: "row",
    backgroundColor: theme.colors.surface,
    borderWidth: 1,
    borderColor: theme.colors.border,
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
    backgroundColor: theme.colors.success,
    alignItems: "center",
    justifyContent: "center",
    boxShadow: `0 4px 10px ${theme.colors.success}55`,
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
    backgroundColor: theme.colors.surface,
    borderWidth: 1,
    borderColor: theme.colors.border,
    borderRadius: 20,
    paddingVertical: 4,
    paddingHorizontal: 16,
    boxShadow: theme.dark
      ? "0 3px 10px rgba(0,0,0,0.24)"
      : "0 3px 10px rgba(60,50,40,0.05)",
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
    borderTopColor: theme.colors.divider,
  },
  });
}
