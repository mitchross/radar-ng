import { useState } from "react";
import { Pressable, StyleSheet, Text, View } from "react-native";
import { useWeatherStore } from "../../stores/useWeatherStore";
import { MAP_CHROME_MAX_FONT_SCALE } from "../../lib/constants";
import { provenance } from "../../lib/level2";
import { useMapChromeInsets } from "../../hooks/useMapChromeInsets";

/** Close-up only: one line on which radar and sweep is showing, expandable to the full scan details. */
export function Level2Note({ top }: { top: number }) {
  const frame = useWeatherStore((s) => s.frames[s.currentFrameIndex]?.level2);
  const centre = useWeatherStore((s) => s.level2.centre);
  const fallback = useWeatherStore((s) => s.level2.fallback);
  const chrome = useMapChromeInsets();
  const [open, setOpen] = useState(false);

  if (fallback) {
    return (
      <View accessibilityRole="alert" accessible accessibilityLabel={`${fallback.title}. ${fallback.reason}`} style={[styles.box, { top, left: chrome.left + 58 }]}>
        <Text maxFontSizeMultiplier={MAP_CHROME_MAX_FONT_SCALE} style={styles.title}>{fallback.title}</Text>
        <Text maxFontSizeMultiplier={MAP_CHROME_MAX_FONT_SCALE} style={styles.sub}>{fallback.reason}</Text>
      </View>
    );
  }
  if (!frame || !centre) return null;
  const p = provenance(frame, centre);
  return (
    <Pressable
      accessibilityRole="button"
      accessibilityLabel={`${p.headline}. ${p.groundRange}`}
      accessibilityState={{ expanded: open }}
      onPress={() => setOpen((v) => !v)}
      style={[styles.box, { top, left: chrome.left + 58 }]}
    >
      <Text maxFontSizeMultiplier={MAP_CHROME_MAX_FONT_SCALE} style={styles.title}>{p.headline}</Text>
      <Text maxFontSizeMultiplier={MAP_CHROME_MAX_FONT_SCALE} style={styles.sub}>{p.groundRange}</Text>
      {open ? p.rows.map((r) => (
        <Text key={r.label} maxFontSizeMultiplier={MAP_CHROME_MAX_FONT_SCALE} style={styles.row}>{r.label}: {r.value}</Text>
      )) : null}
    </Pressable>
  );
}

const styles = StyleSheet.create({
  box: { position: "absolute", maxWidth: 260, minHeight: 44, justifyContent: "center", backgroundColor: "rgba(0,0,0,0.88)", paddingHorizontal: 10, paddingVertical: 6, borderRadius: 10, gap: 2 },
  title: { color: "#fff", fontSize: 12, fontWeight: "600" },
  sub: { color: "rgba(255,255,255,0.8)", fontSize: 11 },
  row: { color: "rgba(255,255,255,0.9)", fontSize: 11 },
});
