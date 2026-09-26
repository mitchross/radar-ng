/**
 * Cumulus radar timeline — Apple-Weather-inspired "forecast pill".
 * Violet play button + layer/date header + 1h/48h segmented zoom + segmented
 * track (past / nowcast / HRRR / long-range) + NOW marker + draggable thumb.
 * Playback follows the configured speed within the active zoom window.
 */
import { View, Text, Pressable, StyleSheet } from "react-native";
import Slider from "@react-native-community/slider";
import { SymbolView } from "expo-symbols";
import { useEffect, useMemo, useRef, useState } from "react";
import { useWeatherStore } from "../../stores/useWeatherStore";
import { cumulus } from "../../lib/cumulusTheme";
import { findClosestIdx } from "../../lib/frameIndex";
import { offsetLabel, playbackSequence, positionOf, type TimelineZoom } from "../../lib/playbackSequence";
import { createThrottle } from "../../lib/throttle";
import { useAppActive } from "../../hooks/useAppActive";
import { useNow } from "../../hooks/useNow";
import { useMapChromeInsets } from "../../hooks/useMapChromeInsets";
import { useIsFocused } from "expo-router";
import type { LayerType } from "../../types/weather";
import { MAP_CHROME_MAX_FONT_SCALE } from "../../lib/constants";
import { MapChromeSurface } from "../ui/MapChromeSurface";

const NOWCAST_MIN = 60;
const NOW_REFRESH_MS = 60_000;
// Each committed index remounts a raster source, so a drag commits at most this often.
const SCRUB_COMMIT_MS = 100;
// Built once: constructing a formatter per render showed up on every playback tick.
const FRAME_DATE_FORMAT = new Intl.DateTimeFormat(undefined, {
  weekday: "long",
  hour: "numeric",
  minute: "2-digit",
});

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
  "air-quality": "Air Quality",
  ozone: "Ozone",
};

type Zoom = TimelineZoom;
// Axis ticks at these fractions of the sequence, labelled with the frame's real offset.
const AXIS_FRACTIONS = [0, 0.25, 0.5, 0.75, 1];

export function TimelineBar() {
  const chrome = useMapChromeInsets();
  const frames = useWeatherStore((s) => s.frames);
  const currentFrameIndex = useWeatherStore((s) => s.currentFrameIndex);
  const setCurrentFrameIndex = useWeatherStore((s) => s.setCurrentFrameIndex);
  const isPlaying = useWeatherStore((s) => s.isPlaying);
  const playbackSpeed = useWeatherStore((s) => s.playbackSpeed);
  const togglePlaying = useWeatherStore((s) => s.togglePlaying);
  const setIsPlaying = useWeatherStore((s) => s.setIsPlaying);
  const activeLayer = useWeatherStore((s) => s.activeLayer);
  const setPlaybackWindow = useWeatherStore((s) => s.setPlaybackWindow);

  const [zoom, setZoom] = useState<Zoom>("1h");
  const [scrub] = useState(() => createThrottle(setCurrentFrameIndex, SCRUB_COMMIT_MS));
  useEffect(() => () => scrub.cancel(), [scrub]);

  // Frozen-at-mount "now" drifted the 1h window and NOW marker into the past after ~30 min on the tab.
  const nowSec = useNowSec();
  const appActive = useAppActive();
  const focused = useIsFocused();

  // The frames playback visits, in order. 1h is every frame within an hour of
  // now; 48h thins the dense past so the loop is mostly forecast. The slider
  // and the track both run over positions in this sequence, not frame indices.
  const sequence = useMemo(() => playbackSequence(frames, zoom, nowSec), [frames, zoom, nowSec]);
  const lastPos = Math.max(0, sequence.length - 1);
  const position = positionOf(sequence, frames, currentFrameIndex);
  const posRef = useRef(position);
  useEffect(() => { posRef.current = position; }, [position]);
  const inSequence = sequence.includes(currentFrameIndex);

  // Publish the sequence so the raster carousel prefetches the frames playback
  // will visit (incl. the loop wrap). Cleared on unmount so it can't go stale.
  useEffect(() => {
    if (sequence.length === 0) return;
    setPlaybackWindow({ start: sequence[0], end: sequence[lastPos], sequence });
  }, [sequence, lastPos, setPlaybackWindow]);
  useEffect(() => () => setPlaybackWindow(null), [setPlaybackWindow]);

  // Snap the current frame onto the sequence when switching zoom.
  useEffect(() => {
    if (sequence.length === 0 || inSequence) return;
    const nowPos = positionOf(sequence, frames, findClosestIdx(frames, nowSec));
    setCurrentFrameIndex(sequence[nowPos]);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [zoom, sequence]);

  // Playback tick along the sequence; paused while backgrounded (each tick remounts a RasterSource).
  useEffect(() => {
    if (!appActive || !focused || !isPlaying || sequence.length < 2) return;
    const id = setInterval(() => {
      const next = posRef.current >= sequence.length - 1 ? 0 : posRef.current + 1;
      posRef.current = next;
      setCurrentFrameIndex(sequence[next]);
    }, 1000 / playbackSpeed);
    return () => clearInterval(id);
  }, [appActive, focused, isPlaying, playbackSpeed, sequence, setCurrentFrameIndex]);

  // Segment boundaries and axis labels don't depend on the playback index —
  // memoize so the playback tick doesn't rescan frames. Must stay above the
  // early return below (rules-of-hooks).
  const { nowPct, nowcastPct, axisLabels } = useMemo(() => {
    const toPct = (p: number) => (lastPos === 0 ? 0 : (p / lastPos) * 100);
    // Last position at or before a time; -1 when the sequence starts after it.
    const lastPosAtOrBefore = (sec: number) => {
      let p = -1;
      while (p + 1 < sequence.length && frames[sequence[p + 1]].time <= sec) p++;
      return p;
    };
    const nowP = lastPosAtOrBefore(nowSec);
    const nowcastP = lastPosAtOrBefore(nowSec + NOWCAST_MIN * 60);
    const labels = sequence.length === 0
      ? []
      : AXIS_FRACTIONS.map((f) => offsetLabel(frames[sequence[Math.round(f * lastPos)]].time, nowSec));
    return {
      nowPct: nowP < 0 ? 0 : clampPct(toPct(nowP)),
      nowcastPct: nowcastP < 0 ? 0 : clampPct(toPct(nowcastP)),
      axisLabels: labels,
    };
  }, [frames, sequence, lastPos, nowSec]);

  if (frames.length === 0) return null;

  const currentFrame = frames[currentFrameIndex];
  const offsetMin = currentFrame ? Math.round((currentFrame.time - nowSec) / 60) : 0;
  const layerTitle = LAYER_TITLE[activeLayer] ?? "Radar";
  const frameDate = new Date((currentFrame?.time ?? nowSec) * 1000);
  const dateLabel = FRAME_DATE_FORMAT.format(frameDate);

  // MRMS lands a few minutes late; the latest scan within 5 min reads as "Now".
  const mode = Math.abs(offsetMin) <= 5 ? "Now" : `${offsetMin > 0 ? "Forecast" : "Past"} ${offsetLabel(currentFrame?.time ?? nowSec, nowSec)}`;

  return (
    <View style={[styles.container, { left: chrome.left, right: chrome.right, bottom: chrome.bottom }]}>
      <MapChromeSurface style={styles.card} fallbackStyle={styles.cardFill} colorScheme="light">
        <View style={styles.headerRow}>
          <Pressable
            style={({ pressed }) => [styles.playBtn, pressed ? styles.controlPressed : null]}
            onPress={togglePlaying}
            accessibilityRole="button"
            accessibilityLabel={isPlaying ? "Pause radar animation" : "Play radar animation"}
            accessibilityState={{ selected: isPlaying }}
          >
            <SymbolView
              name={isPlaying ? { ios: "pause.fill", android: "pause" } : { ios: "play.fill", android: "play_arrow" }}
              size={18}
              tintColor="#0b1220"
            />
          </Pressable>

          <View style={{ flex: 1, minWidth: 0 }}>
            <Text maxFontSizeMultiplier={MAP_CHROME_MAX_FONT_SCALE} style={styles.layerTitle} numberOfLines={1}>
              {layerTitle} · {mode}
            </Text>
            <Text maxFontSizeMultiplier={MAP_CHROME_MAX_FONT_SCALE} style={styles.dateLabel} numberOfLines={1}>{dateLabel}</Text>
          </View>

          <View style={styles.segmented}>
            {(["1h", "48h"] as const).map((z) => (
              <Pressable
                key={z}
                onPress={() => setZoom(z)}
                style={({ pressed }) => [
                  styles.seg,
                  zoom === z ? styles.segActive : null,
                  pressed ? styles.controlPressed : null,
                ]}
                accessibilityRole="radio"
                accessibilityLabel={`${z} radar timeline`}
                accessibilityState={{ checked: zoom === z }}
              >
                <Text maxFontSizeMultiplier={MAP_CHROME_MAX_FONT_SCALE} style={[styles.segText, zoom === z ? styles.segTextActive : null]}>{z}</Text>
              </Pressable>
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
          {/* model forecast */}
          {100 > nowcastPct && (
            <View
              style={[
                styles.segmentHrrr,
                { left: `${nowcastPct}%`, width: `${100 - nowcastPct}%` },
              ]}
            >
              <DashedRow color="rgba(139,124,255,0.5)" />
            </View>
          )}
          {/* NOW marker */}
          {nowPct > 0 && nowPct < 100 && (
            <View style={[styles.nowMarker, { left: `${nowPct}%` }]} />
          )}
          <Slider
            style={styles.slider}
            minimumValue={0}
            maximumValue={Math.max(1, lastPos)}
            step={1}
            value={position}
            disabled={sequence.length < 2}
            onSlidingStart={() => setIsPlaying(false)}
            onValueChange={(v) => {
              setIsPlaying(false);
              scrub.call(sequence[Math.round(v)] ?? currentFrameIndex);
            }}
            onSlidingComplete={(v) => scrub.flush(sequence[Math.round(v)] ?? currentFrameIndex)}
            minimumTrackTintColor="transparent"
            maximumTrackTintColor="transparent"
            thumbTintColor="#ffffff"
            accessibilityLabel={`${layerTitle} timeline, ${dateLabel}`}
          />
        </View>

        <View style={styles.axisRow}>
          {axisLabels.map((label, i) => (
            <Text maxFontSizeMultiplier={MAP_CHROME_MAX_FONT_SCALE}
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
      </MapChromeSurface>
    </View>
  );
}

/** Epoch seconds, refreshed once a minute while mounted. */
function useNowSec(): number {
  return Math.floor(useNow(NOW_REFRESH_MS) / 1000);
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

function clampPct(p: number) { return Math.max(0, Math.min(100, p)); }

const styles = StyleSheet.create({
  container: { position: "absolute", left: 12, right: 12, bottom: 44, zIndex: 30 },
  card: {
    borderRadius: 28,
    paddingVertical: 10,
    paddingHorizontal: 14,
  },
  cardFill: {
    backgroundColor: "rgba(255,255,255,0.88)",
    borderWidth: StyleSheet.hairlineWidth,
    borderColor: "rgba(255,255,255,0.95)",
    shadowColor: "#14234f",
    shadowOffset: { width: 0, height: 12 },
    shadowOpacity: 0.18,
    shadowRadius: 24,
    elevation: 12,
  },
  headerRow: { flexDirection: "row", alignItems: "center", gap: 10 },
  playBtn: {
    minWidth: 44,
    minHeight: 44,
    borderRadius: 22,
    backgroundColor: "rgba(11,18,32,0.08)",
    alignItems: "center",
    justifyContent: "center",
  },

  layerTitle: { color: "#0b1220", fontSize: 14, fontWeight: "700", letterSpacing: -0.2 },
  dateLabel: { color: "rgba(11,18,32,0.58)", fontSize: 12, marginTop: 1 },

  segmented: {
    flexDirection: "row",
    backgroundColor: "rgba(11,18,32,0.08)",
    borderRadius: 24,
    padding: 2,
    minHeight: 44,
  },
  seg: {
    paddingHorizontal: 12,
    minHeight: 44,
    justifyContent: "center",
    borderRadius: 22,
  },
  controlPressed: { opacity: 0.68 },
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
  dashRow: { flexDirection: "row", height: "100%" },
  nowMarker: {
    position: "absolute", top: 4, width: 2, height: 14,
    borderRadius: 1, backgroundColor: "#0b1220", marginLeft: -1,
  },
  slider: { position: "absolute", left: -10, right: -10, top: -5, height: 30 },

  axisRow: { flexDirection: "row", justifyContent: "space-between", marginTop: 6 },
  axisTick: { color: "rgba(11,18,32,0.55)", fontSize: 10.5, fontWeight: "500", fontVariant: ["tabular-nums"] },
});
