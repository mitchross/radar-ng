import MapLibreGL, { type MapViewRef } from "@maplibre/maplibre-react-native";
import { Children, isValidElement, useEffect, useMemo, useRef, useState } from "react";
import { Pressable, StyleSheet, Text, View } from "react-native";
import { useWeatherStore } from "../../stores/useWeatherStore";
import { DEFAULTS, resolveMapStyleUrl } from "../../lib/constants";
import { trace } from "../../lib/telemetry";

const ZOOM_MIN = 1;
const ZOOM_MAX = 15;

MapLibreGL.setAccessToken(null);

interface WeatherMapProps {
  children?: React.ReactNode;
  onLongPress?: (lat: number, lon: number) => void;
  onCameraChanged?: (camera: { lon: number; lat: number; zoom: number }) => void;
}

/**
 * Fetch the Protomaps style JSON from the tile-server and rewrite its
 * source `tiles` / `url` entries to absolute URLs. MapLibre Native doesn't
 * always resolve root-relative paths against the fetched style URL, so
 * serving the patched JSON inline is the reliable path.
 */
function usePatchedMapStyle(serverUrl: string, mapStyle: "light" | "dark" | "satellite") {
  const styleUrl = resolveMapStyleUrl(serverUrl, mapStyle);
  const [patched, setPatched] = useState<string | null>(null);

  useEffect(() => {
    // Satellite already points at an absolute URL — no patching needed.
    if (mapStyle === "satellite") {
      setPatched(styleUrl);
      return;
    }
    let cancelled = false;
    trace(
      "map.fetchStyle",
      async (span) => {
        const r = await fetch(styleUrl);
        span.setAttribute("http.status_code", r.status);
        const json = (await r.json()) as {
          sources?: Record<string, { tiles?: string[]; url?: string }>;
        };
        const sources = json.sources ?? {};
        for (const src of Object.values(sources)) {
          if (Array.isArray(src.tiles)) {
            src.tiles = src.tiles.map((t) => (t.startsWith("http") ? t : `${serverUrl}${t}`));
          }
          if (typeof src.url === "string" && !src.url.startsWith("http")) {
            src.url = `${serverUrl}${src.url}`;
          }
        }
        if (!cancelled) setPatched(JSON.stringify(json));
      },
      { "map.style": mapStyle },
    ).catch(() => {
      // Fall back to the URL; MapLibre will at least try to fetch it.
      if (!cancelled) setPatched(styleUrl);
    });
    return () => {
      cancelled = true;
    };
  }, [serverUrl, mapStyle, styleUrl]);

  return patched;
}

export function WeatherMap({ children, onLongPress, onCameraChanged }: WeatherMapProps) {
  const mapRef = useRef<MapViewRef>(null);
  const cameraRef = useRef<MapLibreGL.CameraRef>(null);
  const mapStyle = useWeatherStore((s) => s.mapStyle);
  const serverUrl = useWeatherStore((s) => s.serverUrl);
  const latitude = useWeatherStore((s) => s.latitude);
  const longitude = useWeatherStore((s) => s.longitude);
  const initialZoom = latitude != null ? 7 : DEFAULTS.ZOOM;
  // Mirror current camera zoom so the +/- buttons can clamp without round-tripping.
  const zoomRef = useRef<number>(initialZoom);

  const patchedStyle = usePatchedMapStyle(serverUrl, mapStyle);

  const centerCoord = useMemo<[number, number]>(
    () => [
      longitude ?? DEFAULTS.LONGITUDE,
      latitude ?? DEFAULTS.LATITUDE,
    ],
    [latitude, longitude],
  );

  function zoomBy(delta: number) {
    const next = Math.max(ZOOM_MIN, Math.min(ZOOM_MAX, zoomRef.current + delta));
    zoomRef.current = next;
    cameraRef.current?.setCamera({ zoomLevel: next, animationDuration: 220 });
  }

  useEffect(() => {
    cameraRef.current?.setCamera({
      centerCoordinate: centerCoord,
      zoomLevel: initialZoom,
      animationDuration: 0,
    });
    zoomRef.current = initialZoom;
  }, [centerCoord, initialZoom]);

  if (!patchedStyle) return null;

  return (
    <View style={styles.map}>
      <MapLibreGL.MapView
        ref={mapRef}
        style={styles.map}
        mapStyle={patchedStyle}
        logoEnabled={false}
        attributionEnabled={true}
        attributionPosition={{ bottom: 8, left: 8 }}
        onLongPress={(feature) => {
          const coords = (feature?.geometry as { coordinates?: [number, number] } | undefined)?.coordinates;
          if (coords && onLongPress) onLongPress(coords[1], coords[0]);
        }}
        onRegionIsChanging={(feature) => {
          const coords = feature.geometry.coordinates as [number, number];
          zoomRef.current = feature.properties.zoomLevel;
          if (!onCameraChanged) return;
          onCameraChanged({
            lon: coords[0],
            lat: coords[1],
            zoom: feature.properties.zoomLevel,
          });
        }}
        onRegionDidChange={(feature) => {
          const coords = feature.geometry.coordinates as [number, number];
          zoomRef.current = feature.properties.zoomLevel;
          if (!onCameraChanged) return;
          onCameraChanged({
            lon: coords[0],
            lat: coords[1],
            zoom: feature.properties.zoomLevel,
          });
        }}
      >
        <MapLibreGL.Camera
          ref={cameraRef}
          defaultSettings={{
            centerCoordinate: centerCoord,
            zoomLevel: initialZoom,
          }}
        />
        {Children.toArray(children).filter(isValidElement)}
      </MapLibreGL.MapView>

      {/* Manual zoom controls — pinch still works, this is for one-handed use. */}
      <View style={styles.zoomWrap} pointerEvents="box-none">
        <Pressable
          onPress={() => zoomBy(+1)}
          style={({ pressed }) => [styles.zoomBtn, pressed && styles.zoomBtnPressed]}
          hitSlop={6}
          accessibilityLabel="Zoom in"
        >
          <Text style={styles.zoomLabel}>+</Text>
        </Pressable>
        <View style={styles.zoomDivider} />
        <Pressable
          onPress={() => zoomBy(-1)}
          style={({ pressed }) => [styles.zoomBtn, pressed && styles.zoomBtnPressed]}
          hitSlop={6}
          accessibilityLabel="Zoom out"
        >
          <Text style={styles.zoomLabel}>−</Text>
        </Pressable>
      </View>
    </View>
  );
}

const styles = StyleSheet.create({
  map: {
    flex: 1,
  },
  zoomWrap: {
    position: "absolute",
    right: 12,
    bottom: 240,            // sits above the timeline bar
    zIndex: 14,
    backgroundColor: "rgba(15,18,30,0.86)",
    borderRadius: 14,
    borderWidth: 1,
    borderColor: "rgba(255,255,255,0.08)",
    overflow: "hidden",
  },
  zoomBtn: {
    width: 44,
    height: 44,
    alignItems: "center",
    justifyContent: "center",
  },
  zoomBtnPressed: { backgroundColor: "rgba(255,255,255,0.08)" },
  zoomDivider: { height: StyleSheet.hairlineWidth, backgroundColor: "rgba(255,255,255,0.1)" },
  zoomLabel: {
    color: "#fff",
    fontSize: 22,
    fontWeight: "300",
    lineHeight: 24,
  },
});
