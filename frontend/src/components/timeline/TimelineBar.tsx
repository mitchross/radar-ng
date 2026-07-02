/**
 * Cumulus radar timeline — Apple-Weather-inspired "forecast pill".
 * Violet play button + layer/date header + 1h/12h segmented zoom + segmented
 * track (past / nowcast / HRRR / long-range) + NOW marker + draggable thumb.
 *
 * Playback itself lives in hooks/usePlayback.ts (called from the radar
 * screen). Per-tick state (`currentFrameIndex`) is only subscribed to by
 * two leaf components — FrameDateLabel and FrameSlider — so the rest of
 * this card doesn't re-render 2×/second during playback.
 */
import { View, Text, TouchableOpacity, StyleSheet } from "react-native";
import Slider from "@react-native-community/slider";
import { useEffect, useMemo, useState } from "react";
import { useWeatherStore } from "../../stores/useWeatherStore";
import { cumulus } from "../../lib/cumulusTheme";
import type { LayerType } from "../../types/weather";

const NOWCAST_MIN = 60;
const HRRR_MIN = 360;

const LAYER_TITLE: Record<LayerType, string> = {
  radar: "Radar",
  "radar-composite": "Radar",
  "radar-hrrr": "Radar",
  temperature: "Temperature",
  wind: "Wind",
  "precip-type": "Radar",
  "precip-accum": "Rain Total",
  cloud: "Clouds",
  cape: "Storm Energy",
};

type Zoom = "1h" | "12h";

export function TimelineBar() {
  const frames = useWeatherStore((s) => s.frames);
  const setCurrentFrameIndex = useWeatherStore((s) => s.setCurrentFrameIndex);
  const isPlaying = useWeatherStore((s) => s.isPlaying);
  const togglePlaying = useWeatherStore((s) => s.togglePlaying);
  const activeLayer = useWeatherStore((s) => s.activeLayer);
  const setPlaybackWindow = useWeatherStore((s) => s.setPlaybackWindow);

  const [zoom, setZoom] = useState<Zoom>("1h");

  const nowSec = useMemo(() => Math.floor(Date.now() / 1000), []);

  // Zoom window indices
  const { startIdx, endIdx } = useMemo(() => {
    if (frames.length === 0) return { startIdx: 0, endIdx: 0 };
    if (zoom === "12h") return { startIdx: 0, endIdx: frames.length - 1 };
    let s = 0, e = frames.length - 1;
    const lo = nowSec - 60 * 60;
    const hi = nowSec + 60 * 60;
    while (s < frames.length - 1 && frames[s].time < lo) s++;
    while (e > 0 && frames[e].time > hi) e--;
    if (e < s) e = s;
    return { startIdx: s, endIdx: e };
  }, [frames, zoom, nowSec]);

  // Publish the zoom window so the radar carousel prefetches the frames
  // playback will actually visit (including the loop wrap). Cleared on
  // unmount so a stale window never constrains another screen.
  useEffect(() => {
    setPlaybackWindow({ start: startIdx, end: endIdx });
  }, [startIdx, endIdx, setPlaybackWindow]);
  useEffect(() => () => setPlaybackWindow(null), [setPlaybackWindow]);

  // Snap current frame into zoom window when switching. Reads the index via
  // getState() so this component doesn't re-render on every playback tick.
  useEffect(() => {
    if (frames.length === 0) return;
    const currentFrameIndex = useWeatherStore.getState().currentFrameIndex;
    if (currentFrameIndex < startIdx || currentFrameIndex > endIdx) {
      const nowIdx = findClosestIdx(frames, nowSec);
      setCurrentFrameIndex(Math.max(startIdx, Math.min(endIdx, nowIdx)));
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [zoom, startIdx, endIdx, frames.length]);

  // Loading shell instead of nothing — prevents the whole card popping into
  // existence once the manifest arrives.
  if (frames.length === 0) {
    return (
      <View style={styles.container}>
        <View style={styles.card}>
          <View style={styles.headerRow}>
            <View style={[styles.playBtn, styles.playBtnDisabled]}>
              <View style={styles.playIcon} />
            </View>
            <View style={{ flex: 1, minWidth: 0 }}>
              <Text style={styles.layerTitle} numberOfLines={1}>
                {LAYER_TITLE[activeLayer] ?? "Radar"}
              </Text>
              <Text style={styles.dateLabel} numberOfLines={1}>Loading frames…</Text>
            </View>
          </View>
          <View style={styles.trackContainer} />
        </View>
      </View>
    );
  }

  const layerTitle = LAYER_TITLE[activeLayer] ?? "Radar";
  const winLen = Math.max(1, endIdx - startIdx);
  const frameToPct = (i: number) => ((i - startIdx) / winLen) * 100;
  const nowIdx = findClosestIdx(frames, nowSec);
  const nowcastEndIdx = findClosestIdx(frames, nowSec + NOWCAST_MIN * 60);
  const hrrrEndIdx = findClosestIdx(frames, nowSec + HRRR_MIN * 60);
  const nowPct = clampPct(frameToPct(nowIdx));
  const nowcastPct = clampPct(frameToPct(nowcastEndIdx));
  const hrrrPct = clampPct(frameToPct(hrrrEndIdx));

  return (
    <View style={styles.container}>
      <View style={styles.card}>
        <View style={styles.headerRow}>
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

          <FrameDateLabel layerTitle={layerTitle} nowSec={nowSec} />

          <View style={styles.segmented}>
            {(["1h", "12h"] as const).map((z) => (
              <TouchableOpacity
                key={z}
                onPress={() => setZoom(z)}
                style={[styles.seg, zoom === z && styles.segActive]}
                activeOpacity={0.8}
              >
                <Text style={[styles.segText, zoom === z && styles.segTextActive]}>{z}</Text>
              </TouchableOpacity>
            ))}
          </View>
        </View>

        <View style={styles.trackContainer}>
          {/* past */}
          {nowPct > 0 && (
            <View style={[styles.segmentPast, { left: 0, width: `${nowPct}%` }]} />
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
          <FrameSlider startIdx={startIdx} endIdx={endIdx} />
        </View>

        <View style={styles.axisRow}>
          {(zoom === "1h"
            ? ["-60", "-30", "Now", "+30", "+60"]
            : ["-1h", "Now", "+3h", "+12h", "+24h"]
          ).map((label, i) => (
            <Text
              key={i}
              style={[
                styles.axisTick,
                label === "Now" && { color: cumulus.ok, fontWeight: "700" },
              ]}
            >
              {label}
            </Text>
          ))}
        </View>
      </View>
    </View>
  );
}

/**
 * Per-tick leaves — the ONLY components that subscribe to
 * `currentFrameIndex`, so a playback tick re-renders two small text nodes
 * and a slider thumb instead of the whole timeline card.
 */
function FrameDateLabel({ layerTitle, nowSec }: { layerTitle: string; nowSec: number }) {
  const frames = useWeatherStore((s) => s.frames);
  const currentFrameIndex = useWeatherStore((s) => s.currentFrameIndex);

  const currentFrame = frames[currentFrameIndex];
  const offsetMin = currentFrame ? Math.round((currentFrame.time - nowSec) / 60) : 0;
  const frameDate = new Date((currentFrame?.time ?? nowSec) * 1000);
  const dateLabel = frameDate.toLocaleDateString([], {
    weekday: "long",
    hour: "numeric",
    minute: "2-digit",
  });
  const mode = offsetMin === 0 ? "Now" : offsetMin > 0 ? "Forecast" : "Past";

  return (
    <View style={{ flex: 1, minWidth: 0 }}>
      <Text style={styles.layerTitle} numberOfLines={1}>
        {layerTitle} · {mode}
      </Text>
      <Text style={styles.dateLabel} numberOfLines={1}>{dateLabel}</Text>
    </View>
  );
}

function FrameSlider({ startIdx, endIdx }: { startIdx: number; endIdx: number }) {
  const currentFrameIndex = useWeatherStore((s) => s.currentFrameIndex);
  const setCurrentFrameIndex = useWeatherStore((s) => s.setCurrentFrameIndex);
  const setIsPlaying = useWeatherStore((s) => s.setIsPlaying);

  return (
    <Slider
      style={styles.slider}
      minimumValue={startIdx}
      maximumValue={endIdx}
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
  );
}

function DashedRow({ color }: { color: string }) {
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
    if (d < bestDiff) { bestDiff = d; best = i; }
  }
  if (target > frames[frames.length - 1].time) return frames.length - 1;
  return best;
}

function clampPct(p: number) { return Math.max(0, Math.min(100, p)); }

const styles = StyleSheet.create({
  container: { position: "absolute", left: 12, right: 12, bottom: 44, zIndex: 30 },
  card: {
    backgroundColor: "rgba(255,255,255,0.88)",
    borderWidth: StyleSheet.hairlineWidth,
    borderColor: "rgba(255,255,255,0.95)",
    borderRadius: 28,
    paddingVertical: 10,
    paddingHorizontal: 14,
    shadowColor: "#14234f",
    shadowOffset: { width: 0, height: 12 },
    shadowOpacity: 0.18,
    shadowRadius: 24,
    elevation: 12,
  },
  headerRow: { flexDirection: "row", alignItems: "center", gap: 10 },
  playBtn: {
    width: 30,
    height: 30,
    borderRadius: 15,
    backgroundColor: "rgba(11,18,32,0.08)",
    alignItems: "center",
    justifyContent: "center",
  },
  playIcon: {
    width: 0, height: 0,
    borderLeftWidth: 8, borderTopWidth: 5, borderBottomWidth: 5,
    borderLeftColor: "#0b1220",
    borderTopColor: "transparent",
    borderBottomColor: "transparent",
    marginLeft: 2,
  },
  playBtnDisabled: { opacity: 0.4 },
  pauseIcon: { flexDirection: "row", gap: 3 },
  pauseBar: { width: 3, height: 11, backgroundColor: "#0b1220", borderRadius: 1 },

  layerTitle: { color: "#0b1220", fontSize: 14, fontWeight: "700", letterSpacing: -0.2 },
  dateLabel: { color: "rgba(11,18,32,0.58)", fontSize: 12, marginTop: 1 },

  segmented: {
    flexDirection: "row",
    backgroundColor: "rgba(11,18,32,0.08)",
    borderRadius: 14,
    padding: 2,
    height: 28,
  },
  seg: {
    paddingHorizontal: 12,
    height: 24,
    justifyContent: "center",
    borderRadius: 12,
  },
  segActive: {
    backgroundColor: "#fff",
    shadowColor: "#000",
    shadowOffset: { width: 0, height: 1 },
    shadowOpacity: 0.14,
    shadowRadius: 3,
    elevation: 2,
  },
  segText: { color: "rgba(11,18,32,0.55)", fontSize: 13, fontWeight: "600" },
  segTextActive: { color: "#0b1220", fontWeight: "700" },

  trackContainer: { height: 22, marginTop: 10, justifyContent: "center" },
  segmentPast: {
    position: "absolute", top: 9, height: 3,
    backgroundColor: "rgba(11,18,32,0.55)", borderRadius: 2,
  },
  segmentNowcast: {
    position: "absolute", top: 9, height: 3,
    backgroundColor: cumulus.accent, borderRadius: 2,
  },
  segmentHrrr: { position: "absolute", top: 9, height: 3, borderRadius: 2, overflow: "hidden" },
  segmentLongRange: { position: "absolute", top: 9, height: 3, borderRadius: 2, overflow: "hidden" },
  dashRow: { flexDirection: "row", height: "100%" },
  nowMarker: {
    position: "absolute", top: 4, width: 2, height: 14,
    borderRadius: 1, backgroundColor: "#0b1220", marginLeft: -1,
  },
  slider: { position: "absolute", left: -10, right: -10, top: -5, height: 30 },

  axisRow: { flexDirection: "row", justifyContent: "space-between", marginTop: 6 },
  axisTick: { color: "rgba(11,18,32,0.55)", fontSize: 10.5, fontWeight: "500", fontVariant: ["tabular-nums"] },
});
