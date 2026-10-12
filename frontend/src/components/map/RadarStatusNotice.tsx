import { ActivityIndicator, Pressable, StyleSheet, Text, View } from "react-native";
import { MAP_CHROME_MAX_FONT_SCALE } from "../../lib/constants";
import type { RadarStatus } from "../../lib/radarStatus";
import { MapChromeSurface } from "../ui/MapChromeSurface";

interface Props {
  status: RadarStatus;
  /** True when the timeline has frames to show under the notice. */
  hasFrames: boolean;
  onRetry: () => void;
}

/** What to say when the radar is loading, offline or unavailable; null for live and stale. */
export function radarNoticeCopy(status: RadarStatus, hasFrames: boolean): { title: string; message: string; retry: boolean } | null {
  if (status.tone === "unavailable") {
    if (status.label === "LOADING" && !hasFrames) {
      return { title: "Loading radar", message: "Fetching the latest frames from your server.", retry: false };
    }
    return { title: "Radar unavailable", message: "The radar server is unreachable. Check the connection and try again.", retry: true };
  }
  if (status.tone === "offline") {
    return { title: "Radar offline", message: "Showing the last saved observation until the server answers.", retry: true };
  }
  return null;
}

/** A card over the map for the states the LIVE chip alone cannot explain. */
export function RadarStatusNotice({ status, hasFrames, onRetry, top }: Props & { top: number }) {
  const copy = radarNoticeCopy(status, hasFrames);
  if (!copy) return null;
  const loading = !copy.retry;
  return (
    <View style={[styles.wrap, { top }]} pointerEvents="box-none">
      <MapChromeSurface style={styles.card} fallbackStyle={styles.cardFill} colorScheme="light" interactive>
        <View
          accessible
          accessibilityRole={loading ? "progressbar" : "alert"}
          accessibilityLabel={`${copy.title}. ${copy.message}`}
          style={styles.row}
        >
          {loading ? <ActivityIndicator size="small" color="#0b1220" /> : <View style={styles.mark} />}
          <View style={styles.copy}>
            <Text maxFontSizeMultiplier={MAP_CHROME_MAX_FONT_SCALE} style={styles.title}>{copy.title}</Text>
            <Text maxFontSizeMultiplier={MAP_CHROME_MAX_FONT_SCALE} style={styles.message}>{copy.message}</Text>
          </View>
        </View>
        {copy.retry ? (
          <Pressable
            onPress={onRetry}
            accessibilityRole="button"
            accessibilityLabel="Retry loading radar"
            style={({ pressed }) => [styles.retry, pressed ? { opacity: 0.6 } : null]}
          >
            <Text maxFontSizeMultiplier={MAP_CHROME_MAX_FONT_SCALE} style={styles.retryText}>Retry</Text>
          </Pressable>
        ) : null}
      </MapChromeSurface>
    </View>
  );
}

const styles = StyleSheet.create({
  wrap: { position: "absolute", left: 12, right: 12, zIndex: 20, alignItems: "center" },
  card: { borderRadius: 18, paddingVertical: 12, paddingHorizontal: 14, maxWidth: 420, width: "100%", gap: 10 },
  cardFill: {
    backgroundColor: "rgba(255,255,255,0.92)",
    borderWidth: StyleSheet.hairlineWidth,
    borderColor: "rgba(10,20,40,0.12)",
    shadowColor: "#000",
    shadowOpacity: 0.14,
    shadowRadius: 8,
    shadowOffset: { width: 0, height: 2 },
    elevation: 4,
  },
  row: { flexDirection: "row", alignItems: "center", gap: 12 },
  mark: { width: 8, height: 8, borderRadius: 4, backgroundColor: "#df6a6a" },
  copy: { flex: 1, minWidth: 0 },
  title: { color: "#0b1220", fontSize: 14, fontWeight: "700" },
  message: { color: "rgba(11,18,32,0.7)", fontSize: 12, marginTop: 2 },
  retry: { alignSelf: "flex-start", minHeight: 44, justifyContent: "center", paddingHorizontal: 14, borderRadius: 22, backgroundColor: "rgba(11,18,32,0.08)" },
  retryText: { color: "#0b1220", fontSize: 13, fontWeight: "700" },
});
