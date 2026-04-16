import { View, Text, StyleSheet, Switch, TouchableOpacity, TextInput } from "react-native";
import { SafeAreaView } from "react-native-safe-area-context";
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
    <SafeAreaView style={styles.container}>
      <Text style={styles.title}>Settings</Text>

      <View style={styles.sectionCard}>
        <Section title="Map">
          <Row label="Dark Mode">
            <Switch
              value={mapStyle === "dark"}
              onValueChange={(v) => setMapStyle(v ? "dark" : "light")}
              trackColor={{ true: "#1E88E5" }}
            />
          </Row>
        </Section>
      </View>

      <View style={styles.sectionCard}>
        <Section title="Units">
          <Row label="Temperature">
            <SegmentedControl
              options={["F", "C"]}
              selected={temperatureUnit === "fahrenheit" ? "F" : "C"}
              onSelect={(v) =>
                setTemperatureUnit(v === "F" ? "fahrenheit" : "celsius")
              }
            />
          </Row>
        </Section>
      </View>

      <View style={styles.sectionCard}>
        <Section title="Radar">
          <Row label={`Opacity: ${Math.round(radarOpacity * 100)}%`}>
            <Slider
              style={styles.slider}
              minimumValue={0.1}
              maximumValue={1}
              step={0.05}
              value={radarOpacity}
              onValueChange={setRadarOpacity}
              minimumTrackTintColor="#1E88E5"
              maximumTrackTintColor="#555"
              thumbTintColor="#1E88E5"
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
              minimumTrackTintColor="#1E88E5"
              maximumTrackTintColor="#555"
              thumbTintColor="#1E88E5"
            />
          </Row>
        </Section>
      </View>

      <View style={styles.sectionCard}>
        <Section title="Data Source">
          <Row label="Source">
            <SegmentedControl
              options={["Free", "Self-Hosted"]}
              selected={dataSource === "rainviewer" ? "Free" : "Self-Hosted"}
              onSelect={(v) => setDataSource(v === "Free" ? "rainviewer" : "selfhosted")}
            />
          </Row>
          {dataSource === "selfhosted" && (
            <Row label="Server URL">
              <TextInput
                style={styles.textInput}
                value={serverUrl}
                onChangeText={setServerUrl}
                placeholder="http://192.168.1.x:8080"
                placeholderTextColor="#555"
                autoCapitalize="none"
                autoCorrect={false}
              />
            </Row>
          )}
        </Section>
      </View>

      <Text style={styles.footer}>
        StormScope v1.0{"\n"}
        Data: RainViewer, Open-Meteo, NWS
      </Text>
    </SafeAreaView>
  );
}

function Section({
  title,
  children,
}: {
  title: string;
  children: React.ReactNode;
}) {
  return (
    <View style={styles.section}>
      <Text style={styles.sectionTitle}>{title}</Text>
      {children}
    </View>
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
          style={[
            styles.segment,
            opt === selected && styles.segmentSelected,
          ]}
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
    backgroundColor: "#0d1117",
    padding: 16,
  },
  title: {
    fontSize: 28,
    fontWeight: "700",
    color: "#fff",
    marginBottom: 24,
  },
  sectionCard: {
    backgroundColor: "#161b22",
    borderRadius: 16,
    borderWidth: 1,
    borderColor: "#21262d",
    padding: 16,
    marginBottom: 16,
  },
  section: {
    marginBottom: 0,
  },
  sectionTitle: {
    fontSize: 12,
    fontWeight: "700",
    color: "#8b949e",
    textTransform: "uppercase",
    letterSpacing: 1,
    marginBottom: 8,
  },
  row: {
    flexDirection: "row",
    justifyContent: "space-between",
    alignItems: "center",
    paddingVertical: 12,
    borderBottomWidth: StyleSheet.hairlineWidth,
    borderBottomColor: "#21262d",
  },
  rowLabel: {
    fontSize: 16,
    color: "#c9d1d9",
  },
  slider: {
    width: 150,
  },
  segmented: {
    flexDirection: "row",
    backgroundColor: "#333",
    borderRadius: 8,
    overflow: "hidden",
  },
  segment: {
    paddingHorizontal: 16,
    paddingVertical: 8,
  },
  segmentSelected: {
    backgroundColor: "#1E88E5",
  },
  segmentText: {
    color: "#aaa",
    fontWeight: "600",
  },
  segmentTextSelected: {
    color: "#fff",
  },
  footer: {
    marginTop: "auto",
    textAlign: "center",
    color: "#484f58",
    fontSize: 12,
    lineHeight: 18,
  },
  textInput: {
    color: "#fff",
    fontSize: 14,
    backgroundColor: "#333",
    borderRadius: 8,
    paddingHorizontal: 12,
    paddingVertical: 8,
    width: 200,
    textAlign: "right",
  },
});
