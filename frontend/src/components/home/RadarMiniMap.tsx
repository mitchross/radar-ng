/**
 * Home-screen mini-radar — single static tile centered on user location.
 *
 * Why a static <Image> and not a MapLibre view: MapLibre Native is a heavy
 * dependency to drag onto a tab that's also rendered server-side / on web,
 * and the home page is already cross-platform safe. A 256×256 z=6 tile
 * covers ~1000 km — enough to show "is it raining near me" without a map.
 *
 * Live data source: tile-server manifest gives latest radar timestamp,
 * we compute slippy XY from user lat/lon and fetch one PNG. Refreshes on
 * a 60s interval matching the rest of the app's cadence.
 */
import { useMemo } from "react";
import { View, Text, StyleSheet, Pressable } from "react-native";
import { Image } from "expo-image";
import { useQuery } from "@tanstack/react-query";
import { useRouter } from "expo-router";
import { useWeatherStore } from "../../stores/useWeatherStore";
import { fetchSelfHostedManifest } from "../../lib/api";
import { DEFAULTS } from "../../lib/constants";
import { useWeatherClearTheme } from "../../theme/WeatherClearThemeProvider";
import type { WeatherClearTheme } from "../../theme/weatherClearTheme";

const MINI_ZOOM = 6;

function lonLatToTile(lon: number, lat: number, z: number): { x: number; y: number } {
  const n = 2 ** z;
  const x = Math.floor(((lon + 180) / 360) * n);
  const latRad = (lat * Math.PI) / 180;
  const y = Math.floor(
    ((1 - Math.log(Math.tan(latRad) + 1 / Math.cos(latRad)) / Math.PI) / 2) * n,
  );
  return {
    x: Math.max(0, Math.min(n - 1, x)),
    y: Math.max(0, Math.min(n - 1, y)),
  };
}

export function RadarMiniMap({ headline }: { headline?: string }) {
  const router = useRouter();
  const { theme } = useWeatherClearTheme();
  const styles = useMemo(() => createStyles(theme), [theme]);
  const serverUrl = useWeatherStore((s) => s.serverUrl);
  const activePalette = useWeatherStore((s) => s.activePalette);
  const lat = useWeatherStore((s) => s.latitude) ?? DEFAULTS.LATITUDE;
  const lon = useWeatherStore((s) => s.longitude) ?? DEFAULTS.LONGITUDE;

  const { data: manifest } = useQuery({
    queryKey: ["manifest", serverUrl, "mini"],
    queryFn: () => fetchSelfHostedManifest(serverUrl),
    refetchInterval: 60_000,
    staleTime: 30_000,
  });

  const tile = useMemo(() => lonLatToTile(lon, lat, MINI_ZOOM), [lat, lon]);

  // Prefer the QC'd radar layer if the backend exposes it; fall back to the
  // legacy `radar` key. Composite gets used too if present (better visual).
  const layerKey = manifest?.layers?.["radar-composite"]
    ? "radar-composite"
    : "radar";
  const layer = manifest?.layers?.[layerKey];
  const latest = layer?.timestamps?.[layer.timestamps.length - 1];
  const radarUrl = latest
    ? `${serverUrl}/tiles/${layerKey}/${activePalette}/${encodeURIComponent(latest)}/${MINI_ZOOM}/${tile.x}/${tile.y}.png`
    : null;

  return (
    <Pressable
      accessibilityRole="button"
      accessibilityLabel={`Open full radar. ${headline ?? (latest ? "Live radar available" : "Radar loading")}`}
      style={styles.wrap}
      onPress={() => router.push("/radar")}
    >
      {/* Dark gradient background. We don't pull a basemap tile here
          because OSM's tile-usage policy forbids embedded mobile-app
          consumption (returns "Access blocked" image), and this is a
          tap-target preview anyway — the full map opens on press. */}
      <View style={styles.basemapTint} pointerEvents="none" />

      {/* radar overlay */}
      {radarUrl ? (
        <Image
          source={{ uri: radarUrl }}
          style={styles.radar}
          contentFit="cover"
        />
      ) : null}

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
            {headline ?? (latest ? "Tap to open full radar" : "Loading…")}
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
    height: 180,
    borderRadius: 18,
    borderWidth: 1,
    borderColor: theme.colors.border,
    overflow: "hidden",
    backgroundColor: "#0d1428",
  },
  basemapTint: {
    ...StyleSheet.absoluteFill,
    backgroundColor: "#0d1428",
  },
  radar: { ...StyleSheet.absoluteFill, opacity: 0.85 },
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
