/**
 * Segmented Current / Forecast control — lives above the TimelineBar on the
 * radar screen. Current = past MRMS only; Forecast = past MRMS + nowcast
 * (pysteps) + HRRR (+1h..+48h), merged into one timeline by useManifest.
 */
import { View, Text, TouchableOpacity, StyleSheet } from "react-native";
import { useWeatherStore } from "../../stores/useWeatherStore";
import { cumulus } from "../../lib/cumulusTheme";
import type { TimelineMode } from "../../types/weather";

const SEGMENTS: { id: TimelineMode; label: string }[] = [
  { id: "current", label: "Current" },
  { id: "forecast", label: "Forecast" },
];

export function CurrentForecastToggle() {
  const mode = useWeatherStore((s) => s.timelineMode);
  const setMode = useWeatherStore((s) => s.setTimelineMode);

  return (
    <View style={styles.wrap}>
      <View style={styles.segments}>
        {SEGMENTS.map((s) => {
          const active = mode === s.id;
          return (
            <TouchableOpacity
              key={s.id}
              onPress={() => setMode(s.id)}
              style={[styles.seg, active && styles.segActive]}
              activeOpacity={0.85}
            >
              <Text style={[styles.segLabel, active && styles.segLabelActive]}>
                {s.label}
              </Text>
            </TouchableOpacity>
          );
        })}
      </View>
    </View>
  );
}

const styles = StyleSheet.create({
  wrap: {
    position: "absolute",
    bottom: 192,
    left: 0,
    right: 0,
    zIndex: 28,
    alignItems: "center",
  },
  segments: {
    flexDirection: "row",
    backgroundColor: "rgba(10,14,26,0.85)",
    borderRadius: 14,
    padding: 3,
    borderWidth: 1,
    borderColor: "rgba(255,255,255,0.1)",
  },
  seg: {
    paddingHorizontal: 18,
    paddingVertical: 7,
    borderRadius: 10,
  },
  segActive: { backgroundColor: cumulus.accent },
  segLabel: { color: cumulus.inkDim, fontSize: 12, fontWeight: "600" },
  segLabelActive: { color: "#fff", fontWeight: "700" },
});
