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
import { useIsFocused, useRouter } from "expo-router";
import { useAppActive } from "../../hooks/useAppActive";
import { useWeatherStore } from "../../stores/useWeatherStore";
import { DEFAULTS } from "../../lib/constants";
import { radarButtonAccessibilityLabel } from "../../lib/radarStatus";
import { buildSelfHostedTileUrl } from "../../lib/tileUrl";
import { useRadarStatus } from "../../hooks/useRadarStatus";
import { useBasemapStyle } from "../../hooks/useBasemapStyle";
import { useWeatherClearTheme } from "../../theme/WeatherClearThemeProvider";
import { RadarStatusChip } from "../map/RadarStatusChip";
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
  const { style: patchedStyle } = useBasemapStyle(serverUrl, theme.dark ? "dark" : "light");

  const { status, nowFrame, layerKey } = useRadarStatus();
  const radarUrl = nowFrame ? buildSelfHostedTileUrl(serverUrl, layerKey, nowFrame.path, activePalette) : null;
  // Source and layer ids carry the frame: iOS can reuse a stale native source whose id is reused.
  const frameKey = `${activePalette}-${layerKey}-${nowFrame?.path ?? "none"}`;
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
            // A SurfaceView inside Home's ScrollView tears and ghosts when
            // scrolled; a TextureView composites with the rest of the screen.
            androidView="texture"
          >
            <Camera center={[lon, lat]} zoom={MINI_ZOOM} minZoom={MINI_ZOOM} maxZoom={MINI_ZOOM} />
            {radarUrl ? (
              <RasterSource
                id={`home-radar-source-${frameKey}`}
                key={frameKey}
                tiles={[radarUrl]}
                tileSize={256}
                minzoom={MINI_SOURCE_MIN_ZOOM}
                maxzoom={MINI_SOURCE_MAX_ZOOM}
              >
                <Layer
                  type="raster"
                  id={`home-radar-layer-${frameKey}`}
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
      <RadarStatusChip status={status} style={styles.statusBadge} />

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
  statusBadge: { position: "absolute", top: 10, left: 12 },
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
