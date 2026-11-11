/** Famous storms picker: catalogue cases from GET /api/replays; picking one enters replay. */
import { ScrollView, Pressable, StyleSheet, Text, View } from "react-native";
import { MapInfoSheet } from "./MapInfoSheet";
import { cumulus } from "../../lib/cumulusTheme";
import { replayTimeLabel } from "../../lib/replaySource";
import type { ReplayCase } from "../../lib/replayApi";
import { useReplayCatalogue } from "../../hooks/useReplay";
import { useWeatherStore } from "../../stores/useWeatherStore";

export function FamousStormsSheet({ visible, onClose }: { visible: boolean; onClose: () => void }) {
  const catalogue = useReplayCatalogue(visible);
  const enterReplay = useWeatherStore((s) => s.enterReplay);
  const activeCase = useWeatherStore((s) => s.replayCase?.id ?? null);

  return (
    <MapInfoSheet visible={visible} eyebrow="FAMOUS STORMS" title="Replay a storm" subtitle="Historical radar and the warnings in force" accent="#c0392b" onClose={onClose}>
      {catalogue.isPending ? <Text style={styles.note}>Loading storms…</Text> : null}
      {catalogue.isError ? (
        <View>
          <Text style={styles.error} accessibilityRole="alert">Could not load famous storms. {catalogue.error.message}</Text>
          <Pressable onPress={() => void catalogue.refetch()} style={styles.retry} accessibilityRole="button" accessibilityLabel="Retry loading famous storms">
            <Text style={styles.retryText}>Retry</Text>
          </Pressable>
        </View>
      ) : null}
      {catalogue.data && catalogue.data.cases.length === 0 ? <Text style={styles.note}>No storms are available yet.</Text> : null}
      {catalogue.data ? (
        <ScrollView style={styles.list}>
          {catalogue.data.cases.map((c: ReplayCase) => (
            <Pressable
              key={`${c.id}:${c.revision}`}
              onPress={() => { enterReplay(c); onClose(); }}
              style={({ pressed }) => [styles.row, c.id === activeCase ? styles.rowActive : null, pressed ? { opacity: 0.7 } : null]}
              accessibilityRole="button"
              accessibilityLabel={`Replay ${c.title}`}
              accessibilityHint={c.summary}
            >
              <Text style={styles.rowTitle}>{c.title}</Text>
              <Text style={styles.rowDate}>{replayTimeLabel(Math.floor(Date.parse(c.start) / 1000), c.timezone)}</Text>
              <Text style={styles.rowSummary}>{c.summary}</Text>
            </Pressable>
          ))}
        </ScrollView>
      ) : null}
    </MapInfoSheet>
  );
}

const styles = StyleSheet.create({
  list: { maxHeight: 360, marginTop: 12 },
  row: { paddingVertical: 12, paddingHorizontal: 12, borderRadius: 14, backgroundColor: "rgba(33,31,27,0.05)", marginBottom: 8 },
  rowActive: { borderWidth: 1.5, borderColor: "#c0392b" },
  rowTitle: { color: cumulus.ink, fontSize: 16, fontWeight: "700" },
  rowDate: { color: cumulus.inkMuted, fontSize: 12, marginTop: 2 },
  rowSummary: { color: cumulus.inkDim, fontSize: 13, lineHeight: 18, marginTop: 4 },
  note: { color: cumulus.inkMuted, fontSize: 13, marginTop: 12 },
  error: { color: "#b42318", fontSize: 13, marginTop: 12 },
  retry: { alignSelf: "flex-start", minHeight: 44, justifyContent: "center", paddingHorizontal: 14 },
  retryText: { color: "#6a4df0", fontSize: 15, fontWeight: "700" },
});
