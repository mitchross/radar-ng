import { useEffect, useRef } from "react";
import { TouchableOpacity, Text, StyleSheet } from "react-native";
import { useWeatherStore } from "../../stores/useWeatherStore";

export function PlayButton() {
  const isPlaying = useWeatherStore((s) => s.isPlaying);
  const togglePlaying = useWeatherStore((s) => s.togglePlaying);
  const nextFrame = useWeatherStore((s) => s.nextFrame);
  const playbackSpeed = useWeatherStore((s) => s.playbackSpeed);
  const frames = useWeatherStore((s) => s.frames);
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

  return (
    <TouchableOpacity
      style={styles.button}
      onPress={togglePlaying}
      activeOpacity={0.7}
    >
      <Text style={styles.icon}>{isPlaying ? "\u23F8" : "\u25B6\uFE0F"}</Text>
    </TouchableOpacity>
  );
}

const styles = StyleSheet.create({
  button: {
    width: 48,
    height: 48,
    borderRadius: 24,
    backgroundColor: "rgba(26, 26, 46, 0.9)",
    justifyContent: "center",
    alignItems: "center",
    marginLeft: 12,
  },
  icon: {
    fontSize: 20,
  },
});
