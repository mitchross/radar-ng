import {
  Camera,
  Map,
  type CameraRef,
  type MapRef,
  type PressEvent,
  type ViewStateChangeEvent,
} from "@maplibre/maplibre-react-native";
import { Children, isValidElement, useEffect, useMemo, useRef, useState } from "react";
import { Pressable, StyleSheet, Text, View, type NativeSyntheticEvent } from "react-native";
import { useWeatherStore } from "../../stores/useWeatherStore";
import { DEFAULTS, resolveMapStyleUrl } from "../../lib/constants";
import { trace } from "../../lib/telemetry";

const ZOOM_MIN = 1;
const ZOOM_MAX = 15;

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

    async function attempt(): Promise<string> {
      const r = await fetch(styleUrl);
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
      return JSON.stringify(json);
    }

    trace(
      "map.fetchStyle",
      async (span) => {
        // Cold-start race: on Android, the JS fetch can fire before the
        // emulator's network stack is fully up, throwing a DNS error
        // immediately. Retry with backoff so the basemap doesn't fall
        // back to MapLibre's native loader (which does NOT rewrite the
        // relative `/basemap/tiles/{z}/{x}/{y}.mvt` paths in the
        // Protomaps style → solid black map).
        const delays = [400, 800, 1600];
        let lastErr: unknown;
        for (let i = 0; i < delays.length + 1; i++) {
          if (cancelled) return;
          try {
            const result = await attempt();
            span.setAttribute("map.fetchStyle.attempts", i + 1);
            if (!cancelled) setPatched(result);
            return;
          } catch (err) {
            lastErr = err;
            if (i < delays.length) {
              await new Promise((res) => setTimeout(res, delays[i]));
            }
          }
        }
        throw lastErr;
      },
      { "map.style": mapStyle },
    ).catch(() => {
      // All retries failed → hand MapLibre the raw URL. It still won't
      // rewrite relative tile paths, but at least the user sees the
      // attribution + zoom controls instead of nothing.
      if (!cancelled) setPatched(styleUrl);
    });
    return () => {
      cancelled = true;
    };
  }, [serverUrl, mapStyle, styleUrl]);

  return patched;
}

export function WeatherMap({ children, onLongPress, onCameraChanged }: WeatherMapProps) {
  const mapRef = useRef<MapRef>(null);
  const cameraRef = useRef<CameraRef>(null);
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
    cameraRef.current?.setStop({ zoom: next, duration: 220 });
  }

  useEffect(() => {
    cameraRef.current?.setStop({
      center: centerCoord,
      zoom: initialZoom,
      duration: 0,
    });
    zoomRef.current = initialZoom;
  }, [centerCoord, initialZoom]);

  if (!patchedStyle) return null;

  const handleLongPress = (event: NativeSyntheticEvent<PressEvent>) => {
    const [lon, lat] = event.nativeEvent.lngLat;
    onLongPress?.(lat, lon);
  };

  const handleRegionChange = (
    event: NativeSyntheticEvent<ViewStateChangeEvent>,
  ) => {
    const { center, zoom } = event.nativeEvent;
    zoomRef.current = zoom;
    onCameraChanged?.({
      lon: center[0],
      lat: center[1],
      zoom,
    });
  };

  return (
    <View style={styles.map}>
      <Map
        ref={mapRef}
        style={styles.map}
        mapStyle={patchedStyle}
        logo={false}
        attribution={true}
        attributionPosition={{ bottom: 8, left: 8 }}
        onLongPress={handleLongPress}
        onRegionIsChanging={handleRegionChange}
        onRegionDidChange={handleRegionChange}
      >
        <Camera
          ref={cameraRef}
          initialViewState={{
            center: centerCoord,
            zoom: initialZoom,
          }}
        />
        {Children.toArray(children).filter(isValidElement)}
      </Map>

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
