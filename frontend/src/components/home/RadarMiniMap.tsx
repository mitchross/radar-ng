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
import { useQuery } from "@tanstack/react-query";
import { useRouter } from "expo-router";
import { useWeatherStore } from "../../stores/useWeatherStore";
import { fetchSelfHostedManifest } from "../../lib/api";
import { DEFAULTS } from "../../lib/constants";
import { buildSelfHostedTileUrl } from "../../lib/tileUrl";
import { pickNowFrameIndex } from "../../hooks/useManifest";
import { usePatchedMapStyle } from "../map/WeatherMap";
import { useWeatherClearTheme } from "../../theme/WeatherClearThemeProvider";
import type { LayerType, RadarFrame } from "../../types/weather";
import type { WeatherClearTheme } from "../../theme/weatherClearTheme";

// City/metro level — matches the dedicated map's default zoom.
const MINI_ZOOM = 8;
const MINI_SOURCE_MIN_ZOOM = 4;
const MINI_SOURCE_MAX_ZOOM = 7;

export function RadarMiniMap({ headline }: { headline?: string }) {
  const router = useRouter();
  const { theme } = useWeatherClearTheme();
  const styles = useMemo(() => createStyles(theme), [theme]);
  const serverUrl = useWeatherStore((s) => s.serverUrl);
  const activePalette = useWeatherStore((s) => s.activePalette);
  const lat = useWeatherStore((s) => s.latitude) ?? DEFAULTS.LATITUDE;
  const lon = useWeatherStore((s) => s.longitude) ?? DEFAULTS.LONGITUDE;
  const patchedStyle = usePatchedMapStyle(serverUrl, "light");

  const { data: manifest } = useQuery({
    queryKey: ["manifest", serverUrl, "mini"],
    queryFn: () => fetchSelfHostedManifest(serverUrl),
    refetchInterval: 60_000,
    staleTime: 30_000,
  });

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

  return (
    <View style={styles.wrap}>
      <View style={styles.mapWrap} pointerEvents="none">
        {patchedStyle ? (
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
      </View>

      {/* user-location pin */}
      <View style={styles.pinWrap} pointerEvents="none">
        <View style={styles.pinRing} />
        <View style={styles.pinDot} />
      </View>

      {/* LIVE badge */}
      <View style={styles.liveBadge}>
        <View style={styles.liveDot} />
        <Text style={styles.liveText}>LIVE</Text>
      </View>

      {/* footer label */}
      <View style={styles.footer}>
        <View style={{ flex: 1 }}>
          <Text style={styles.footerLabel}>RADAR</Text>
          <Text style={styles.footerTitle} numberOfLines={1}>
            {headline ?? (nowFrame ? "Tap to open full radar" : "Loading…")}
          </Text>
        </View>
        <View style={styles.chevronBox}>
          <Text style={styles.chevron}>{"›"}</Text>
        </View>
      </View>
      <Pressable
        accessibilityRole="button"
        accessibilityLabel={`Open full radar. ${headline ?? (nowFrame ? "Live radar available" : "Radar loading")}`}
        style={styles.hitArea}
        onPress={() => router.push("/radar")}
      />
    </View>
  );
}

function createStyles(theme: WeatherClearTheme) {
  return StyleSheet.create({
  wrap: {
    marginHorizontal: 16,
    marginTop: 18,
    height: 180,
    borderRadius: 18,
    borderWidth: 1,
    borderColor: theme.colors.border,
    overflow: "hidden",
    backgroundColor: "#0d1428",
  },
  hitArea: {
    ...StyleSheet.absoluteFill,
    zIndex: 10,
  },
  mapWrap: {
    ...StyleSheet.absoluteFill,
    backgroundColor: "#e8ece5",
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
  liveBadge: {
    position: "absolute",
    top: 10,
    left: 12,
    flexDirection: "row",
    alignItems: "center",
    gap: 5,
    backgroundColor: "rgba(0,0,0,0.45)",
    paddingHorizontal: 8,
    paddingVertical: 3,
    borderRadius: 8,
  },
  liveDot: { width: 6, height: 6, borderRadius: 3, backgroundColor: theme.colors.success },
  liveText: {
    color: theme.colors.success,
    fontSize: 10,
    fontFamily: theme.typography.uiBold,
    letterSpacing: 1.4,
  },
  footer: {
    position: "absolute",
    left: 12,
    right: 12,
    bottom: 10,
    flexDirection: "row",
    alignItems: "center",
    backgroundColor: "rgba(0,0,0,0.45)",
    borderRadius: 10,
    paddingHorizontal: 10,
    paddingVertical: 8,
  },
  footerLabel: {
    color: "rgba(255,255,255,0.7)",
    fontSize: 10,
    fontFamily: theme.typography.uiSemibold,
    letterSpacing: 1.6,
  },
  footerTitle: {
    color: "#ffffff",
    fontSize: 14,
    fontFamily: theme.typography.uiSemibold,
    marginTop: 2,
  },
  chevronBox: {
    width: 28,
    height: 28,
    borderRadius: 9,
    backgroundColor: "rgba(255,255,255,0.18)",
    alignItems: "center",
    justifyContent: "center",
  },
  chevron: { color: "#ffffff", fontSize: 18 },
  });
}
