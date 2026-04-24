import MapLibreGL, { type MapViewRef } from "@maplibre/maplibre-react-native";
import { Children, isValidElement, useEffect, useRef, useState } from "react";
import { StyleSheet } from "react-native";
import { useWeatherStore } from "../../stores/useWeatherStore";
import { DEFAULTS, resolveMapStyleUrl } from "../../lib/constants";

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
    fetch(styleUrl)
      .then((r) => r.json())
      .then((json: { sources?: Record<string, { tiles?: string[]; url?: string }> }) => {
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
      })
      .catch(() => {
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
  const mapStyle = useWeatherStore((s) => s.mapStyle);
  const serverUrl = useWeatherStore((s) => s.serverUrl);
  const latitude = useWeatherStore((s) => s.latitude);
  const longitude = useWeatherStore((s) => s.longitude);

  const patchedStyle = usePatchedMapStyle(serverUrl, mapStyle);

  const centerCoord: [number, number] = [
    longitude ?? DEFAULTS.LONGITUDE,
    latitude ?? DEFAULTS.LATITUDE,
  ];

  if (!patchedStyle) return null;

  return (
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
        if (!onCameraChanged) return;
        const coords = feature.geometry.coordinates as [number, number];
        onCameraChanged({
          lon: coords[0],
          lat: coords[1],
          zoom: feature.properties.zoomLevel,
        });
      }}
      onRegionDidChange={(feature) => {
        if (!onCameraChanged) return;
        const coords = feature.geometry.coordinates as [number, number];
        onCameraChanged({
          lon: coords[0],
          lat: coords[1],
          zoom: feature.properties.zoomLevel,
        });
      }}
    >
      <MapLibreGL.Camera
        defaultSettings={{
          centerCoordinate: centerCoord,
          zoomLevel: latitude ? 7 : DEFAULTS.ZOOM,
        }}
      />
      <MapLibreGL.UserLocation visible={true} />
      {Children.toArray(children).filter(isValidElement)}
    </MapLibreGL.MapView>
  );
}

const styles = StyleSheet.create({
  map: {
    flex: 1,
  },
});
