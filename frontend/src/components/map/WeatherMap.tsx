import {
  Camera,
  Map,
  type CameraRef,
  type MapRef,
  type PressEvent,
  type ViewStateChangeEvent,
} from "@maplibre/maplibre-react-native";
import { Children, isValidElement, useEffect, useMemo, useRef } from "react";
import { Pressable, StyleSheet, Text, View, type NativeSyntheticEvent } from "react-native";
import { useWeatherStore } from "../../stores/useWeatherStore";
import { DEFAULTS } from "../../lib/constants";
import { useBasemapStyle } from "../../hooks/useBasemapStyle";

const ZOOM_MIN = 1;
const ZOOM_MAX = 15;

interface WeatherMapProps {
  children?: React.ReactNode;
  onLongPress?: (lat: number, lon: number) => void;
  onCameraChanged?: (camera: { lon: number; lat: number; zoom: number }) => void;
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

  const { style: patchedStyle } = useBasemapStyle(serverUrl, mapStyle);

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
          accessibilityRole="button"
          accessibilityLabel="Zoom in"
        >
          <Text style={styles.zoomLabel}>+</Text>
        </Pressable>
        <View style={styles.zoomDivider} />
        <Pressable
          onPress={() => zoomBy(-1)}
          style={({ pressed }) => [styles.zoomBtn, pressed && styles.zoomBtnPressed]}
          hitSlop={6}
          accessibilityRole="button"
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
    minWidth: 44,
    minHeight: 44,
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
