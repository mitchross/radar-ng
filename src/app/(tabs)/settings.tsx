/**
 * Cumulus Settings tab — per design_handoff_radar_app/src/settings.jsx.
 * Self-hosted stack hero, Data Sources list, Docker Stack, Tile Cache,
 * Network, Preferences, About. The stack URL is the single source of truth
 * for every derived endpoint in the Data Sources list.
 */
import { useEffect, useMemo, useState } from "react";
import {
  ScrollView,
  View,
  Text,
  StyleSheet,
  TouchableOpacity,
  Switch,
  TextInput,
  ActivityIndicator,
} from "react-native";
import { SafeAreaView } from "react-native-safe-area-context";
import { LinearGradient } from "expo-linear-gradient";
import Slider from "@react-native-community/slider";
import { useWeatherStore } from "../../stores/useWeatherStore";
import { checkServerHealth } from "../../lib/api";
import { SELF_HOSTED } from "../../lib/constants";
import { cumulus, CONDITION_GRADIENTS } from "../../lib/cumulusTheme";
import { PaletteSelector } from "../../components/palette/PaletteSelector";

type SourceKey = "radar" | "satellite" | "forecast" | "basemap" | "alerts";
type SourceStatus = "healthy" | "stale" | "error" | "disabled";

export default function SettingsScreen() {
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

  const [editing, setEditing] = useState<SourceKey | null>(null);
  const [urlDraft, setUrlDraft] = useState(serverUrl);
  const [stackHealthy, setStackHealthy] = useState<boolean | null>(null);
  const [refreshLabel, setRefreshLabel] = useState("2 min");

  useEffect(() => {
    let cancelled = false;
    setStackHealthy(null);
    checkServerHealth(serverUrl).then((ok) => {
      if (!cancelled) setStackHealthy(ok);
    });
    return () => { cancelled = true; };
  }, [serverUrl]);

  const sources = useMemo(
    () => buildSources(serverUrl, stackHealthy),
    [serverUrl, stackHealthy]
  );

  const stackHost = useMemo(() => hostOf(serverUrl), [serverUrl]);

  return (
    <LinearGradient colors={CONDITION_GRADIENTS.clearNight} style={styles.container}>
      <SafeAreaView style={styles.flex} edges={["top"]}>
        <ScrollView
          style={styles.flex}
          contentContainerStyle={styles.scroll}
          showsVerticalScrollIndicator={false}
        >
          {/* Header */}
          <View style={styles.headerRow}>
            <Text style={styles.title}>Settings</Text>
          </View>

          {/* Hero — self-hosted stack */}
          <View style={styles.hero}>
            <View style={styles.heroHeader}>
              <Text style={styles.heroKicker}>SELF-HOSTED STACK</Text>
              <StatusDot ok={stackHealthy === true} loading={stackHealthy === null} />
            </View>
            <Text style={styles.heroName}>radar-ng</Text>
            <Text style={styles.heroUrl}>{stackHost}</Text>
            <View style={styles.heroStatsRow}>
              <Stat label="UPTIME" value="—" />
              <Stat label="TILES/DAY" value="—" />
              <Stat label="CACHE" value="—" />
            </View>
          </View>

          <SectionHeader>Stack URL</SectionHeader>
          <View style={styles.card}>
            <View style={styles.urlRow}>
              <TextInput
                style={styles.urlInput}
                value={urlDraft}
                onChangeText={setUrlDraft}
                autoCapitalize="none"
                autoCorrect={false}
                placeholder="https://radar-ng-api.example.com"
                placeholderTextColor={cumulus.inkFaint}
              />
              <TouchableOpacity
                style={[
                  styles.saveBtn,
                  urlDraft === serverUrl && styles.saveBtnDisabled,
                ]}
                disabled={urlDraft === serverUrl}
                onPress={() => setServerUrl(urlDraft.trim())}
              >
                <Text style={styles.saveBtnText}>Save</Text>
              </TouchableOpacity>
            </View>
          </View>

          <SectionHeader>Data Sources</SectionHeader>
          <View style={styles.card}>
            {sources.map((src, i) => (
              <View key={src.key}>
                <SourceRow
                  name={src.name}
                  icon={src.icon}
                  endpoint={src.endpoint}
                  status={src.status}
                  expanded={editing === src.key}
                  onPress={() => setEditing(editing === src.key ? null : src.key)}
                />
                {editing === src.key && <SourceEditor src={src} />}
                {i < sources.length - 1 && <Sep />}
              </View>
            ))}
          </View>

          <SectionHeader>Docker Stack</SectionHeader>
          <View style={styles.card}>
            {DOCKER_CONTAINERS.map((c, i) => (
              <View key={c.name}>
                <ContainerRow {...c} />
                {i < DOCKER_CONTAINERS.length - 1 && <Sep />}
              </View>
            ))}
          </View>
          <View style={styles.btnRow}>
            <BigBtn primary>Pull & Restart</BigBtn>
            <BigBtn>View Logs</BigBtn>
          </View>

          <SectionHeader>Tile Cache</SectionHeader>
          <View style={styles.card}>
            <Row>
              <RowLeft title="Storage" sub="— of 8 GB used" />
              <Text style={styles.monoDim}>—</Text>
            </Row>
            <View style={{ paddingHorizontal: 14, paddingBottom: 12 }}>
              <View style={styles.progressTrack}>
                <View style={[styles.progressFill, { width: "0%" }]} />
              </View>
            </View>
            <Sep />
            <Row>
              <RowLeft title="Purge cache" sub="Frees radar + basemap tiles" />
              <Text style={styles.clearBtn}>Clear</Text>
            </Row>
          </View>

          <SectionHeader>Network</SectionHeader>
          <View style={styles.card}>
            <SelectRow
              label="Refresh interval"
              value={refreshLabel}
              options={["30 sec", "1 min", "2 min", "5 min"]}
              onChange={setRefreshLabel}
            />
            <Sep />
            <ToggleRow
              label="Use cellular for tiles"
              value={true}
              onChange={() => {}}
            />
          </View>

          <SectionHeader>Preferences</SectionHeader>
          <View style={styles.card}>
            <ToggleRow
              label="Dark basemap"
              sub="Affects Radar map style"
              value={mapStyle === "dark"}
              onChange={(v) => setMapStyle(v ? "dark" : "light")}
            />
            <Sep />
            <Row>
              <RowLeft title="Temperature" />
              <Segmented
                options={["°F", "°C"]}
                selected={temperatureUnit === "fahrenheit" ? "°F" : "°C"}
                onSelect={(v) => setTemperatureUnit(v === "°F" ? "fahrenheit" : "celsius")}
              />
            </Row>
            <Sep />
            <View style={{ padding: 14 }}>
              <Text style={styles.rowLabel}>Radar palette</Text>
              <View style={{ marginTop: 10 }}>
                <PaletteSelector />
              </View>
            </View>
            <Sep />
            <View style={{ paddingHorizontal: 14, paddingVertical: 10 }}>
              <Text style={styles.rowLabel}>Radar opacity {Math.round(radarOpacity * 100)}%</Text>
              <Slider
                minimumValue={0.1}
                maximumValue={1}
                step={0.05}
                value={radarOpacity}
                onValueChange={setRadarOpacity}
                minimumTrackTintColor={cumulus.accent}
                maximumTrackTintColor="rgba(255,255,255,0.15)"
                thumbTintColor="#fff"
                style={{ marginTop: 4 }}
              />
            </View>
            <Sep />
            <View style={{ paddingHorizontal: 14, paddingVertical: 10 }}>
              <Text style={styles.rowLabel}>Playback {playbackSpeed} FPS</Text>
              <Slider
                minimumValue={1}
                maximumValue={15}
                step={1}
                value={playbackSpeed}
                onValueChange={setPlaybackSpeed}
                minimumTrackTintColor={cumulus.accent}
                maximumTrackTintColor="rgba(255,255,255,0.15)"
                thumbTintColor="#fff"
                style={{ marginTop: 4 }}
              />
            </View>
          </View>

          <SectionHeader>About</SectionHeader>
          <View style={styles.card}>
            <Row><RowLeft title="App version" /><Text style={styles.monoDim}>1.0.0</Text></Row>
            <Sep />
            <Row><RowLeft title="Server" /><Text style={styles.monoDim}>{stackHost}</Text></Row>
            <Sep />
            <Row><RowLeft title="Stack status" /><Text style={[styles.monoDim, { color: stackHealthy ? cumulus.ok : "#FF6E7A" }]}>{stackHealthy === null ? "…" : stackHealthy ? "ONLINE" : "OFFLINE"}</Text></Row>
          </View>

          <Text style={styles.footer}>
            radar-ng — Cumulus UI{"\n"}
            Self-hosted: MRMS · HRRR · Open-Meteo · Protomaps · NWS alerts
          </Text>
          <View style={{ height: 140 }} />
        </ScrollView>
      </SafeAreaView>
    </LinearGradient>
  );
}

function buildSources(
  serverUrl: string,
  ok: boolean | null
): Array<{ key: SourceKey; name: string; icon: string; endpoint: string; status: SourceStatus }> {
  const status: SourceStatus =
    ok === null ? "stale" : ok ? "healthy" : "error";
  return [
    {
      key: "radar",
      name: "Radar tiles (MRMS)",
      icon: "⛈",
      endpoint: `${serverUrl}${SELF_HOSTED.TILE_PATTERN}`,
      status,
    },
    {
      key: "forecast",
      name: "Forecast (HRRR + Open-Meteo)",
      icon: "🌡",
      endpoint: `${serverUrl}${SELF_HOSTED.FORECAST_PATH}/{lat}/{lon}`,
      status,
    },
    {
      key: "basemap",
      name: "Base map tiles",
      icon: "🗺",
      endpoint: `${serverUrl}${SELF_HOSTED.BASEMAP_TILE_PATTERN}`,
      status,
    },
    {
      key: "satellite",
      name: "Satellite (GOES)",
      icon: "🛰",
      endpoint: `${serverUrl}/tiles/goes/{z}/{x}/{y}.png`,
      status: "disabled",
    },
    {
      key: "alerts",
      name: "Alerts (NWS CAP)",
      icon: "⚠︎",
      endpoint: "https://api.weather.gov/alerts/active",
      status: "healthy",
    },
  ];
}

function hostOf(url: string): string {
  try {
    return new URL(url).host;
  } catch {
    return url;
  }
}

function StatusDot({ ok, loading }: { ok: boolean; loading?: boolean }) {
  if (loading) return <ActivityIndicator size="small" color={cumulus.inkMuted} />;
  const color = ok ? cumulus.ok : "#FF6E7A";
  const label = ok ? "ONLINE" : "ERROR";
  return (
    <View style={styles.statusRow}>
      <View style={[styles.statusDot, { backgroundColor: color, shadowColor: color }]} />
      <Text style={[styles.statusLabel, { color }]}>{label}</Text>
    </View>
  );
}

function Stat({ label, value }: { label: string; value: string }) {
  return (
    <View style={{ flex: 1 }}>
      <Text style={styles.statLabel}>{label}</Text>
      <Text style={styles.statValue}>{value}</Text>
    </View>
  );
}

function SectionHeader({ children }: { children: string }) {
  return <Text style={styles.sectionHeader}>{children}</Text>;
}

function Sep() {
  return <View style={styles.sep} />;
}

function Row({ children }: { children: React.ReactNode }) {
  return <View style={styles.row}>{children}</View>;
}

function RowLeft({ title, sub }: { title: string; sub?: string }) {
  return (
    <View style={{ flex: 1, minWidth: 0 }}>
      <Text style={styles.rowLabel}>{title}</Text>
      {sub && <Text style={styles.rowSub}>{sub}</Text>}
    </View>
  );
}

function Pill({ color, children }: { color: string; children: string }) {
  return (
    <View style={[styles.pill, { backgroundColor: `${color}22`, borderColor: `${color}55` }]}>
      <Text style={[styles.pillText, { color }]}>{children}</Text>
    </View>
  );
}

function SourceRow({
  name, icon, endpoint, status, expanded, onPress,
}: {
  name: string;
  icon: string;
  endpoint: string;
  status: SourceStatus;
  expanded: boolean;
  onPress: () => void;
}) {
  const colors: Record<SourceStatus, string> = {
    healthy: cumulus.ok,
    stale: "#F5A524",
    error: "#FF6E7A",
    disabled: "rgba(255,255,255,0.3)",
  };
  const pill = { healthy: "OK", stale: "STALE", error: "ERROR", disabled: "OFF" }[status];
  return (
    <TouchableOpacity onPress={onPress} activeOpacity={0.7} style={styles.sourceRow}>
      <Text style={styles.sourceIcon}>{icon}</Text>
      <View style={{ flex: 1, minWidth: 0 }}>
        <Text style={styles.sourceName}>{name}</Text>
        <Text style={styles.sourceEndpoint} numberOfLines={1}>{endpoint}</Text>
      </View>
      <Pill color={colors[status]}>{pill}</Pill>
      <Text style={[styles.chev, expanded && { transform: [{ rotate: "90deg" }] }]}>›</Text>
    </TouchableOpacity>
  );
}

function SourceEditor({ src }: { src: { name: string; endpoint: string } }) {
  return (
    <View style={styles.editor}>
      <Text style={styles.editorKicker}>EDIT ENDPOINT</Text>
      <Field label="URL TEMPLATE" value={src.endpoint} />
      <Field label="AUTH" value="None" />
      <View style={{ flexDirection: "row", gap: 8 }}>
        <View style={{ flex: 1 }}><Field label="TIMEOUT" value="8s" /></View>
        <View style={{ flex: 1 }}><Field label="RETRIES" value="3" /></View>
      </View>
      <Text style={styles.editorHint}>
        Individual source URLs derive from the Stack URL above.
        Change the stack URL to re-point every data source.
      </Text>
    </View>
  );
}

function Field({ label, value }: { label: string; value: string }) {
  return (
    <View style={{ marginBottom: 8 }}>
      <Text style={styles.fieldLabel}>{label}</Text>
      <View style={styles.fieldBox}>
        <Text style={styles.fieldValue} numberOfLines={1}>{value}</Text>
      </View>
    </View>
  );
}

function ContainerRow({
  name, image, status, ports,
}: { name: string; image: string; status: "healthy" | "updating" | "error"; ports: string }) {
  const color = { healthy: cumulus.ok, updating: "#F5A524", error: "#FF6E7A" }[status];
  return (
    <View style={styles.containerRow}>
      <View style={[styles.containerDot, { backgroundColor: color, shadowColor: color }]} />
      <View style={{ flex: 1, minWidth: 0 }}>
        <Text style={styles.containerName}>{name}</Text>
        <Text style={styles.containerImage} numberOfLines={1}>{image}</Text>
      </View>
      <Text style={styles.containerPorts}>{ports}</Text>
    </View>
  );
}

function ToggleRow({
  label, sub, value, onChange,
}: { label: string; sub?: string; value: boolean; onChange: (v: boolean) => void }) {
  return (
    <View style={styles.row}>
      <RowLeft title={label} sub={sub} />
      <Switch
        value={value}
        onValueChange={onChange}
        trackColor={{ true: cumulus.accent, false: "rgba(255,255,255,0.15)" }}
        thumbColor="#fff"
      />
    </View>
  );
}

function SelectRow({
  label, value, options, onChange,
}: { label: string; value: string; options: string[]; onChange: (v: string) => void }) {
  const [open, setOpen] = useState(false);
  return (
    <View style={{ paddingHorizontal: 14, paddingVertical: 8 }}>
      <TouchableOpacity
        style={{ flexDirection: "row", alignItems: "center", paddingVertical: 4 }}
        onPress={() => setOpen(!open)}
        activeOpacity={0.7}
      >
        <Text style={[styles.rowLabel, { flex: 1 }]}>{label}</Text>
        <Text style={styles.monoDim}>{value}</Text>
        <Text style={styles.chev}>{open ? "▾" : "▸"}</Text>
      </TouchableOpacity>
      {open && (
        <View style={{ flexDirection: "row", flexWrap: "wrap", gap: 6, paddingTop: 6 }}>
          {options.map((opt) => (
            <TouchableOpacity
              key={opt}
              onPress={() => { onChange(opt); setOpen(false); }}
              style={[
                styles.selectChip,
                opt === value && { backgroundColor: cumulus.accent },
              ]}
            >
              <Text style={styles.selectChipText}>{opt}</Text>
            </TouchableOpacity>
          ))}
        </View>
      )}
    </View>
  );
}

function Segmented({
  options, selected, onSelect,
}: { options: string[]; selected: string; onSelect: (v: string) => void }) {
  return (
    <View style={styles.segmented}>
      {options.map((opt) => (
        <TouchableOpacity
          key={opt}
          style={[styles.segment, opt === selected && styles.segmentSelected]}
          onPress={() => onSelect(opt)}
        >
          <Text
            style={[styles.segmentText, opt === selected && styles.segmentTextSelected]}
          >
            {opt}
          </Text>
        </TouchableOpacity>
      ))}
    </View>
  );
}

function BigBtn({ children, primary }: { children: string; primary?: boolean }) {
  return (
    <TouchableOpacity
      style={[styles.bigBtn, primary ? styles.bigBtnPrimary : styles.bigBtnGhost]}
      activeOpacity={0.8}
    >
      <Text style={styles.bigBtnText}>{children}</Text>
    </TouchableOpacity>
  );
}

const DOCKER_CONTAINERS: Array<{
  name: string;
  image: string;
  status: "healthy" | "updating" | "error";
  ports: string;
}> = [
  { name: "tile-server", image: "ghcr.io/radar-ng/tile-server:latest", status: "healthy", ports: "8080→80" },
  { name: "ingest-mrms", image: "ghcr.io/radar-ng/ingest-mrms:latest", status: "healthy", ports: "—" },
  { name: "ingest-hrrr", image: "ghcr.io/radar-ng/ingest-hrrr:latest", status: "healthy", ports: "—" },
  { name: "nowcast-pysteps", image: "ghcr.io/radar-ng/nowcast:latest", status: "updating", ports: "—" },
  { name: "open-meteo", image: "ghcr.io/open-meteo/open-meteo:latest", status: "healthy", ports: "8081→8080" },
  { name: "basemap", image: "protomaps/go-pmtiles:serve", status: "healthy", ports: "8082→8081" },
  { name: "caddy", image: "caddy:2.7", status: "healthy", ports: "443→443" },
];

const styles = StyleSheet.create({
  container: { flex: 1 },
  flex: { flex: 1 },
  scroll: { paddingHorizontal: 0, paddingTop: 0, paddingBottom: 40 },

  headerRow: {
    flexDirection: "row",
    alignItems: "center",
    paddingHorizontal: 20,
    paddingTop: 4,
    paddingBottom: 6,
  },
  title: { fontSize: 28, fontWeight: "700", color: cumulus.ink, letterSpacing: -0.5 },

  hero: {
    marginHorizontal: 14,
    padding: 14,
    borderRadius: 18,
    backgroundColor: "rgba(139,124,255,0.14)",
    borderWidth: 1,
    borderColor: "rgba(139,124,255,0.3)",
  },
  heroHeader: {
    flexDirection: "row",
    alignItems: "center",
    justifyContent: "space-between",
    marginBottom: 8,
  },
  heroKicker: {
    fontSize: 10,
    fontFamily: "SF Mono",
    letterSpacing: 1.6,
    color: "#C7BDFF",
    fontWeight: "700",
  },
  heroName: { fontSize: 16, fontWeight: "700", color: cumulus.ink, marginBottom: 2 },
  heroUrl: { fontSize: 12, fontFamily: "SF Mono", color: cumulus.inkDim },
  heroStatsRow: { flexDirection: "row", marginTop: 14, gap: 10 },

  statusRow: { flexDirection: "row", alignItems: "center", gap: 6 },
  statusDot: {
    width: 7, height: 7, borderRadius: 4,
    shadowOpacity: 0.8, shadowRadius: 4, shadowOffset: { width: 0, height: 0 },
  },
  statusLabel: {
    fontSize: 10.5,
    fontFamily: "SF Mono",
    fontWeight: "700",
    letterSpacing: 0.8,
  },

  statLabel: { fontSize: 9, fontFamily: "SF Mono", color: cumulus.inkMuted, letterSpacing: 1.2, fontWeight: "700" },
  statValue: { fontSize: 15, fontWeight: "700", color: cumulus.ink, marginTop: 2 },

  sectionHeader: {
    paddingHorizontal: 20,
    paddingTop: 18,
    paddingBottom: 6,
    fontSize: 10,
    fontFamily: "SF Mono",
    color: cumulus.inkFaint,
    letterSpacing: 1.6,
    fontWeight: "700",
    textTransform: "uppercase",
  },

  card: {
    marginHorizontal: 14,
    backgroundColor: cumulus.card,
    borderWidth: 1,
    borderColor: cumulus.cardLine,
    borderRadius: 16,
    overflow: "hidden",
  },
  sep: { height: StyleSheet.hairlineWidth, backgroundColor: "rgba(255,255,255,0.07)", marginLeft: 14 },

  urlRow: { flexDirection: "row", alignItems: "center", padding: 10, gap: 8 },
  urlInput: {
    flex: 1,
    color: cumulus.ink,
    fontFamily: "SF Mono",
    fontSize: 12,
    backgroundColor: "rgba(0,0,0,0.25)",
    borderRadius: 10,
    borderWidth: StyleSheet.hairlineWidth,
    borderColor: "rgba(255,255,255,0.08)",
    paddingHorizontal: 10,
    paddingVertical: 8,
  },
  saveBtn: {
    paddingHorizontal: 14,
    paddingVertical: 8,
    borderRadius: 10,
    backgroundColor: cumulus.accent,
  },
  saveBtnDisabled: { backgroundColor: "rgba(255,255,255,0.1)" },
  saveBtnText: { color: "#fff", fontWeight: "700", fontSize: 13 },

  row: { flexDirection: "row", alignItems: "center", paddingHorizontal: 14, paddingVertical: 12, gap: 10 },
  rowLabel: { fontSize: 14, color: cumulus.ink, fontWeight: "500" },
  rowSub: { fontSize: 11, color: cumulus.inkMuted, fontFamily: "SF Mono", marginTop: 2 },
  monoDim: { fontSize: 12, fontFamily: "SF Mono", color: cumulus.inkMuted },

  sourceRow: {
    flexDirection: "row",
    alignItems: "center",
    paddingHorizontal: 14,
    paddingVertical: 12,
    gap: 12,
  },
  sourceIcon: { fontSize: 20 },
  sourceName: { fontSize: 14, fontWeight: "500", color: cumulus.ink, marginBottom: 2 },
  sourceEndpoint: { fontSize: 11, fontFamily: "SF Mono", color: cumulus.inkMuted },
  chev: { color: cumulus.inkMuted, fontSize: 20, marginLeft: 4 },

  pill: {
    paddingHorizontal: 8,
    paddingVertical: 3,
    borderRadius: 6,
    borderWidth: StyleSheet.hairlineWidth,
  },
  pillText: { fontSize: 10, fontWeight: "800", letterSpacing: 0.8, fontFamily: "SF Mono" },

  editor: {
    marginHorizontal: 10,
    marginTop: 4,
    marginBottom: 10,
    padding: 14,
    borderRadius: 14,
    backgroundColor: "rgba(139,124,255,0.08)",
    borderWidth: 1,
    borderColor: "rgba(139,124,255,0.25)",
  },
  editorKicker: {
    fontSize: 10,
    fontFamily: "SF Mono",
    color: "#C7BDFF",
    letterSpacing: 1.2,
    fontWeight: "700",
    marginBottom: 10,
  },
  editorHint: {
    fontSize: 11,
    color: cumulus.inkMuted,
    lineHeight: 16,
    marginTop: 6,
  },

  fieldLabel: {
    fontSize: 9,
    fontFamily: "SF Mono",
    color: cumulus.inkFaint,
    letterSpacing: 1.2,
    fontWeight: "700",
    marginBottom: 4,
  },
  fieldBox: {
    paddingHorizontal: 10,
    paddingVertical: 8,
    backgroundColor: "rgba(0,0,0,0.3)",
    borderRadius: 8,
    borderWidth: StyleSheet.hairlineWidth,
    borderColor: "rgba(255,255,255,0.08)",
  },
  fieldValue: { fontFamily: "SF Mono", fontSize: 12, color: cumulus.ink },

  containerRow: { flexDirection: "row", alignItems: "center", paddingHorizontal: 14, paddingVertical: 10, gap: 10 },
  containerDot: {
    width: 7, height: 7, borderRadius: 4,
    shadowOpacity: 0.7, shadowRadius: 3, shadowOffset: { width: 0, height: 0 },
  },
  containerName: { fontSize: 13, fontFamily: "SF Mono", fontWeight: "700", color: cumulus.ink },
  containerImage: { fontSize: 10, fontFamily: "SF Mono", color: cumulus.inkMuted },
  containerPorts: { fontSize: 10, fontFamily: "SF Mono", color: cumulus.inkMuted },

  btnRow: { flexDirection: "row", paddingHorizontal: 14, paddingTop: 10, gap: 10 },
  bigBtn: { flex: 1, paddingVertical: 12, borderRadius: 14, alignItems: "center" },
  bigBtnPrimary: { backgroundColor: cumulus.accent },
  bigBtnGhost: { backgroundColor: "rgba(255,255,255,0.08)", borderWidth: 1, borderColor: cumulus.inkLine },
  bigBtnText: { color: cumulus.ink, fontSize: 14, fontWeight: "700" },

  progressTrack: { height: 4, borderRadius: 2, backgroundColor: "rgba(255,255,255,0.08)", overflow: "hidden" },
  progressFill: { height: "100%", backgroundColor: cumulus.accent },
  clearBtn: { color: "#FF6E7A", fontWeight: "700", fontSize: 13 },

  selectChip: {
    paddingHorizontal: 10,
    paddingVertical: 5,
    borderRadius: 8,
    backgroundColor: "rgba(255,255,255,0.08)",
  },
  selectChipText: { color: cumulus.ink, fontSize: 12, fontFamily: "SF Mono" },

  segmented: {
    flexDirection: "row",
    backgroundColor: "rgba(255,255,255,0.08)",
    borderRadius: 10,
    padding: 2,
  },
  segment: { paddingHorizontal: 14, paddingVertical: 6, borderRadius: 8 },
  segmentSelected: { backgroundColor: cumulus.accent },
  segmentText: { color: cumulus.inkMuted, fontWeight: "600", fontSize: 13 },
  segmentTextSelected: { color: "#fff" },

  footer: {
    marginTop: 20,
    textAlign: "center",
    color: cumulus.inkFaint,
    fontSize: 11,
    lineHeight: 16,
    fontFamily: "SF Mono",
  },
});
