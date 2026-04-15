import { useEffect, useRef } from "react";
import { TouchableOpacity, View, StyleSheet } from "react-native";
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
      {isPlaying ? (
        <View style={styles.pauseIcon}>
          <View style={styles.pauseBar} />
          <View style={styles.pauseBar} />
        </View>
      ) : (
        <View style={styles.playIcon} />
      )}
    </TouchableOpacity>
  );
}

const styles = StyleSheet.create({
  button: {
    width: 48,
    height: 48,
    borderRadius: 24,
    backgroundColor: "#4fc3f7",
    justifyContent: "center",
    alignItems: "center",
    marginLeft: 12,
  },
  playIcon: {
    width: 0,
    height: 0,
    borderLeftWidth: 14,
    borderTopWidth: 9,
    borderBottomWidth: 9,
    borderLeftColor: "#fff",
    borderTopColor: "transparent",
    borderBottomColor: "transparent",
    marginLeft: 3,
  },
  pauseIcon: {
    flexDirection: "row",
    gap: 4,
  },
  pauseBar: {
    width: 5,
    height: 16,
    backgroundColor: "#fff",
    borderRadius: 1,
  },
});
