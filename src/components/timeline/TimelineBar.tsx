/**
 * Cumulus radar timeline — segmented track (past / nowcast / HRRR / long-range)
 * + NOW marker + tick ticks + draggable thumb.
 *
 * Segments are computed from frame unix-seconds relative to "now":
 *   past        < 0 min         rgba(255,255,255,0.22)  solid
 *   nowcast     0…+60 min       rgba(139,124,255,0.55)  solid violet
 *   HRRR        +60…+360 min    dashed violet
 *   long-range  +360 min …      fainter dashed
 *
 * IEM (free tier) never has future frames, so only the past segment renders.
 * Self-hosted with `radar-hrrr` layer exposes nowcast + HRRR + long-range.
 */
import { View, Text, TouchableOpacity, StyleSheet } from "react-native";
import Slider from "@react-native-community/slider";
import { useEffect, useRef } from "react";
import { useWeatherStore } from "../../stores/useWeatherStore";
import { cumulus } from "../../lib/cumulusTheme";
import type { LayerType } from "../../types/weather";

const NOWCAST_MIN = 60;
const HRRR_MIN = 360;

const LAYER_TITLE: Record<LayerType, string> = {
  radar: "Reflectivity",
  "radar-hrrr": "HRRR Forecast",
  temperature: "Temperature",
  wind: "Wind Speed",
  "precip-type": "Precipitation",
  "precip-accum": "Rainfall (1h)",
  cloud: "Cloud Cover",
  cape: "CAPE",
};

export function TimelineBar() {
  const frames = useWeatherStore((s) => s.frames);
  const currentFrameIndex = useWeatherStore((s) => s.currentFrameIndex);
  const setCurrentFrameIndex = useWeatherStore((s) => s.setCurrentFrameIndex);
  const isPlaying = useWeatherStore((s) => s.isPlaying);
  const togglePlaying = useWeatherStore((s) => s.togglePlaying);
  const setIsPlaying = useWeatherStore((s) => s.setIsPlaying);
  const nextFrame = useWeatherStore((s) => s.nextFrame);
  const playbackSpeed = useWeatherStore((s) => s.playbackSpeed);
  const activeLayer = useWeatherStore((s) => s.activeLayer);
  const intervalRef = useRef<ReturnType<typeof setInterval> | null>(null);

  useEffect(() => {
    if (isPlaying && frames.length > 0) {
      intervalRef.current = setInterval(() => nextFrame(), 1000 / playbackSpeed);
    }
    return () => {
      if (intervalRef.current) clearInterval(intervalRef.current);
    };
  }, [isPlaying, playbackSpeed, frames.length]);

  if (frames.length === 0) return null;

  const nowSec = Math.floor(Date.now() / 1000);
  const currentFrame = frames[currentFrameIndex];
  const offsetMin = currentFrame ? Math.round((currentFrame.time - nowSec) / 60) : 0;
  const layerTitle = LAYER_TITLE[activeLayer] ?? "Radar";
  const frameDate = new Date((currentFrame?.time ?? nowSec) * 1000);
  const dateLabel = frameDate.toLocaleDateString([], {
    weekday: "long",
    month: "long",
    day: "numeric",
    year: "numeric",
  });

  // Locate NOW & segment boundaries within the frame list
  const nowIdx = findClosestIdx(frames, nowSec);
  const nowcastEndIdx = findClosestIdx(frames, nowSec + NOWCAST_MIN * 60);
  const hrrrEndIdx = findClosestIdx(frames, nowSec + HRRR_MIN * 60);
  const last = Math.max(1, frames.length - 1);
  const nowPct = (nowIdx / last) * 100;
  const nowcastPct = (nowcastEndIdx / last) * 100;
  const hrrrPct = (hrrrEndIdx / last) * 100;
  const thumbPct = (currentFrameIndex / last) * 100;

  const mode = offsetMin === 0 ? "LIVE" : offsetMin < 0 ? "HISTORY" : offsetMin <= NOWCAST_MIN ? "NOWCAST" : offsetMin <= HRRR_MIN ? "HRRR" : "LONG-RANGE";
  const modeColor = offsetMin === 0 ? cumulus.ok : "#C7BDFF";
  const clock = formatClock(currentFrame?.time ?? nowSec);
  const offsetLabel = formatOffset(offsetMin);

  return (
    <View style={styles.container}>
      <View style={styles.card}>
        <View style={styles.row}>
          <TouchableOpacity style={styles.playBtn} onPress={togglePlaying} activeOpacity={0.8}>
            {isPlaying ? (
              <View style={styles.pauseIcon}>
                <View style={styles.pauseBar} />
                <View style={styles.pauseBar} />
              </View>
            ) : (
              <View style={styles.playIcon} />
            )}
          </TouchableOpacity>

          <View style={{ flex: 1, minWidth: 0 }}>
            {/* Header row: layer title + formatted date, Apple-Weather style */}
            <View style={styles.headerRow}>
              <View style={{ flex: 1, minWidth: 0 }}>
                <Text style={styles.layerTitle} numberOfLines={1}>
                  {layerTitle}
                </Text>
                <Text style={styles.dateLabel} numberOfLines={1}>
                  {dateLabel}
                </Text>
              </View>
              <View style={{ alignItems: "flex-end" }}>
                <Text style={styles.modeLabel}>{mode}</Text>
                <Text style={[styles.offsetLabel, { color: modeColor }]}>
                  {offsetLabel} {"\u00B7"} {clock}
                </Text>
              </View>
            </View>

            {/* Segmented track */}
            <View style={styles.trackContainer}>
              {/* past */}
              {nowPct > 0 && (
                <View
                  style={[
                    styles.segmentPast,
                    { left: 0, width: `${nowPct}%` },
                  ]}
                />
              )}
              {/* nowcast */}
              {nowcastPct > nowPct && (
                <View
                  style={[
                    styles.segmentNowcast,
                    { left: `${nowPct}%`, width: `${nowcastPct - nowPct}%` },
                  ]}
                />
              )}
              {/* HRRR */}
              {hrrrPct > nowcastPct && (
                <View
                  style={[
                    styles.segmentHrrr,
                    { left: `${nowcastPct}%`, width: `${hrrrPct - nowcastPct}%` },
                  ]}
                >
                  <DashedRow color="rgba(139,124,255,0.5)" />
                </View>
              )}
              {/* long-range */}
              {100 > hrrrPct && (
                <View
                  style={[
                    styles.segmentLongRange,
                    { left: `${hrrrPct}%`, width: `${100 - hrrrPct}%` },
                  ]}
                >
                  <DashedRow color="rgba(139,124,255,0.28)" />
                </View>
              )}
              {/* NOW marker */}
              {nowPct > 0 && nowPct < 100 && (
                <View style={[styles.nowMarker, { left: `${nowPct}%` }]} />
              )}
              {/* Slider overlay (thumb) */}
              <Slider
                style={styles.slider}
                minimumValue={0}
                maximumValue={last}
                step={1}
                value={currentFrameIndex}
                onValueChange={(v) => {
                  setIsPlaying(false);
                  setCurrentFrameIndex(Math.round(v));
                }}
                minimumTrackTintColor="transparent"
                maximumTrackTintColor="transparent"
                thumbTintColor="#ffffff"
              />
            </View>

            {/* Axis */}
            <View style={styles.axisRow}>
              <Text style={styles.axisTick}>-1h</Text>
              <Text style={[styles.axisTick, { color: cumulus.ok, fontWeight: "700" }]}>NOW</Text>
              <Text style={styles.axisTick}>+1h</Text>
              <Text style={styles.axisTick}>+6h</Text>
              <Text style={styles.axisTick}>+24h</Text>
            </View>
          </View>
        </View>
      </View>
    </View>
  );
}

function DashedRow({ color }: { color: string }) {
  // 12 dashes across the segment
  const dashes = Array.from({ length: 12 });
  return (
    <View style={styles.dashRow}>
      {dashes.map((_, i) => (
        <View key={i} style={{ flex: 1, height: "100%", backgroundColor: i % 2 === 0 ? color : "transparent" }} />
      ))}
    </View>
  );
}

function findClosestIdx(frames: { time: number }[], target: number): number {
  let best = 0;
  let bestDiff = Infinity;
  for (let i = 0; i < frames.length; i++) {
    const d = Math.abs(frames[i].time - target);
    if (d < bestDiff) {
      bestDiff = d;
      best = i;
    }
  }
  // Snap to end if target is past the last frame
  if (target > frames[frames.length - 1].time) return frames.length - 1;
  return best;
}

function formatOffset(min: number): string {
  if (min === 0) return "NOW";
  const abs = Math.abs(min);
  const sign = min > 0 ? "+" : "-";
  if (abs < 60) return `${sign}${abs}m`;
  const h = abs / 60;
  if (h < 24) return `${sign}${h % 1 === 0 ? h : h.toFixed(1)}h`;
  return `${sign}${(h / 24).toFixed(1)}d`;
}

function formatClock(sec: number): string {
  const d = new Date(sec * 1000);
  return d.toLocaleTimeString([], { hour: "numeric", minute: "2-digit" });
}

const styles = StyleSheet.create({
  container: {
    position: "absolute",
    left: 12,
    right: 12,
    bottom: 92,
    zIndex: 30,
  },
  card: {
    backgroundColor: "rgba(10,14,26,0.85)",
    borderWidth: 1,
    borderColor: "rgba(255,255,255,0.1)",
    borderRadius: 18,
    paddingVertical: 10,
    paddingHorizontal: 12,
  },
  row: { flexDirection: "row", alignItems: "center", gap: 10 },
  playBtn: {
    width: 34,
    height: 34,
    borderRadius: 10,
    backgroundColor: cumulus.accent,
    alignItems: "center",
    justifyContent: "center",
  },
  playIcon: {
    width: 0,
    height: 0,
    borderLeftWidth: 9,
    borderTopWidth: 6,
    borderBottomWidth: 6,
    borderLeftColor: "#fff",
    borderTopColor: "transparent",
    borderBottomColor: "transparent",
    marginLeft: 2,
  },
  pauseIcon: { flexDirection: "row", gap: 3 },
  pauseBar: { width: 3, height: 12, backgroundColor: "#fff", borderRadius: 1 },

  headerRow: {
    flexDirection: "row",
    alignItems: "flex-start",
    gap: 10,
    marginBottom: 6,
  },
  layerTitle: {
    color: cumulus.ink,
    fontSize: 15,
    fontWeight: "700",
    letterSpacing: 0.2,
  },
  dateLabel: {
    color: cumulus.inkDim,
    fontSize: 11,
    marginTop: 1,
  },
  modeLabel: {
    color: cumulus.inkMuted,
    fontSize: 9,
    letterSpacing: 1.2,
    fontWeight: "700",
  },
  offsetLabel: {
    fontSize: 12,
    fontWeight: "700",
    fontVariant: ["tabular-nums"],
  },

  trackContainer: {
    height: 22,
    justifyContent: "center",
  },
  segmentPast: {
    position: "absolute",
    top: 9,
    height: 4,
    backgroundColor: "rgba(255,255,255,0.22)",
    borderRadius: 2,
  },
  segmentNowcast: {
    position: "absolute",
    top: 9,
    height: 4,
    backgroundColor: "rgba(139,124,255,0.55)",
    borderRadius: 2,
  },
  segmentHrrr: {
    position: "absolute",
    top: 9,
    height: 4,
    borderRadius: 2,
    overflow: "hidden",
  },
  segmentLongRange: {
    position: "absolute",
    top: 9,
    height: 4,
    borderRadius: 2,
    overflow: "hidden",
  },
  dashRow: { flexDirection: "row", height: "100%" },
  nowMarker: {
    position: "absolute",
    top: 4,
    width: 2,
    height: 14,
    borderRadius: 1,
    backgroundColor: cumulus.ok,
    marginLeft: -1,
  },
  slider: {
    position: "absolute",
    left: -10,
    right: -10,
    top: -5,
    height: 30,
  },

  axisRow: {
    flexDirection: "row",
    justifyContent: "space-between",
    marginTop: 6,
  },
  axisTick: {
    color: cumulus.inkMuted,
    fontSize: 9,
    letterSpacing: 0.4,
    fontVariant: ["tabular-nums"],
  },
});
