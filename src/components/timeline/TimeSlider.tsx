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

  return (
    <View style={styles.container}>
      <View style={styles.labelRow}>
        <Text style={styles.timeText}>{timeLabel}</Text>
        {isLatest && <Text style={styles.liveBadge}>LIVE</Text>}
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
        maximumTrackTintColor="#555"
        thumbTintColor="#4fc3f7"
      />
    </View>
  );
}

const styles = StyleSheet.create({
  container: {
    paddingHorizontal: 16,
    paddingBottom: 8,
    backgroundColor: "rgba(26, 26, 46, 0.9)",
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
    color: "#4caf50",
    fontSize: 12,
    fontWeight: "700",
    backgroundColor: "rgba(76, 175, 80, 0.2)",
    paddingHorizontal: 6,
    paddingVertical: 2,
    borderRadius: 4,
    overflow: "hidden",
  },
  slider: {
    width: "100%",
    height: 40,
  },
});
