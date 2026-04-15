import { View, Text, StyleSheet } from "react-native";
import Slider from "@react-native-community/slider";
import { useWeatherStore } from "../../stores/useWeatherStore";

export function TimeSlider() {
  const frames = useWeatherStore((s) => s.frames);
  const currentFrameIndex = useWeatherStore((s) => s.currentFrameIndex);
  const setCurrentFrameIndex = useWeatherStore((s) => s.setCurrentFrameIndex);
  const setIsPlaying = useWeatherStore((s) => s.setIsPlaying);

  if (frames.length === 0) return null;

  const currentFrame = frames[currentFrameIndex];
  const timeLabel = currentFrame
    ? new Date(currentFrame.time * 1000).toLocaleTimeString([], {
        hour: "numeric",
        minute: "2-digit",
      })
    : "";

  const isLatest = currentFrameIndex === frames.length - 1;

  // Calculate how long ago this frame is
  const nowSec = Math.floor(Date.now() / 1000);
  const frameSec = currentFrame?.time ?? nowSec;
  const diffMin = Math.round((nowSec - frameSec) / 60);
  const agoLabel =
    diffMin <= 1 ? "" : diffMin < 60 ? `${diffMin}m ago` : `${Math.round(diffMin / 60)}h ago`;

  return (
    <View style={styles.container}>
      <View style={styles.labelRow}>
        <Text style={styles.timeText}>{timeLabel}</Text>
        {isLatest ? (
          <View style={styles.liveBadge}>
            <Text style={styles.liveText}>LIVE</Text>
          </View>
        ) : agoLabel ? (
          <Text style={styles.agoText}>{agoLabel}</Text>
        ) : null}
      </View>
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
        minimumTrackTintColor="#4fc3f7"
        maximumTrackTintColor="#444"
        thumbTintColor="#fff"
      />
      <View style={styles.frameCount}>
        <Text style={styles.frameText}>
          {currentFrameIndex + 1}/{frames.length}
        </Text>
      </View>
    </View>
  );
}

const styles = StyleSheet.create({
  container: {
    paddingHorizontal: 16,
    paddingBottom: 4,
  },
  labelRow: {
    flexDirection: "row",
    justifyContent: "center",
    alignItems: "center",
    gap: 8,
    paddingTop: 8,
  },
  timeText: {
    color: "#fff",
    fontSize: 16,
    fontWeight: "600",
    fontVariant: ["tabular-nums"],
  },
  liveBadge: {
    backgroundColor: "#4caf50",
    paddingHorizontal: 8,
    paddingVertical: 2,
    borderRadius: 4,
  },
  liveText: {
    color: "#fff",
    fontSize: 11,
    fontWeight: "700",
  },
  agoText: {
    color: "#888",
    fontSize: 12,
  },
  slider: {
    width: "100%",
    height: 36,
  },
  frameCount: {
    alignItems: "flex-end",
  },
  frameText: {
    color: "#555",
    fontSize: 10,
    fontVariant: ["tabular-nums"],
  },
});
