/**
 * Inspector eyedropper — long-press the map to pin a point, shows a readout
 * pill with the active-layer value at that point. Uses /api/inspect on the
 * self-hosted tile-server when available, falls back to Open-Meteo for
 * temperature/wind, shows "—" otherwise.
 */
import { View, Text, StyleSheet, Pressable } from "react-native";
import { useEffect, useState } from "react";
import { Marker } from "@maplibre/maplibre-react-native";
import { useWeatherStore } from "../../stores/useWeatherStore";
import { DEFAULTS, MAP_CHROME_MAX_FONT_SCALE } from "../../lib/constants";
import { cumulus } from "../../lib/cumulusTheme";
import { formatReading, inspectPoint, type InspectReading } from "../../lib/inspector";
import type { LayerType } from "../../types/weather";
import { useMapChromeInsets } from "../../hooks/useMapChromeInsets";

const LAYER_LABEL: Record<LayerType, string> = {
  radar: "REFLECTIVITY",
  "radar-composite": "COMPOSITE",
  "radar-hrrr": "HRRR FORECAST",
  temperature: "TEMPERATURE",
  wind: "WIND",
  "precip-type": "PRECIP TYPE",
  "precip-accum": "RAINFALL 1H",
  cloud: "CLOUD COVER",
  cape: "CAPE",
  "air-quality": "PM2.5",
  ozone: "OZONE",
};

// One /api/inspect per playback tick was a fetch storm.
const INSPECT_DEBOUNCE_MS = 300;

export interface PinnedPoint {
  lat: number;
  lon: number;
}

interface Props {
  pinned: PinnedPoint | null;
  onClear: () => void;
}

export function EyedropperPin({ pinned, onClear }: Props) {
  const chrome = useMapChromeInsets();
  const activeLayer = useWeatherStore((s) => s.activeLayer);
  const serverUrl = useWeatherStore((s) => s.serverUrl);
  const isPlaying = useWeatherStore((s) => s.isPlaying);

  // Each reading remembers the pin it belongs to, so a new or cleared pin
  // never shows a stale value, and no effect has to reset state.
  const [result, setResult] = useState<{ pin: PinnedPoint; reading: InspectReading | null } | null>(null);
  const [pendingRequest, setPendingRequest] = useState<string | null>(null);

  // The timestamp only matters with a pin and paused playback; selecting null
  // otherwise keeps this native marker from re-rendering on every tick.
  // (Keyed on the timestamp, not the frame object, which every manifest poll rebuilds.)
  const hasPin = pinned != null;
  const frameTimestamp = useWeatherStore((s) =>
    hasPin && !s.isPlaying ? (s.frames[s.currentFrameIndex]?.timestamp ?? null) : null,
  );

  // Keep the last reading on screen while frames tick by; refetch once paused.
  const requestKey =
    pinned && !isPlaying && frameTimestamp
      ? `${serverUrl}|${activeLayer}|${frameTimestamp}|${pinned.lat}|${pinned.lon}`
      : null;

  useEffect(() => {
    if (!pinned || !requestKey || !frameTimestamp) return;
    const ctrl = new AbortController();
    const timer = setTimeout(() => {
      setPendingRequest(requestKey);
      inspectPoint({
        serverUrl,
        layer: activeLayer,
        timestamp: frameTimestamp,
        lat: pinned.lat,
        lon: pinned.lon,
        signal: ctrl.signal,
      })
        .then((r) => {
          if (!ctrl.signal.aborted) setResult({ pin: pinned, reading: r });
        })
        .finally(() => {
          if (!ctrl.signal.aborted) setPendingRequest(null);
        });
    }, INSPECT_DEBOUNCE_MS);
    return () => {
      clearTimeout(timer);
      ctrl.abort();
    };
  }, [pinned, requestKey, activeLayer, frameTimestamp, serverUrl]);

  const reading = result && result.pin === pinned ? result.reading : null;
  const loading = requestKey !== null && pendingRequest === requestKey;

  // Always mounted: the Marker is a native child of <Map>, so it hides (opacity 0,
  // parked at the last pin) instead of unmounting to keep the child count constant.
  const [lastPin, setLastPin] = useState<PinnedPoint>({ lat: DEFAULTS.LATITUDE, lon: DEFAULTS.LONGITUDE });
  if (pinned && pinned !== lastPin) setLastPin(pinned);
  const shown = pinned ?? lastPin;
  const hidden = pinned == null;

  const readout = loading ? "…" : reading ? formatReading(activeLayer, reading) : "\u2014";
  const sourceLabel = reading?.source === "grid" ? "Grid" : "N/A";

  return (
    <>
      <Marker lngLat={[shown.lon, shown.lat]} anchor="bottom">
        <View style={[styles.markerWrap, hidden ? styles.hidden : null]} pointerEvents="none">
          <View style={styles.marker}>
            <Text maxFontSizeMultiplier={MAP_CHROME_MAX_FONT_SCALE} style={styles.markerText}>{readout}</Text>
          </View>
          <View style={styles.tail} />
          <View style={styles.crosshairDot} />
        </View>
      </Marker>

      <View
        style={[styles.panel, { top: chrome.top, left: chrome.left, right: chrome.right }, hidden ? styles.hidden : null]}
        pointerEvents={hidden ? "none" : "box-none"}
        accessibilityElementsHidden={hidden}
        importantForAccessibility={hidden ? "no-hide-descendants" : "auto"}
      >
        <View style={styles.panelHeader}>
          <Text maxFontSizeMultiplier={MAP_CHROME_MAX_FONT_SCALE} style={styles.panelKicker}>{LAYER_LABEL[activeLayer]}</Text>
          <Pressable
            onPress={onClear}
            style={styles.closeBtn}
            accessibilityRole="button"
            accessibilityLabel="Clear inspected point"
          >
            <View style={styles.closeCircle}>
              <Text maxFontSizeMultiplier={MAP_CHROME_MAX_FONT_SCALE} style={styles.closeX}>✕</Text>
            </View>
          </Pressable>
        </View>
        <Text maxFontSizeMultiplier={MAP_CHROME_MAX_FONT_SCALE} style={styles.panelValue}>{readout}</Text>
        <View style={styles.panelMeta}>
          <Text maxFontSizeMultiplier={MAP_CHROME_MAX_FONT_SCALE} style={styles.panelMetaText}>
            {shown.lat.toFixed(4)}, {shown.lon.toFixed(4)}
          </Text>
          <Text maxFontSizeMultiplier={MAP_CHROME_MAX_FONT_SCALE} style={styles.panelSource}>{sourceLabel}</Text>
        </View>
      </View>
    </>
  );
}

const MARKER_BG = "rgba(139,124,255,0.95)";

const styles = StyleSheet.create({
  hidden: { opacity: 0 },
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
    width: 44,
    height: 44,
    marginRight: -11,
    marginVertical: -11,
    alignItems: "center",
    justifyContent: "center",
  },
  closeCircle: {
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
