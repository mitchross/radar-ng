/** Persistent "Historical replay" banner: case title plus Return to live. */
import { Pressable, StyleSheet, Text, View } from "react-native";
import { useQueryClient } from "@tanstack/react-query";
import { useWeatherStore } from "../../stores/useWeatherStore";
import { exitReplay } from "../../lib/replayControl";
import { useMapChromeInsets } from "../../hooks/useMapChromeInsets";
import { MAP_CHROME_MAX_FONT_SCALE } from "../../lib/constants";
import { useReplayManifestQuery, useReplayWarningsQuery } from "../../hooks/useReplay";

export function ReplayBanner() {
  const chrome = useMapChromeInsets();
  const queryClient = useQueryClient();
  const replayCase = useWeatherStore((s) => s.replayCase);
  const manifest = useReplayManifestQuery();
  const warnings = useReplayWarningsQuery();
  if (!replayCase) return null;
  // Fetch failures are shown, never replaced by live data.
  const error = manifest.error ?? warnings.error;

  return (
    <View style={[styles.wrap, { top: chrome.top, left: chrome.left + 58, right: chrome.right + 58 }]} testID="replay-banner">
      <View style={styles.row}>
        <View style={styles.text}>
          <Text maxFontSizeMultiplier={MAP_CHROME_MAX_FONT_SCALE} style={styles.eyebrow}>HISTORICAL REPLAY</Text>
          <Text maxFontSizeMultiplier={MAP_CHROME_MAX_FONT_SCALE} style={styles.title} numberOfLines={1}>{replayCase.title}</Text>
        </View>
        <Pressable
          onPress={() => void exitReplay(queryClient)}
          style={({ pressed }) => [styles.button, pressed ? { opacity: 0.7 } : null]}
          accessibilityRole="button"
          accessibilityLabel="Return to live radar"
        >
          <Text maxFontSizeMultiplier={MAP_CHROME_MAX_FONT_SCALE} style={styles.buttonText}>Return to live</Text>
        </Pressable>
      </View>
      {error ? (
        <Text maxFontSizeMultiplier={MAP_CHROME_MAX_FONT_SCALE} style={styles.error} accessibilityRole="alert">
          Could not load this replay: {error.message}
        </Text>
      ) : null}
    </View>
  );
}

const styles = StyleSheet.create({
  wrap: { position: "absolute", zIndex: 16, borderRadius: 16, backgroundColor: "rgba(120,24,24,0.94)", paddingHorizontal: 12, paddingVertical: 4 },
  row: { flexDirection: "row", alignItems: "center", gap: 8, minHeight: 40 },
  text: { flex: 1, minWidth: 0 },
  eyebrow: { color: "#ffd9d9", fontSize: 9.5, fontWeight: "800", letterSpacing: 1.2 },
  title: { color: "#ffffff", fontSize: 13, fontWeight: "700" },
  button: { minHeight: 44, minWidth: 44, justifyContent: "center", paddingHorizontal: 10, borderRadius: 12, backgroundColor: "rgba(255,255,255,0.18)" },
  buttonText: { color: "#ffffff", fontSize: 12, fontWeight: "800" },
  error: { color: "#ffe0e0", fontSize: 11, paddingBottom: 6 },
});
