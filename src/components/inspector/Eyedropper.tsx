/**
 * Inspector eyedropper — long-press the map to pin a point, shows a readout
 * pill with the active-layer value at that point. Uses /api/inspect on the
 * self-hosted tile-server when available, falls back to Open-Meteo for
 * temperature/wind, shows "—" otherwise.
 */
import { View, Text, StyleSheet, TouchableOpacity } from "react-native";
import { useCallback, useEffect, useState } from "react";
import MapLibreGL from "@maplibre/maplibre-react-native";
import { useWeatherStore } from "../../stores/useWeatherStore";
import { cumulus } from "../../lib/cumulusTheme";
import { formatReading, inspectPoint, type InspectReading } from "../../lib/inspector";
import type { LayerType } from "../../types/weather";

const LAYER_LABEL: Record<LayerType, string> = {
  radar: "REFLECTIVITY",
  "radar-hrrr": "HRRR FORECAST",
  temperature: "TEMPERATURE",
  wind: "WIND",
  "precip-type": "PRECIP TYPE",
  "precip-accum": "RAINFALL 1H",
  cloud: "CLOUD COVER",
  cape: "CAPE",
};

export interface PinnedPoint {
  lat: number;
  lon: number;
}

interface Props {
  pinned: PinnedPoint | null;
  onClear: () => void;
}

export function EyedropperPin({ pinned, onClear }: Props) {
  const activeLayer = useWeatherStore((s) => s.activeLayer);
  const frames = useWeatherStore((s) => s.frames);
  const currentFrameIndex = useWeatherStore((s) => s.currentFrameIndex);
  const dataSource = useWeatherStore((s) => s.dataSource);
  const serverUrl = useWeatherStore((s) => s.serverUrl);

  const [reading, setReading] = useState<InspectReading | null>(null);
  const [loading, setLoading] = useState(false);

  const frame = frames[currentFrameIndex];

  useEffect(() => {
    if (!pinned || !frame) {
      setReading(null);
      return;
    }
    let cancelled = false;
    setLoading(true);
    inspectPoint({
      dataSource,
      serverUrl,
      layer: activeLayer,
      timestamp: frame.path,
      lat: pinned.lat,
      lon: pinned.lon,
    })
      .then((r) => {
        if (!cancelled) setReading(r);
      })
      .finally(() => {
        if (!cancelled) setLoading(false);
      });
    return () => {
      cancelled = true;
    };
  }, [pinned?.lat, pinned?.lon, activeLayer, frame?.path, dataSource, serverUrl]);

  if (!pinned) return null;

  const readout = loading ? "…" : reading ? formatReading(activeLayer, reading) : "\u2014";
  const sourceLabel =
    reading?.source === "grid" ? "Grid" : reading?.source === "forecast" ? "Forecast" : "N/A";

  return (
    <>
      <MapLibreGL.MarkerView
        coordinate={[pinned.lon, pinned.lat]}
        anchor={{ x: 0.5, y: 1 }}
        allowOverlap
      >
        <View style={styles.markerWrap} pointerEvents="none">
          <View style={styles.marker}>
            <Text style={styles.markerText}>{readout}</Text>
          </View>
          <View style={styles.tail} />
          <View style={styles.crosshairDot} />
        </View>
      </MapLibreGL.MarkerView>

      <View style={styles.panel} pointerEvents="box-none">
        <View style={styles.panelHeader}>
          <Text style={styles.panelKicker}>{LAYER_LABEL[activeLayer]}</Text>
          <TouchableOpacity onPress={onClear} hitSlop={8} style={styles.closeBtn}>
            <Text style={styles.closeX}>✕</Text>
          </TouchableOpacity>
        </View>
        <Text style={styles.panelValue}>{readout}</Text>
        <View style={styles.panelMeta}>
          <Text style={styles.panelMetaText}>
            {pinned.lat.toFixed(4)}, {pinned.lon.toFixed(4)}
          </Text>
          <Text style={styles.panelSource}>{sourceLabel}</Text>
        </View>
      </View>
    </>
  );
}

const MARKER_BG = "rgba(139,124,255,0.95)";

const styles = StyleSheet.create({
  markerWrap: { alignItems: "center" },
  marker: {
    backgroundColor: MARKER_BG,
    borderRadius: 14,
    paddingHorizontal: 10,
    paddingVertical: 5,
    borderWidth: 1.5,
    borderColor: "#fff",
    shadowColor: "#000",
    shadowOpacity: 0.25,
    shadowRadius: 4,
    shadowOffset: { width: 0, height: 2 },
    elevation: 4,
  },
  markerText: {
    color: "#fff",
    fontSize: 12,
    fontWeight: "700",
    fontVariant: ["tabular-nums"],
  },
  tail: {
    width: 0,
    height: 0,
    borderLeftWidth: 5,
    borderRightWidth: 5,
    borderTopWidth: 6,
    borderLeftColor: "transparent",
    borderRightColor: "transparent",
    borderTopColor: MARKER_BG,
    marginTop: -1,
  },
  crosshairDot: {
    width: 8,
    height: 8,
    borderRadius: 4,
    backgroundColor: "#fff",
    borderWidth: 2,
    borderColor: cumulus.accent,
    marginTop: 2,
  },

  panel: {
    position: "absolute",
    left: 12,
    right: 12,
    top: 112,
    zIndex: 18,
    backgroundColor: "rgba(10,14,26,0.9)",
    borderWidth: 1,
    borderColor: "rgba(139,124,255,0.45)",
    borderRadius: 16,
    padding: 12,
  },
  panelHeader: {
    flexDirection: "row",
    justifyContent: "space-between",
    alignItems: "center",
  },
  panelKicker: {
    color: "#C7BDFF",
    fontSize: 10,
    fontWeight: "700",
    letterSpacing: 1.4,
  },
  closeBtn: {
    width: 22,
    height: 22,
    borderRadius: 11,
    backgroundColor: "rgba(255,255,255,0.1)",
    alignItems: "center",
    justifyContent: "center",
  },
  closeX: { color: cumulus.ink, fontSize: 12, fontWeight: "600" },
  panelValue: {
    color: cumulus.ink,
    fontSize: 24,
    fontWeight: "700",
    marginTop: 6,
    fontVariant: ["tabular-nums"],
  },
  panelMeta: {
    flexDirection: "row",
    justifyContent: "space-between",
    marginTop: 6,
  },
  panelMetaText: {
    color: cumulus.inkMuted,
    fontSize: 10,
    fontVariant: ["tabular-nums"],
  },
  panelSource: {
    color: cumulus.accentBright,
    fontSize: 10,
    fontWeight: "700",
    letterSpacing: 0.8,
  },
});
