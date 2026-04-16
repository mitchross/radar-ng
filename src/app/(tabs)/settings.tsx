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
import { useWeatherStore } from "../../stores/useWeatherStore";

export default function SettingsScreen() {
  const mapStyle = useWeatherStore((s) => s.mapStyle);
  const setMapStyle = useWeatherStore((s) => s.setMapStyle);
  const temperatureUnit = useWeatherStore((s) => s.temperatureUnit);
  const setTemperatureUnit = useWeatherStore((s) => s.setTemperatureUnit);
  const radarOpacity = useWeatherStore((s) => s.radarOpacity);
  const setRadarOpacity = useWeatherStore((s) => s.setRadarOpacity);
  const playbackSpeed = useWeatherStore((s) => s.playbackSpeed);
  const setPlaybackSpeed = useWeatherStore((s) => s.setPlaybackSpeed);
  const dataSource = useWeatherStore((s) => s.dataSource);
  const setDataSource = useWeatherStore((s) => s.setDataSource);
  const serverUrl = useWeatherStore((s) => s.serverUrl);
  const setServerUrl = useWeatherStore((s) => s.setServerUrl);

  return (
    <LinearGradient colors={["#0D1B2A", "#1B2838", "#263238"]} style={styles.container}>
      <SafeAreaView style={styles.flex}>
        <ScrollView showsVerticalScrollIndicator={false} contentContainerStyle={styles.scroll}>
          <Text style={styles.title}>Settings</Text>

          <View style={styles.card}>
            <Text style={styles.sectionTitle}>MAP</Text>
            <Row label="Dark Mode">
              <Switch
                value={mapStyle === "dark"}
                onValueChange={(v) => setMapStyle(v ? "dark" : "light")}
                trackColor={{ true: "#42A5F5", false: "rgba(255,255,255,0.15)" }}
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
                onSelect={(v) =>
                  setTemperatureUnit(v === "F" ? "fahrenheit" : "celsius")
                }
              />
            </Row>
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
                minimumTrackTintColor="#42A5F5"
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
                minimumTrackTintColor="#42A5F5"
                maximumTrackTintColor="rgba(255,255,255,0.15)"
                thumbTintColor="#fff"
              />
            </Row>
          </View>

          <View style={styles.card}>
            <Text style={styles.sectionTitle}>DATA SOURCE</Text>
            <Row label="Source">
              <SegmentedControl
                options={["Free", "Self-Hosted"]}
                selected={dataSource === "rainviewer" ? "Free" : "Self-Hosted"}
                onSelect={(v) =>
                  setDataSource(v === "Free" ? "rainviewer" : "selfhosted")
                }
              />
            </Row>
            {dataSource === "selfhosted" && (
              <Row label="Server URL">
                <TextInput
                  style={styles.textInput}
                  value={serverUrl}
                  onChangeText={setServerUrl}
                  placeholder="http://192.168.1.x:8080"
                  placeholderTextColor="rgba(255,255,255,0.25)"
                  autoCapitalize="none"
                  autoCorrect={false}
                />
              </Row>
            )}
          </View>

          <Text style={styles.footer}>
            StormScope v1.0{"\n"}
            Data: IEM NEXRAD, Open-Meteo, NWS
          </Text>

          <View style={{ height: 100 }} />
        </ScrollView>
      </SafeAreaView>
    </LinearGradient>
  );
}

function Row({
  label,
  children,
}: {
  label: string;
  children: React.ReactNode;
}) {
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
          <Text
            style={[
              styles.segmentText,
              opt === selected && styles.segmentTextSelected,
            ]}
          >
            {opt}
          </Text>
        </TouchableOpacity>
      ))}
    </View>
  );
}

const styles = StyleSheet.create({
  container: {
    flex: 1,
  },
  flex: {
    flex: 1,
  },
  scroll: {
    padding: 20,
  },
  title: {
    fontSize: 32,
    fontWeight: "800",
    color: "#fff",
    marginBottom: 28,
    letterSpacing: -0.5,
  },
  card: {
    backgroundColor: "rgba(255,255,255,0.08)",
    borderRadius: 20,
    borderWidth: 1,
    borderColor: "rgba(255,255,255,0.12)",
    padding: 18,
    marginBottom: 16,
  },
  sectionTitle: {
    fontSize: 13,
    fontWeight: "700",
    color: "rgba(255,255,255,0.5)",
    letterSpacing: 1.2,
    marginBottom: 8,
  },
  row: {
    flexDirection: "row",
    justifyContent: "space-between",
    alignItems: "center",
    paddingVertical: 12,
    borderBottomWidth: StyleSheet.hairlineWidth,
    borderBottomColor: "rgba(255,255,255,0.08)",
  },
  rowLabel: {
    fontSize: 16,
    color: "rgba(255,255,255,0.85)",
    fontWeight: "500",
  },
  slider: {
    width: 150,
  },
  segmented: {
    flexDirection: "row",
    backgroundColor: "rgba(255,255,255,0.1)",
    borderRadius: 10,
    overflow: "hidden",
  },
  segment: {
    paddingHorizontal: 16,
    paddingVertical: 8,
  },
  segmentSelected: {
    backgroundColor: "#42A5F5",
    borderRadius: 8,
  },
  segmentText: {
    color: "rgba(255,255,255,0.5)",
    fontWeight: "600",
    fontSize: 14,
  },
  segmentTextSelected: {
    color: "#fff",
  },
  textInput: {
    color: "#fff",
    fontSize: 14,
    backgroundColor: "rgba(255,255,255,0.1)",
    borderRadius: 10,
    paddingHorizontal: 12,
    paddingVertical: 8,
    width: 200,
    textAlign: "right",
  },
  footer: {
    marginTop: 24,
    textAlign: "center",
    color: "rgba(255,255,255,0.3)",
    fontSize: 12,
    lineHeight: 18,
  },
});
