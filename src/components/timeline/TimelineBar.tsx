/**
 * CARROT-style timeline bar for radar scrubbing.
 * Clean design with play button, timestamp labels, LIVE badge.
 */
import { View, Text, TouchableOpacity, StyleSheet } from "react-native";
import Slider from "@react-native-community/slider";
import { useEffect, useRef } from "react";
import { useWeatherStore } from "../../stores/useWeatherStore";

export function TimelineBar() {
  const frames = useWeatherStore((s) => s.frames);
  const currentFrameIndex = useWeatherStore((s) => s.currentFrameIndex);
  const setCurrentFrameIndex = useWeatherStore((s) => s.setCurrentFrameIndex);
  const isPlaying = useWeatherStore((s) => s.isPlaying);
  const togglePlaying = useWeatherStore((s) => s.togglePlaying);
  const setIsPlaying = useWeatherStore((s) => s.setIsPlaying);
  const nextFrame = useWeatherStore((s) => s.nextFrame);
  const playbackSpeed = useWeatherStore((s) => s.playbackSpeed);
  const intervalRef = useRef<ReturnType<typeof setInterval> | null>(null);

  useEffect(() => {
    if (isPlaying && frames.length > 0) {
      intervalRef.current = setInterval(() => {
        nextFrame();
      }, 1000 / playbackSpeed);
    }
    return () => {
      if (intervalRef.current) clearInterval(intervalRef.current);
    };
  }, [isPlaying, playbackSpeed, frames.length]);

  if (frames.length === 0) return null;

  const currentFrame = frames[currentFrameIndex];
  const currentTime = currentFrame
    ? new Date(currentFrame.time * 1000).toLocaleTimeString([], {
        hour: "numeric",
        minute: "2-digit",
      })
    : "";

  const isLatest = currentFrameIndex === frames.length - 1;

  // Build timestamp ticks — show first, middle, and last frame times
  const tickIndices = [0, Math.floor(frames.length / 2), frames.length - 1];
  const tickLabels = tickIndices.map((idx) => {
    const f = frames[idx];
    if (!f) return "";
    return new Date(f.time * 1000).toLocaleTimeString([], {
      hour: "numeric",
      minute: "2-digit",
    });
  });

  return (
    <View style={styles.container}>
      {/* Current time display */}
      <View style={styles.timeRow}>
        <Text style={styles.currentTime}>{currentTime}</Text>
        {isLatest && (
          <View style={styles.liveBadge}>
            <Text style={styles.liveText}>LIVE</Text>
          </View>
        )}
      </View>

      {/* Play button + slider */}
      <View style={styles.controlsRow}>
        <TouchableOpacity
          style={styles.playButton}
          onPress={togglePlaying}
          activeOpacity={0.7}
        >
          {isPlaying ? (
            <View style={styles.pauseIcon}>
              <View style={styles.pauseBar} />
              <View style={styles.pauseBar} />
            </View>
          ) : (
            <View style={styles.playIcon} />
          )}
        </TouchableOpacity>

        <View style={styles.sliderContainer}>
          <Slider
            style={styles.slider}
            minimumValue={0}
            maximumValue={frames.length - 1}
            step={1}
            value={currentFrameIndex}
            onValueChange={(value) => {
              setIsPlaying(false);
              setCurrentFrameIndex(Math.round(value));
            }}
            minimumTrackTintColor="#2196F3"
            maximumTrackTintColor="rgba(0,0,0,0.15)"
            thumbTintColor="#2196F3"
          />
          {/* Timestamp ticks below slider */}
          <View style={styles.tickRow}>
            {tickLabels.map((label, i) => (
              <Text
                key={i}
                style={[
                  styles.tickLabel,
                  i === 1 && styles.tickLabelCenter,
                  i === 2 && styles.tickLabelRight,
                ]}
              >
                {label}
              </Text>
            ))}
          </View>
        </View>
      </View>
    </View>
  );
}

const styles = StyleSheet.create({
  container: {
    position: "absolute",
    bottom: 62,
    left: 0,
    right: 0,
    backgroundColor: "rgba(255,255,255,0.95)",
    borderTopWidth: StyleSheet.hairlineWidth,
    borderTopColor: "rgba(0,0,0,0.1)",
    paddingTop: 8,
    paddingBottom: 6,
    paddingHorizontal: 12,
  },
  timeRow: {
    flexDirection: "row",
    alignItems: "center",
    gap: 8,
    marginBottom: 2,
    paddingHorizontal: 4,
  },
  currentTime: {
    fontSize: 15,
    fontWeight: "700",
    color: "#1a1a1a",
    fontVariant: ["tabular-nums"],
  },
  liveBadge: {
    backgroundColor: "#4CAF50",
    paddingHorizontal: 6,
    paddingVertical: 2,
    borderRadius: 4,
  },
  liveText: {
    color: "#fff",
    fontSize: 9,
    fontWeight: "800",
    letterSpacing: 0.8,
  },
  controlsRow: {
    flexDirection: "row",
    alignItems: "center",
    gap: 4,
  },
  playButton: {
    width: 38,
    height: 38,
    borderRadius: 19,
    backgroundColor: "#2196F3",
    justifyContent: "center",
    alignItems: "center",
    shadowColor: "#2196F3",
    shadowOffset: { width: 0, height: 2 },
    shadowOpacity: 0.3,
    shadowRadius: 4,
    elevation: 3,
  },
  playIcon: {
    width: 0,
    height: 0,
    borderLeftWidth: 11,
    borderTopWidth: 7,
    borderBottomWidth: 7,
    borderLeftColor: "#fff",
    borderTopColor: "transparent",
    borderBottomColor: "transparent",
    marginLeft: 3,
  },
  pauseIcon: {
    flexDirection: "row",
    gap: 3,
  },
  pauseBar: {
    width: 3.5,
    height: 14,
    backgroundColor: "#fff",
    borderRadius: 1,
  },
  sliderContainer: {
    flex: 1,
  },
  slider: {
    width: "100%",
    height: 32,
  },
  tickRow: {
    flexDirection: "row",
    paddingHorizontal: 14,
    marginTop: -4,
  },
  tickLabel: {
    fontSize: 10,
    color: "#999",
    fontVariant: ["tabular-nums"],
  },
  tickLabelCenter: {
    flex: 1,
    textAlign: "center",
  },
  tickLabelRight: {
    textAlign: "right",
  },
});
