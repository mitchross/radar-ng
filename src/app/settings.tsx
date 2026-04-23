/**
 * Settings modal — reachable via the gear icon on the Home screen.
 * Keeps the self-hosted data source + server URL config alive now that
 * the Settings tab is retired in the Cumulus design.
 */
import {
  View,
  Text,
  StyleSheet,
  Switch,
  TouchableOpacity,
  TextInput,
  ScrollView,
} from "react-native";
import { SafeAreaView } from "react-native-safe-area-context";
import { LinearGradient } from "expo-linear-gradient";
import Slider from "@react-native-community/slider";
import { useRouter } from "expo-router";
import { useWeatherStore } from "../stores/useWeatherStore";
import { cumulus, CONDITION_GRADIENTS } from "../lib/cumulusTheme";
import { PaletteSelector } from "../components/palette/PaletteSelector";

export default function SettingsScreen() {
  const router = useRouter();
  const mapStyle = useWeatherStore((s) => s.mapStyle);
  const setMapStyle = useWeatherStore((s) => s.setMapStyle);
  const temperatureUnit = useWeatherStore((s) => s.temperatureUnit);
  const setTemperatureUnit = useWeatherStore((s) => s.setTemperatureUnit);
  const radarOpacity = useWeatherStore((s) => s.radarOpacity);
  const setRadarOpacity = useWeatherStore((s) => s.setRadarOpacity);
  const playbackSpeed = useWeatherStore((s) => s.playbackSpeed);
  const setPlaybackSpeed = useWeatherStore((s) => s.setPlaybackSpeed);
  const serverUrl = useWeatherStore((s) => s.serverUrl);
  const setServerUrl = useWeatherStore((s) => s.setServerUrl);

  return (
    <LinearGradient colors={CONDITION_GRADIENTS.clearNight} style={styles.container}>
      <SafeAreaView style={styles.flex} edges={["top"]}>
        <View style={styles.header}>
          <TouchableOpacity onPress={() => router.back()} style={styles.closeBtn}>
            <Text style={styles.closeIcon}>{"\u2715"}</Text>
          </TouchableOpacity>
          <Text style={styles.title}>Settings</Text>
          <View style={{ width: 36 }} />
        </View>

        <ScrollView showsVerticalScrollIndicator={false} contentContainerStyle={styles.scroll}>
          <View style={styles.card}>
            <Text style={styles.sectionTitle}>MAP</Text>
            <Row label="Dark Basemap">
              <Switch
                value={mapStyle === "dark"}
                onValueChange={(v) => setMapStyle(v ? "dark" : "light")}
                trackColor={{ true: cumulus.accent, false: "rgba(255,255,255,0.15)" }}
                thumbColor="#fff"
              />
            </Row>
          </View>

          <View style={styles.card}>
            <Text style={styles.sectionTitle}>UNITS</Text>
            <Row label="Temperature">
              <SegmentedControl
                options={["F", "C"]}
                selected={temperatureUnit === "fahrenheit" ? "F" : "C"}
                onSelect={(v) => setTemperatureUnit(v === "F" ? "fahrenheit" : "celsius")}
              />
            </Row>
          </View>

          <View style={styles.card}>
            <Text style={styles.sectionTitle}>PALETTE</Text>
            <View style={{ marginTop: 6 }}>
              <PaletteSelector />
            </View>
            <Text style={styles.hint}>
              Color scheme for radar + HRRR layers. Self-hosted servers need
              PALETTES=classic,vivid,muted on the ingestors to render every swatch.
            </Text>
          </View>

          <View style={styles.card}>
            <Text style={styles.sectionTitle}>RADAR</Text>
            <Row label={`Opacity: ${Math.round(radarOpacity * 100)}%`}>
              <Slider
                style={styles.slider}
                minimumValue={0.1}
                maximumValue={1}
                step={0.05}
                value={radarOpacity}
                onValueChange={setRadarOpacity}
                minimumTrackTintColor={cumulus.accent}
                maximumTrackTintColor="rgba(255,255,255,0.15)"
                thumbTintColor="#fff"
              />
            </Row>
            <Row label={`Playback: ${playbackSpeed} FPS`}>
              <Slider
                style={styles.slider}
                minimumValue={1}
                maximumValue={15}
                step={1}
                value={playbackSpeed}
                onValueChange={setPlaybackSpeed}
                minimumTrackTintColor={cumulus.accent}
                maximumTrackTintColor="rgba(255,255,255,0.15)"
                thumbTintColor="#fff"
              />
            </Row>
          </View>

          <View style={styles.card}>
            <Text style={styles.sectionTitle}>BACKEND</Text>
            <Row label="Server URL">
              <TextInput
                style={styles.textInput}
                value={serverUrl}
                onChangeText={setServerUrl}
                placeholder="https://radar-ng-api.vanillax.me"
                placeholderTextColor="rgba(255,255,255,0.25)"
                autoCapitalize="none"
                autoCorrect={false}
              />
            </Row>
            <Text style={styles.hint}>
              Point at your radar-ng tile-server. All weather data — radar,
              HRRR forecast, basemap, lightning, storms — comes from here.
            </Text>
          </View>

          <Text style={styles.footer}>
            radar-ng · Cumulus UI{"\n"}
            Self-hosted: MRMS, HRRR, Open-Meteo, Protomaps · NWS alerts (gov)
          </Text>

          <View style={{ height: 60 }} />
        </ScrollView>
      </SafeAreaView>
    </LinearGradient>
  );
}

function Row({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <View style={styles.row}>
      <Text style={styles.rowLabel}>{label}</Text>
      {children}
    </View>
  );
}

function SegmentedControl({
  options,
  selected,
  onSelect,
}: {
  options: string[];
  selected: string;
  onSelect: (value: string) => void;
}) {
  return (
    <View style={styles.segmented}>
      {options.map((opt) => (
        <TouchableOpacity
          key={opt}
          style={[styles.segment, opt === selected && styles.segmentSelected]}
          onPress={() => onSelect(opt)}
        >
          <Text style={[styles.segmentText, opt === selected && styles.segmentTextSelected]}>
            {opt}
          </Text>
        </TouchableOpacity>
      ))}
    </View>
  );
}

const styles = StyleSheet.create({
  container: { flex: 1 },
  flex: { flex: 1 },
  header: {
    flexDirection: "row",
    alignItems: "center",
    paddingHorizontal: 16,
    paddingTop: 8,
    paddingBottom: 4,
  },
  closeBtn: {
    width: 36,
    height: 36,
    borderRadius: 12,
    backgroundColor: cumulus.cardStrong,
    borderWidth: 1,
    borderColor: cumulus.inkLine,
    alignItems: "center",
    justifyContent: "center",
  },
  closeIcon: { color: cumulus.ink, fontSize: 16, fontWeight: "600" },
  title: {
    flex: 1,
    textAlign: "center",
    color: cumulus.ink,
    fontSize: 17,
    fontWeight: "700",
  },
  scroll: { padding: 16 },
  card: {
    backgroundColor: cumulus.card,
    borderRadius: 18,
    borderWidth: 1,
    borderColor: cumulus.cardLine,
    padding: 14,
    marginBottom: 12,
  },
  sectionTitle: {
    fontSize: 11,
    fontWeight: "700",
    color: cumulus.inkMuted,
    letterSpacing: 1.4,
    marginBottom: 4,
  },
  row: {
    flexDirection: "row",
    justifyContent: "space-between",
    alignItems: "center",
    paddingVertical: 10,
  },
  rowLabel: { fontSize: 15, color: cumulus.inkDim, fontWeight: "500" },
  slider: { width: 150 },
  segmented: {
    flexDirection: "row",
    backgroundColor: "rgba(255,255,255,0.08)",
    borderRadius: 10,
    overflow: "hidden",
  },
  segment: { paddingHorizontal: 14, paddingVertical: 7 },
  segmentSelected: { backgroundColor: cumulus.accent, borderRadius: 8 },
  segmentText: { color: "rgba(255,255,255,0.5)", fontWeight: "600", fontSize: 13 },
  segmentTextSelected: { color: "#fff" },
  textInput: {
    color: cumulus.ink,
    fontSize: 13,
    backgroundColor: "rgba(255,255,255,0.08)",
    borderRadius: 10,
    paddingHorizontal: 12,
    paddingVertical: 7,
    width: 200,
    textAlign: "right",
  },
  hint: {
    fontSize: 11,
    color: cumulus.inkMuted,
    lineHeight: 16,
    paddingTop: 8,
    paddingHorizontal: 2,
  },
  footer: {
    marginTop: 12,
    textAlign: "center",
    color: cumulus.inkFaint,
    fontSize: 11,
    lineHeight: 17,
  },
});
