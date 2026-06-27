/**
 * Cumulus Settings tab — Redesigned for Editorial Light.
 * Self-hosted stack controls gated under Advanced mode.
 */
import { useCallback, useEffect, useMemo, useState } from "react";
import {
  ScrollView,
  View,
  Text,
  StyleSheet,
  Pressable,
  Switch,
  TextInput,
  ActivityIndicator,
  RefreshControl,
} from "react-native";
import { SafeAreaView } from "react-native-safe-area-context";
import { LinearGradient } from "expo-linear-gradient";
import Slider from "@react-native-community/slider";
import { useWeatherStore } from "../../stores/useWeatherStore";
import { checkServerHealth } from "../../lib/api";
import { activeLocationLabel, formatPlaceLabel } from "../../lib/locationLabel";
import { SELF_HOSTED } from "../../lib/constants";
import { CONDITION_GRADIENTS } from "../../lib/cumulusTheme";
import { PaletteSelector } from "../../components/palette/PaletteSelector";
import {
  SectionLabel,
  SegmentedControl,
} from "../../components/ui/WeatherClearUI";
import { useCitySearch } from "../../hooks/useCitySearch";
import type { SelectedPlace } from "../../types/location";
import { useWeatherClearTheme } from "../../theme/WeatherClearThemeProvider";
import type { WeatherClearTheme } from "../../theme/weatherClearTheme";

type SourceKey = "radar" | "satellite" | "forecast" | "basemap" | "alerts";
type SourceStatus = "healthy" | "stale" | "error" | "disabled";

function useSettingsTheme() {
  const { theme } = useWeatherClearTheme();
  const styles = useMemo(() => createStyles(theme), [theme]);
  return { theme, styles };
}

export default function SettingsScreen() {
  const { theme, styles } = useSettingsTheme();
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
  const locationMode = useWeatherStore((s) => s.locationMode);
  const selectedPlace = useWeatherStore((s) => s.selectedPlace);
  const devicePlace = useWeatherStore((s) => s.devicePlace);
  const setSelectedPlace = useWeatherStore((s) => s.setSelectedPlace);
  const useDeviceLocation = useWeatherStore((s) => s.useDeviceLocation);

  const viewMode = useWeatherStore((s) => s.viewMode);
  const setViewMode = useWeatherStore((s) => s.setViewMode);
  const appearanceMode = useWeatherStore((s) => s.appearanceMode);
  const setAppearanceMode = useWeatherStore((s) => s.setAppearanceMode);

  const [editing, setEditing] = useState<SourceKey | null>(null);
  const [urlDraft, setUrlDraft] = useState(serverUrl);
  const [cityQuery, setCityQuery] = useState("");
  const [debouncedCityQuery, setDebouncedCityQuery] = useState("");
  const [stackHealthy, setStackHealthy] = useState<boolean | null>(null);
  const [refreshLabel, setRefreshLabel] = useState("2 min");
  const [refreshTick, setRefreshTick] = useState(0);
  const [refreshing, setRefreshing] = useState(false);

  useEffect(() => {
    let cancelled = false;
    setStackHealthy(null);
    checkServerHealth(serverUrl).then((ok) => {
      if (!cancelled) setStackHealthy(ok);
    });
    return () => {
      cancelled = true;
    };
  }, [serverUrl, refreshTick]);

  useEffect(() => {
    const timer = setTimeout(() => setDebouncedCityQuery(cityQuery), 250);
    return () => clearTimeout(timer);
  }, [cityQuery]);

  const {
    data: cityResults = [],
    isFetching: citySearching,
    error: citySearchError,
  } = useCitySearch(debouncedCityQuery);

  const onRefresh = useCallback(async () => {
    setRefreshing(true);
    setRefreshTick((t) => t + 1);
    try {
      await checkServerHealth(serverUrl);
    } finally {
      setRefreshing(false);
    }
  }, [serverUrl]);

  const sources = useMemo(
    () => buildSources(serverUrl, stackHealthy),
    [serverUrl, stackHealthy]
  );

  const stackHost = useMemo(() => hostOf(serverUrl), [serverUrl]);
  const locationLabel = activeLocationLabel(locationMode, selectedPlace, devicePlace);
  const isAdv = viewMode === "advanced";
  const gradient = theme.dark
    ? ([theme.colors.canvas, theme.colors.surfaceStrong] as const)
    : CONDITION_GRADIENTS.clearNight;

  return (
    <LinearGradient
      accessibilityLabel="Weather settings"
      colors={gradient}
      style={styles.container}
    >
      <SafeAreaView style={styles.flex} edges={["top"]}>
        <ScrollView
          style={styles.flex}
          contentContainerStyle={styles.scroll}
          showsVerticalScrollIndicator={false}
          refreshControl={
            <RefreshControl
              refreshing={refreshing}
              onRefresh={onRefresh}
              tintColor={theme.colors.text}
              colors={[theme.colors.accent]}
            />
          }
        >
          {/* Header Row with Toggle */}
          <View style={styles.headerRow}>
            <Text style={styles.title}>Settings</Text>

            <SegmentedControl
              accessibilityLabel="Settings detail"
              options={[
                { label: "Simple", value: "simple" },
                { label: "Adv", value: "advanced" },
              ]}
              value={viewMode}
              onChange={setViewMode}
            />
          </View>

          {/* Advanced Mode: Self-hosted stack hero card */}
          {isAdv && (
            <View style={styles.hero}>
              <View style={styles.heroHeader}>
                <Text style={styles.heroKicker}>SELF-HOSTED STACK</Text>
                <StatusDot ok={stackHealthy === true} loading={stackHealthy === null} />
              </View>
              <Text style={styles.heroName}>radar-ng</Text>
              <Text style={styles.heroUrl}>{stackHost}</Text>
              <View style={styles.heroStatsRow}>
                <Stat label="UPTIME" value="14d" />
                <Stat label="TILES/DAY" value="48.2k" />
                <Stat label="CACHE" value="87%" />
              </View>
            </View>
          )}

          <SectionHeader>Location</SectionHeader>
          <View style={styles.card}>
            <Row>
              <RowLeft title="Radar location" sub={locationLabel} />
              <Pressable
                accessibilityRole="button"
                accessibilityLabel="Use device GPS location"
                accessibilityState={{ disabled: locationMode === "device" }}
                style={[styles.saveBtn, locationMode === "device" ? styles.saveBtnDisabled : null]}
                disabled={locationMode === "device"}
                onPress={useDeviceLocation}
              >
                <Text style={styles.saveBtnText}>Use GPS</Text>
              </Pressable>
            </Row>
            <Sep />
            <View style={styles.locationSearch}>
              <View style={styles.searchRow}>
                <TextInput
                  style={styles.searchInput}
                  value={cityQuery}
                  onChangeText={setCityQuery}
                  autoCapitalize="words"
                  autoCorrect={false}
                  placeholder="Search city"
                  accessibilityLabel="Search city"
                  placeholderTextColor={theme.colors.textFaint}
                />
                {citySearching ? <ActivityIndicator size="small" color={theme.colors.textMuted} /> : null}
              </View>
              {citySearchError ? (
                <Text style={styles.searchError}>City search unavailable</Text>
              ) : null}
              {cityResults.map((place) => (
                <CityResultRow
                  key={place.id}
                  place={place}
                  selected={selectedPlace?.id === place.id && locationMode === "city"}
                  onPress={() => {
                    setSelectedPlace(place);
                    setCityQuery(formatPlaceLabel(place));
                  }}
                />
              ))}
            </View>
          </View>

          {/* Advanced Mode: Stack configuration URL */}
          {isAdv && (
            <>
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
                    accessibilityLabel="Stack URL"
                    placeholderTextColor={theme.colors.textFaint}
                  />
                  <Pressable
                    accessibilityRole="button"
                    accessibilityLabel="Save stack URL"
                    style={[
                      styles.saveBtn,
                      urlDraft === serverUrl ? styles.saveBtnDisabled : null,
                    ]}
                    disabled={urlDraft === serverUrl}
                    onPress={() => setServerUrl(urlDraft.trim())}
                  >
                    <Text style={styles.saveBtnText}>Save</Text>
                  </Pressable>
                </View>
              </View>
            </>
          )}

          {/* Advanced Mode: Data Sources */}
          {isAdv && (
            <>
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
            </>
          )}

          {/* Advanced Mode: Docker stack services list */}
          {isAdv && (
            <>
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
            </>
          )}

          {/* Advanced Mode: Tile Cache details */}
          {isAdv && (
            <>
              <SectionHeader>Tile Cache</SectionHeader>
              <View style={styles.card}>
                <Row>
                  <RowLeft title="Storage" sub="3.4 GB of 8 GB used" />
                  <Text style={styles.monoDim}>42.5%</Text>
                </Row>
                <View style={{ paddingHorizontal: 14, paddingBottom: 12 }}>
                  <View style={styles.progressTrack}>
                    <View style={[styles.progressFill, { width: "42.5%" }]} />
                  </View>
                </View>
                <Sep />
                <Row>
                  <RowLeft title="Purge cache" sub="Frees radar + basemap tiles" />
                  <Text style={styles.clearBtn}>Clear</Text>
                </Row>
              </View>
            </>
          )}

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
            <Row>
              <RowLeft title="Appearance" sub="App chrome and content" />
              <SegmentedControl
                accessibilityLabel="App appearance"
                options={[
                  { label: "Light", value: "light" },
                  { label: "Dark", value: "dark" },
                  { label: "System", value: "system" },
                ]}
                value={appearanceMode}
                onChange={setAppearanceMode}
              />
            </Row>
            <Sep />
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
                minimumTrackTintColor={theme.colors.accent}
                maximumTrackTintColor={theme.colors.divider}
                thumbTintColor="#ffffff"
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
                minimumTrackTintColor={theme.colors.accent}
                maximumTrackTintColor={theme.colors.divider}
                thumbTintColor="#ffffff"
                style={{ marginTop: 4 }}
              />
            </View>
          </View>

          {/* Advanced Mode: About details */}
          {isAdv && (
            <>
              <SectionHeader>About</SectionHeader>
              <View style={styles.card}>
                <Row>
                  <RowLeft title="App version" />
                  <Text style={styles.monoDim}>1.0.0</Text>
                </Row>
                <Sep />
                <Row>
                  <RowLeft title="Server" />
                  <Text style={styles.monoDim}>{stackHost}</Text>
                </Row>
                <Sep />
                <Row>
                  <RowLeft title="Stack status" />
                  <Text
                    style={[
                      styles.monoDim,
                          { color: stackHealthy ? theme.colors.success : theme.colors.destructive },
                    ]}
                  >
                    {stackHealthy === null
                      ? "…"
                      : stackHealthy
                      ? "ONLINE"
                      : "OFFLINE"}
                  </Text>
                </Row>
              </View>
            </>
          )}

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
): { key: SourceKey; name: string; icon: string; endpoint: string; status: SourceStatus }[] {
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
  const { theme, styles } = useSettingsTheme();
  if (loading) return <ActivityIndicator size="small" color={theme.colors.textMuted} />;
  const color = ok ? theme.colors.success : theme.colors.destructive;
  const label = ok ? "ONLINE" : "ERROR";
  return (
    <View style={styles.statusRow}>
      <View style={[styles.statusDot, { backgroundColor: color }]} />
      <Text style={[styles.statusLabel, { color }]}>{label}</Text>
    </View>
  );
}

function Stat({ label, value }: { label: string; value: string }) {
  const { styles } = useSettingsTheme();
  return (
    <View style={{ flex: 1 }}>
      <Text style={styles.statLabel}>{label}</Text>
      <Text style={styles.statValue}>{value}</Text>
    </View>
  );
}

function SectionHeader({ children }: { children: string }) {
  const { styles } = useSettingsTheme();
  return (
    <View style={styles.sectionHeader}>
      <SectionLabel>{children.toUpperCase()}</SectionLabel>
    </View>
  );
}

// Divider Line
function Sep() {
  const { styles } = useSettingsTheme();
  return <View style={styles.sep} />;
}

function Row({ children }: { children: React.ReactNode }) {
  const { styles } = useSettingsTheme();
  return <View style={styles.row}>{children}</View>;
}

function RowLeft({ title, sub }: { title: string; sub?: string }) {
  const { styles } = useSettingsTheme();
  return (
    <View style={{ flex: 1, minWidth: 0 }}>
      <Text style={styles.rowLabel}>{title}</Text>
      {sub ? <Text style={styles.rowSub}>{sub}</Text> : null}
    </View>
  );
}

function Pill({ color, children }: { color: string; children: string }) {
  const { styles } = useSettingsTheme();
  return (
    <View style={[styles.pill, { backgroundColor: `${color}12`, borderColor: `${color}33` }]}>
      <Text style={[styles.pillText, { color }]}>{children}</Text>
    </View>
  );
}

function SourceRow({
  name,
  icon,
  endpoint,
  status,
  expanded,
  onPress,
}: {
  name: string;
  icon: string;
  endpoint: string;
  status: SourceStatus;
  expanded: boolean;
  onPress: () => void;
}) {
  const { theme, styles } = useSettingsTheme();
  const colors: Record<SourceStatus, string> = {
    healthy: theme.colors.success,
    stale: theme.colors.warning,
    error: theme.colors.destructive,
    disabled: theme.colors.textFaint,
  };
  const pill = { healthy: "OK", stale: "STALE", error: "ERROR", disabled: "OFF" }[status];
  return (
    <Pressable
      accessibilityRole="button"
      accessibilityLabel={`${name}, ${pill}`}
      accessibilityState={{ expanded }}
      onPress={onPress}
      style={styles.sourceRow}
    >
      <Text style={styles.sourceIcon}>{icon}</Text>
      <View style={{ flex: 1, minWidth: 0 }}>
        <Text style={styles.sourceName}>{name}</Text>
        <Text style={styles.sourceEndpoint} numberOfLines={1}>{endpoint}</Text>
      </View>
      <Pill color={colors[status]}>{pill}</Pill>
      <Text style={[styles.chev, expanded ? { transform: [{ rotate: "90deg" }] } : null]}>›</Text>
    </Pressable>
  );
}

function SourceEditor({ src }: { src: { name: string; endpoint: string } }) {
  const { styles } = useSettingsTheme();
  return (
    <View style={styles.editor}>
      <Text style={styles.editorKicker}>EDIT ENDPOINT</Text>
      <Field label="URL TEMPLATE" value={src.endpoint} />
      <Field label="AUTH" value="None" />
      <View style={{ flexDirection: "row", gap: 8 }}>
        <View style={{ flex: 1 }}>
          <Field label="TIMEOUT" value="8s" />
        </View>
        <View style={{ flex: 1 }}>
          <Field label="RETRIES" value="3" />
        </View>
      </View>
      <Text style={styles.editorHint}>
        Individual source URLs derive from the Stack URL above. Change the stack URL to
        re-point every data source.
      </Text>
    </View>
  );
}

function Field({ label, value }: { label: string; value: string }) {
  const { styles } = useSettingsTheme();
  return (
    <View style={{ marginBottom: 8 }}>
      <Text style={styles.fieldLabel}>{label}</Text>
      <View style={styles.fieldBox}>
        <Text style={styles.fieldValue} numberOfLines={1}>{value}</Text>
      </View>
    </View>
  );
}

function CityResultRow({
  place,
  selected,
  onPress,
}: {
  place: SelectedPlace;
  selected: boolean;
  onPress: () => void;
}) {
  const { theme, styles } = useSettingsTheme();
  return (
    <Pressable
      accessibilityRole="button"
      accessibilityLabel={`Use ${formatPlaceLabel(place)}`}
      accessibilityState={{ selected }}
      style={[styles.cityRow, selected ? styles.cityRowSelected : null]}
      onPress={onPress}
    >
      <View style={{ flex: 1, minWidth: 0 }}>
        <Text style={styles.cityName}>{place.name}</Text>
        <Text style={styles.cityMeta} numberOfLines={1}>
          {[place.admin1, place.country].filter(Boolean).join(" · ")}
        </Text>
      </View>
      {selected ? <Pill color={theme.colors.success}>SET</Pill> : null}
    </Pressable>
  );
}

function ContainerRow({
  name,
  image,
  status,
  ports,
}: {
  name: string;
  image: string;
  status: "healthy" | "updating" | "error";
  ports: string;
}) {
  const { theme, styles } = useSettingsTheme();
  const color = {
    healthy: theme.colors.success,
    updating: theme.colors.warning,
    error: theme.colors.destructive,
  }[status];
  return (
    <View style={styles.containerRow}>
      <View style={[styles.containerDot, { backgroundColor: color }]} />
      <View style={{ flex: 1, minWidth: 0 }}>
        <Text style={styles.containerName}>{name}</Text>
        <Text style={styles.containerImage} numberOfLines={1}>{image}</Text>
      </View>
      <Text style={styles.containerPorts}>{ports}</Text>
    </View>
  );
}

function ToggleRow({
  label,
  sub,
  value,
  onChange,
}: {
  label: string;
  sub?: string;
  value: boolean;
  onChange: (v: boolean) => void;
}) {
  const { theme, styles } = useSettingsTheme();
  return (
    <View style={styles.row}>
      <RowLeft title={label} sub={sub} />
      <Switch
        value={value}
        onValueChange={onChange}
        accessibilityLabel={label}
        trackColor={{ true: theme.colors.accent, false: theme.colors.surfaceMuted }}
        thumbColor={theme.colors.surface}
      />
    </View>
  );
}

function SelectRow({
  label,
  value,
  options,
  onChange,
}: {
  label: string;
  value: string;
  options: string[];
  onChange: (v: string) => void;
}) {
  const { theme, styles } = useSettingsTheme();
  const [open, setOpen] = useState(false);
  return (
    <View style={{ paddingHorizontal: 14, paddingVertical: 8 }}>
      <Pressable
        accessibilityRole="button"
        accessibilityLabel={`${label}, ${value}`}
        accessibilityState={{ expanded: open }}
        style={{ flexDirection: "row", alignItems: "center", paddingVertical: 4 }}
        onPress={() => setOpen(!open)}
      >
        <Text style={[styles.rowLabel, { flex: 1 }]}>{label}</Text>
        <Text style={styles.monoDim}>{value}</Text>
        <Text style={styles.chev}>{open ? "▾" : "▸"}</Text>
      </Pressable>
      {open ? (
        <View style={{ flexDirection: "row", flexWrap: "wrap", gap: 6, paddingTop: 6 }}>
          {options.map((opt) => (
            <Pressable
              key={opt}
              accessibilityRole="radio"
              accessibilityState={{ selected: opt === value }}
              onPress={() => {
                onChange(opt);
                setOpen(false);
              }}
                style={[
                  styles.selectChip,
                  opt === value ? { backgroundColor: theme.colors.accent } : null,
                ]}
              >
                <Text style={[styles.selectChipText, opt === value ? { color: "#ffffff" } : null]}>
                  {opt}
                </Text>
              </Pressable>
          ))}
        </View>
      ) : null}
    </View>
  );
}

function Segmented({
  options,
  selected,
  onSelect,
}: {
  options: string[];
  selected: string;
  onSelect: (v: string) => void;
}) {
  const { styles } = useSettingsTheme();
  return (
    <View style={styles.segmented}>
      {options.map((opt) => (
        <Pressable
          key={opt}
          accessibilityRole="radio"
          accessibilityState={{ selected: opt === selected }}
          style={[styles.segment, opt === selected ? styles.segmentSelected : null]}
          onPress={() => onSelect(opt)}
        >
          <Text style={[styles.segmentText, opt === selected ? styles.segmentTextSelected : null]}>
            {opt}
          </Text>
        </Pressable>
      ))}
    </View>
  );
}

function BigBtn({ children, primary }: { children: string; primary?: boolean }) {
  const { styles } = useSettingsTheme();
  return (
    <Pressable
      accessibilityRole="button"
      style={[styles.bigBtn, primary ? styles.bigBtnPrimary : styles.bigBtnGhost]}
    >
      <Text style={[styles.bigBtnText, primary ? { color: "#ffffff" } : null]}>{children}</Text>
    </Pressable>
  );
}

const DOCKER_CONTAINERS: {
  name: string;
  image: string;
  status: "healthy" | "updating" | "error";
  ports: string;
}[] = [
  { name: "tile-server", image: "ghcr.io/radar-ng/tile-server:latest", status: "healthy", ports: "8080→80" },
  { name: "ingest-mrms", image: "ghcr.io/radar-ng/ingest-mrms:latest", status: "healthy", ports: "—" },
  { name: "ingest-hrrr", image: "ghcr.io/radar-ng/ingest-hrrr:latest", status: "healthy", ports: "—" },
  { name: "nowcast-pysteps", image: "ghcr.io/radar-ng/nowcast:latest", status: "updating", ports: "—" },
  { name: "open-meteo", image: "ghcr.io/open-meteo/open-meteo:latest", status: "healthy", ports: "8081→8080" },
  { name: "basemap", image: "protomaps/go-pmtiles:serve", status: "healthy", ports: "8082→8081" },
  { name: "caddy", image: "caddy:2.7", status: "healthy", ports: "443→443" },
];

function createStyles(theme: WeatherClearTheme) {
  const cumulus = {
    accent: theme.colors.accent,
    alert: theme.colors.destructive,
    ok: theme.colors.success,
    ink: theme.colors.text,
    inkMuted: theme.colors.textMuted,
    inkFaint: theme.colors.textFaint,
  };
  const cumulusFonts = {
    display: theme.typography.display,
    ui: theme.typography.ui,
    mono: theme.typography.mono,
  };

  return StyleSheet.create({
  container: { flex: 1 },
  flex: { flex: 1 },
  scroll: { paddingHorizontal: 0, paddingTop: 0, paddingBottom: 140 },

  // Header row
  headerRow: {
    flexDirection: "row",
    alignItems: "center",
    justifyContent: "space-between",
    paddingHorizontal: 24,
    paddingTop: 16,
    paddingBottom: 8,
  },
  title: {
    fontSize: 34,
    fontWeight: "500",
    color: cumulus.ink,
    letterSpacing: -0.5,
    fontFamily: cumulusFonts.display,
  },

  toggleContainer: {
    flexDirection: "row",
    backgroundColor: "#eae4d8",
    borderRadius: 11,
    padding: 3,
    alignItems: "center",
  },
  toggleBtn: {
    paddingVertical: 5,
    paddingHorizontal: 9,
    borderRadius: 8,
  },
  toggleBtnActive: {
    backgroundColor: cumulus.accent,
  },
  toggleBtnText: {
    fontFamily: cumulusFonts.ui,
    fontSize: 10,
    fontWeight: "700",
    color: cumulus.inkMuted,
  },
  toggleBtnTextActive: {
    color: "#ffffff",
  },

  // Hero stack details
  hero: {
    marginHorizontal: 16,
    marginTop: 8,
    padding: 16,
    borderRadius: 20,
    backgroundColor: "#3a3266",
    borderWidth: 1,
    borderColor: "rgba(255,255,255,0.06)",
    shadowColor: "rgba(60,50,40,0.12)",
    shadowOffset: { width: 0, height: 6 },
    shadowOpacity: 0.45,
    shadowRadius: 12,
    elevation: 4,
  },
  heroHeader: {
    flexDirection: "row",
    alignItems: "center",
    justifyContent: "space-between",
    marginBottom: 8,
  },
  heroKicker: {
    fontSize: 9,
    fontFamily: cumulusFonts.mono,
    letterSpacing: 1.6,
    color: "rgba(255,255,255,0.6)",
    fontWeight: "700",
  },
  heroName: { fontSize: 26, fontWeight: "600", color: "#ffffff", fontFamily: cumulusFonts.display, marginBottom: 2 },
  heroUrl: { fontSize: 12, fontFamily: cumulusFonts.ui, color: "rgba(255,255,255,0.6)" },
  heroStatsRow: { flexDirection: "row", marginTop: 16, gap: 10 },

  statusRow: { flexDirection: "row", alignItems: "center", gap: 6 },
  statusDot: {
    width: 6,
    height: 6,
    borderRadius: 3,
  },
  statusLabel: {
    fontSize: 10,
    fontFamily: cumulusFonts.mono,
    fontWeight: "700",
    letterSpacing: 0.8,
  },

  statLabel: { fontSize: 9, fontFamily: cumulusFonts.mono, color: "rgba(255,255,255,0.5)", letterSpacing: 1.2, fontWeight: "700" },
  statValue: { fontSize: 19, fontWeight: "500", color: "#ffffff", fontFamily: cumulusFonts.display, marginTop: 3 },

  sectionHeader: {
    paddingHorizontal: 24,
    paddingTop: 24,
    paddingBottom: 8,
  },

  card: {
    marginHorizontal: 16,
    backgroundColor: theme.colors.surface,
    borderWidth: 1,
    borderColor: theme.colors.border,
    borderRadius: 20,
    overflow: "hidden",
    boxShadow: theme.dark
      ? "0 3px 10px rgba(0,0,0,0.24)"
      : "0 3px 10px rgba(60,50,40,0.05)",
  },
  sep: { height: 1, backgroundColor: theme.colors.divider, marginLeft: 16 },

  urlRow: { flexDirection: "row", alignItems: "center", padding: 10, gap: 8 },
  locationSearch: { paddingHorizontal: 12, paddingVertical: 12 },
  searchRow: { flexDirection: "row", alignItems: "center", gap: 8 },
  searchInput: {
    flex: 1,
    color: cumulus.ink,
    fontSize: 14,
    backgroundColor: theme.colors.surfaceStrong,
    borderRadius: 12,
    borderWidth: 1,
    borderColor: theme.colors.border,
    paddingHorizontal: 12,
    paddingVertical: 9,
    fontFamily: cumulusFonts.ui,
  },
  searchError: {
    color: cumulus.alert,
    fontSize: 12,
    marginTop: 8,
    fontFamily: cumulusFonts.ui,
  },
  cityRow: {
    flexDirection: "row",
    alignItems: "center",
    gap: 10,
    marginTop: 8,
    paddingHorizontal: 12,
    paddingVertical: 10,
    borderRadius: 12,
    backgroundColor: theme.colors.surface,
    borderWidth: 1,
    borderColor: theme.colors.border,
  },
  cityRowSelected: {
    backgroundColor: theme.colors.accentSoft,
    borderColor: theme.colors.accentBorder,
  },
  cityName: { fontSize: 14, color: cumulus.ink, fontWeight: "600", fontFamily: cumulusFonts.ui },
  cityMeta: { fontSize: 11, color: cumulus.inkMuted, marginTop: 2, fontFamily: cumulusFonts.ui },
  urlInput: {
    flex: 1,
    color: cumulus.ink,
    fontFamily: cumulusFonts.mono,
    fontSize: 12,
    backgroundColor: theme.colors.surfaceStrong,
    borderRadius: 12,
    borderWidth: 1,
    borderColor: theme.colors.border,
    paddingHorizontal: 12,
    paddingVertical: 8,
  },
  saveBtn: {
    paddingHorizontal: 14,
    paddingVertical: 8,
    borderRadius: 10,
    backgroundColor: cumulus.accent,
  },
  saveBtnDisabled: { backgroundColor: theme.colors.surfaceMuted },
  saveBtnText: { color: "#ffffff", fontWeight: "700", fontSize: 13, fontFamily: cumulusFonts.ui },

  row: { flexDirection: "row", alignItems: "center", paddingHorizontal: 16, paddingVertical: 14, gap: 10 },
  rowLabel: { fontSize: 14, color: cumulus.ink, fontWeight: "600", fontFamily: cumulusFonts.ui },
  rowSub: { fontSize: 11, color: cumulus.inkMuted, fontFamily: cumulusFonts.ui, marginTop: 2 },
  monoDim: { fontSize: 12, fontFamily: cumulusFonts.mono, color: cumulus.inkMuted },

  sourceRow: {
    flexDirection: "row",
    alignItems: "center",
    paddingHorizontal: 16,
    paddingVertical: 14,
    gap: 12,
  },
  sourceIcon: { fontSize: 20 },
  sourceName: { fontSize: 14, fontWeight: "600", color: cumulus.ink, marginBottom: 2, fontFamily: cumulusFonts.ui },
  sourceEndpoint: { fontSize: 11, fontFamily: cumulusFonts.mono, color: cumulus.inkMuted },
  chev: { color: cumulus.inkMuted, fontSize: 18, marginLeft: 4 },

  pill: {
    paddingHorizontal: 8,
    paddingVertical: 3,
    borderRadius: 6,
    borderWidth: 1,
  },
  pillText: { fontSize: 10, fontWeight: "800", letterSpacing: 0.8, fontFamily: cumulusFonts.mono },

  editor: {
    marginHorizontal: 12,
    marginTop: 4,
    marginBottom: 12,
    padding: 14,
    borderRadius: 16,
    backgroundColor: theme.colors.accentSoft,
    borderWidth: 1,
    borderColor: theme.colors.accentBorder,
  },
  editorKicker: {
    fontSize: 9,
    fontFamily: cumulusFonts.mono,
    color: cumulus.accent,
    letterSpacing: 1.2,
    fontWeight: "700",
    marginBottom: 10,
  },
  editorHint: {
    fontSize: 11,
    color: cumulus.inkMuted,
    lineHeight: 16,
    marginTop: 6,
    fontFamily: cumulusFonts.ui,
  },

  fieldLabel: {
    fontSize: 9,
    fontFamily: cumulusFonts.mono,
    color: cumulus.inkMuted,
    letterSpacing: 1.2,
    fontWeight: "700",
    marginBottom: 4,
  },
  fieldBox: {
    paddingHorizontal: 10,
    paddingVertical: 8,
    backgroundColor: theme.colors.surface,
    borderRadius: 8,
    borderWidth: 1,
    borderColor: theme.colors.border,
  },
  fieldValue: { fontFamily: cumulusFonts.mono, fontSize: 12, color: cumulus.ink },

  containerRow: { flexDirection: "row", alignItems: "center", paddingHorizontal: 16, paddingVertical: 12, gap: 10 },
  containerDot: {
    width: 6,
    height: 6,
    borderRadius: 3,
  },
  containerName: { fontSize: 13, fontFamily: cumulusFonts.mono, fontWeight: "700", color: cumulus.ink },
  containerImage: { fontSize: 10, fontFamily: cumulusFonts.mono, color: cumulus.inkMuted },
  containerPorts: { fontSize: 10, fontFamily: cumulusFonts.mono, color: cumulus.inkMuted },

  btnRow: { flexDirection: "row", paddingHorizontal: 16, paddingTop: 12, gap: 10 },
  bigBtn: { flex: 1, minHeight: 44, paddingVertical: 12, borderRadius: 14, alignItems: "center" },
  bigBtnPrimary: { backgroundColor: cumulus.accent },
  bigBtnGhost: { backgroundColor: theme.colors.surface, borderWidth: 1, borderColor: theme.colors.border },
  bigBtnText: { color: cumulus.ink, fontSize: 14, fontWeight: "700", fontFamily: cumulusFonts.ui },

  progressTrack: { height: 6, borderRadius: 3, backgroundColor: theme.colors.divider, overflow: "hidden" },
  progressFill: { height: "100%", backgroundColor: cumulus.accent },
  clearBtn: { color: cumulus.alert, fontWeight: "700", fontSize: 13, fontFamily: cumulusFonts.ui },

  selectChip: {
    paddingHorizontal: 10,
    paddingVertical: 5,
    borderRadius: 8,
    backgroundColor: theme.colors.surfaceStrong,
    borderWidth: 1,
    borderColor: theme.colors.border,
  },
  selectChipText: { color: cumulus.ink, fontSize: 12, fontFamily: cumulusFonts.mono },

  segmented: {
    flexDirection: "row",
    backgroundColor: theme.colors.surfaceMuted,
    borderRadius: 10,
    padding: 2,
  },
  segment: { paddingHorizontal: 14, paddingVertical: 6, borderRadius: 8 },
  segmentSelected: { backgroundColor: cumulus.accent },
  segmentText: { color: cumulus.inkMuted, fontWeight: "700", fontSize: 13, fontFamily: cumulusFonts.ui },
  segmentTextSelected: { color: "#ffffff" },

  footer: {
    marginTop: 24,
    textAlign: "center",
    color: cumulus.inkFaint,
    fontSize: 11,
    lineHeight: 16,
    fontFamily: cumulusFonts.mono,
  },
  });
}
