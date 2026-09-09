/**
 * Home-screen mini-radar — compact, non-interactive MapLibre preview.
 *
 * Live data source: tile-server manifest gives latest radar timestamp,
 * then the same basemap + raster overlay stack as the dedicated Radar tab
 * renders a card-sized view centered on the user's location.
 */
import { useMemo } from "react";
import { View, Text, StyleSheet, Pressable } from "react-native";
import { Camera, Layer, Map, RasterSource } from "@maplibre/maplibre-react-native";
import { useRouter } from "expo-router";
import { useIsFocused } from "expo-router/react-navigation";
import { useAppActive } from "../../hooks/useAppActive";
import { useWeatherStore } from "../../stores/useWeatherStore";
import { DEFAULTS } from "../../lib/constants";
import {
  radarButtonAccessibilityLabel,
  radarQueryIsOffline,
  radarStatus,
} from "../../lib/radarStatus";
import { buildSelfHostedTileUrl } from "../../lib/tileUrl";
import { pickNowFrameIndex, useManifestQuery } from "../../hooks/useManifest";
import { usePatchedMapStyle } from "../map/WeatherMap";
import { useWeatherClearTheme } from "../../theme/WeatherClearThemeProvider";
import type { LayerType, RadarFrame } from "../../types/weather";
import type { WeatherClearTheme } from "../../theme/weatherClearTheme";

// City/metro level — matches the dedicated map's default zoom.
const MINI_ZOOM = 8;
const MINI_SOURCE_MIN_ZOOM = 4;
const MINI_SOURCE_MAX_ZOOM = 7;

export function RadarMiniMap() {
  const focused = useIsFocused();
  const appActive = useAppActive();
  const router = useRouter();
  const { theme } = useWeatherClearTheme();
  const styles = useMemo(() => createStyles(theme), [theme]);
  const serverUrl = useWeatherStore((s) => s.serverUrl);
  const activePalette = useWeatherStore((s) => s.activePalette);
  const lat = useWeatherStore((s) => s.latitude) ?? DEFAULTS.LATITUDE;
  const lon = useWeatherStore((s) => s.longitude) ?? DEFAULTS.LONGITUDE;
  const patchedStyle = usePatchedMapStyle(serverUrl, theme.dark ? "dark" : "light");

  const { data: manifest, dataUpdatedAt, isError, isPaused, isPending } = useManifestQuery();

  // Match the dedicated Radar tab's observed MRMS source first. Composite is
  // only a compatibility fallback for clusters that still publish it.
  const layerKey = manifest?.layers?.radar
    ? "radar"
    : manifest?.layers?.["radar-composite"]
      ? "radar-composite"
      : "radar";
  const layer = manifest?.layers?.[layerKey];
  const frames = useMemo<RadarFrame[]>(() => (
    (layer?.frames ?? layer?.timestamps.map((timestamp) => ({ timestamp, path: timestamp })) ?? []).map((frame) => ({
      time: Math.floor(new Date(frame.timestamp).getTime() / 1000),
      timestamp: frame.timestamp,
      path: frame.path,
      source: "radar",
    })) ?? []
  ), [layer?.frames, layer?.timestamps]);
  const nowFrameIndex = pickNowFrameIndex(frames);
  const nowFrame = nowFrameIndex >= 0 ? frames[nowFrameIndex] : null;
  const radarUrl = nowFrame ? buildSelfHostedTileUrl(serverUrl, layerKey as LayerType, nowFrame.path, activePalette) : null;
  // Tracking dataUpdatedAt makes a successful no-change poll rerender this
  // clock-derived badge instead of leaving LIVE frozen during an ingest stall.
  const statusNow = dataUpdatedAt > 0 ? Math.max(Date.now(), dataUpdatedAt) : Date.now();
  const status = radarStatus({
    frameTimeSeconds: nowFrame?.time ?? null,
    refreshFailed: radarQueryIsOffline(isError, isPaused),
    loading: isPending,
    nowMilliseconds: statusNow,
  });
  const statusColor = {
    live: theme.colors.success,
    stale: theme.colors.warning,
    offline: theme.colors.destructive,
    unavailable: theme.colors.textFaint,
  }[status.tone];
  const fallbackHeadline = nowFrame
    ? "Explore nearby precipitation"
    : status.tone === "unavailable" && status.label !== "LOADING"
      ? "Radar unavailable"
      : "Loading…";

  return (
    <Pressable
      accessibilityRole="button"
      accessibilityLabel={radarButtonAccessibilityLabel(status)}
      onPress={() => router.push("/radar")}
      style={({ pressed }) => [styles.wrap, pressed && { opacity: 0.85 }]}
    >
      <View style={styles.mapWrap} pointerEvents="none">
        {patchedStyle && focused && appActive ? (
          <Map
            style={styles.map}
            mapStyle={patchedStyle}
            logo={false}
            attribution={false}
            dragPan={false}
            touchZoom={false}
            doubleTapZoom={false}
            doubleTapHoldZoom={false}
            touchRotate={false}
            touchPitch={false}
            preferredFramesPerSecond={30}
          >
            <Camera center={[lon, lat]} zoom={MINI_ZOOM} minZoom={MINI_ZOOM} maxZoom={MINI_ZOOM} />
            {radarUrl ? (
              <RasterSource
                id="home-radar-source"
                key={`${activePalette}-${layerKey}-${nowFrame?.path ?? "none"}`}
                tiles={[radarUrl]}
                tileSize={256}
                minzoom={MINI_SOURCE_MIN_ZOOM}
                maxzoom={MINI_SOURCE_MAX_ZOOM}
              >
                <Layer
                  type="raster"
                  id="home-radar-layer"
                  paint={{
                    "raster-opacity": 0.42,
                    "raster-fade-duration": 0,
                  }}
                />
              </RasterSource>
            ) : null}
          </Map>
        ) : (
          <View style={styles.basemapTint} />
        )}
        <View style={styles.mapVignette} />

      {/* user-location pin */}
      <View style={styles.pinWrap} pointerEvents="none">
        <View style={styles.pinRing} />
        <View style={styles.pinDot} />
      </View>

      {/* Data freshness badge */}
      <View style={styles.statusBadge}>
        <View style={[styles.statusDot, { backgroundColor: statusColor }]} />
        <Text style={styles.statusText}>{status.label}</Text>
      </View>

      </View>

      {/* Keep copy outside the native map surface so labels cannot collide. */}
      <View style={styles.footer}>
        <View style={{ flex: 1 }}>
          <Text style={styles.footerLabel}>Weather radar</Text>
          <Text style={styles.footerTitle} numberOfLines={1}>
            {fallbackHeadline}
          </Text>
        </View>
        <View style={styles.chevronBox}>
          <Text style={styles.chevron}>{"›"}</Text>
        </View>
      </View>
    </Pressable>
  );
}

function createStyles(theme: WeatherClearTheme) {
  return StyleSheet.create({
  wrap: {
    marginHorizontal: 16,
    marginTop: 18,
    borderCurve: "continuous",
    borderRadius: 18,
    borderWidth: 1,
    borderColor: theme.colors.border,
    overflow: "hidden",
    backgroundColor: theme.colors.surface,
  },
  mapWrap: {
    height: 176,
    overflow: "hidden",
    backgroundColor: theme.colors.surfaceMuted,
  },
  map: {
    flex: 1,
  },
  basemapTint: {
    ...StyleSheet.absoluteFill,
    backgroundColor: "#e8ece5",
  },
  mapVignette: {
    ...StyleSheet.absoluteFill,
    backgroundColor: "rgba(255,255,255,0.03)",
    borderWidth: 1,
    borderColor: "rgba(255,255,255,0.08)",
  },
  pinWrap: {
    position: "absolute",
    left: "50%",
    top: "50%",
    width: 18,
    height: 18,
    marginLeft: -9,
    marginTop: -9,
    alignItems: "center",
    justifyContent: "center",
  },
  pinRing: {
    position: "absolute",
    width: 18,
    height: 18,
    borderRadius: 9,
    borderWidth: 2,
    borderColor: "rgba(255,255,255,0.85)",
  },
  pinDot: {
    width: 6,
    height: 6,
    borderRadius: 3,
    backgroundColor: theme.colors.accent,
  },
  statusBadge: {
    position: "absolute",
    top: 10,
    left: 12,
    flexDirection: "row",
    alignItems: "center",
    gap: 5,
    // A near-opaque scrim keeps the white status readable over any map tile.
    backgroundColor: "rgba(0,0,0,0.88)",
    paddingHorizontal: 8,
    paddingVertical: 3,
    borderRadius: 8,
  },
  statusDot: { width: 6, height: 6, borderRadius: 3 },
  statusText: {
    color: "#ffffff",
    fontSize: 10,
    fontFamily: theme.typography.uiBold,
    letterSpacing: 1.4,
  },
  footer: {
    flexDirection: "row",
    alignItems: "center",
    gap: theme.spacing.md,
    backgroundColor: theme.colors.surface,
    padding: theme.spacing.lg,
    minHeight: 72,
  },
  footerLabel: {
    color: theme.colors.text,
    fontSize: 17,
    fontFamily: theme.typography.uiSemibold,
  },
  footerTitle: {
    color: theme.colors.textSecondary,
    fontSize: 13,
    fontFamily: theme.typography.ui,
    marginTop: 4,
  },
  chevronBox: {
    width: 28,
    height: 28,
    borderRadius: 9,
    backgroundColor: theme.colors.surfaceMuted,
    alignItems: "center",
    justifyContent: "center",
  },
  chevron: { color: theme.colors.textSecondary, fontSize: 24 },
  });
}
