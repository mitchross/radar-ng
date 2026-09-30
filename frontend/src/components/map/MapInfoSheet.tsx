/**
 * Bottom sheet for a tapped map feature (storm cell, warning polygon) —
 * same card language as TropicalDetailSheet.
 */
import type { ReactNode } from "react";
import { Modal, Pressable, StyleSheet, Text, View } from "react-native";
import { SafeAreaView } from "react-native-safe-area-context";
import { cumulus, cumulusFonts } from "../../lib/cumulusTheme";

export interface InfoMetric {
  label: string;
  value: string;
  unit?: string;
}

export function MapInfoSheet({
  visible,
  eyebrow,
  title,
  subtitle,
  accent,
  metrics = [],
  pill,
  children,
  onClose,
}: {
  visible: boolean;
  eyebrow: string;
  title: string;
  subtitle?: string;
  accent: string;
  metrics?: InfoMetric[];
  pill?: string | null;
  children?: ReactNode;
  onClose: () => void;
}) {
  if (!visible) return null;
  return (
    <Modal transparent visible animationType="fade" onRequestClose={onClose}>
      <View style={styles.modal}>
        <Pressable style={StyleSheet.absoluteFill} onPress={onClose} accessibilityRole="button" accessibilityLabel={`Close ${eyebrow.toLowerCase()}`} />
        <SafeAreaView style={styles.safeArea} edges={["bottom"]}>
          <View style={styles.sheet} accessibilityViewIsModal>
            <View style={styles.handle} />
            <View style={styles.header}>
              <View style={[styles.dot, { backgroundColor: accent }]} />
              <View style={styles.titleBlock}>
                <Text style={[styles.eyebrow, { color: accent }]}>{eyebrow}</Text>
                <Text style={styles.title} numberOfLines={2}>{title}</Text>
                {subtitle ? <Text style={styles.subtitle}>{subtitle}</Text> : null}
              </View>
              <Pressable onPress={onClose} hitSlop={8} style={styles.close} accessibilityRole="button" accessibilityLabel="Close">
                <Text style={styles.closeText}>×</Text>
              </Pressable>
            </View>
            {pill ? <Text style={[styles.pill, { color: accent, borderColor: accent }]}>{pill}</Text> : null}
            {metrics.length ? (
              <View style={styles.metrics}>
                {metrics.map((m) => (
                  <View key={m.label} style={styles.metric}>
                    <Text style={styles.metricLabel}>{m.label}</Text>
                    <Text style={styles.metricValue}>
                      {m.value}
                      {m.unit ? <Text style={styles.metricUnit}> {m.unit}</Text> : null}
                    </Text>
                  </View>
                ))}
              </View>
            ) : null}
            {children}
          </View>
        </SafeAreaView>
      </View>
    </Modal>
  );
}

/** "✦ In plain English" block shared by warning sheets and the alert screen. */
export function PlainEnglish({ what, doText }: { what: string; doText: string }) {
  return (
    <View style={styles.ai} accessibilityLabel={`In plain English. ${what} Do: ${doText}`}>
      <Text style={styles.aiTitle}>{"✦"} In plain English</Text>
      <Text style={styles.aiText}>{what}</Text>
      {doText ? <Text style={styles.aiText}><Text style={styles.aiDo}>Do: </Text>{doText}</Text> : null}
    </View>
  );
}

const styles = StyleSheet.create({
  modal: { flex: 1, justifyContent: "flex-end", backgroundColor: "rgba(8,12,20,0.34)" },
  safeArea: { justifyContent: "flex-end" },
  sheet: {
    alignSelf: "center", width: "94%", maxWidth: 540, marginVertical: 12, paddingHorizontal: 18, paddingTop: 8, paddingBottom: 18,
    borderRadius: 28, backgroundColor: "rgba(250,248,243,0.98)", borderWidth: StyleSheet.hairlineWidth, borderColor: "rgba(255,255,255,0.95)",
    shadowColor: "#0B1220", shadowOpacity: 0.24, shadowRadius: 26, shadowOffset: { width: 0, height: 12 }, elevation: 18,
  },
  handle: { width: 38, height: 4, borderRadius: 2, alignSelf: "center", marginBottom: 12, backgroundColor: "rgba(33,31,27,0.18)" },
  header: { flexDirection: "row", alignItems: "center" },
  dot: { width: 14, height: 14, borderRadius: 7, borderWidth: 2, borderColor: "#fff" },
  titleBlock: { flex: 1, marginLeft: 12 },
  eyebrow: { fontSize: 10, fontWeight: "700", letterSpacing: 1.2 },
  title: { color: cumulus.ink, fontFamily: cumulusFonts.display, fontSize: 24, lineHeight: 28 },
  subtitle: { color: cumulus.inkDim, fontSize: 13, marginTop: 2 },
  close: { width: 32, height: 32, borderRadius: 16, alignItems: "center", justifyContent: "center", backgroundColor: "rgba(33,31,27,0.07)" },
  closeText: { color: cumulus.inkDim, fontSize: 22, lineHeight: 24 },
  pill: { alignSelf: "flex-start", marginTop: 12, paddingHorizontal: 10, paddingVertical: 3, borderRadius: 999, borderWidth: 1, fontSize: 12, fontWeight: "700", overflow: "hidden" },
  metrics: { flexDirection: "row", marginTop: 14, gap: 16 },
  metric: { flex: 1 },
  metricLabel: { color: cumulus.inkMuted, fontSize: 10, fontWeight: "700", letterSpacing: 1 },
  metricValue: { color: cumulus.ink, fontFamily: cumulusFonts.display, fontSize: 22, marginTop: 2 },
  metricUnit: { fontSize: 12, color: cumulus.inkDim, fontFamily: cumulusFonts.ui },
  ai: { marginTop: 14, padding: 12, borderRadius: 16, backgroundColor: "rgba(124,92,255,0.08)", borderWidth: 1, borderColor: "rgba(124,92,255,0.25)" },
  aiTitle: { color: "#6a4df0", fontSize: 12, fontWeight: "700", marginBottom: 4 },
  aiText: { color: cumulus.ink, fontSize: 14, lineHeight: 20, marginTop: 2 },
  aiDo: { fontWeight: "700" },
});
