import { Modal, Pressable, StyleSheet, Text, View } from "react-native";
import { SafeAreaView } from "react-native-safe-area-context";
import { cumulus, cumulusFonts } from "../../lib/cumulusTheme";
import type { TropicalStormDetails } from "./TropicalOverlay";

export function TropicalDetailSheet({
  storm,
  onClose,
}: {
  storm: TropicalStormDetails | null;
  onClose: () => void;
}) {
  if (!storm) return null;
  const classification = classificationName(storm.classification);

  return (
    <Modal transparent visible animationType="fade" onRequestClose={onClose}>
      <View style={styles.modal}>
        <Pressable
          style={StyleSheet.absoluteFill}
          onPress={onClose}
          accessibilityRole="button"
          accessibilityLabel="Close tropical storm details"
        />
        <SafeAreaView style={styles.safeArea} edges={["bottom"]}>
          <View style={styles.sheet} accessibilityViewIsModal>
            <View style={styles.handle} />
            <View style={styles.header}>
              <View style={styles.stormIcon}>
                <View style={styles.stormEye} />
              </View>
              <View style={styles.titleBlock}>
                <Text style={styles.eyebrow}>ACTIVE TROPICAL SYSTEM</Text>
                <Text style={styles.title}>{storm.name}</Text>
                <Text style={styles.subtitle}>{classification}</Text>
              </View>
              <Pressable
                onPress={onClose}
                hitSlop={8}
                style={({ pressed }) => [styles.close, pressed && styles.pressed]}
                accessibilityRole="button"
                accessibilityLabel="Close tropical storm details"
              >
                <Text style={styles.closeText}>×</Text>
              </Pressable>
            </View>

            <View style={styles.metrics}>
              <Metric
                label="MAX WIND"
                value={storm.windMph == null ? "—" : `${Math.round(storm.windMph)}`}
                unit="mph"
              />
              <View style={styles.divider} />
              <Metric
                label="PRESSURE"
                value={storm.pressureMb == null ? "—" : `${Math.round(storm.pressureMb)}`}
                unit="mb"
              />
            </View>

            <View style={styles.advisory}>
              <Text style={styles.advisoryTitle}>Official NHC advisory</Text>
              <Text style={styles.advisoryText}>
                {storm.updatedAt ? `Updated ${formatAdvisoryTime(storm.updatedAt)} · ` : ""}
                {storm.stormId.toUpperCase()} · served by your radar-ng backend
              </Text>
            </View>
          </View>
        </SafeAreaView>
      </View>
    </Modal>
  );
}

function Metric({ label, value, unit }: { label: string; value: string; unit: string }) {
  return (
    <View style={styles.metric}>
      <Text style={styles.metricLabel}>{label}</Text>
      <Text style={styles.metricValue}>
        {value} <Text style={styles.metricUnit}>{unit}</Text>
      </Text>
    </View>
  );
}

function classificationName(value?: string): string {
  switch (value?.toUpperCase()) {
    case "TD": return "Tropical Depression";
    case "TS": return "Tropical Storm";
    case "HU": return "Hurricane";
    case "MH": return "Major Hurricane";
    case "PTC": return "Potential Tropical Cyclone";
    default: return value || "Tropical Cyclone";
  }
}

function formatAdvisoryTime(value: string): string {
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) return value;
  return new Intl.DateTimeFormat(undefined, {
    month: "short",
    day: "numeric",
    hour: "numeric",
    minute: "2-digit",
  }).format(date);
}

const styles = StyleSheet.create({
  modal: {
    flex: 1,
    justifyContent: "flex-end",
    backgroundColor: "rgba(8,12,20,0.34)",
  },
  safeArea: { justifyContent: "flex-end" },
  sheet: {
    margin: 12,
    paddingHorizontal: 18,
    paddingTop: 8,
    paddingBottom: 18,
    borderRadius: 28,
    backgroundColor: "rgba(250,248,243,0.98)",
    borderWidth: StyleSheet.hairlineWidth,
    borderColor: "rgba(255,255,255,0.95)",
    shadowColor: "#0B1220",
    shadowOpacity: 0.24,
    shadowRadius: 26,
    shadowOffset: { width: 0, height: 12 },
    elevation: 18,
  },
  handle: {
    width: 38,
    height: 4,
    borderRadius: 2,
    alignSelf: "center",
    marginBottom: 12,
    backgroundColor: "rgba(33,31,27,0.18)",
  },
  header: { flexDirection: "row", alignItems: "center" },
  stormIcon: {
    width: 48,
    height: 48,
    borderRadius: 24,
    alignItems: "center",
    justifyContent: "center",
    backgroundColor: "#FF3B4A",
  },
  stormEye: {
    width: 15,
    height: 15,
    borderRadius: 8,
    borderWidth: 4,
    borderColor: "#FFFFFF",
  },
  titleBlock: { flex: 1, marginLeft: 12 },
  eyebrow: {
    color: "#9F1422",
    fontSize: 10,
    fontWeight: "700",
    letterSpacing: 1.2,
  },
  title: {
    color: cumulus.ink,
    fontFamily: cumulusFonts.display,
    fontSize: 27,
    lineHeight: 31,
  },
  subtitle: { color: cumulus.inkDim, fontSize: 13 },
  close: {
    width: 44,
    height: 44,
    borderRadius: 22,
    alignItems: "center",
    justifyContent: "center",
    backgroundColor: "rgba(33,31,27,0.07)",
  },
  closeText: { color: cumulus.ink, fontSize: 28, lineHeight: 30, fontWeight: "300" },
  pressed: { opacity: 0.62 },
  metrics: {
    flexDirection: "row",
    marginTop: 18,
    paddingVertical: 14,
    borderTopWidth: StyleSheet.hairlineWidth,
    borderBottomWidth: StyleSheet.hairlineWidth,
    borderColor: cumulus.cardLine,
  },
  metric: { flex: 1, alignItems: "center" },
  metricLabel: { color: cumulus.inkMuted, fontSize: 9, fontWeight: "700", letterSpacing: 1.1 },
  metricValue: { color: cumulus.ink, fontSize: 25, fontWeight: "600", marginTop: 4 },
  metricUnit: { color: cumulus.inkMuted, fontSize: 12, fontWeight: "600" },
  divider: { width: StyleSheet.hairlineWidth, backgroundColor: cumulus.cardLine },
  advisory: { marginTop: 14 },
  advisoryTitle: { color: cumulus.ink, fontSize: 13, fontWeight: "700" },
  advisoryText: { color: cumulus.inkMuted, fontSize: 11, lineHeight: 16, marginTop: 3 },
});
